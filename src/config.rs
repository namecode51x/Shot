//! Versioned local settings file: a minimal INI in `%APPDATA%\NameShot\settings.ini`.
//!
//! Design goals: zero dependencies, human readable, tolerant parsing (invalid
//! or missing values fall back to safe defaults), and it **never** stores
//! screenshot pixels or any captured content.

use crate::hotkey::{self, HotkeySpec};
use std::fmt;
use std::path::{Path, PathBuf};

pub const CURRENT_VERSION: u32 = 1;
pub const APP_DIR: &str = "NameShot";
pub const SETTINGS_FILE: &str = "settings.ini";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Png,
    Jpeg,
    Bmp,
}

impl OutputFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputFormat::Png => "png",
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Bmp => "bmp",
        }
    }

    pub fn from_str_lossy(s: &str) -> Option<OutputFormat> {
        match s.trim().to_ascii_lowercase().as_str() {
            "png" => Some(OutputFormat::Png),
            "jpg" | "jpeg" => Some(OutputFormat::Jpeg),
            "bmp" => Some(OutputFormat::Bmp),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            OutputFormat::Png => "PNG",
            OutputFormat::Jpeg => "JPEG",
            OutputFormat::Bmp => "BMP",
        }
    }

    pub fn extension(&self) -> &'static str {
        self.as_str()
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub version: u32,
    /// Capture hotkey; default `PrintScreen`.
    pub hotkey: HotkeySpec,
    /// Default Save-As format; default PNG.
    pub format: OutputFormat,
    /// Default save folder; empty = system Pictures folder.
    pub save_dir: String,
    /// Opt-in "Quick save" tray action (never overwrites, off by default).
    pub quick_save: bool,
    /// Opt-in start-at-sign-in (writes one HKCU Run value; off by default).
    pub launch_at_login: bool,
    /// JPEG quality 1..=100 (ignored for PNG/BMP).
    pub jpeg_quality: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: CURRENT_VERSION,
            hotkey: HotkeySpec::print_screen(),
            format: OutputFormat::Png,
            save_dir: String::new(),
            quick_save: false,
            launch_at_login: false,
            jpeg_quality: 90,
        }
    }
}

/// Parse INI text. Unknown keys are ignored, invalid values fall back to the
/// default from [`Settings::default`]. A missing or different `version` means
/// the file is from another release: the whole file is ignored (safe default).
pub fn parse(text: &str) -> Settings {
    let mut out = Settings::default();
    out.version = 0; // 0 = "not seen yet"; only set when version=1 is found

    let mut version_seen: Option<u32> = None;
    let mut values: Vec<(String, String)> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with(';')
            || line.starts_with('[')
        {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            values.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }

    for (k, v) in &values {
        if k == "version" {
            version_seen = v.parse::<u32>().ok();
        }
    }

    let Some(1) = version_seen else {
        return Settings::default();
    };

    for (k, v) in values {
        match k.as_str() {
            "version" => {}
            "hotkey" => {
                if let Ok(spec) = hotkey::parse(&v) {
                    if spec.validate().is_ok() {
                        out.hotkey = spec;
                    }
                }
            }
            "format" => {
                if let Some(f) = OutputFormat::from_str_lossy(&v) {
                    out.format = f;
                }
            }
            "save_dir" => {
                // Never allow control characters in a path taken from the file.
                if v.chars().all(|c| !c.is_control()) && v.len() < 1024 {
                    out.save_dir = v;
                }
            }
            "quick_save" => out.quick_save = parse_bool(&v).unwrap_or(false),
            "launch_at_login" => out.launch_at_login = parse_bool(&v).unwrap_or(false),
            "jpeg_quality" => {
                if let Ok(q) = v.parse::<u8>() {
                    if (1..=100).contains(&q) {
                        out.jpeg_quality = q;
                    }
                }
            }
            _ => {}
        }
    }

    out.version = CURRENT_VERSION;
    out
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Serialize to stable INI text (same key order every time).
pub fn serialize(s: &Settings) -> String {
    let mut out = String::with_capacity(256);
    out.push_str("# NameShot settings. Local file; contains no screenshot data.\n");
    out.push_str("[settings]\n");
    out.push_str(&format!("version={}\n", CURRENT_VERSION));
    out.push_str(&format!("hotkey={}\n", s.hotkey.format()));
    out.push_str(&format!("format={}\n", s.format.as_str()));
    out.push_str(&format!("save_dir={}\n", sanitize_for_ini(&s.save_dir)));
    out.push_str(&format!("quick_save={}\n", bool_str(s.quick_save)));
    out.push_str(&format!(
        "launch_at_login={}\n",
        bool_str(s.launch_at_login)
    ));
    out.push_str(&format!("jpeg_quality={}\n", s.jpeg_quality.clamp(1, 100)));
    out
}

fn bool_str(v: bool) -> &'static str {
    if v { "1" } else { "0" }
}

