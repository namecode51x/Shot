//! Global hotkey registration (thin wrapper over `RegisterHotKey`).
//! Registration happens **once** at startup and on explicit settings changes —
//! never in a polling loop. Conflicts are reported, not fought over.

use crate::error::{Error, Result};
use crate::hotkey::HotkeySpec;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};

pub const HOTKEY_ID: i32 = 0x4E53; // 'NS'

/// `ERROR_HOTKEY_ALREADY_REGISTERED` as HRESULT_FROM_WIN32.
const HRESULT_HOTKEY_TAKEN: i32 = 0x8007_0000u32 as i32 | 1409;

pub fn register(hwnd: HWND, spec: &HotkeySpec) -> Result<()> {
    let mods = HOT_KEY_MODIFIERS(spec.modifiers as u32 | MOD_NOREPEAT.0);
    unsafe { RegisterHotKey(Some(hwnd), HOTKEY_ID, mods, spec.vk as u32) }
        .map_err(|e| Error::Win32(e).with_context("RegisterHotKey"))
}

pub fn unregister(hwnd: HWND) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
    }
}

/// True when the failure means "someone else owns this key".
pub fn is_conflict(err: &Error) -> bool {
    match err {
        Error::Win32(e) => e.code().0 == HRESULT_HOTKEY_TAKEN,
        Error::Msg(m) => m.contains("RegisterHotKey"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::HRESULT;

    #[test]
    fn conflict_detection_on_wrapped_errors() {
        let e = Error::Win32(windows::core::Error::from_hresult(HRESULT(
            HRESULT_HOTKEY_TAKEN,
        )));
        assert!(is_conflict(&e));
        let other = Error::Win32(windows::core::Error::from_hresult(HRESULT(
            0x8007_0005u32 as i32,
        )));
        assert!(!is_conflict(&other));
        assert!(!is_conflict(&Error::invalid("nope")));
    }
}
