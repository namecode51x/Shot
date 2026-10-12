//! Windows integration layer. Everything that talks to Win32 lives here or in
//! a sibling module (`capture`, `overlay`, `editor`, `encode`).

pub mod a11y;
pub mod clipboard;
pub mod dark;
pub mod dialog;
pub mod gdi;
pub mod hotkey;
pub mod tray;

use std::sync::OnceLock;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON,
    SM_CYICON, SM_CYSMICON, WM_APP,
};
use windows::core::PCWSTR;

/// `MAKEINTRESOURCE(1)`: the icon resource id embedded by `build.rs` from
/// `assets/icon.ico`.
// The explicit cast is intentional: clippy's suggested replacement
// `std::ptr::dangling::<u16>()` equals align-of(u16) = 2, which would request
// resource id 2 instead of id 1.
#[allow(clippy::manual_dangling_ptr)]
fn icon_resource() -> PCWSTR {
    PCWSTR(1usize as *const u16)
}

/// The application icon loaded from the executable's embedded icon resource
/// (id 1, embedded by `build.rs` from `assets/icon.ico`, generated from
/// `assets/icon.png`).
///
/// `small` selects the title-bar/tray size (`SM_CXSMICON`), otherwise the
/// taskbar/Alt-Tab size (`SM_CXICON`). The handle is cached for the process
/// lifetime because window classes keep their class icons alive as long as
/// the class exists (it must never be destroyed by callers). Returns
/// `HICON::default()` when the resource is missing, so a build without
/// windres still runs with the system fallback icon.
pub fn app_icon(small: bool) -> HICON {
    static BIG: OnceLock<isize> = OnceLock::new();
    static SMALL: OnceLock<isize> = OnceLock::new();
    let cached = *if small { &SMALL } else { &BIG }.get_or_init(|| unsafe {
        let (cx, cy) = if small {
            (SM_CXSMICON, SM_CYSMICON)
        } else {
            (SM_CXICON, SM_CYICON)
        };
        let w = GetSystemMetrics(cx).max(16);
        let h = GetSystemMetrics(cy).max(16);
        LoadImageW(
            Some(gdi::this_module()),
            icon_resource(),
            IMAGE_ICON,
            w,
            h,
            LR_DEFAULTCOLOR,
        )
        .map(|handle| handle.0 as isize)
        .unwrap_or(0)
    });
    HICON(cached as *mut core::ffi::c_void)
}

/// Load the application icon once at the requested pixel size as an owned
/// handle the caller must `DestroyIcon`. Used by the tray (which owns and
/// frees its icon); window classes use [`app_icon`] instead.
pub fn owned_app_icon(size: i32) -> windows::core::Result<HICON> {
    unsafe {
        let handle = LoadImageW(
            Some(gdi::this_module()),
            icon_resource(),
            IMAGE_ICON,
            size.max(16),
            size.max(16),
            LR_DEFAULTCOLOR,
        )?;
        Ok(HICON(handle.0))
    }
}

/// Tray icon callback: `lParam` holds the mouse message.
pub const WM_APP_TRAY: u32 = WM_APP + 1;
/// An editor window was destroyed; `wParam` is its `HWND` as `usize`.
pub const WM_APP_EDITOR_CLOSED: u32 = WM_APP + 2;
/// The overlay was destroyed without a selection; `wParam` is its `HWND`.
pub const WM_APP_OVERLAY_CLOSED: u32 = WM_APP + 3;
/// `wParam` = HWND of the window that posted a status string (text lives in
/// the app state; this message only signals "repaint the status area").
pub const WM_APP_STATUS: u32 = WM_APP + 4;

/// NUL-terminated UTF-16 copy of `s`.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copy `s` into a fixed-size wide buffer, truncating and always
/// NUL-terminating (`buf.len() - 1` chars max).
pub fn copy_into(buf: &mut [u16], s: &str) {
    if buf.is_empty() {
        return;
    }
    let mut n = 0;
    for c in s.encode_utf16() {
        if n + 1 >= buf.len() {
            break;
        }
        buf[n] = c;
        n += 1;
    }
    for b in buf.iter_mut().skip(n) {
        *b = 0;
    }
}

/// Read a NUL-terminated wide string from a raw pointer.
///
/// # Safety
/// `ptr` must be valid and NUL-terminated for the lifetime of the read.
pub unsafe fn from_wide(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe {
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
            if len > 1 << 20 {
                break;
            }
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}
