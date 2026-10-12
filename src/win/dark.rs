//! Dark visual theme: title bars, control colors, window brushes.
//! Palette values are owned by [`crate::theme`]; this module re-exports them
//! under their historic names and holds the Win32 helpers (brush, DWM,
//! control theming). Restrained near-black surfaces, high-contrast text, one
//! accent color, no gradients or ornamental animation.

use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, HBRUSH};
use windows::Win32::UI::Controls::SetWindowTheme;
use windows::core::{PCWSTR, w};

use crate::theme;

/// Background (near-black), COLORREF 0x00BBGGRR.
pub const BG: u32 = theme::BG_U32;
/// Slightly lighter panel (status bar, hint boxes).
pub const BG_ALT: u32 = theme::ELEVATED_U32;
/// Canvas behind the image.
pub const CANVAS: u32 = theme::CANVAS_U32;
/// Primary text (near-white).
pub const FG: u32 = theme::FG_U32;
/// Dimmed text / hints.
pub const FG_DIM: u32 = theme::FG_DIM_U32;
/// Selected/hover highlight for owner-drawn items.
pub const SEL_BG: u32 = theme::ELEVATED_U32;

/// The single restrained accent color (cool blue).
pub use crate::theme::ACCENT as ACCENT_RGB;
pub use crate::theme::ERROR as ERROR_RGB;
/// Selection border drawn over arbitrary screen content: near-black outline
/// under the accent line keeps contrast on both light and dark wallpapers.
pub use crate::theme::SEL_OUTLINE as SEL_OUTLINE_RGB;

pub fn colorref(rgb: (u8, u8, u8)) -> COLORREF {
    COLORREF(theme::cref(rgb))
}

pub fn colorref_u32(v: u32) -> COLORREF {
    COLORREF(v)
}

/// Process-lifetime solid brush for window class backgrounds (Settings,
/// About, hotkey capture, legacy editor, overlay class fallbacks — all use
/// the theme `SURFACE`).
/// Leaked on purpose: window classes hold the handle for the process lifetime.
/// The static stores the raw handle as `usize` because `HBRUSH` is a raw
/// pointer and therefore not `Sync`.
pub fn bg_brush() -> HBRUSH {
    use std::sync::OnceLock;
    static BRUSH: OnceLock<usize> = OnceLock::new();
    let raw = *BRUSH
        .get_or_init(|| unsafe { CreateSolidBrush(colorref_u32(theme::SURFACE_U32)) }.0 as usize);
    HBRUSH(raw as *mut core::ffi::c_void)
}

/// Ask DWM for a dark title bar (attribute 20 on Win10 2004+, 19 earlier).
pub fn set_dark_titlebar(hwnd: HWND, dark: bool) {
    unsafe {
        let value: u32 = dark as u32;
        for attr in [DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE(19)] {
            if DwmSetWindowAttribute(
                hwnd,
                attr,
                &value as *const u32 as *const _,
                std::mem::size_of::<u32>() as u32,
            )
            .is_ok()
            {
                return;
            }
        }
    }
}

/// Opt a control into dark visual styles where the OS supports it.
pub fn dark_control(hwnd: HWND) {
    unsafe {
        let _ = SetWindowTheme(hwnd, w!("DarkMode_Explorer"), PCWSTR(std::ptr::null()));
    }
}

/// Restore the default (light) theme for a control.
pub fn reset_control_theme(hwnd: HWND) {
    unsafe {
        let _ = SetWindowTheme(hwnd, PCWSTR(std::ptr::null()), PCWSTR(std::ptr::null()));
    }
}
