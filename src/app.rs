//! Application shell: process start-up, window classes, the hidden main
//! window, tray icon, global hotkey, the single message loop and the
//! top-level actions reachable from the tray menu.
//!
//! There is exactly one message loop in the whole process. Modal UI (tray
//! menu, message boxes, file dialogs) runs its own nested system loop, never
//! one of ours. Every window procedure recovers the shared state through
//! [`app_of`] (`GWLP_USERDATA`, set during `WM_NCCREATE`).

use crate::config::Settings;
use crate::editor;
use crate::error::{Error, Result};
use crate::hotkey::HotkeySpec;
use crate::image::Frame;
use crate::overlay;
use crate::session::{Route, SaveOutcome};
use crate::settings_ui::{self, CLASS_ABOUT, CLASS_HOTKEY, CLASS_SETTINGS};
use crate::strings::{fill, s};
use crate::win::gdi::{self, OwnedGdi};
use crate::win::{self, WM_APP_TRAY, dark, dialog, tray};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ANSI_CHARSET, ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_GUI_FONT,
    FW_NORMAL, GetStockObject, HBRUSH, HFONT, OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::NIN_SELECT;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    FindWindowW, GA_ROOT, GWLP_USERDATA, GetAncestor, GetClassNameW, GetMessageW,
    GetWindowLongPtrW, HCURSOR, IDC_ARROW, IsDialogMessageW, IsWindow, LoadCursorW, MSG,
    PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetForegroundWindow,
    SetWindowLongPtrW, TranslateMessage, WINDOW_STYLE, WM_CHAR, WM_CONTEXTMENU, WM_DESTROY,
    WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONUP, WM_NCCREATE, WM_NULL, WM_RBUTTONUP, WM_SYSCHAR,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASS_STYLES, WNDCLASSEXW, WNDPROC,
};
use windows::core::{PCWSTR, w};

/// `NIN_KEYSELECT`: icon activated with the keyboard.
const NIN_KEYSELECT: u32 = 1025;

/// Main window class (the hidden message hub). Also used by the single-
/// instance check, which looks the window up by class name.
pub const CLASS_MAIN: &str = "NameShot.Main";
const CLASS_OVERLAY: &str = "NameShot.Overlay";
const CLASS_EDITOR: &str = "NameShot.Editor";

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Application state, shared by every window procedure. Leaked on purpose:
/// it lives for the whole process and is never mutably aliased.
pub struct App {
    /// Live settings; only replaced when the Settings window is confirmed.
    pub settings: RefCell<Settings>,
    pub settings_path: PathBuf,
    /// Hotkey currently registered with Windows (as restored on failure).
    pub registered_hotkey: Cell<HotkeySpec>,
    /// Last hotkey registration attempt succeeded.
    pub hotkey_ok: Cell<bool>,
    pub main_hwnd: Cell<HWND>,
    /// `TaskbarCreated` broadcast id (0 until registered).
    pub taskbar_created: Cell<u32>,
    pub tray: RefCell<Option<tray::TrayIcon>>,
    pub overlay: RefCell<Option<overlay::OverlaySession>>,
    pub editor: RefCell<Option<editor::EditorSession>>,
    pub settings_ui: RefCell<Option<settings_ui::SettingsSession>>,
    pub about_hwnd: Cell<HWND>,
    pub hotkey_hwnd: Cell<HWND>,
    pub hotkey_captured: Cell<Option<HotkeySpec>>,
    /// The in-place Text-tool edit currently owns keyboard focus.
    pub text_edit_active: Cell<bool>,
    pub font_ui: OwnedGdi<HFONT>,
    pub font_small: OwnedGdi<HFONT>,
    /// Title font, created on first About open (never at startup: most
    /// sessions never open About, and font creation is pure start-up cost).
    pub font_title: std::sync::OnceLock<OwnedGdi<HFONT>>,
}

impl App {
    /// Title font for the About window, created once on first use.
    pub fn font_title(&self) -> &OwnedGdi<HFONT> {
        self.font_title.get_or_init(|| {
            make_font(
                crate::theme::FONT_TITLE_PX,
                crate::theme::FONT_TITLE_WEIGHT,
            )
        })
    }
}

