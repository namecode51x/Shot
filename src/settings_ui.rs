//! Modeless Settings and About windows plus the hotkey-capture window.
//! All are plain windows on the main thread (no nested loops of our own);
//! the app-level message prefilter gives Settings/About dialog navigation
//! (Tab/arrows, Enter = OK, Esc = close) via `IsDialogMessageW`.
//!
//! Pending-changes model: [`App::settings`] is only replaced on OK, so Cancel
//! and close need no restore step. The hotkey capture window writes into the
//! pending settings of the open Settings window (never into live state).

use crate::app::{self, App};
use crate::config::{self, OutputFormat, Settings};
use crate::hotkey::{HotkeySpec, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
use crate::strings::{fill, s};
use crate::win::dark;
use crate::win::dialog;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DT_LEFT, DT_SINGLELINE, DT_VCENTER, HDC, SetBkColor, SetTextColor,
};
use windows::Win32::UI::Controls::{
    BST_CHECKED, DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED, ODT_BUTTON,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetAsyncKeyState, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, BM_GETCHECK, BM_SETCHECK, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL,
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetCursorPos, GetDlgCtrlID, GetDlgItem, GetDlgItemTextW, HMENU, IsWindow, SW_SHOW,
    SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowTextW, ShowWindow, WINDOW_STYLE,
    WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_DRAWITEM, WM_KEYDOWN, WM_NCCREATE, WM_PAINT, WM_SETFONT, WM_SYSKEYDOWN, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_OVERLAPPED, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::PCWSTR;

use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};

/// Window classes (registered by [`crate::app`]; also used to recognize the
/// roots in the message prefilter).
pub const CLASS_SETTINGS: &str = "NameShot.Settings";
pub const CLASS_HOTKEY: &str = "NameShot.Hotkey";
pub const CLASS_ABOUT: &str = "NameShot.About";

// Control ids -------------------------------------------------------------
const IDC_HOTKEY_LABEL: u32 = 1001;
const IDC_HOTKEY_VALUE: u32 = 1002;
const IDC_CHANGE: u32 = 1003;
const IDC_CAPTURE_NOW: u32 = 1004;
const IDC_FORMAT_LABEL: u32 = 1005;
const IDC_FORMAT: u32 = 1006;
const IDC_JPEG_LABEL: u32 = 1007;
const IDC_JPEG: u32 = 1008;
const IDC_FOLDER_LABEL: u32 = 1009;
const IDC_FOLDER: u32 = 1010;
const IDC_BROWSE: u32 = 1011;
const IDC_QUICK: u32 = 1012;
const IDC_QUICK_HINT: u32 = 1013;
const IDC_STARTUP: u32 = 1014;
const IDC_STARTUP_HINT: u32 = 1015;
const IDC_OK: u32 = 1016;
const IDC_CANCEL: u32 = 1017;

const HK_PROMPT: u32 = 2001;
const HK_VALUE: u32 = 2002;
const HK_HINT: u32 = 2003;
const HK_APPLY: u32 = 2004;
const HK_CANCEL: u32 = 2005;

const AB_CLOSE: u32 = 3001;

/// `IDOK` / `IDCANCEL`, as sent by the dialog manager in the message
/// prefilter ([`crate::app`]) when Enter/Esc is pressed.
const CMD_OK: u32 = 1;
const CMD_CANCEL: u32 = 2;

// Settings layout (4 px grid; shared by build_controls and paint_settings) ---
const SET_W: i32 = 484;
const SET_H: i32 = 368;
/// Left margin for headers, dividers and rows.
const SET_L: i32 = 16;
/// Right edge for headers and dividers.
const SET_R: i32 = SET_W - 16;
// Group 1: Capture & Hotkey.
const G1_TEXT_TOP: i32 = 8;
const G1_DIV: i32 = 32;
const ROW_HOTKEY_Y: i32 = 40;
const ROW_H: i32 = 26;
// Group 2: Save & Format.
const G2_TEXT_TOP: i32 = 72;
const G2_DIV: i32 = 96;
const ROW_FORMAT_Y: i32 = 104;
const ROW_FOLDER_Y: i32 = 140;
const ROW_EDIT_H: i32 = 24;
const QUICK_Y: i32 = 172;
const HINT_Y: i32 = 200;
const HINT_H: i32 = 20;
// Group 3: Startup.
const G3_TEXT_TOP: i32 = 232;
const G3_DIV: i32 = 256;
const STARTUP_Y: i32 = 264;
const STARTUP_HINT_Y: i32 = 292;
// Footer buttons.
const BTN_Y: i32 = 320;
const BTN_H: i32 = 30;