fn sanitize_for_ini(v: &str) -> String {
    v.chars().filter(|c| !c.is_control()).collect()
}

/// `%APPDATA%\NameShot` (falls back to the user profile when APPDATA is unset).
/// Windows behaviour is unchanged from the original implementation.
#[cfg(windows)]
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|p| PathBuf::from(p).join("AppData").join("Roaming"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(APP_DIR)
}

/// `$XDG_CONFIG_HOME/NameShot` → `$HOME/.config/NameShot` → `./NameShot`
/// (the XDG base-directory spec; M1's `paths` adapter for Linux).
#[cfg(not(windows))]
pub fn config_dir() -> PathBuf {
    config_dir_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// XDG resolution, pure and unit-tested: `xdg_config_home` counts only when
/// absolute (per spec), `home` provides the `.config` fallback, anything else
/// lands in the working directory — never a panic, never a root path.
/// Compiled for tests and non-Windows targets only, so the Windows release
/// binary stays byte-identical to M0.
#[cfg(any(test, not(windows)))]
pub fn config_dir_from(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> PathBuf {
    let base = xdg_config_home
        .map(PathBuf::from)
        // `has_root` (not just `is_absolute`) so the pure rule also holds on
        // a Windows test host, where "/xdg" has a root but no drive prefix.
        .filter(|p| p.is_absolute() || p.has_root())
        .or_else(|| home.map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(APP_DIR)
}

pub fn settings_path() -> PathBuf {
    config_dir().join(SETTINGS_FILE)
}

/// Load settings; a missing file yields defaults, an unreadable file yields
/// defaults plus the error so the caller can decide to surface it.
pub fn load(path: &Path) -> Result<Settings, std::io::Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(parse(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e),
    }
}

/// Save settings atomically: write `settings.ini.tmp`, then rename over the
/// target (no partial file if the process dies mid-write).
pub fn save(path: &Path, settings: &Settings) -> Result<(), std::io::Error> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("ini.tmp");
    std::fs::write(&tmp, serialize(settings))?;
    // Windows `rename` fails when the destination exists; replace explicitly.
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp, path)
        }
    }
}

