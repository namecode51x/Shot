//! Accessibility: give controls an accessible name when their window text is
//! empty (an edit that shows a value only after typing, a combo that shows the
//! selection). Best-effort — a failure here never blocks the UI.
//!
//! Uses the Accessibility property-services API (`IAccPropServices`) to set
//! the `PROPID_ACC_NAME` property on both object ids; MSAA clients and screen
//! readers read it as the element's accessible name.

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Accessibility::{CLSID_AccPropServices, IAccPropServices, PROPID_ACC_NAME};
use windows::Win32::UI::WindowsAndMessaging::{OBJID_CLIENT, OBJID_WINDOW};
use windows::core::PCWSTR;

/// Expose `name` as the accessible name of `hwnd`.
pub fn set_name(hwnd: HWND, name: &str) {
    let svc: std::result::Result<IAccPropServices, _> =
        unsafe { CoCreateInstance(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) };
    let Ok(svc) = svc else { return };
    let w = crate::win::wide(name);
    for objid in [OBJID_CLIENT.0 as u32, OBJID_WINDOW.0 as u32] {
        let _ = unsafe { svc.SetHwndPropStr(hwnd, objid, 0, PROPID_ACC_NAME, PCWSTR(w.as_ptr())) };
    }
}