// Button/extension styles live as plain integers (the corresponding typed
// constants are behind Windows features we do not enable).
const BS_OWNERDRAW_ST: WINDOW_STYLE = WINDOW_STYLE(11); // BS_OWNERDRAW
const BS_CHECKBOX_ST: WINDOW_STYLE = WINDOW_STYLE(3); // BS_AUTOCHECKBOX
const SS_CENTER: WINDOW_STYLE = WINDOW_STYLE(1); // SS_CENTER
const SS_CENTERIMAGE: WINDOW_STYLE = WINDOW_STYLE(512); // SS_CENTERIMAGE
const ES_READONLY_ST: WINDOW_STYLE = WINDOW_STYLE(2048); // ES_READONLY
const ES_NUMBER_ST: WINDOW_STYLE = WINDOW_STYLE(8192); // ES_NUMBER
const ES_AUTOHSCROLL_ST: WINDOW_STYLE = WINDOW_STYLE(128); // ES_AUTOHSCROLL
const CBS_DROPDOWNLIST_ST: WINDOW_STYLE = WINDOW_STYLE(3); // CBS_DROPDOWNLIST

/// Live state of the open Settings window (pending values, not yet applied).
pub struct SettingsSession {
    pub hwnd: HWND,
    pub pending: Settings,
}

fn key_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn ctrl(
    parent: HWND,
    class: &str,
    text: &str,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u32,
) -> Option<HWND> {
    let c = crate::win::wide(class);
    let t = crate::win::wide(text);
    unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(c.as_ptr()),
            PCWSTR(t.as_ptr()),
            style,
            x,
            y,
            w,
            h,
            Some(parent),
            Some(HMENU(id as usize as *mut core::ffi::c_void)),
            None,
            None,
        )
        .ok()
    }
}

fn set_font(app: &App, h: HWND) {
    unsafe {
        SendMessageW(
            h,
            WM_SETFONT,
            Some(WPARAM(app.font_ui.0.0 as usize)),
            Some(LPARAM(1)),
        );
    }
}

fn get_text(parent: HWND, id: u32) -> String {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetDlgItemTextW(parent, id as i32, &mut buf) };
    String::from_utf16_lossy(&buf[..n as usize])
}

fn set_text(parent: HWND, id: u32, text: &str) {
    if let Ok(h) = unsafe { GetDlgItem(Some(parent), id as i32) } {
        let t = crate::win::wide(text);
        unsafe {
            let _ = SetWindowTextW(h, PCWSTR(t.as_ptr()));
        }
    }
}

fn child(parent: HWND, id: u32) -> Option<HWND> {
    unsafe { GetDlgItem(Some(parent), id as i32).ok() }
}

fn get_combo_sel(parent: HWND, id: u32) -> Option<usize> {
    let h = child(parent, id)?;
    let v = unsafe { SendMessageW(h, CB_GETCURSEL, None, None) }.0 as i32;
    if v < 0 { None } else { Some(v as usize) }
}

fn is_checked(parent: HWND, id: u32) -> bool {
    child(parent, id)
        .map(|h| unsafe { SendMessageW(h, BM_GETCHECK, None, None) }.0 as u32 == BST_CHECKED.0)
        .unwrap_or(false)
}

fn set_check(parent: HWND, id: u32, on: bool) {
    if let Some(h) = child(parent, id) {
        unsafe {
            SendMessageW(h, BM_SETCHECK, Some(WPARAM(BST_CHECKED.0 as usize)), None);
        }
        if !on {
            unsafe {
                SendMessageW(h, BM_SETCHECK, Some(WPARAM(0)), None);
            }
        }
    }
}

/// Shared `WM_CTLCOLO*` handler for the dark palette.
/// `wparam` is the control's DC, `lparam` its HWND.
fn ctl_color(hdc: HDC, ctl: HWND) -> LRESULT {
    let id = unsafe { GetDlgCtrlID(ctl) } as u32;
    let dim = id == IDC_QUICK_HINT || id == IDC_STARTUP_HINT || id == HK_HINT;
    let color = if dim { dark::FG_DIM } else { dark::FG };
    unsafe {
        SetTextColor(hdc, dark::colorref_u32(color));
        SetBkColor(hdc, dark::colorref_u32(crate::theme::SURFACE_U32));
        let br = dark::bg_brush();
        LRESULT(br.0 as *mut core::ffi::c_void as isize)
    }
}

// ---------------------------------------------------------------------------
// Settings window
// ---------------------------------------------------------------------------

