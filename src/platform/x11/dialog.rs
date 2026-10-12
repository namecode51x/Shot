//! Save / folder / message dialogs through the desktop's CLI dialog tool:
//! `zenity` first, `kdialog` as the KDE fallback (plan §4: evaluated against
//! `rfd` — the hand-rolled path adds no dependency, no GTK/Qt link and no
//! runtime cost). A machine with neither tool gets a clean, reportable
//! error; nothing is silently swallowed.
//!
//! Cancellation contract: the tool exits with code 1 → `Ok(None)`.
//! Output paths arrive trimmed on stdout; empty output counts as cancelled.

use crate::error::{Error, Result};
use crate::platform::x11_logic;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const NO_TOOL: &str =
    "no dialog tool found (install `zenity` or `kdialog` — NameShot does not link GTK/Qt)";

/// Run `zenity` with `args`; on `NotFound` fall back to the equivalent
/// `kdialog` invocation built by `kdialog_alt`.
fn run(args: &[String], kdialog_alt: impl FnOnce() -> Vec<String>) -> Result<Option<String>> {
    let out = match Command::new("zenity").args(args).output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let args = kdialog_alt();
            let out = Command::new("kdialog").args(&args).output().map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::msg(NO_TOOL)
                } else {
                    Error::msg(format!("kdialog: {e}"))
                }
            })?;
            return interpret(&out, "kdialog");
        }
        Err(e) => return Err(Error::msg(format!("zenity: {e}"))),
    };
    interpret(&out, "zenity")
}

/// Success → trimmed stdout (`None` when empty = cancelled), exit code 1 →
/// cancelled, anything else → error.
fn interpret(out: &Output, tool: &str) -> Result<Option<String>> {
    if out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return Ok(if text.is_empty() { None } else { Some(text) });
    }
    if out.status.code() == Some(1) {
        return Ok(None); // user pressed Cancel
    }
    Err(Error::msg(format!(
        "{tool} exited with {}",
        out.status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "a signal".into())
    )))
}

/// "Save as" file picker. `Ok(None)` = cancelled.
pub fn save_as(title: &str, default: &Path) -> Result<Option<PathBuf>> {
    let zargs = x11_logic::zenity_save_args(title, default);
    let path = run(&zargs, || {
        vec![
            "--getsavefilename".into(),
            default.display().to_string(),
            format!("--title={title}"),
        ]
    })?;
    Ok(path.map(PathBuf::from))
}

/// Folder picker. `Ok(None)` = cancelled.
pub fn folder_browse(title: &str, default: &Path) -> Result<Option<PathBuf>> {
    let zargs = x11_logic::zenity_dir_args(title, default);
    let path = run(&zargs, || {
        vec![
            "--getexistingdirectory".into(),
            default.display().to_string(),
            format!("--title={title}"),
        ]
    })?;
    Ok(path.map(PathBuf::from))
}

/// Blocking message box (`error` selects the dialog icon/severity).
pub fn msgbox(title: &str, text: &str, error: bool) -> Result<()> {
    let zargs = x11_logic::zenity_msg_args(title, text, error);
    let got = run(&zargs, || {
        if error {
            vec!["--error".into(), text.into(), format!("--title={title}")]
        } else {
            vec!["--msgbox".into(), text.into(), format!("--title={title}")]
        }
    })?;
    // Message boxes have no meaningful stdout; a tool that printed one and
    // exited 0 is still success.
    let _ = got;
    Ok(())
}
