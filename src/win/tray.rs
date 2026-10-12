//! Notification-area (tray) icon: the app's embedded icon (with a
//! programmatic fallback when the resource is missing), context menu,
//! balloons, and re-add support after an Explorer restart.

use crate::error::{Error, Result};
use crate::strings::s;
use crate::win::{WM_APP_TRAY, copy_into};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    GetSystemMetrics, HCURSOR, HICON, ICONINFO, IDC_ARROW, LoadCursorW, MF_SEPARATOR, MF_STRING,
    SM_CXSMICON, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
};
use windows::core::{BOOL, PCWSTR};

/// Menu command ids returned by [`show_menu`].
pub const ID_CAPTURE: usize = 1001;
pub const ID_QUICK_SAVE: usize = 1002;
pub const ID_SETTINGS: usize = 1003;
pub const ID_ABOUT: usize = 1004;
pub const ID_EXIT: usize = 1005;

const NID_SIZE: u32 = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
const ICON_ID: u32 = 1;

/// The tray icon owned by the main window. Removes itself on drop.
pub struct TrayIcon {
    hwnd: HWND,
    icon: HICON,
    added: bool,
}

impl TrayIcon {
    /// Add the icon. The tooltip is set in the same call.
    pub fn add(owner: HWND) -> Result<TrayIcon> {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        let icon = match crate::win::owned_app_icon(size) {
            Ok(icon) => icon,
            Err(_) => draw_icon()?,
        };
        let mut tray = TrayIcon {
            hwnd: owner,
            icon,
            added: false,
        };
        if let Err(e) = tray.shell(NIM_ADD) {
            unsafe {
                let _ = DestroyIcon(icon);
            }
            return Err(e);
        }
        tray.added = true;
        Ok(tray)
    }

    /// Re-add after Explorer restarted (the `TaskbarCreated` message).
    ///
    /// A restarted Explorer has rebuilt the notification area, so the icon
    /// must be registered again with `NIM_ADD` carrying the complete
    /// payload (callback, icon, tooltip) — a `NIM_MODIFY` alone cannot
    /// resurrect it. If `NIM_ADD` fails the icon may already be present
    /// (duplicate broadcast); a successful `NIM_MODIFY` then refreshes it
    /// in place. If both fail the icon is recorded as unregistered so the
    /// next broadcast retries `NIM_ADD` instead of issuing a no-op
    /// `NIM_MODIFY` forever. Never touches Explorer itself.
    pub fn readd(&mut self) {
        let add_ok = self.shell(NIM_ADD).is_ok();
        let modify_ok = if add_ok {
            false
        } else {
            self.shell(NIM_MODIFY).is_ok()
        };
        self.added = recovery_state(add_ok, modify_ok);
    }

    pub fn remove(&mut self) {
        if self.added {
            let _ = self.shell(NIM_DELETE);
            self.added = false;
        }
    }

    /// Show a balloon (best effort — never fails the calling action).
    pub fn balloon(&self, title: &str, text: &str, warning: bool) {
        unsafe {
            let mut n: NOTIFYICONDATAW = std::mem::zeroed();
            n.cbSize = NID_SIZE;
            n.hWnd = self.hwnd;
            n.uID = ICON_ID;
            n.uFlags = NIF_INFO;
            copy_into(&mut n.szInfoTitle, title);
            copy_into(&mut n.szInfo, text);
            n.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
            let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
        }
    }

    /// The complete notify-icon payload every operation sends: handle, id,
    /// callback, icon, and tooltip in one place, so `NIM_ADD` and
    /// `NIM_MODIFY` can never drift apart.
    fn nid(&self) -> NOTIFYICONDATAW {
        let mut n: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        n.cbSize = NID_SIZE;
        n.hWnd = self.hwnd;
        n.uID = ICON_ID;
        n.hIcon = self.icon;
        n.uCallbackMessage = WM_APP_TRAY;
        n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        copy_into(&mut n.szTip, s().tray_tooltip);
        n
    }

    fn shell(&self, op: NOTIFY_ICON_MESSAGE) -> Result<()> {
        unsafe {
            let n = self.nid();
            if !Shell_NotifyIconW(op, &n).as_bool() {
                return Err(Error::Win32(windows::core::Error::from_thread())
                    .with_context("Shell_NotifyIconW"));
            }
            Ok(())
        }
    }
}