impl App {
    /// HWND of the open editor window, or a null handle when there is none.
    pub fn editor_hwnd(&self) -> HWND {
        self.editor_hwnd_opt().unwrap_or_default()
    }

    pub fn editor_hwnd_opt(&self) -> Option<HWND> {
        self.editor.borrow().as_ref().map(|s| s.hwnd)
    }
}

/// Recover the shared state from a window's `GWLP_USERDATA`.
pub fn app_of(hwnd: HWND) -> Option<&'static App> {
    unsafe {
        let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if p == 0 {
            None
        } else {
            Some(&*(p as *const App))
        }
    }
}

/// Register `spec` for the main window (Settings OK path).
pub fn register_hotkey(app: &App, spec: &HotkeySpec) -> Result<()> {
    win::hotkey::register(app.main_hwnd.get(), spec)
}

// ---------------------------------------------------------------------------
// Top-level actions
// ---------------------------------------------------------------------------

/// Start a capture session. An open overlay or editor is re-focused instead:
/// neither ever silently replaces a capture the user has not finished with.
pub fn start_capture(app: &'static App) {
    if overlay::focus_existing(app).is_some() {
        return;
    }
    if let Some(h) = app.editor_hwnd_opt() {
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    if let Err(e) = overlay::open(app) {
        let st = s();
        dialog::error_box(
            None,
            st.capture_fail_title,
            &fill(st.capture_fail_msg, "DETAIL", &e.to_string()),
        );
    }
}

/// "Quick save": the open inline editor's (or legacy editor window's) current
/// image, else a fresh capture, written to the configured folder under a name
/// that never overwrites.
fn quick_save(app: &'static App) {
    let st = s();
    // Deliberate action: a pending text draft commits before export so it
    // is included in the saved image.
    crate::overlay::edit::commit_any_text(app);
    // Whose frame is being written? A quick save of session work ends that
    // session on success — exactly the rule of its own Save button — while
    // a fresh capture of the desktop leaves any open session alone.
    let mut from_overlay = false;
    let mut from_editor = false;
    let inline: Option<Frame> = {
        let mut b = app.overlay.borrow_mut();
        b.as_mut()
            .and_then(|o| o.edit.as_mut())
            .map(|e| e.output_frame())
    };
    let frame = match inline {
        Some(f) => {
            from_overlay = true;
            f
        }
        None => {
            let edited = {
                let mut b = app.editor.borrow_mut();
                b.as_mut().map(|s| {
                    let items = s.history.items();
                    s.preview.render(items, None).clone()
                })
            };
            match edited {
                Some(f) => {
                    from_editor = true;
                    f
                }
                None => match crate::capture::capture_virtual_screen() {
                    Ok(c) => c.frame,
                    Err(e) => {
                        dialog::error_box(
                            None,
                            st.capture_fail_title,
                            &fill(st.capture_fail_msg, "DETAIL", &e.to_string()),
                        );
                        return;
                    }
                },
            }
        }
    };
    // The close policy is traced for every outcome of this save.
    let finish_route = |outcome: SaveOutcome| {
        if from_overlay {
            overlay::apply_route(app, Route::Save(outcome));
        }
        if from_editor {
            editor::apply_route(app, Route::Save(outcome));
        }
    };

    let settings = app.settings.borrow().clone();
    let dir = dialog::resolve_save_dir(&settings.save_dir);
    let _ = std::fs::create_dir_all(&dir);
    let name = crate::output::default_filename(settings.format, crate::os::local_time());
    let desired = dir.join(name);
    let unique = crate::output::unique_path(&desired.to_string_lossy(), |p| {
        std::path::Path::new(p).exists()
    });
    let path = std::path::PathBuf::from(unique);
    match crate::encode::encode_to_file(&path, &frame, settings.format, settings.jpeg_quality) {
        Ok(()) => {
            if let Some(t) = app.tray.borrow().as_ref() {
                t.balloon(st.state_saved, &path.to_string_lossy(), false);
            }
            finish_route(SaveOutcome::Saved);
        }
        Err(e) => {
            dialog::error_box(
                None,
                st.state_save_fail_title,
                &fill(st.state_save_fail, "DETAIL", &e.to_string()),
            );
            finish_route(SaveOutcome::Failed);
        }
    }
}

/// Tray menu "Exit": finalize the capture session (the close policy never
/// prompts and never saves), destroy every window we own, then the main
/// window (its `WM_DESTROY` quits the message loop).
fn exit_app(app: &'static App) {
    // Policy: Exit closes an open session without asking, then quits.
    overlay::apply_route(app, Route::Exit);

    // Handles first, borrows gone: the child window procedures re-enter App.
    let children = [
        app.settings_ui.borrow().as_ref().map(|s| s.hwnd),
        Some(app.hotkey_hwnd.get()),
        Some(app.about_hwnd.get()),
        app.overlay.borrow().as_ref().map(|s| s.hwnd),
        app.editor.borrow().as_ref().map(|s| s.hwnd),
    ];
    for h in children.into_iter().flatten() {
        if !h.0.is_null() && unsafe { IsWindow(Some(h)) }.as_bool() {
            unsafe {
                let _ = DestroyWindow(h);
            }
        }
    }
    let main = app.main_hwnd.get();
    if !main.0.is_null() {
        unsafe {
            let _ = DestroyWindow(main);
        }
    }
}

/// Show the tray context menu and run the chosen action. The menu pumps
/// messages, so everything it needs is read *before* it opens.
fn tray_menu(app: &'static App) {
    let main = app.main_hwnd.get();
    if main.0.is_null() {
        return;
    }
    let quick = app.settings.borrow().quick_save;
    unsafe {
        let _ = SetForegroundWindow(main);
    }
    let cmd = tray::show_menu(main, quick);
    unsafe {
        // Required so the menu dismisses cleanly on the next outside click.
        let _ = PostMessageW(Some(main), WM_NULL, WPARAM(0), LPARAM(0));
    }
    match cmd {
        tray::MenuCmd::Capture => start_capture(app),
        tray::MenuCmd::QuickSave => quick_save(app),
        tray::MenuCmd::Settings => {
            settings_ui::open(app);
        }
        tray::MenuCmd::About => {
            settings_ui::open_about(app);
        }
        tray::MenuCmd::Exit => exit_app(app),
        tray::MenuCmd::None => {}
    }
}

// ---------------------------------------------------------------------------
// Message loop
// ---------------------------------------------------------------------------

/// The single blocking message loop. Returns the process exit code carried
/// by `WM_QUIT`.
unsafe fn message_loop() -> i32 {
    let mut msg: MSG = std::mem::zeroed();
    loop {
        let n = GetMessageW(&mut msg, None, 0, 0).0;
        if n > 0 {
            if prefilter(&mut msg) {
                continue;
            }
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        } else if n == 0 {
            return msg.wParam.0 as i32;
        } else {
            return 1; // GetMessage failed (bad window filter)
        }
    }
}

/// Pre-dispatch hook.
///
/// Settings and About are plain windows that behave like dialogs: the
/// dialog manager (`IsDialogMessageW`) does Tab/arrow navigation, Enter
/// (= `IDOK`) and Esc (= `IDCANCEL`) — the window procedures handle those
/// command ids. The hotkey capture window is excluded on purpose: it must
/// see raw keys, and every key is routed to the window itself whatever
/// child control happens to have focus.
///
/// Returns `true` when the message was consumed and must not be dispatched.
unsafe fn prefilter(msg: &mut MSG) -> bool {
    match msg.message {
        WM_KEYDOWN | WM_KEYUP | WM_CHAR | WM_SYSKEYDOWN | WM_SYSKEYUP | WM_SYSCHAR => {}
        _ => return false,
    }
    if msg.hwnd.0.is_null() {
        return false;
    }
    let root = GetAncestor(msg.hwnd, GA_ROOT);
    if root.0.is_null() {
        return false;
    }
    if window_class_is(root, CLASS_HOTKEY) {
        msg.hwnd = root;
        return false;
    }
    if window_class_is(root, CLASS_SETTINGS) || window_class_is(root, CLASS_ABOUT) {
        return IsDialogMessageW(root, msg).as_bool();
    }
    false
}

/// Zero-alloc window-class check for the message prefilter, which runs on
/// every keystroke: compares the UTF-16 class name in place instead of
/// building a `String` per message.
fn window_class_is(hwnd: HWND, expected: &str) -> bool {
    unsafe {
        let mut buf = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut buf);
        if n <= 0 {
            return false;
        }
        expected.encode_utf16().cmp(buf[..n as usize].iter().copied())
            == std::cmp::Ordering::Equal
    }
}