/// Open (or focus) the Settings window. Returns its HWND.
pub fn open(app: &'static App) -> HWND {
    if let Some(h) = app.settings_ui.borrow().as_ref().map(|s| s.hwnd) {
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        return h;
    }

    let (cw, ch) = (SET_W, SET_H);
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: cw,
        bottom: ch,
    };
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    if unsafe { AdjustWindowRectEx(&mut outer, style, false, Default::default()) }.is_err() {
        return HWND::default();
    }
    let ow = outer.right - outer.left;
    let oh = outer.bottom - outer.top;

    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    let wa = crate::os::work_area_for(&RECT {
        left: pt.x,
        top: pt.y,
        right: pt.x + 1,
        bottom: pt.y + 1,
    });
    let x = wa.left + ((wa.right - wa.left) - ow) / 2;
    let y = wa.top + ((wa.bottom - wa.top) - oh) / 2;

    let title = crate::win::wide(s().settings_title);
    let cls = crate::win::wide(CLASS_SETTINGS);
    let hwnd = match unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(cls.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
            x,
            y,
            ow,
            oh,
            None,
            None,
            Some(crate::win::gdi::this_module()),
            Some(app as *const App as *const core::ffi::c_void),
        )
    } {
        Ok(h) => h,
        Err(_) => return HWND::default(),
    };

    let pending = app.settings.borrow().clone();
    app.settings_ui
        .replace(Some(SettingsSession { hwnd, pending }));

    build_controls(app, hwnd);
    populate(app, hwnd);

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        crate::win::dark::set_dark_titlebar(hwnd, true);
    }
    hwnd
}

fn build_controls(app: &'static App, hwnd: HWND) {
    let st = s();
    let button = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW_ST;
    let label = WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE;

    let items: [(&str, &str, WINDOW_STYLE, i32, i32, i32, i32, u32); 17] = [
        (
            "STATIC",
            st.settings_hotkey,
            label,
            SET_L,
            ROW_HOTKEY_Y,
            110,
            ROW_H,
            IDC_HOTKEY_LABEL,
        ),
        (
            "STATIC",
            "",
            label | WS_BORDER,
            132,
            ROW_HOTKEY_Y,
            150,
            ROW_H,
            IDC_HOTKEY_VALUE,
        ),
        (
            "BUTTON",
            st.settings_change,
            button,
            290,
            ROW_HOTKEY_Y,
            88,
            ROW_H,
            IDC_CHANGE,
        ),
        (
            "BUTTON",
            st.settings_capture_now,
            button,
            386,
            ROW_HOTKEY_Y,
            82,
            ROW_H,
            IDC_CAPTURE_NOW,
        ),
        (
            "STATIC",
            st.settings_format,
            label,
            SET_L,
            ROW_FORMAT_Y,
            110,
            ROW_H,
            IDC_FORMAT_LABEL,
        ),
        (
            "COMBOBOX",
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST_ST | WS_VSCROLL,
            132,
            ROW_FORMAT_Y,
            90,
            200,
            IDC_FORMAT,
        ),
        (
            "STATIC",
            st.settings_jpeg_quality,
            label,
            240,
            ROW_FORMAT_Y,
            90,
            ROW_H,
            IDC_JPEG_LABEL,
        ),
        (
            "EDIT",
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_NUMBER_ST | ES_AUTOHSCROLL_ST,
            336,
            ROW_FORMAT_Y,
            50,
            ROW_EDIT_H,
            IDC_JPEG,
        ),
        (
            "STATIC",
            st.settings_folder,
            label,
            SET_L,
            ROW_FOLDER_Y,
            110,
            ROW_EDIT_H,
            IDC_FOLDER_LABEL,
        ),
        (
            "EDIT",
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_READONLY_ST | ES_AUTOHSCROLL_ST,
            132,
            ROW_FOLDER_Y,
            240,
            ROW_EDIT_H,
            IDC_FOLDER,
        ),
        (
            "BUTTON",
            st.settings_browse,
            button,
            380,
            ROW_FOLDER_Y,
            88,
            ROW_H,
            IDC_BROWSE,
        ),
        (
            "BUTTON",
            st.settings_quick_save,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_CHECKBOX_ST,
            SET_L,
            QUICK_Y,
            452,
            ROW_EDIT_H,
            IDC_QUICK,
        ),
        (
            "STATIC",
            st.settings_quick_save_hint,
            WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE,
            SET_L,
            HINT_Y,
            452,
            HINT_H,
            IDC_QUICK_HINT,
        ),
        (
            "BUTTON",
            st.settings_startup,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_CHECKBOX_ST,
            SET_L,
            STARTUP_Y,
            452,
            ROW_EDIT_H,
            IDC_STARTUP,
        ),
        (
            "STATIC",
            st.settings_startup_hint,
            WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE,
            SET_L,
            STARTUP_HINT_Y,
            452,
            HINT_H,
            IDC_STARTUP_HINT,
        ),
        (
            "BUTTON",
            st.settings_ok,
            button,
            252,
            BTN_Y,
            104,
            BTN_H,
            IDC_OK,
        ),
        (
            "BUTTON",
            st.settings_cancel,
            button,
            364,
            BTN_Y,
            104,
            BTN_H,
            IDC_CANCEL,
        ),
    ];
    for (class, text, style, x, y, w, h, id) in items {
        if let Some(c) = ctrl(hwnd, class, text, style, x, y, w, h, id) {
            set_font(app, c);
            dark::dark_control(c);
        }
    }

    // Accessible names for the three controls that show no text of their own
    // (combo selection, value edits); everything else has a visible label or
    // its own window text.
    for (id, name) in [
        (IDC_FORMAT, st.settings_format),
        (IDC_JPEG, st.settings_jpeg_quality),
        (IDC_FOLDER, st.settings_folder),
    ] {
        if let Some(c) = child(hwnd, id) {
            crate::win::a11y::set_name(c, name);
        }
    }
}