/// Pure recovery decision for [`TrayIcon::readd`]: the icon counts as
/// registered when either the fresh `NIM_ADD` succeeded or — the add was
/// rejected because the entry already exists — a `NIM_MODIFY` proved it is
/// present. Only when both fail is the icon gone, and `added` must flip to
/// `false` so the next `TaskbarCreated` retries `NIM_ADD`.
fn recovery_state(add_ok: bool, modify_ok: bool) -> bool {
    add_ok || modify_ok
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        self.remove();
        unsafe {
            let _ = DestroyIcon(self.icon);
        }
    }
}

pub enum MenuCmd {
    Capture,
    QuickSave,
    Settings,
    About,
    Exit,
    None,
}

/// Show the tray context menu at the cursor and return the chosen command.
pub fn show_menu(owner: HWND, quick_save: bool) -> MenuCmd {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return MenuCmd::None,
        };
        let st = s();
        let capture = crate::win::wide(st.tray_capture);
        let quick = crate::win::wide("Quick save");
        let settings = crate::win::wide(st.tray_settings);
        let about = crate::win::wide(st.tray_about);
        let exit = crate::win::wide(st.tray_exit);
        let _ = AppendMenuW(menu, MF_STRING, ID_CAPTURE, PCWSTR(capture.as_ptr()));
        if quick_save {
            let _ = AppendMenuW(menu, MF_STRING, ID_QUICK_SAVE, PCWSTR(quick.as_ptr()));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR(std::ptr::null()));
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, PCWSTR(settings.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, ID_ABOUT, PCWSTR(about.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR(std::ptr::null()));
        let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, PCWSTR(exit.as_ptr()));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            pt.x,
            pt.y,
            None,
            owner,
            None,
        )
        .0 as usize;
        let _ = DestroyMenu(menu);

        match cmd {
            ID_CAPTURE => MenuCmd::Capture,
            ID_QUICK_SAVE => MenuCmd::QuickSave,
            ID_SETTINGS => MenuCmd::Settings,
            ID_ABOUT => MenuCmd::About,
            ID_EXIT => MenuCmd::Exit,
            _ => MenuCmd::None,
        }
    }
}

/// Build the tray icon in memory: dark rounded square with accent-colored
/// crop brackets in the corners — the same motif as the capture overlay.
fn draw_icon() -> Result<HICON> {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
        let w = size as i32;
        let h = size as i32;
        let stride = size as usize * 4;
        let total = stride * size as usize;

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: total as u32,
                ..std::mem::zeroed()
            },
            bmiColors: [std::mem::zeroed()],
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|e| Error::Win32(e).with_context("CreateDIBSection(icon)"))?;
        if bits.is_null() {
            let _ = DeleteObject(dib.into());
            return Err(Error::msg("icon DIB has no bits"));
        }
        std::ptr::write_bytes(bits as *mut u8, 0, total);

        let px = std::slice::from_raw_parts_mut(bits as *mut u8, total);
        let radius = (size / 4).max(3) as i32;
        // Colors are COLORREF 0x00BBGGRR.
        let bg = crate::win::dark::BG;
        let (bg_r, bg_g, bg_b) = (
            ((bg >> 16) & 0xFF) as u8,
            ((bg >> 8) & 0xFF) as u8,
            (bg & 0xFF) as u8,
        );
        let (ar, ag, ab) = crate::win::dark::ACCENT_RGB;

        for y in 0..h {
            for x in 0..w {
                let inside = if x < radius && y < radius {
                    inside_circle(x - radius + 1, y - radius + 1, radius)
                } else if x >= w - radius && y < radius {
                    inside_circle(x - (w - radius), y - radius + 1, radius)
                } else if x < radius && y >= h - radius {
                    inside_circle(x - radius + 1, y - (h - radius), radius)
                } else if x >= w - radius && y >= h - radius {
                    inside_circle(x - (w - radius), y - (h - radius), radius)
                } else {
                    true
                };
                if !inside {
                    continue;
                }
                let i = (y as usize * stride) + x as usize * 4;
                px[i] = bg_b;
                px[i + 1] = bg_g;
                px[i + 2] = bg_r;
                px[i + 3] = 255;
            }
        }

        // Accent crop brackets in each corner (2px thick, inset 3px).
        let inset = 3i32;
        let arm = (size as i32 / 3).max(4);
        let thick = 2i32;
        let corners: [(i32, i32, bool, bool); 4] = [
            (0, 0, false, false),
            (w - 1, 0, true, false),
            (0, h - 1, false, true),
            (w - 1, h - 1, true, true),
        ];
        for (cx, cy, right, bottom) in corners {
            let x0 = if right { cx - inset - arm } else { cx + inset };
            let y0 = if bottom { cy - inset - arm } else { cy + inset };
            for y in y0..y0 + thick {
                for x in x0..x0 + arm {
                    set_px(px, stride as i32, w, h, x, y, ar, ag, ab);
                }
            }
            let x1 = if right {
                cx - inset - thick
            } else {
                cx + inset
            };
            let y1 = if bottom { cy - inset - arm } else { cy + inset };
            for y in y1..y1 + arm {
                for x in x1..x1 + thick {
                    set_px(px, stride as i32, w, h, x, y, ar, ag, ab);
                }
            }
        }

        // Monochrome mask (all transparent): with a 32bpp color bitmap the
        // shell honors the alpha channel.
        let mask_stride = ((size as usize + 31) / 32) * 4;
        let mask_buf = vec![0u8; mask_stride * size as usize];
        let mask = CreateBitmap(
            w,
            h,
            1,
            1,
            Some(mask_buf.as_ptr() as *const core::ffi::c_void),
        );
        if mask.is_invalid() {
            let _ = DeleteObject(dib.into());
            return Err(Error::msg("could not create the icon mask"));
        }

        let info = ICONINFO {
            fIcon: BOOL(1),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: dib,
        };
        let icon = match CreateIconIndirect(&info) {
            Ok(h) => h,
            Err(e) => {
                let _ = DeleteObject(dib.into());
                let _ = DeleteObject(mask.into());
                return Err(Error::Win32(e).with_context("CreateIconIndirect"));
            }
        };
        let _ = DeleteObject(dib.into());
        let _ = DeleteObject(mask.into());
        Ok(icon)
    }
}