// ---------------------------------------------------------------------------
// Window classes and fonts
// ---------------------------------------------------------------------------

fn register_classes() -> Result<()> {
    let cursor: HCURSOR = unsafe { LoadCursorW(None, IDC_ARROW) }
        .map_err(|e| Error::Win32(e).with_context("LoadCursorW"))?;
    let bg = dark::bg_brush();
    unsafe {
        register_class(
            CLASS_MAIN,
            Some(main_wndproc),
            cursor,
            HBRUSH::default(),
            false,
        )?;
        register_class(
            CLASS_OVERLAY,
            Some(crate::overlay::wndproc),
            cursor,
            HBRUSH::default(),
            true,
        )?;
        register_class(
            CLASS_EDITOR,
            Some(crate::editor::wndproc),
            cursor,
            HBRUSH::default(),
            true,
        )?;
        register_class(
            CLASS_SETTINGS,
            Some(settings_ui::settings_wndproc),
            cursor,
            bg,
            false,
        )?;
        register_class(
            CLASS_HOTKEY,
            Some(settings_ui::hotkey_wndproc),
            cursor,
            bg,
            false,
        )?;
        register_class(
            CLASS_ABOUT,
            Some(settings_ui::about_wndproc),
            cursor,
            bg,
            false,
        )?;
    }
    Ok(())
}