fn populate(app: &'static App, hwnd: HWND) {
    let pending = app.settings_ui.borrow().as_ref().map(|p| p.pending.clone());
    let Some(p) = pending else { return };
    set_text(hwnd, IDC_HOTKEY_VALUE, &p.hotkey.format());

    if let Some(combo) = child(hwnd, IDC_FORMAT) {
        for name in ["PNG", "JPEG", "BMP"] {
            let n = crate::win::wide(name);
            unsafe {
                SendMessageW(combo, CB_ADDSTRING, None, Some(LPARAM(n.as_ptr() as isize)));
            }
        }
        let idx = match p.format {
            OutputFormat::Png => 0,
            OutputFormat::Jpeg => 1,
            OutputFormat::Bmp => 2,
        };
        unsafe {
            SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(idx)), None);
        }
    }
    set_text(hwnd, IDC_FOLDER, &p.save_dir);
    set_text(hwnd, IDC_JPEG, &p.jpeg_quality.to_string());
    set_check(hwnd, IDC_QUICK, p.quick_save);
    set_check(hwnd, IDC_STARTUP, p.launch_at_login);
}

/// Read the controls, persist, re-register the hotkey if it changed, then
/// close the window. Cancel does nothing (live state was never touched).
fn apply(app: &'static App, hwnd: HWND) {
    let st = s();
    let mut new = app
        .settings_ui
        .borrow()
        .as_ref()
        .map(|s| s.pending.clone())
        .unwrap_or_default();

    // Format, quality, folder, checkboxes come from the controls.
    if let Some(idx) = get_combo_sel(hwnd, IDC_FORMAT) {
        new.format = match idx {
            1 => OutputFormat::Jpeg,
            2 => OutputFormat::Bmp,
            _ => OutputFormat::Png,
        };
    }
    let q: u32 = get_text(hwnd, IDC_JPEG)
        .trim()
        .parse()
        .unwrap_or(new.jpeg_quality as u32);
    new.jpeg_quality = q.clamp(1, 100) as u8;
    new.save_dir = get_text(hwnd, IDC_FOLDER);
    new.save_dir.retain(|c| !c.is_control());
    new.quick_save = is_checked(hwnd, IDC_QUICK);
    new.launch_at_login = is_checked(hwnd, IDC_STARTUP);

    // Hotkey: only re-register when it actually changed.
    let old = app.registered_hotkey.get();
    if new.hotkey != old {
        crate::win::hotkey::unregister(app.main_hwnd.get());
        match app::register_hotkey(app, &new.hotkey) {
            Ok(()) => {
                app.registered_hotkey.set(new.hotkey);
                app.hotkey_ok.set(true);
            }
            Err(e) => {
                if crate::win::hotkey::is_conflict(&e) {
                    dialog::warn_box(
                        Some(hwnd),
                        st.hotkey_conflict_title,
                        &fill(st.hotkey_conflict_msg, "H", &new.hotkey.format()),
                    );
                } else {
                    dialog::warn_box(Some(hwnd), st.hotkey_unavailable_title, &e.to_string());
                }
                // Restore the previous registration; the setting reverts too.
                new.hotkey = old;
                if app::register_hotkey(app, &old).is_ok() {
                    app.registered_hotkey.set(old);
                    app.hotkey_ok.set(true);
                } else {
                    app.hotkey_ok.set(false);
                }
            }
        }
    }

    // Start-at-sign-in: registry only; failure is a warning, never fatal.
    if new.launch_at_login != app.settings.borrow().launch_at_login {
        if let Err(e) = crate::os::set_launch_at_login(new.launch_at_login) {
            dialog::warn_box(Some(hwnd), st.app_name, &e.to_string());
        }
    }

    *app.settings.borrow_mut() = new.clone();
    if let Err(e) = config::save(&app.settings_path, &new) {
        dialog::error_box(
            Some(hwnd),
            st.state_save_fail_title,
            &fill(st.state_save_fail, "DETAIL", &e.to_string()),
        );
    }

    // Close a leftover hotkey-capture window, then this one.
    close_hotkey_window(app);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    dialog::info_box(None, st.app_name, st.settings_saved);
}