fn set_px(px: &mut [u8], stride: i32, w: i32, h: i32, x: i32, y: i32, r: u8, g: u8, b: u8) {
    if x < 0 || y < 0 || x >= w || y >= h {
        return;
    }
    let i = (y as usize * stride as usize) + x as usize * 4;
    if px[i + 3] == 0 {
        return; // outside the rounded rect
    }
    px[i] = b;
    px[i + 1] = g;
    px[i + 2] = r;
    px[i + 3] = 255;
}

fn inside_circle(dx: i32, dy: i32, r: i32) -> bool {
    dx * dx + dy * dy <= r * r
}

/// The default arrow cursor (system cursors are shared; never freed).
pub fn arrow_cursor() -> HCURSOR {
    unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_else(|_| std::mem::zeroed()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{forget, transmute};

    #[test]
    fn recovery_state_is_add_first_with_modify_fallback() {
        // Fresh `NIM_ADD` succeeded: registered.
        assert!(recovery_state(true, false));
        // `NIM_ADD` rejected (icon already present) but `NIM_MODIFY`
        // succeeded: the entry exists and its payload was refreshed.
        assert!(recovery_state(false, true));
        // Both failed: the icon is gone — `added` must become false so the
        // next TaskbarCreated retries NIM_ADD (the old code stayed true
        // and only ever issued a no-op NIM_MODIFY, losing the icon for
        // the rest of the session).
        assert!(!recovery_state(false, false));
        // `modify_ok` is not even attempted when the add won; the result
        // still counts as registered.
        assert!(recovery_state(true, false));
    }

    #[test]
    fn nid_payload_is_complete_for_add_and_modify() {
        // A re-add must carry the full payload; a struct missing the
        // callback/icon/tooltip cannot restore the icon after an Explorer
        // restart. Distinct fake values prove each field is copied.
        let fake_hwnd: HWND = unsafe { transmute::<usize, HWND>(0x1234) };
        let fake_icon: HICON = unsafe { transmute::<usize, HICON>(0x5678) };
        let t = TrayIcon {
            hwnd: fake_hwnd,
            icon: fake_icon,
            added: false,
        };
        let n = t.nid();
        forget(t); // never run Drop: it would call Shell_NotifyIconW/DestroyIcon
        assert_eq!(n.cbSize, NID_SIZE);
        assert_eq!(n.hWnd, fake_hwnd);
        assert_eq!(n.uID, ICON_ID);
        assert_eq!(n.uCallbackMessage, WM_APP_TRAY);
        assert_eq!(n.hIcon, fake_icon);
        assert!(n.uFlags.contains(NIF_MESSAGE), "callback flag missing");
        assert!(n.uFlags.contains(NIF_ICON), "icon flag missing");
        assert!(n.uFlags.contains(NIF_TIP), "tooltip flag missing");
        let units: Vec<u16> = n.szTip.iter().take_while(|&&c| c != 0).copied().collect();
        let tip = String::from_utf16_lossy(&units);
        assert_eq!(tip, s().tray_tooltip, "tooltip must be the current string");
        // Shell_NotifyIconW is deliberately never called from tests.
    }
}