unsafe fn register_class(
    name: &str,
    proc: WNDPROC,
    cursor: HCURSOR,
    brush: HBRUSH,
    dblclicks: bool,
) -> Result<()> {
    let n = win::wide(name);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: if dblclicks {
            CS_DBLCLKS
        } else {
            WNDCLASS_STYLES(0)
        },
        lpfnWndProc: proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: gdi::this_module(),
        hIcon: crate::win::app_icon(false),
        hCursor: cursor,
        hbrBackground: brush,
        lpszMenuName: PCWSTR::null(),
        lpszClassName: PCWSTR(n.as_ptr()),
        hIconSm: crate::win::app_icon(true),
    };
    if RegisterClassExW(&wc) == 0 {
        return Err(crate::os::last_error("RegisterClassExW"));
    }
    Ok(())
}

/// UI font at `px` physical pixels with a `FW_*` weight; falls back to the
/// stock GUI font.
fn make_font(px: i32, weight: i32) -> OwnedGdi<HFONT> {
    unsafe {
        let font = CreateFontW(
            -px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            ANSI_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            w!("Segoe UI"),
        );
        if let Some(f) = OwnedGdi::new(font, font.is_invalid()) {
            return f;
        }
        let stock = GetStockObject(DEFAULT_GUI_FONT);
        OwnedGdi(HFONT(stock.0))
    }
}

// ---------------------------------------------------------------------------
// Start-up
// ---------------------------------------------------------------------------

