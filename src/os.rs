//! OS-specific helpers: version gate, DPI awareness, virtual-screen metrics,
//! monitor work areas, local time. Keeps Win32 details out of the app logic.

use crate::error::{Error, Result};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::SystemInformation::{GetLocalTime, OSVERSIONINFOW};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};
use windows::core::{PCWSTR, s, w};

/// Minimum supported OS build: Windows 10 RTM.
pub const MIN_BUILD: u32 = 10240;

#[derive(Debug, Clone, Copy)]
pub struct VirtualScreen {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl VirtualScreen {
    pub fn is_valid(&self) -> bool {
        self.w > 0 && self.h > 0
    }
}

/// Windows build number via `RtlGetVersion` (unlike `GetVersionEx` it is not
/// affected by application manifests). `None` when the call fails.
pub fn os_build_number() -> Option<u32> {
    unsafe {
        let ntdll = GetModuleHandleW(w!("ntdll.dll")).ok()?;
        let proc = GetProcAddress(ntdll, s!("RtlGetVersion"))?;
        let func: unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32 = std::mem::transmute(proc);
        let mut info = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..std::mem::zeroed()
        };
        if func(&mut info) == 0 {
            Some(info.dwBuildNumber)
        } else {
            None
        }
    }
}

/// Returns the OS name when the platform is unsupported, `None` when OK.
pub fn unsupported_os_message() -> Option<String> {
    match os_build_number() {
        Some(build) if build >= MIN_BUILD => None,
        Some(build) => Some(crate::strings::fill(
            &crate::strings::s().os_unsupported_msg,
            "BUILD",
            &build.to_string(),
        )),
        // Could not determine: proceed, but the app is Windows-10+ by design.
        None => None,
    }
}

/// Opt in to per-monitor-v2 DPI before any window exists, so every window and
/// mouse coordinate is in physical pixels that map 1:1 onto the captured frame.
pub fn enable_process_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn virtual_screen() -> VirtualScreen {
    unsafe {
        VirtualScreen {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            w: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            h: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

/// Work area of the monitor that contains most of `rect` (clamped fallback:
/// primary monitor work area).
pub fn work_area_for(rect: &RECT) -> RECT {
    unsafe {
        let mon: HMONITOR = MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..std::mem::zeroed()
        };
        if GetMonitorInfoW(mon, &mut mi).as_bool() && (mi.rcWork.right > mi.rcWork.left) {
            mi.rcWork
        } else {
            RECT {
                left: 0,
                top: 0,
                right: 1024,
                bottom: 768,
            }
        }
    }
}

/// Local wall-clock time as `(year, month, day, hour, minute, second)`.
pub fn local_time() -> (i32, u32, u32, u32, u32, u32) {
    let st = unsafe { GetLocalTime() };
    (
        st.wYear as i32,
        st.wMonth as u32,
        st.wDay as u32,
        st.wHour as u32,
        st.wMinute as u32,
        st.wSecond as u32,
    )
}

/// `CreateWindowEx`-style failure with the API name attached.
pub fn last_error(api: &'static str) -> Error {
    Error::Win32(windows::core::Error::from_thread()).with_context(api)
}

/// Unused-parameter guard: keeps `PCWSTR` import alive for callers that build
/// strings via `w!`.
pub fn _pcwstr(p: PCWSTR) -> PCWSTR {
    p
}

pub fn is_window_visible(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

/// Enable/disable "start NameShot when I sign in" by writing one HKCU `Run`
/// value. Disabling removes the value; a missing value is not an error.
pub fn set_launch_at_login(enabled: bool) -> Result<()> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPEN_CREATE_OPTIONS, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
    };
    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const VALUE_NAME: &str = "NameShot";
    unsafe {
        let subkey = crate::win::wide(RUN_KEY);
        let mut hkey = std::mem::zeroed::<HKEY>();
        let err = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            PCWSTR(std::ptr::null()),
            REG_OPEN_CREATE_OPTIONS(0),
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );
        if err.0 != 0 {
            return Err(Error::msg(format!(
                "could not open the Run registry key (error {})",
                err.0
            )));
        }
        let name = crate::win::wide(VALUE_NAME);
        let result = if enabled {
            match std::env::current_exe() {
                Ok(exe) => {
                    let value = format!("\"{}\"", exe.to_string_lossy());
                    let mut w: Vec<u16> = value.encode_utf16().collect();
                    w.push(0);
                    let bytes: Vec<u8> = w.iter().flat_map(|c| c.to_le_bytes()).collect();
                    let err =
                        RegSetValueExW(hkey, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&bytes));
                    if err.0 == 0 {
                        Ok(())
                    } else {
                        Err(Error::msg(format!(
                            "could not write the Run value (error {})",
                            err.0
                        )))
                    }
                }
                Err(e) => Err(Error::io("current_exe", e)),
            }
        } else {
            let err = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
            // 0 = deleted, 2 = ERROR_FILE_NOT_FOUND (already disabled).
            if err.0 == 0 || err.0 == 2 {
                Ok(())
            } else {
                Err(Error::msg(format!(
                    "could not remove the Run value (error {})",
                    err.0
                )))
            }
        };
        let _ = RegCloseKey(hkey);
        result
    }
}
