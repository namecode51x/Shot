//! Hotkey specification: parsing and formatting of strings like
//! `PrintScreen`, `Ctrl+Shift+S`. Pure code (no OS calls) so it is unit
//! testable; `crate::win::hotkey` turns a spec into `RegisterHotKey` args.
//!
//! Modifier bits use the same numeric values as Win32 `MOD_*` so they pass
//! through unchanged (Win32: ALT=1, CONTROL=2, SHIFT=4, WIN=8).

pub const MOD_ALT: u8 = 0x01;
pub const MOD_CONTROL: u8 = 0x02;
pub const MOD_SHIFT: u8 = 0x04;
pub const MOD_WIN: u8 = 0x08;

/// Virtual-key codes used by NameShot (values are Win32 ABI constants).
pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const PAUSE: u16 = 0x13;
    pub const CAPITAL: u16 = 0x14;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const PRIOR: u16 = 0x21;
    pub const NEXT: u16 = 0x22;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const INSERT: u16 = 0x2D;
    pub const DELETE: u16 = 0x2E;
    pub const PRINTSCREEN: u16 = 0x2C;
    pub const NUMLOCK: u16 = 0x90;
    pub const SCROLL: u16 = 0x91;
    pub const F1: u16 = 0x70;
    pub const N0: u16 = 0x30;
    pub const A: u16 = 0x41;
    pub const OEM_3: u16 = 0xC0;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotkeySpec {
    pub modifiers: u8,
    pub vk: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyError {
    Empty,
    UnknownToken(String),
    ModifierOnly,
    BareKeyNotAllowed,
    NoKey,
}

impl HotkeyError {
    pub fn message(&self) -> &'static str {
        match self {
            HotkeyError::Empty => "Hotkey is empty.",
            HotkeyError::UnknownToken(_) => "Unknown key in hotkey.",
            HotkeyError::ModifierOnly => "A non-modifier key is required.",
            HotkeyError::BareKeyNotAllowed => {
                "That key needs at least one modifier (Ctrl/Alt/Shift/Win)."
            }
            HotkeyError::NoKey => "A key is required.",
        }
    }
}

impl HotkeySpec {
    pub const fn print_screen() -> Self {
        HotkeySpec {
            modifiers: 0,
            vk: vk::PRINTSCREEN,
        }
    }

    /// Keys allowed without modifiers (function/media-ish keys that no one
    /// types as input). Letters, digits, Space, Tab, Esc, arrows are rejected
    /// bare because registering them globally would break normal typing.
    pub fn validate(&self) -> Result<(), HotkeyError> {
        if self.vk == 0 {
            return Err(HotkeyError::NoKey);
        }
        if self.modifiers == 0 && !bare_key_allowed(self.vk) {
            return Err(HotkeyError::BareKeyNotAllowed);
        }
        Ok(())
    }

    pub fn format(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.modifiers & MOD_CONTROL != 0 {
            parts.push("Ctrl");
        }
        if self.modifiers & MOD_ALT != 0 {
            parts.push("Alt");
        }
        if self.modifiers & MOD_SHIFT != 0 {
            parts.push("Shift");
        }
        if self.modifiers & MOD_WIN != 0 {
            parts.push("Win");
        }
        let key = key_name(self.vk);
        if key.is_empty() {
            String::new()
        } else if parts.is_empty() {
            key.to_string()
        } else {
            format!("{}+{}", parts.join("+"), key)
        }
    }
}

pub fn bare_key_allowed(vk: u16) -> bool {
    matches!(
        vk,
        vk::PRINTSCREEN
            | vk::SCROLL
            | vk::PAUSE
            | vk::INSERT
            | vk::DELETE
            | vk::HOME
            | vk::END
            | vk::PRIOR
            | vk::NEXT
            | vk::NUMLOCK
            | 0x70..=0x7B // F1..F12
    )
}

/// Parse strings like `Ctrl+Alt+S`, `PrintScreen`, `shift+f5`.
pub fn parse(text: &str) -> Result<HotkeySpec, HotkeyError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(HotkeyError::Empty);
    }
    let mut modifiers = 0u8;
    let mut key: Option<u16> = None;
    let mut saw_any = false;
    for raw in trimmed.split('+') {
        let token = raw.trim();
        if token.is_empty() {
            return Err(HotkeyError::UnknownToken(raw.to_string()));
        }
        saw_any = true;
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "alt" | "menu" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "windows" | "super" => modifiers |= MOD_WIN,
            other => {
                let vk =
                    parse_key(other).ok_or_else(|| HotkeyError::UnknownToken(other.to_string()))?;
                if key.is_some() {
                    return Err(HotkeyError::UnknownToken(other.to_string()));
                }
                key = Some(vk);
            }
        }
    }
    if !saw_any {
        return Err(HotkeyError::Empty);
    }
    let vk = key.ok_or(HotkeyError::ModifierOnly)?;
    Ok(HotkeySpec { modifiers, vk })
}