/// Process entry point: returns the exit code.
pub fn run() -> i32 {
    // WIC and the modern file dialogs are COM objects and must run on an
    // initialized thread. STA matches the single UI thread. RPC_E_CHANGED_MODE
    // only means the apartment was already chosen (e.g. by the shell) — COM
    // is initialized either way, so every result is safe to ignore.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    crate::os::enable_process_dpi_awareness();

    if let Some(msg) = crate::os::unsupported_os_message() {
        dialog::error_box(None, s().os_unsupported_title, &msg);
        return 1;
    }

    // One instance only: a second copy just points at the tray icon.
    let cls = win::wide(CLASS_MAIN);
    unsafe {
        if let Ok(existing) = FindWindowW(PCWSTR(cls.as_ptr()), PCWSTR::null()) {
            if !existing.0.is_null() {
                let _ = SetForegroundWindow(existing);
                dialog::info_box(None, s().already_running_title, s().already_running_msg);
                return 0;
            }
        }
    }

    let settings_path = crate::config::settings_path();
    let settings = crate::config::load_or_default(&settings_path);
    let spec = settings.hotkey;

    if let Err(e) = register_classes() {
        dialog::error_box(None, s().app_name, &e.to_string());
        return 1;
    }

    let app: &'static App = Box::leak(Box::new(App {
        settings: RefCell::new(settings),
        settings_path,
        registered_hotkey: Cell::new(spec),
        hotkey_ok: Cell::new(false),
        main_hwnd: Cell::new(HWND::default()),
        taskbar_created: Cell::new(0),
        tray: RefCell::new(None),
        overlay: RefCell::new(None),
        editor: RefCell::new(None),
        settings_ui: RefCell::new(None),
        about_hwnd: Cell::new(HWND::default()),
        hotkey_hwnd: Cell::new(HWND::default()),
        hotkey_captured: Cell::new(None),
        text_edit_active: Cell::new(false),
        font_ui: make_font(crate::theme::FONT_UI_PX, FW_NORMAL.0 as i32),
        font_small: make_font(crate::theme::FONT_SMALL_PX, FW_NORMAL.0 as i32),
        font_title: std::sync::OnceLock::new(),
    }));

    // Hidden main window: the tray target, hotkey target and message hub.
    let title = win::wide(s().app_name);
    let main = match unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(cls.as_ptr()),
            PCWSTR(title.as_ptr()),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(gdi::this_module()),
            Some(app as *const App as *const core::ffi::c_void),
        )
    } {
        Ok(h) => h,
        Err(e) => {
            dialog::error_box(
                None,
                s().app_name,
                &Error::Win32(e)
                    .with_context("CreateWindowExW(main)")
                    .to_string(),
            );
            return 1;
        }
    };
    app.main_hwnd.set(main);

    match tray::TrayIcon::add(main) {
        Ok(t) => *app.tray.borrow_mut() = Some(t),
        Err(e) => dialog::warn_box(None, s().app_name, &e.to_string()),
    }

    app.taskbar_created
        .set(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });

    let st = s();
    match win::hotkey::register(main, &spec) {
        Ok(()) => {
            app.registered_hotkey.set(spec);
            app.hotkey_ok.set(true);
        }
        Err(e) => {
            app.hotkey_ok.set(false);
            if win::hotkey::is_conflict(&e) {
                dialog::warn_box(
                    None,
                    st.hotkey_conflict_title,
                    &fill(st.hotkey_conflict_msg, "H", &spec.format()),
                );
            } else {
                dialog::warn_box(None, st.hotkey_unavailable_title, &e.to_string());
            }
        }
    }

    unsafe { message_loop() }
}

// ---------------------------------------------------------------------------
// Main window procedure
// ---------------------------------------------------------------------------

unsafe extern "system" fn main_wndproc(
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
    let Some(app) = app_of(hwnd) else {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    };

    if msg == WM_HOTKEY {
        if wparam.0 == win::hotkey::HOTKEY_ID as usize {
            start_capture(app);
        }
        return LRESULT(0);
    }

    if msg == WM_APP_TRAY {
        match (lparam.0 & 0xFFFF) as u32 {
            WM_LBUTTONUP | NIN_SELECT | NIN_KEYSELECT => start_capture(app),
            WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(app),
            _ => {}
        }
        return LRESULT(0);
    }

    let broadcast = app.taskbar_created.get();
    if broadcast != 0 && msg == broadcast {
        if let Some(t) = app.tray.borrow_mut().as_mut() {
            t.readd();
        }
        return LRESULT(0);
    }

    match msg {
        WM_DESTROY => {
            win::hotkey::unregister(hwnd);
            app.tray.take();
            app.overlay.take();
            app.editor.take();
            app.settings_ui.take();
            app.text_edit_active.set(false);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