fn browse(app: &'static App, hwnd: HWND) {
    let initial = app
        .settings_ui
        .borrow()
        .as_ref()
        .map(|s| s.pending.save_dir.clone());
    let initial = initial.filter(|v| !v.trim().is_empty());
    match dialog::pick_folder(Some(hwnd), initial.as_deref().map(std::path::Path::new)) {
        Ok(Some(p)) => {
            let path = p.to_string_lossy().into_owned();
            {
                let mut b = app.settings_ui.borrow_mut();
                if let Some(st) = b.as_mut() {
                    st.pending.save_dir = path.clone();
                }
            }
            set_text(hwnd, IDC_FOLDER, &path);
        }
        Ok(None) => {}
        Err(e) => dialog::warn_box(Some(hwnd), s().app_name, &e.to_string()),
    }
}

/// Group headers + dividers painted over the class-brush background
/// (`FONT_SMALL` in `FG_DIM`, 1 px `BORDER_INPUT` divider — spec §3).
fn paint_settings(app: &App, hwnd: HWND) {
    let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    if hdc.is_invalid() {
        unsafe {
            let _ = EndPaint(hwnd, &ps);
        }
        return;
    }
    let st = s();
    let groups: [(&str, i32, i32); 3] = [
        (st.settings_group_capture, G1_TEXT_TOP, G1_DIV),
        (st.settings_group_save, G2_TEXT_TOP, G2_DIV),
        (st.settings_group_startup, G3_TEXT_TOP, G3_DIV),
    ];
    unsafe {
        let prev = windows::Win32::Graphics::Gdi::SelectObject(hdc, app.font_small.0.into());
        for (text, top, div) in groups {
            crate::win::gdi::draw_label(
                hdc,
                RECT {
                    left: SET_L,
                    top,
                    right: SET_R,
                    bottom: top + 20,
                },
                text,
                crate::theme::FG_DIM_U32,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
            crate::win::gdi::fill_solid(
                hdc,
                &RECT {
                    left: SET_L,
                    top: div,
                    right: SET_R,
                    bottom: div + 1,
                },
                crate::theme::BORDER_INPUT_U32,
            );
        }
        let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, prev);
    }
    unsafe {
        let _ = EndPaint(hwnd, &ps);
    }
}