fn parse_key(lower: &str) -> Option<u16> {
    if lower.len() == 1 {
        let c = lower.chars().next().unwrap();
        if c.is_ascii_alphanumeric() {
            return if c.is_ascii_alphabetic() {
                Some(vk::A + (c.to_ascii_uppercase() as u16 - b'A' as u16))
            } else {
                Some(vk::N0 + (c as u16 - b'0' as u16))
            };
        }
        return match c {
            '`' => Some(vk::OEM_3),
            '-' => Some(0xBD),
            '=' => Some(0xBB),
            '[' => Some(0xDB),
            ']' => Some(0xDD),
            '\\' => Some(0xDC),
            ';' => Some(0xBA),
            '\'' => Some(0xDE),
            ',' => Some(0xBC),
            '.' => Some(0xBE),
            '/' => Some(0xBF),
            _ => None,
        };
    }
    if let Some(rest) = lower.strip_prefix('f') {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=12).contains(&n) {
                return Some(vk::F1 + (n as u16 - 1));
            }
        }
    }
    match lower {
        "printscreen" | "print" | "prtsc" | "prtscn" => Some(vk::PRINTSCREEN),
        "esc" | "escape" => Some(vk::ESCAPE),
        "space" => Some(vk::SPACE),
        "tab" => Some(vk::TAB),
        "enter" | "return" => Some(vk::RETURN),
        "insert" | "ins" => Some(vk::INSERT),
        "delete" | "del" => Some(vk::DELETE),
        "home" => Some(vk::HOME),
        "end" => Some(vk::END),
        "pageup" | "pgup" => Some(vk::PRIOR),
        "pagedown" | "pgdn" | "pgdown" => Some(vk::NEXT),
        "left" => Some(vk::LEFT),
        "right" => Some(vk::RIGHT),
        "up" => Some(vk::UP),
        "down" => Some(vk::DOWN),
        "numlock" => Some(vk::NUMLOCK),
        "scrolllock" => Some(vk::SCROLL),
        "capslock" => Some(vk::CAPITAL),
        "pause" | "break" => Some(vk::PAUSE),
        "backquote" => Some(vk::OEM_3),
        _ => None,
    }
}

/// Canonical name for a virtual-key code ("" when unknown).
pub fn key_name(vk_code: u16) -> String {
    if (0x41..=0x5A).contains(&vk_code) {
        return ((b'A' + (vk_code - 0x41) as u8) as char).to_string();
    }
    if (0x30..=0x39).contains(&vk_code) {
        return ((b'0' + (vk_code - 0x30) as u8) as char).to_string();
    }
    if (0x70..=0x7B).contains(&vk_code) {
        return format!("F{}", vk_code - 0x6F);
    }
    let name = match vk_code {
        0x2C => "PrintScreen",
        0x1B => "Esc",
        0x20 => "Space",
        0x09 => "Tab",
        0x0D => "Enter",
        0x2D => "Insert",
        0x2E => "Delete",
        0x24 => "Home",
        0x23 => "End",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x90 => "NumLock",
        0x91 => "ScrollLock",
        0x14 => "CapsLock",
        0x13 => "Pause",
        0xC0 => "`",
        0xBD => "-",
        0xBB => "=",
        0xDB => "[",
        0xDD => "]",
        0xDC => "\\",
        0xBA => ";",
        0xDE => "'",
        0xBC => ",",
        0xBE => ".",
        0xBF => "/",
        _ => "",
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_print_screen() {
        let h = HotkeySpec::print_screen();
        assert_eq!(h.format(), "PrintScreen");
        assert!(h.validate().is_ok());
    }

    #[test]
    fn parses_modifier_combinations() {
        let h = parse("Ctrl+Shift+S").unwrap();
        assert_eq!(h.modifiers, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(h.vk, vk::A + 18);
        assert_eq!(h.format(), "Ctrl+Shift+S");

        let h = parse(" alt + F5 ").unwrap();
        assert_eq!(h.modifiers, MOD_ALT);
        assert_eq!(h.vk, vk::F1 + 4);
        assert_eq!(h.format(), "Alt+F5");

        let h = parse("Win+Ctrl+Alt+PrtSc").unwrap();
        assert_eq!(h.modifiers, MOD_CONTROL | MOD_ALT | MOD_WIN);
        assert_eq!(h.format(), "Ctrl+Alt+Win+PrintScreen");
    }

    #[test]
    fn case_insensitive_and_aliases() {
        assert_eq!(
            parse("control+shift+a").unwrap(),
            parse("Ctrl+Shift+A").unwrap()
        );
        assert_eq!(parse("PRINTSCREEN").unwrap(), HotkeySpec::print_screen());
        assert_eq!(parse("shift+pgdn").unwrap().vk, vk::NEXT);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(parse("").unwrap_err(), HotkeyError::Empty);
        assert_eq!(parse("   ").unwrap_err(), HotkeyError::Empty);
        assert_eq!(
            parse("Ctrl+").unwrap_err(),
            HotkeyError::UnknownToken(String::new())
        );
        assert_eq!(parse("Ctrl+Shift").unwrap_err(), HotkeyError::ModifierOnly);
        assert_eq!(
            parse("Ctrl+Alt+Q+R").unwrap_err(),
            HotkeyError::UnknownToken("r".into())
        );
        assert_eq!(
            parse("Boo").unwrap_err(),
            HotkeyError::UnknownToken("boo".into())
        );
    }

    #[test]
    fn bare_letter_keys_are_rejected_but_print_screen_allowed() {
        assert_eq!(
            parse("s").unwrap().validate().unwrap_err(),
            HotkeyError::BareKeyNotAllowed
        );
        assert_eq!(
            parse("space").unwrap().validate().unwrap_err(),
            HotkeyError::BareKeyNotAllowed
        );
        assert_eq!(
            parse("esc").unwrap().validate().unwrap_err(),
            HotkeyError::BareKeyNotAllowed
        );
        assert!(parse("ctrl+s").unwrap().validate().is_ok());
        assert!(parse("printscreen").unwrap().validate().is_ok());
        assert!(parse("f9").unwrap().validate().is_ok());
        assert!(parse("ctrl+f9").unwrap().validate().is_ok());
    }

    #[test]
    fn format_round_trips() {
        for text in [
            "Ctrl+Alt+S",
            "Ctrl+Shift+F12",
            "PrintScreen",
            "Win+PrintScreen",
        ] {
            let h = parse(text).unwrap();
            assert_eq!(h.format(), text, "round trip {text}");
            assert_eq!(parse(&h.format()).unwrap(), h);
        }
    }
}