/// Load-or-default with no I/O errors propagating (used at startup).
pub fn load_or_default(path: &Path) -> Settings {
    load(path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_privacy_safe_defaults() {
        let s = Settings::default();
        assert_eq!(s.hotkey, HotkeySpec::print_screen());
        assert_eq!(s.format, OutputFormat::Png);
        assert!(!s.quick_save);
        assert!(!s.launch_at_login);
        assert_eq!(s.jpeg_quality, 90);
        assert!(s.save_dir.is_empty());
        assert_eq!(s.version, CURRENT_VERSION);
    }

    #[test]
    fn round_trip() {
        let mut s = Settings::default();
        s.hotkey = hotkey::parse("Ctrl+Alt+F6").unwrap();
        s.format = OutputFormat::Jpeg;
        s.save_dir = r"D:\Screens\shots".to_string();
        s.quick_save = true;
        s.launch_at_login = false;
        s.jpeg_quality = 77;
        let text = serialize(&s);
        assert_eq!(parse(&text), s);
    }

    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let text = "\
version=1\n\
hotkey=NotAKey\n\
format=tiff\n\
save_dir=\n\
quick_save=banana\n\
jpeg_quality=999\n";
        let s = parse(text);
        assert_eq!(s.hotkey, HotkeySpec::print_screen());
        assert_eq!(s.format, OutputFormat::Png);
        assert!(!s.quick_save);
        assert_eq!(s.jpeg_quality, 90);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let text = "version=1\nunknown_key=42\ntelemetry=on\nhotkey=Ctrl+Shift+K\n";
        let s = parse(text);
        assert_eq!(s.hotkey, hotkey::parse("Ctrl+Shift+K").unwrap());
        assert!(!s.launch_at_login);
    }

    #[test]
    fn foreign_version_is_rejected_to_defaults() {
        let text = "version=99\nhotkey=Ctrl+Shift+K\nformat=bmp\n";
        let s = parse(text);
        assert_eq!(s, Settings::default());
        // missing version line entirely
        let s2 = parse("hotkey=Ctrl+Shift+K\n");
        assert_eq!(s2, Settings::default());
    }

    #[test]
    fn rejected_hotkey_in_file_uses_default() {
        // Bare "s" would break typing everywhere.
        let s = parse("version=1\nhotkey=s\n");
        assert_eq!(s.hotkey, HotkeySpec::print_screen());
    }

    #[test]
    fn save_and_load_file() {
        let dir = std::env::temp_dir().join(format!("nameshot-test-{}", std::process::id()));
        let path = dir.join("settings.ini");
        let _ = std::fs::remove_file(&path);

        // Missing file -> defaults
        let s = load(&path).unwrap();
        assert_eq!(s, Settings::default());

        let mut want = Settings::default();
        want.format = OutputFormat::Bmp;
        want.quick_save = true;
        want.save_dir = "C:\\Users\\Public\\Pictures".to_string();
        save(&path, &want).unwrap();
        let got = load(&path).unwrap();
        assert_eq!(got, want);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serialize_has_no_control_chars_in_values() {
        let mut s = Settings::default();
        s.save_dir = "bad\npath\u{0}x".to_string();
        let text = serialize(&s);
        assert!(!text.contains('\u{0}'));
        let loaded = parse(&text);
        assert!(!loaded.save_dir.contains('\n'));
    }

    #[test]
    fn xdg_config_dir_resolution() {
        use std::ffi::OsString;
        // Absolute XDG_CONFIG_HOME wins (spec requirement).
        assert_eq!(
            config_dir_from(
                Some(OsString::from("/xdg")),
                Some(OsString::from("/home/u"))
            ),
            PathBuf::from("/xdg/NameShot")
        );
        // Relative XDG path is ignored per spec → $HOME/.config.
        assert_eq!(
            config_dir_from(
                Some(OsString::from("relative/x")),
                Some(OsString::from("/home/u"))
            ),
            PathBuf::from("/home/u/.config/NameShot")
        );
        // No XDG → $HOME/.config.
        assert_eq!(
            config_dir_from(None, Some(OsString::from("/home/u"))),
            PathBuf::from("/home/u/.config/NameShot")
        );
        // Nothing set → working-directory fallback (never panics).
        assert_eq!(config_dir_from(None, None), PathBuf::from("./NameShot"));
        // App dir is always appended exactly once.
        let p = config_dir_from(Some(OsString::from("/xdg")), None);
        assert_eq!(p, PathBuf::from("/xdg/NameShot"));
        assert_eq!(p.file_name().unwrap(), "NameShot");
    }
}