pub unsafe extern "system" fn settings_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        return LRESULT(1);
    }
    let Some(app) = crate::app::app_of(hwnd) else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };

    match msg {
        WM_COMMAND => {
            let id = (wparam.0 as u32) & 0xFFFF;
            match id {
                IDC_CHANGE => open_hotkey(app),
                IDC_CAPTURE_NOW => app::start_capture(app),
                IDC_BROWSE => browse(app, hwnd),
                IDC_OK | CMD_OK => apply(app, hwnd),
                IDC_CANCEL | CMD_CANCEL => unsafe {
                    let _ = DestroyWindow(hwnd);
                },
                _ => {}
            }
            LRESULT(0)
        }
        WM_PAINT => {
            paint_settings(app, hwnd);
            LRESULT(0)
        }
        WM_DRAWITEM => draw_item(app, lparam),
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLORLISTBOX => ctl_color(
            HDC(wparam.0 as *mut core::ffi::c_void),
            HWND(lparam.0 as *mut core::ffi::c_void),
        ),
        WM_DESTROY => {
            let owns = app
                .settings_ui
                .borrow()
                .as_ref()
                .map(|s| s.hwnd == hwnd)
                .unwrap_or(false);
            if owns {
                app.settings_ui.take();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Owner-drawn buttons (shared by Settings, hotkey and About windows)
// ---------------------------------------------------------------------------

fn draw_item(app: &App, lparam: LPARAM) -> LRESULT {
    let di = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
    if di.CtlType != ODT_BUTTON {
        return LRESULT(0);
    }
    let st = s();
    let (label, style) = match di.CtlID {
        IDC_OK => (st.settings_ok, crate::editor::toolbar::FaceStyle::Primary),
        IDC_CANCEL => (
            st.settings_cancel,
            crate::editor::toolbar::FaceStyle::Normal,
        ),
        IDC_CHANGE => (
            st.settings_change,
            crate::editor::toolbar::FaceStyle::Normal,
        ),
        IDC_CAPTURE_NOW => (
            st.settings_capture_now,
            crate::editor::toolbar::FaceStyle::Normal,
        ),
        IDC_BROWSE => (
            st.settings_browse,
            crate::editor::toolbar::FaceStyle::Normal,
        ),
        HK_APPLY => (
            st.hotkey_dialog_ok,
            crate::editor::toolbar::FaceStyle::Primary,
        ),
        HK_CANCEL => (
            st.hotkey_dialog_cancel,
            crate::editor::toolbar::FaceStyle::Normal,
        ),
        AB_CLOSE => (st.about_close, crate::editor::toolbar::FaceStyle::Normal),
        _ => return LRESULT(0),
    };
    crate::editor::toolbar::paint_face(
        di.hDC,
        &di.rcItem,
        label,
        di.itemState.0 & ODS_SELECTED.0 != 0,
        di.itemState.0 & ODS_DISABLED.0 == 0,
        di.itemState.0 & ODS_FOCUS.0 != 0,
        style,
        app.font_ui.0,
    );
    LRESULT(1)
}

// ---------------------------------------------------------------------------
// Hotkey capture window
// ---------------------------------------------------------------------------

/// Open (or focus) the hotkey capture window over the Settings window.
fn open_hotkey(app: &'static App) {
    let existing = app.hotkey_hwnd.get();
    if !existing.0.is_null() && unsafe { IsWindow(Some(existing)) }.as_bool() {
        unsafe {
            let _ = SetForegroundWindow(existing);
        }
        return;
    }
    let owner = app.settings_ui.borrow().as_ref().map(|s| s.hwnd);
    let current = app
        .settings_ui
        .borrow()
        .as_ref()
        .map(|s| s.pending.hotkey)
        .unwrap_or_else(|| app.registered_hotkey.get());
    app.hotkey_captured.set(None);

    let (cw, ch) = (360, 170);
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: cw,
        bottom: ch,
    };
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    if unsafe { AdjustWindowRectEx(&mut outer, style, false, Default::default()) }.is_err() {
        return;
    }
    let title = crate::win::wide(s().hotkey_dialog_title);
    let cls = crate::win::wide(CLASS_HOTKEY);
    let hwnd = match unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(cls.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
            0,
            0,
            outer.right - outer.left,
            outer.bottom - outer.top,
            owner,
            None,
            Some(crate::win::gdi::this_module()),
            Some(app as *const App as *const core::ffi::c_void),
        )
    } {
        Ok(h) => h,
        Err(_) => return,
    };
    app.hotkey_hwnd.set(hwnd);

    let st = s();
    let current_text = current.format();
    let button = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW_ST;
    let label = WS_CHILD | WS_VISIBLE | SS_CENTERIMAGE;
    let items: [(&str, &str, WINDOW_STYLE, i32, i32, i32, i32, u32); 5] = [
        (
            "STATIC",
            st.hotkey_dialog_prompt,
            label | SS_CENTER,
            16,
            14,
            328,
            26,
            HK_PROMPT,
        ),
        (
            "STATIC",
            current_text.as_str(),
            label | SS_CENTER | WS_BORDER,
            40,
            48,
            280,
            30,
            HK_VALUE,
        ),
        (
            "STATIC",
            st.hotkey_dialog_hint,
            label | SS_CENTER,
            16,
            86,
            328,
            34,
            HK_HINT,
        ),
        (
            "BUTTON",
            st.hotkey_dialog_ok,
            button,
            160,
            126,
            88,
            30,
            HK_APPLY,
        ),
        (
            "BUTTON",
            st.hotkey_dialog_cancel,
            button,
            256,
            126,
            88,
            30,
            HK_CANCEL,
        ),
    ];
    for (class, text, style, x, y, w, h, id) in items {
        if let Some(c) = ctrl(hwnd, class, text, style, x, y, w, h, id) {
            set_font(app, c);
            dark::dark_control(c);
        }
    }
    if let Some(a) = child(hwnd, HK_APPLY) {
        unsafe {
            let _ = EnableWindow(a, false);
        }
    }

    crate::win::dark::set_dark_titlebar(hwnd, true);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
}

fn close_hotkey_window(app: &App) {
    let h = app.hotkey_hwnd.get();
    if !h.0.is_null() && unsafe { IsWindow(Some(h)) }.as_bool() {
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
    app.hotkey_hwnd.set(HWND::default());
}

fn read_modifiers() -> u8 {
    let mut m = 0;
    if key_down(0x11) {
        m |= MOD_CONTROL;
    }
    if key_down(0x12) {
        m |= MOD_ALT;
    }
    if key_down(0x10) {
        m |= MOD_SHIFT;
    }
    if key_down(0x5B) || key_down(0x5C) {
        m |= MOD_WIN;
    }
    m
}

fn is_modifier_vk(vk: u16) -> bool {
    matches!(
        vk,
        0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5
    )
}

/// Feed a captured combination into the pending settings of the open
/// Settings window (the place the Change button lives in).
fn use_captured(app: &'static App) {
    let Some(spec) = app.hotkey_captured.get() else {
        return;
    };
    let owner = {
        let mut b = app.settings_ui.borrow_mut();
        let Some(sess) = b.as_mut() else { return };
        sess.pending.hotkey = spec;
        sess.hwnd
    };
    set_text(owner, IDC_HOTKEY_VALUE, &spec.format());
}

pub unsafe extern "system" fn hotkey_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        return LRESULT(1);
    }
    let Some(app) = crate::app::app_of(hwnd) else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };
    let st = s();

    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = (wparam.0 as u32 & 0xFF) as u16;
            if is_modifier_vk(vk) {
                return LRESULT(0);
            }
            let mods = read_modifiers();
            if vk == 0x1B && mods == 0 {
                // Esc with nothing held: cancel without applying.
                app.hotkey_captured.set(None);
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            if vk == 0x0D && mods == 0 {
                // Enter: apply the captured combination, if any.
                use_captured(app);
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            let spec = HotkeySpec {
                modifiers: mods,
                vk,
            };
            if spec.validate().is_ok() {
                app.hotkey_captured.set(Some(spec));
                set_text(hwnd, HK_VALUE, &spec.format());
                set_text(hwnd, HK_HINT, st.hotkey_dialog_hint);
                if let Some(a) = child(hwnd, HK_APPLY) {
                    let _ = EnableWindow(a, true);
                }
            } else {
                set_text(hwnd, HK_HINT, st.hotkey_dialog_bad);
                if let Some(a) = child(hwnd, HK_APPLY) {
                    let _ = EnableWindow(a, false);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 as u32) & 0xFFFF;
            match id {
                HK_APPLY => {
                    use_captured(app);
                    let _ = DestroyWindow(hwnd);
                }
                HK_CANCEL | CMD_CANCEL => {
                    app.hotkey_captured.set(None);
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DRAWITEM => draw_item(app, lparam),
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLORLISTBOX => ctl_color(
            HDC(wparam.0 as *mut core::ffi::c_void),
            HWND(lparam.0 as *mut core::ffi::c_void),
        ),
        WM_DESTROY => {
            if app.hotkey_hwnd.get() == hwnd {
                app.hotkey_hwnd.set(HWND::default());
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// About window
// ---------------------------------------------------------------------------

/// Open (or focus) the About window. Returns its HWND.
pub fn open_about(app: &'static App) -> HWND {
    let existing = app.about_hwnd.get();
    if !existing.0.is_null() && unsafe { IsWindow(Some(existing)) }.as_bool() {
        unsafe {
            let _ = SetForegroundWindow(existing);
        }
        return existing;
    }

    let (cw, ch) = (420, 240);
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: cw,
        bottom: ch,
    };
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    if unsafe { AdjustWindowRectEx(&mut outer, style, false, Default::default()) }.is_err() {
        return HWND::default();
    }
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    let wa = crate::os::work_area_for(&RECT {
        left: pt.x,
        top: pt.y,
        right: pt.x + 1,
        bottom: pt.y + 1,
    });
    let x = wa.left + ((wa.right - wa.left) - (outer.right - outer.left)) / 2;
    let y = wa.top + ((wa.bottom - wa.top) - (outer.bottom - outer.top)) / 2;

    let title = crate::win::wide(s().about_title);
    let cls = crate::win::wide(CLASS_ABOUT);
    let hwnd = match unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(cls.as_ptr()),
            PCWSTR(title.as_ptr()),
            style,
            x,
            y,
            outer.right - outer.left,
            outer.bottom - outer.top,
            None,
            None,
            Some(crate::win::gdi::this_module()),
            Some(app as *const App as *const core::ffi::c_void),
        )
    } {
        Ok(h) => h,
        Err(_) => return HWND::default(),
    };
    app.about_hwnd.set(hwnd);

    if let Some(b) = ctrl(
        hwnd,
        "BUTTON",
        s().about_close,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW_ST,
        cw - 16 - 104,
        ch - 16 - 30,
        104,
        30,
        AB_CLOSE,
    ) {
        set_font(app, b);
    }

    crate::win::dark::set_dark_titlebar(hwnd, true);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    hwnd
}

fn paint_about(app: &App, hwnd: HWND) {
    let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    if hdc.is_invalid() {
        unsafe {
            let _ = EndPaint(hwnd, &ps);
        }
        return;
    }
    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    let st = s();
    let os = match crate::os::os_build_number() {
        Some(b) => format!("Windows build {b}"),
        None => "Windows".to_string(),
    };
    let version = fill(st.about_version, "VERSION", env!("CARGO_PKG_VERSION"));
    let os_line = fill(st.about_os, "OS", &os);

    unsafe {
        crate::win::gdi::fill_solid(hdc, &client, crate::theme::SURFACE_U32);
        let prev = windows::Win32::Graphics::Gdi::SelectObject(hdc, app.font_title().0.into());
        crate::win::gdi::draw_label(
            hdc,
            RECT {
                left: 24,
                top: 14,
                right: client.right - 24,
                bottom: 46,
            },
            st.about_title,
            dark::FG,
            windows::Win32::Graphics::Gdi::DT_LEFT | windows::Win32::Graphics::Gdi::DT_SINGLELINE,
        );
        // Short accent divider under the title (spec §3).
        crate::win::gdi::fill_solid(
            hdc,
            &RECT {
                left: 24,
                top: 52,
                right: 88,
                bottom: 54,
            },
            crate::theme::ACCENT_U32,
        );
        let small = windows::Win32::Graphics::Gdi::SelectObject(hdc, app.font_small.0.into());
        let dt =
            |r: RECT, t: &str, c: u32, extra: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT| {
                crate::win::gdi::draw_label(
                    hdc,
                    r,
                    t,
                    c,
                    windows::Win32::Graphics::Gdi::DT_LEFT | extra,
                );
            };
        dt(
            RECT {
                left: 24,
                top: 62,
                right: client.right - 24,
                bottom: 84,
            },
            &version,
            dark::FG_DIM,
            windows::Win32::Graphics::Gdi::DT_SINGLELINE,
        );
        dt(
            RECT {
                left: 24,
                top: 86,
                right: client.right - 24,
                bottom: 108,
            },
            st.app_description,
            dark::FG,
            windows::Win32::Graphics::Gdi::DT_SINGLELINE,
        );
        dt(
            RECT {
                left: 24,
                top: 114,
                right: client.right - 24,
                bottom: 162,
            },
            st.about_privacy,
            dark::FG_DIM,
            windows::Win32::Graphics::Gdi::DT_WORDBREAK,
        );
        dt(
            RECT {
                left: 24,
                top: 166,
                right: client.right - 24,
                bottom: 188,
            },
            &os_line,
            dark::FG_DIM,
            windows::Win32::Graphics::Gdi::DT_SINGLELINE,
        );
        let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, small);
        let _ = windows::Win32::Graphics::Gdi::SelectObject(hdc, prev);
    }
    unsafe {
        let _ = EndPaint(hwnd, &ps);
    }
}

pub unsafe extern "system" fn about_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        return LRESULT(1);
    }
    let Some(app) = crate::app::app_of(hwnd) else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };

    match msg {
        WM_PAINT => {
            paint_about(app, hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 as u32) & 0xFFFF;
            if id == AB_CLOSE || id == CMD_OK || id == CMD_CANCEL {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DRAWITEM => draw_item(app, lparam),
        WM_DESTROY => {
            if app.about_hwnd.get() == hwnd {
                app.about_hwnd.set(HWND::default());
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
// Layout asserts compare compile-time grid constants — that is the test.
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn settings_layout_sits_on_the_4px_grid() {
        for v in [
            SET_W,
            SET_H,
            G1_TEXT_TOP,
            G1_DIV,
            ROW_HOTKEY_Y,
            G2_TEXT_TOP,
            G2_DIV,
            ROW_FORMAT_Y,
            ROW_FOLDER_Y,
            QUICK_Y,
            HINT_Y,
            G3_TEXT_TOP,
            G3_DIV,
            STARTUP_Y,
            STARTUP_HINT_Y,
            BTN_Y,
        ] {
            assert_eq!(v % 4, 0, "{v} must sit on the 4 px grid");
        }
    }

    #[test]
    fn group_rows_never_cross_a_header_or_the_next_group() {
        // Header text clears its divider, divider clears the first row.
        for (top, div, next) in [
            (G1_TEXT_TOP, G1_DIV, ROW_HOTKEY_Y),
            (G2_TEXT_TOP, G2_DIV, ROW_FORMAT_Y),
            (G3_TEXT_TOP, G3_DIV, STARTUP_Y),
        ] {
            assert!(top + 20 <= div, "header text must not touch its divider");
            assert!(div < next, "divider must not touch its first row");
        }
        // Capture row stays inside its group.
        assert!(ROW_HOTKEY_Y + ROW_H <= G2_TEXT_TOP);
        // Save rows stay above the startup header; hint docked under checkbox.
        assert!(QUICK_Y + ROW_EDIT_H <= HINT_Y);
        assert!(HINT_Y + HINT_H <= G3_TEXT_TOP);
        // Startup rows stay above the footer buttons.
        assert!(STARTUP_HINT_Y + HINT_H < BTN_Y);
        // Footer keeps a 16 px bottom margin.
        assert!(BTN_Y + BTN_H <= SET_H - 16);
        // Columns keep their side margins.
        assert!(SET_L < SET_R && SET_R <= SET_W - 16);
    }

    #[test]
    fn group_headers_are_named_in_the_string_pack() {
        let st = s();
        assert!(!st.settings_group_capture.is_empty());
        assert!(!st.settings_group_save.is_empty());
        assert!(!st.settings_group_startup.is_empty());
    }
}
