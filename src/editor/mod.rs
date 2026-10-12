//! Annotation editor window: dark custom-drawn toolbar, zoomable canvas,
//! tools (pen/line/arrow/rect/box/text/highlight/mosaic/redact), undo/redo,
//! copy to clipboard and native Save.
//!
//! Single top-level window that paints everything itself (toolbar, canvas,
//! status bar). The only child window is the transient single-line `EDIT` used
//! by the Text tool, subclassed so Enter commits and Esc cancels. Zoom/pan
//! keep coordinates in image space; the flattened frame comes from
//! [`crate::flatten::Preview`] and is cloned right before each clipboard/file
//! call. No Win32 call that can pump messages runs while the session borrow
//! is held.

pub mod toolbar;

use crate::annotate::{History, Shape, Tool};
use crate::app::App;
use crate::error::{Error, Result};
use crate::flatten::Preview;
use crate::image::{Frame, IRect, Rgba};
use crate::session::{self, Route, SaveOutcome};
use crate::strings::{fill, s};
use crate::win::WM_APP_STATUS;
use crate::win::dialog;
use crate::win::gdi::{OwnedGdi, draw_label, fill_solid, frame_solid};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ANSI_CHARSET, ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint,
    CLIP_DEFAULT_PRECIS, CreateFontW, DIB_RGB_COLORS, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT,
    DT_SINGLELINE, DeleteObject, EndPaint, FW_NORMAL, HALFTONE, HFONT, IntersectClipRect,
    InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, RestoreDC, SRCCOPY, SaveDC, ScreenToClient,
    SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchDIBits,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    ES_AUTOHSCROLL, GWLP_USERDATA, GetClientRect, GetCursorPos, GetWindowTextW, IDC_ARROW,
    IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_SIZEALL, IsWindow, MINMAXINFO, SW_SHOW, SWP_NOACTIVATE,
    SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetCursor, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, WINDOW_STYLE, WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_PAINT, WM_SETCURSOR,
    WM_SETFOCUS, WM_SETFONT, WM_SIZE, WS_BORDER, WS_CHILD, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
    WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

/// Toolbar strip height (re-exported for layout math elsewhere).
pub const TOOLBAR_H: i32 = toolbar::H;
/// Status bar height at the bottom of the editor.
pub const STATUS_H: i32 = 26;

/// Eight-color annotation palette (row 2 of the toolbar).
pub const PALETTE: [Rgba; 8] = [
    Rgba::opaque(255, 255, 255),
    Rgba::opaque(20, 22, 26),
    Rgba::opaque(230, 76, 76),
    Rgba::opaque(245, 180, 66),
    Rgba::opaque(96, 190, 96),
    Rgba::opaque(86, 140, 240),
    Rgba::opaque(64, 190, 175),
    Rgba::opaque(196, 110, 220),
];
/// Stroke widths for pen/line/arrow/box tools.
pub const WIDTHS: [u32; 3] = [2, 4, 8];
/// Font sizes for the Text tool (mapped from the same S/M/L selector).
pub const TEXT_SIZES: [u32; 3] = [16, 24, 36];
/// Block sizes for the Mosaic redaction tool.
pub const BLOCKS: [u32; 3] = [8, 16, 32];

const MIN_ZOOM: f32 = 1.0 / 16.0;
const MAX_ZOOM: f32 = 16.0;
const ZOOM_STEPS: [f32; 11] = [
    0.0625, 0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 1.5, 2.0, 4.0, 8.0,
];

// Mouse-key flags (MK_*) live in SystemServices, which we do not enable —
// the values are stable ABI constants.
const MK_SHIFT_FL: u16 = 0x0004;
const MK_LBUTTON_FL: u16 = 0x0001;

const VK_C: u16 = 0x43;
const VK_S: u16 = 0x53;
const VK_Z: u16 = 0x5A;
const VK_Y: u16 = 0x59;
const VK_CONTROL: u16 = 0x11;
const VK_OEM_PLUS: u16 = 0xBB;
const VK_OEM_MINUS: u16 = 0xBD;
const VK_0: u16 = 0x30;
const VK_ESCAPE: u16 = 0x1B;
const VK_RETURN: u16 = 0x0D;
const VK_SPACE: u16 = 0x20;
const VK_TAB: u16 = 0x09;
const VK_LEFT: u16 = 0x25;
const VK_UP: u16 = 0x26;
const VK_RIGHT: u16 = 0x27;
const VK_DOWN: u16 = 0x28;
const VK_HOME: u16 = 0x24;
const SUBCLASS_ID: usize = 0x4544; // 'ED'

pub enum Drag {
    Shape { anchor: (i32, i32) },
    Pen { pts: Vec<(i32, i32)> },
    Pan { sx: i32, sy: i32, vx: f32, vy: f32 },
}

pub struct EditorSession {
    pub hwnd: HWND,
    pub preview: Preview,
    pub history: History,
    pub tool: Tool,
    pub color_idx: usize,
    pub width_idx: usize,
    pub zoom: f32,
    /// Image-space coordinate at the canvas' top-left (allows centering).
    pub view: (f32, f32),
    /// True while the view follows a "fit to window" computation.
    pub fit_mode: bool,
    pub draft: Option<Shape>,
    pub drag: Option<Drag>,
    pub hover: Option<usize>,
    pub focus: usize,
    /// Keyboard focus is on this window (draw focus rings only then).
    pub kb: bool,
    pub status: String,
    /// Open Text-tool edit: child HWND + image-space anchor.
    pub text_edit: Option<(HWND, (i32, i32))>,
    text_font: Option<OwnedGdi<HFONT>>,
    pub toolbar: toolbar::Toolbar,
    pub tb_rect: RECT,
    pub canvas: RECT,
    pub status_rect: RECT,
    client_w: i32,
    client_h: i32,
}

impl EditorSession {
    fn state(&self) -> toolbar::State {
        toolbar::State {
            tool: self.tool,
            color_idx: self.color_idx,
            width_idx: self.width_idx,
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
        }
    }

    fn color(&self) -> Rgba {
        PALETTE[self.color_idx.min(PALETTE.len() - 1)]
    }

    fn stroke_w(&self) -> u32 {
        WIDTHS[self.width_idx.min(WIDTHS.len() - 1)]
    }

    /// Recompute rectangles, toolbar layout and the view for the client size.
    fn relayout(&mut self, cw: i32, ch: i32) {
        self.client_w = cw;
        self.client_h = ch;
        self.tb_rect = RECT {
            left: 0,
            top: 0,
            right: cw,
            bottom: TOOLBAR_H,
        };
        let status_top = (ch - STATUS_H).max(TOOLBAR_H);
        self.status_rect = RECT {
            left: 0,
            top: status_top,
            right: cw,
            bottom: ch,
        };
        self.canvas = RECT {
            left: 0,
            top: TOOLBAR_H,
            right: cw,
            bottom: status_top,
        };
        let st = self.state();
        self.toolbar = toolbar::Toolbar::layout(cw, st);
        let (cwx, cwy) = (self.canvas_w(), self.canvas_h());
        if self.fit_mode {
            self.fit(cwx, cwy);
        } else {
            self.clamp_view(cwx, cwy);
        }
        self.position_text_edit();
    }

    fn canvas_w(&self) -> i32 {
        (self.canvas.right - self.canvas.left).max(1)
    }

    fn canvas_h(&self) -> i32 {
        (self.canvas.bottom - self.canvas.top).max(1)
    }

    fn fit(&mut self, cw: i32, ch: i32) {
        let sw = self.preview.source().w.max(1) as f32;
        let sh = self.preview.source().h.max(1) as f32;
        let z = (((cw - 32).max(1) as f32) / sw)
            .min(((ch - 32).max(1) as f32) / sh)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        self.zoom = z;
        self.view = ((sw - cw as f32 / z) / 2.0, (sh - ch as f32 / z) / 2.0);
        self.fit_mode = true;
    }

    fn clamp_view(&mut self, cw: i32, ch: i32) {
        let sw = self.preview.source().w as f32;
        let sh = self.preview.source().h as f32;
        let vw = cw as f32 / self.zoom;
        let vh = ch as f32 / self.zoom;
        self.view.0 = if sw <= vw {
            (sw - vw) / 2.0
        } else {
            self.view.0.clamp(0.0, sw - vw)
        };
        self.view.1 = if sh <= vh {
            (sh - vh) / 2.0
        } else {
            self.view.1.clamp(0.0, sh - vh)
        };
    }

    /// Client point -> image-space float coordinate.
    fn dev_to_img(&self, x: i32, y: i32) -> (f32, f32) {
        (
            self.view.0 + (x - self.canvas.left) as f32 / self.zoom,
            self.view.1 + (y - self.canvas.top) as f32 / self.zoom,
        )
    }

    /// Image-space point -> client (device) coordinate.
    fn img_to_dev(&self, ix: f32, iy: f32) -> (i32, i32) {
        (
            self.canvas.left + ((ix - self.view.0) * self.zoom).round() as i32,
            self.canvas.top + ((iy - self.view.1) * self.zoom).round() as i32,
        )
    }

    fn position_text_edit(&self) {
        if let Some((h, at)) = self.text_edit {
            let (x, y) = self.img_to_dev(at.0 as f32, at.1 as f32);
            unsafe {
                let _ = SetWindowPos(
                    h,
                    None,
                    x - 2,
                    y - 2,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
}

/// True when every sampled pixel is (nearly) black — the usual signature of a
/// protected surface (secure desktop / DRM) that GDI could not read.
pub(crate) fn looks_secure(f: &Frame) -> bool {
    let w = f.w as i32;
    let h = f.h as i32;
    if w < 16 || h < 16 {
        return false;
    }
    let mut n = 0;
    let mut black = 0;
    for i in 0..64 {
        let x = (i % 8) * w / 8 + w / 16;
        let y = (i / 8) * h / 8 + h / 16;
        if let Some(p) = f.get(x, y) {
            n += 1;
            if p[0] < 8 && p[1] < 8 && p[2] < 8 {
                black += 1;
            }
        }
    }
    n > 0 && black == n
}

/// Open the editor on `src` (already cropped). Returns the window handle.
pub fn open(app: &'static App, src: Frame) -> Result<HWND> {
    if let Some(existing) = app.editor.borrow().as_ref() {
        let h = existing.hwnd;
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        return Ok(h);
    }

    let secure = looks_secure(&src);
    let iw = src.w as i32;
    let ih = src.h as i32;

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
    let aw = (wa.right - wa.left).max(320);
    let ah = (wa.bottom - wa.top).max(240);
    let cw = (iw + 72).max(760).min(aw * 92 / 100);
    let ch = (ih + TOOLBAR_H + STATUS_H + 56).max(500).min(ah * 92 / 100);

    let mut outer = RECT {
        left: 0,
        top: 0,
        right: cw,
        bottom: ch,
    };
    unsafe {
        AdjustWindowRectEx(&mut outer, WS_OVERLAPPEDWINDOW, false, WS_EX_APPWINDOW)
            .map_err(|e| Error::Win32(e).with_context("AdjustWindowRectEx"))?;
    }
    let ow = (outer.right - outer.left).max(1);
    let oh = (outer.bottom - outer.top).max(1);
    let x = wa.left + (aw - ow) / 2;
    let y = wa.top + (ah - oh) / 2;

    let session = EditorSession {
        hwnd: HWND::default(),
        preview: Preview::new(src),
        history: History::new(),
        tool: Tool::Pen,
        color_idx: 2,
        width_idx: 1,
        zoom: 1.0,
        view: (0.0, 0.0),
        fit_mode: true,
        draft: None,
        drag: None,
        hover: None,
        focus: 0,
        kb: false,
        status: if secure {
            s().secure_surface_msg.to_string()
        } else {
            String::new()
        },
        text_edit: None,
        text_font: None,
        toolbar: toolbar::Toolbar::layout(
            cw,
            toolbar::State {
                tool: Tool::Pen,
                color_idx: 2,
                width_idx: 1,
                can_undo: false,
                can_redo: false,
            },
        ),
        tb_rect: RECT::default(),
        canvas: RECT::default(),
        status_rect: RECT::default(),
        client_w: cw,
        client_h: ch,
    };
    app.editor.replace(Some(session));

    let title = crate::win::wide(s().editor_w);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW,
            w!("NameShot.Editor"),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            ow,
            oh,
            None,
            None,
            Some(crate::win::gdi::this_module()),
            Some(app as *const App as *const core::ffi::c_void),
        )
        .map_err(|e| {
            app.editor.take();
            Error::Win32(e).with_context("CreateWindowExW(editor)")
        })?
    };

    if let Some(sess) = app.editor.borrow_mut().as_mut() {
        sess.hwnd = hwnd;
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        sess.relayout((rc.right - rc.left).max(1), (rc.bottom - rc.top).max(1));
    }
    crate::win::dark::set_dark_titlebar(hwnd, true);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
    Ok(hwnd)
}

// ---------------------------------------------------------------------------
// Window procedure
// ---------------------------------------------------------------------------

pub unsafe extern "system" fn wndproc(
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
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            let cw = (lparam.0 & 0xFFFF) as u16 as i32;
            let ch = ((lparam.0 >> 16) & 0xFFFF) as u16 as i32;
            if cw > 0 && ch > 0 {
                if let Some(s) = app.editor.borrow_mut().as_mut() {
                    s.relayout(cw, ch);
                }
            }
            invalidate_all(hwnd);
            LRESULT(0)
        }
        WM_PAINT => {
            paint(app, hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            on_lbutton_down(app, hwnd, lparam);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            on_lbutton_up(app, lparam);
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => LRESULT(0),
        WM_MBUTTONDOWN => {
            let (x, y) = (mx(lparam), my(lparam));
            let over_canvas = app
                .editor
                .borrow()
                .as_ref()
                .map(|s| y >= s.canvas.top && y < s.canvas.bottom)
                .unwrap_or(false);
            if over_canvas {
                let view = app
                    .editor
                    .borrow()
                    .as_ref()
                    .map(|s| (s.view.0, s.view.1))
                    .unwrap_or((0.0, 0.0));
                if let Some(s) = app.editor.borrow_mut().as_mut() {
                    s.drag = Some(Drag::Pan {
                        sx: x,
                        sy: y,
                        vx: view.0,
                        vy: view.1,
                    });
                    s.draft = None;
                    s.fit_mode = false;
                }
                unsafe {
                    SetCapture(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_MBUTTONUP => {
            let was_pan = matches!(
                app.editor.borrow().as_ref().and_then(|s| s.drag.as_ref()),
                Some(Drag::Pan { .. })
            );
            if was_pan {
                if let Some(s) = app.editor.borrow_mut().as_mut() {
                    s.drag = None;
                }
                unsafe {
                    let _ = ReleaseCapture();
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            on_mouse_move(app, hwnd, lparam);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let changed = app
                .editor
                .borrow()
                .as_ref()
                .map(|s| s.hover.is_some())
                .unwrap_or(false);
            if changed {
                if let Some(s) = app.editor.borrow_mut().as_mut() {
                    s.hover = None;
                }
                invalidate_tb(app, hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            on_wheel(app, wparam, lparam);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            let active = app
                .editor
                .borrow()
                .as_ref()
                .map(|s| s.drag.is_some())
                .unwrap_or(false);
            if active {
                if let Some(s) = app.editor.borrow_mut().as_mut() {
                    s.drag = None;
                    s.draft = None;
                }
                invalidate_canvas(app, hwnd);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            on_key(app, hwnd, wparam);
            LRESULT(0)
        }
        WM_SETFOCUS => {
            if let Some(s) = app.editor.borrow_mut().as_mut() {
                s.kb = true;
            }
            invalidate_tb(app, hwnd);
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            if let Some(s) = app.editor.borrow_mut().as_mut() {
                s.kb = false;
            }
            invalidate_tb(app, hwnd);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let idc = cursor_for(app, lparam);
            let cur = get_cursor_w(idc);
            SetCursor(Some(cur));
            LRESULT(1)
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
            mmi.ptMinTrackSize.x = 640;
            mmi.ptMinTrackSize.y = 480;
            LRESULT(0)
        }
        WM_CLOSE => {
            // Explicit close: discard immediately, no confirmation.
            apply_route(app, Route::Close);
            LRESULT(0)
        }
        WM_DESTROY => {
            let owns = app
                .editor
                .borrow()
                .as_ref()
                .map(|s| s.hwnd == hwnd)
                .unwrap_or(false);
            if owns {
                app.text_edit_active.set(false);
                app.editor.take();
            }
            LRESULT(0)
        }
        WM_APP_STATUS => {
            invalidate_all(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Invalidation helpers (region-limited so hover does not repaint the canvas)
// ---------------------------------------------------------------------------

fn invalidate_tb(app: &App, _hwnd: HWND) {
    let target = {
        let b = app.editor.borrow();
        b.as_ref().map(|s| (s.hwnd, s.tb_rect))
    };
    if let Some((h, r)) = target {
        unsafe {
            let _ = InvalidateRect(Some(h), Some(&raw const r), false);
        }
    }
}

fn invalidate_canvas(app: &App, _hwnd: HWND) {
    let target = {
        let b = app.editor.borrow();
        b.as_ref().map(|s| (s.hwnd, s.canvas))
    };
    if let Some((h, r)) = target {
        unsafe {
            let _ = InvalidateRect(Some(h), Some(&raw const r), false);
        }
    }
}

fn invalidate_all(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

// ---------------------------------------------------------------------------
// Input handling
// ---------------------------------------------------------------------------

#[inline]
fn mx(lp: LPARAM) -> i32 {
    (lp.0 as u16 as i16) as i32
}

#[inline]
fn my(lp: LPARAM) -> i32 {
    (((lp.0 >> 16) & 0xFFFF) as u16 as i16) as i32
}

#[inline]
fn key_down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

fn on_lbutton_down(app: &'static App, hwnd: HWND, lparam: LPARAM) {
    let (x, y) = (mx(lparam), my(lparam));

    let on_toolbar = app
        .editor
        .borrow()
        .as_ref()
        .map(|s| y >= s.tb_rect.top && y < s.tb_rect.bottom)
        .unwrap_or(false);
    if on_toolbar {
        let hit = {
            let b = app.editor.borrow();
            b.as_ref()
                .and_then(|s| s.toolbar.hit(x, y).map(|i| (i, s.toolbar.buttons[i].cmd)))
        };
        let Some((idx, cmd)) = hit else { return };
        commit_any_text(app);
        if let Some(s) = app.editor.borrow_mut().as_mut() {
            s.focus = idx;
            s.kb = true;
            s.hover = Some(idx);
        }
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
        invalidate_tb(app, hwnd);
        command(app, cmd);
        return;
    }

    let over_canvas = app
        .editor
        .borrow()
        .as_ref()
        .map(|s| {
            x >= s.canvas.left && x < s.canvas.right && y >= s.canvas.top && y < s.canvas.bottom
        })
        .unwrap_or(false);
    if !over_canvas {
        return;
    }

    commit_any_text(app);

    let (img, tool, space) = {
        let b = app.editor.borrow();
        let Some(s) = b.as_ref() else { return };
        let (fx, fy) = s.dev_to_img(x, y);
        (
            (fx.round() as i32, fy.round() as i32),
            s.tool,
            key_down(VK_SPACE),
        )
    };

    if space {
        let view = app
            .editor
            .borrow()
            .as_ref()
            .map(|s| (s.view.0, s.view.1))
            .unwrap_or((0.0, 0.0));
        if let Some(s) = app.editor.borrow_mut().as_mut() {
            s.drag = Some(Drag::Pan {
                sx: x,
                sy: y,
                vx: view.0,
                vy: view.1,
            });
            s.draft = None;
            s.fit_mode = false;
        }
        unsafe {
            SetCapture(hwnd);
        }
        return;
    }

    if tool == Tool::Text {
        begin_text(app, (x, y), img);
        return;
    }

    if let Some(s) = app.editor.borrow_mut().as_mut() {
        s.drag = Some(match tool {
            Tool::Pen => Drag::Pen { pts: vec![img] },
            _ => Drag::Shape { anchor: img },
        });
        s.draft = None;
    }
    unsafe {
        SetCapture(hwnd);
    }
}

fn on_lbutton_up(app: &'static App, lparam: LPARAM) {
    let (x, y) = (mx(lparam), my(lparam));
    let drag = {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        s.drag.take()
    };
    let Some(drag) = drag else { return };
    unsafe {
        let _ = ReleaseCapture();
    }

    let mut document_changed = false;
    {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        match drag {
            Drag::Pan { .. } => {
                s.draft = None;
            }
            Drag::Pen { pts } => {
                if pts.len() >= 2 {
                    let shape = Shape::Pen {
                        pts,
                        color: s.color(),
                        width: s.stroke_w(),
                    };
                    s.history.push(shape);
                    s.status.clear();
                    document_changed = true;
                }
            }
            Drag::Shape { anchor } => {
                let (fx, fy) = s.dev_to_img(x, y);
                let img = (fx.round() as i32, fy.round() as i32);
                let shape = make_draft(s.tool, anchor, img, s.color(), s.width_idx);
                s.draft = None;
                if let Some(shape) = shape {
                    if worth(&shape) {
                        s.history.push(shape);
                        s.status.clear();
                        document_changed = true;
                    }
                }
            }
        }
        if document_changed {
            let (cw, ch) = (s.client_w, s.client_h);
            s.relayout(cw, ch);
        }
    }
    invalidate_canvas(app, app.editor_hwnd());
    invalidate_tb(app, app.editor_hwnd());
}

fn on_mouse_move(app: &'static App, hwnd: HWND, lparam: LPARAM) {
    let (x, y) = (mx(lparam), my(lparam));

    unsafe {
        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        let _ = TrackMouseEvent(&mut tme);
    }

    // Hover feedback on the toolbar.
    let hover = {
        let b = app.editor.borrow();
        b.as_ref().and_then(|s| {
            if y >= s.tb_rect.top && y < s.tb_rect.bottom {
                s.toolbar.hit(x, y)
            } else {
                None
            }
        })
    };
    let prev = app.editor.borrow().as_ref().and_then(|s| s.hover);
    if prev != hover {
        if let Some(s) = app.editor.borrow_mut().as_mut() {
            s.hover = hover;
        }
        invalidate_tb(app, hwnd);
    }

    let dragging = app
        .editor
        .borrow()
        .as_ref()
        .map(|s| s.drag.is_some())
        .unwrap_or(false);
    if !dragging {
        return;
    }

    // Compute the image point first (needs an immutable borrow).
    let img = {
        let b = app.editor.borrow();
        let Some(s) = b.as_ref() else { return };
        let (fx, fy) = s.dev_to_img(x, y);
        (fx.round() as i32, fy.round() as i32)
    };
    let panning = matches!(
        app.editor.borrow().as_ref().and_then(|s| s.drag.as_ref()),
        Some(Drag::Pan { .. })
    );

    // Repaint only the union of the previous and current draft bounds
    // (image space → client space, padded, clipped to the canvas). Panning
    // moves the whole view and keeps the full-canvas invalidation.
    // Degenerate drags repaint nothing at all.
    let dirty: Option<RECT> = {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        let old = s.draft.as_ref().map(|d| d.bounds());
        let mut pan = false;
        match s.drag.as_mut() {
            Some(Drag::Pan { sx, sy, vx, vy }) => {
                let _ = panning;
                s.view.0 = *vx - (x - *sx) as f32 / s.zoom;
                s.view.1 = *vy - (y - *sy) as f32 / s.zoom;
                s.fit_mode = false;
                let (cw, ch) = (s.canvas_w(), s.canvas_h());
                s.clamp_view(cw, ch);
                pan = true;
            }
            Some(Drag::Shape { anchor }) => {
                let anchor = *anchor;
                let (tool, color, wi) = (s.tool, s.color(), s.width_idx);
                s.draft = make_draft(tool, anchor, img, color, wi);
            }
            Some(Drag::Pen { pts }) => {
                let last = pts.last().copied();
                let mut added = false;
                if let Some((lx, ly)) = last {
                    if (lx - img.0).abs() >= 2 || (ly - img.1).abs() >= 2 {
                        pts.push(img);
                        added = true;
                    }
                } else {
                    pts.push(img);
                    added = true;
                }
                // Rebuild the draft only when the stroke actually grew: no
                // per-mousemove Vec clone for a stationary pointer.
                if added {
                    let pts = pts.clone();
                    let (color, w) = (s.color(), s.stroke_w());
                    s.draft = Some(Shape::Pen {
                        pts,
                        color,
                        width: w,
                    });
                }
            }
            None => {}
        }
        if pan {
            return invalidate_canvas(app, hwnd);
        }
        let new = s.draft.as_ref().map(|d| d.bounds());
        let u: Option<IRect> = match (old, new) {
            (Some(a), Some(b)) => {
                let x = a.x.min(b.x);
                let y = a.y.min(b.y);
                Some(IRect::new(
                    x,
                    y,
                    a.right().max(b.right()) - x,
                    a.bottom().max(b.bottom()) - y,
                ))
            }
            (Some(a), None) => Some(a),
            (None, b) => b,
        };
        u.map(|r| {
            let (x0, y0) = s.img_to_dev(r.x as f32, r.y as f32);
            let (x1, y1) = s.img_to_dev(r.right() as f32, r.bottom() as f32);
            let c = &s.canvas;
            RECT {
                left: (x0.min(x1) - 2).max(c.left),
                top: (y0.min(y1) - 2).max(c.top),
                right: (x0.max(x1) + 2).min(c.right),
                bottom: (y0.max(y1) + 2).min(c.bottom),
            }
        })
        .filter(|r| r.right > r.left && r.bottom > r.top)
    };
    if let Some(r) = dirty {
        invalidate_rect(app, hwnd, r);
    }
}

/// Invalidate an arbitrary client rect of the editor window.
fn invalidate_rect(app: &App, _hwnd: HWND, r: RECT) {
    let h = app.editor.borrow().as_ref().map(|s| s.hwnd);
    if let Some(h) = h {
        unsafe {
            let _ = InvalidateRect(Some(h), Some(&raw const r), false);
        }
    }
}

fn on_wheel(app: &'static App, wparam: WPARAM, lparam: LPARAM) {
    let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    if delta == 0 {
        return;
    }
    let flags = (wparam.0 & 0xFFFF) as u16;
    let mut pt = POINT {
        x: mx(lparam),
        y: my(lparam),
    };
    let hwnd = { app.editor.borrow().as_ref().map(|s| s.hwnd) };
    let Some(hwnd) = hwnd else { return };
    unsafe {
        let _ = ScreenToClient(hwnd, &mut pt);
    }

    {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        if flags & MK_SHIFT_FL != 0 {
            s.view.0 -= delta as f32 / 120.0 * 96.0 / s.zoom;
            s.fit_mode = false;
            let (cw, ch) = (s.canvas_w(), s.canvas_h());
            s.clamp_view(cw, ch);
        } else {
            let factor = if delta > 0 { 1.25 } else { 0.8 };
            let new_zoom = (s.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
            if (new_zoom - s.zoom).abs() < f32::EPSILON {
                return;
            }
            let anchor = s.dev_to_img(pt.x, pt.y);
            let (ox, oy) = (s.canvas.left, s.canvas.top);
            s.zoom = new_zoom;
            s.view.0 = anchor.0 - (pt.x - ox) as f32 / new_zoom;
            s.view.1 = anchor.1 - (pt.y - oy) as f32 / new_zoom;
            s.fit_mode = false;
            let (cw, ch) = (s.canvas_w(), s.canvas_h());
            s.clamp_view(cw, ch);
            s.position_text_edit();
        }
    }
    invalidate_canvas(app, hwnd);
}

fn on_key(app: &'static App, hwnd: HWND, wparam: WPARAM) {
    let vk = (wparam.0 as u32 & 0xFF) as u16;
    let ctrl = key_down(VK_CONTROL);
    let shift = key_down(0x10); // VK_SHIFT

    match vk {
        VK_C if ctrl => command(app, toolbar::Cmd::Copy),
        VK_S if ctrl => command(app, toolbar::Cmd::Save),
        VK_Z if ctrl => command(
            app,
            if shift {
                toolbar::Cmd::Redo
            } else {
                toolbar::Cmd::Undo
            },
        ),
        VK_Y if ctrl => command(app, toolbar::Cmd::Redo),
        VK_OEM_PLUS | VK_0 if ctrl && vk == VK_OEM_PLUS => command(app, toolbar::Cmd::ZoomIn),
        VK_OEM_MINUS if ctrl => command(app, toolbar::Cmd::ZoomOut),
        VK_0 if ctrl => command(app, toolbar::Cmd::ZoomFit),
        VK_ESCAPE => {
            let _ = apply_route(app, Route::Close);
        }
        VK_TAB => {
            let n = app
                .editor
                .borrow()
                .as_ref()
                .map(|s| s.toolbar.buttons.len())
                .unwrap_or(0);
            if n > 0 {
                if let Some(s) = app.editor.borrow_mut().as_mut() {
                    s.focus = if shift {
                        (s.focus + n - 1) % n
                    } else {
                        (s.focus + 1) % n
                    };
                    s.kb = true;
                }
                invalidate_tb(app, hwnd);
            }
        }
        VK_RETURN => {
            let cmd = {
                let b = app.editor.borrow();
                b.as_ref()
                    .and_then(|s| s.toolbar.buttons.get(s.focus).map(|b| b.cmd))
            };
            if let Some(cmd) = cmd {
                command(app, cmd);
            }
        }
        VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN => {
            let mut b = app.editor.borrow_mut();
            let Some(s) = b.as_mut() else { return };
            let step = 48.0 / s.zoom;
            match vk {
                VK_LEFT => s.view.0 -= step,
                VK_RIGHT => s.view.0 += step,
                VK_UP => s.view.1 -= step,
                _ => s.view.1 += step,
            }
            s.fit_mode = false;
            let (cw, ch) = (s.canvas_w(), s.canvas_h());
            s.clamp_view(cw, ch);
            s.position_text_edit();
            drop(b);
            invalidate_canvas(app, hwnd);
        }
        VK_HOME => command(app, toolbar::Cmd::ZoomFit),
        _ => {
            let _ = MK_LBUTTON_FL; // keep the const referenced for future use
        }
    }
}

fn cursor_for(app: &App, lparam: LPARAM) -> windows::core::PCWSTR {
    let y = my(lparam);
    let info = app.editor.borrow().as_ref().map(|s| {
        (
            s.tb_rect,
            s.tool,
            s.drag.is_some(),
            matches!(s.drag.as_ref(), Some(Drag::Pan { .. })),
        )
    });
    let Some((tb, tool, dragging, panning)) = info else {
        return IDC_CROSS;
    };
    if y >= tb.top && y < tb.bottom {
        return IDC_ARROW;
    }
    if panning || (dragging && key_down(VK_SPACE)) {
        return IDC_SIZEALL;
    }
    if key_down(VK_SPACE) {
        return IDC_HAND;
    }
    match tool {
        Tool::Text => IDC_IBEAM,
        _ => IDC_CROSS,
    }
}

fn get_cursor_w(idc: windows::core::PCWSTR) -> windows::Win32::UI::WindowsAndMessaging::HCURSOR {
    crate::win::gdi::system_cursor(idc)
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

fn bitmapinfo(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: (w.max(0) as u32)
                .saturating_mul(h.max(0) as u32)
                .saturating_mul(4),
            ..unsafe { std::mem::zeroed() }
        },
        bmiColors: [unsafe { std::mem::zeroed() }],
    }
}

unsafe fn paint(app: &'static App, hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    if hdc.is_invalid() {
        let _ = EndPaint(hwnd, &ps);
        return;
    }
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);

    {
        let mut b = app.editor.borrow_mut();
        if let Some(s) = b.as_mut() {
            fill_solid(hdc, &s.tb_rect, crate::win::dark::BG);
            fill_solid(hdc, &s.status_rect, crate::win::dark::BG_ALT);
            fill_solid(hdc, &s.canvas, crate::win::dark::CANVAS);

            // Rendered image (committed shapes + live draft).
            let items = s.history.items();
            let frame = s.preview.render(items, s.draft.as_ref());
            let (iw, ih) = (frame.w as i32, frame.h as i32);
            let z = s.zoom;
            let dx = s.canvas.left - (s.view.0 * z).round() as i32;
            let dy = s.canvas.top - (s.view.1 * z).round() as i32;
            let dw = (iw as f32 * z).round() as i32;
            let dh = (ih as f32 * z).round() as i32;
            if dw > 0 && dh > 0 {
                let saved = SaveDC(hdc);
                let _ = IntersectClipRect(
                    hdc,
                    s.canvas.left,
                    s.canvas.top,
                    s.canvas.right,
                    s.canvas.bottom,
                );
                if z < 1.0 {
                    let _ = SetStretchBltMode(hdc, HALFTONE);
                    let _ = SetBrushOrgEx(hdc, 0, 0, None);
                }
                let bmi = bitmapinfo(iw, ih);
                let _ = StretchDIBits(
                    hdc,
                    dx,
                    dy,
                    dw,
                    dh,
                    0,
                    0,
                    iw,
                    ih,
                    Some(frame.px.as_ptr() as *const core::ffi::c_void),
                    &bmi,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
                let border = RECT {
                    left: dx - 1,
                    top: dy - 1,
                    right: dx + dw + 1,
                    bottom: dy + dh + 1,
                };
                frame_solid(hdc, &border, crate::theme::SEL_OUTLINE_U32);
                let _ = RestoreDC(hdc, saved);
            }

            // Status bar.
            let prev_font = SelectObject(hdc, app.font_small.0.into());
            let left_rect = RECT {
                left: 12,
                top: s.status_rect.top + 4,
                right: (client.right - 260).max(13),
                bottom: s.status_rect.bottom - 2,
            };
            if !s.status.is_empty() {
                draw_label(
                    hdc,
                    left_rect,
                    &s.status,
                    crate::win::dark::FG_DIM,
                    DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS,
                );
            }
            let right_txt = format!("{} x {}   {}%", iw, ih, (z * 100.0).round() as i32);
            let right_rect = RECT {
                left: (client.right - 250).max(13),
                top: s.status_rect.top + 4,
                right: client.right - 12,
                bottom: s.status_rect.bottom - 2,
            };
            draw_label(
                hdc,
                right_rect,
                &right_txt,
                crate::win::dark::FG_DIM,
                DT_RIGHT | DT_SINGLELINE,
            );
            SelectObject(hdc, prev_font);

            // Toolbar (paints with the shared UI font).
            toolbar::paint(hdc, &s.toolbar, s.hover, Some(s.focus), s.kb, app.font_ui.0);
        }
    }
    let _ = EndPaint(hwnd, &ps);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Dispatch a toolbar command. Never holds the session borrow across a call
/// that can pump messages (dialogs, clipboard waits).
pub fn command(app: &'static App, cmd: toolbar::Cmd) {
    match cmd {
        toolbar::Cmd::Tool(t) => {
            let editing = app.text_edit_active.get();
            {
                let mut b = app.editor.borrow_mut();
                if let Some(s) = b.as_mut() {
                    s.tool = t;
                    let (cw, ch) = (s.client_w, s.client_h);
                    s.relayout(cw, ch);
                }
            }
            if editing {
                commit_any_text(app);
            }
            if let Some(h) = app.editor_hwnd_opt() {
                invalidate_tb(app, h);
            }
        }
        toolbar::Cmd::Undo | toolbar::Cmd::Redo => {
            let done = {
                let mut b = app.editor.borrow_mut();
                let Some(s) = b.as_mut() else { return };
                let done = if matches!(cmd, toolbar::Cmd::Undo) {
                    s.history.undo()
                } else {
                    s.history.redo()
                };
                if done {
                    s.draft = None;
                    s.status.clear();
                    let (cw, ch) = (s.client_w, s.client_h);
                    s.relayout(cw, ch);
                }
                done
            };
            if done {
                let h = app.editor_hwnd_opt();
                if let Some(h) = h {
                    invalidate_all(h);
                }
            }
        }
        toolbar::Cmd::Copy => {
            // Explicit action: a pending text draft commits before export so
            // it is included in the copied image.
            commit_any_text(app);
            do_copy(app);
        }
        toolbar::Cmd::Save => {
            commit_any_text(app);
            do_save(app);
        }
        toolbar::Cmd::Close => {
            let _ = apply_route(app, Route::Close);
        }
        toolbar::Cmd::Color(i) => {
            {
                let mut b = app.editor.borrow_mut();
                if let Some(s) = b.as_mut() {
                    s.color_idx = i.min(PALETTE.len() - 1);
                    let (cw, ch) = (s.client_w, s.client_h);
                    s.relayout(cw, ch);
                }
            }
            if let Some(h) = app.editor_hwnd_opt() {
                invalidate_tb(app, h);
            }
        }
        toolbar::Cmd::Width(i) => {
            {
                let mut b = app.editor.borrow_mut();
                if let Some(s) = b.as_mut() {
                    s.width_idx = i.min(WIDTHS.len() - 1);
                    let (cw, ch) = (s.client_w, s.client_h);
                    s.relayout(cw, ch);
                }
            }
            if let Some(h) = app.editor_hwnd_opt() {
                invalidate_tb(app, h);
            }
        }
        toolbar::Cmd::ZoomIn | toolbar::Cmd::ZoomOut | toolbar::Cmd::ZoomFit => {
            zoom_cmd(app, cmd);
        }
    }
}

fn zoom_cmd(app: &'static App, kind: toolbar::Cmd) {
    {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        let (cw, ch) = (s.canvas_w(), s.canvas_h());
        match kind {
            toolbar::Cmd::ZoomFit => {
                s.fit(cw, ch);
            }
            toolbar::Cmd::ZoomIn | toolbar::Cmd::ZoomOut => {
                let cur = s.zoom;
                let new = if matches!(kind, toolbar::Cmd::ZoomIn) {
                    ZOOM_STEPS
                        .iter()
                        .copied()
                        .find(|z| *z > cur * 1.001)
                        .unwrap_or(MAX_ZOOM)
                } else {
                    ZOOM_STEPS
                        .iter()
                        .rev()
                        .copied()
                        .find(|z| *z < cur * 0.999)
                        .unwrap_or(MIN_ZOOM)
                };
                if (new - cur).abs() < f32::EPSILON {
                    return;
                }
                let ccx = s.canvas.left + cw / 2;
                let ccy = s.canvas.top + ch / 2;
                let anchor = s.dev_to_img(ccx, ccy);
                s.zoom = new;
                s.view.0 = anchor.0 - (ccx - s.canvas.left) as f32 / new;
                s.view.1 = anchor.1 - (ccy - s.canvas.top) as f32 / new;
                s.fit_mode = false;
                s.clamp_view(cw, ch);
            }
            _ => {}
        }
        s.position_text_edit();
    }
    if let Some(h) = app.editor_hwnd_opt() {
        invalidate_all(h);
    }
}

fn do_copy(app: &'static App) {
    let st = s();
    let (hwnd, frame) = {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        let items = s.history.items();
        let f = s.preview.render(items, None).clone();
        (s.hwnd, f)
    };
    match crate::win::clipboard::copy_image(Some(hwnd), &frame) {
        Ok(()) => {
            // Success ends the session: the status bar dies with the window,
            // so the confirmation moves to the tray balloon.
            if let Some(t) = app.tray.borrow().as_ref() {
                t.balloon(st.state_copied, st.app_name, false);
            }
            apply_route(app, Route::Copy { ok: true });
        }
        Err(e) => {
            dialog::error_box(
                Some(hwnd),
                st.state_copy_fail_title,
                &fill(st.state_copy_fail, "DETAIL", &e.to_string()),
            );
            apply_route(app, Route::Copy { ok: false });
        }
    }
}

fn do_save(app: &'static App) {
    let st = s();
    let settings = app.settings.borrow().clone();
    let (hwnd, frame) = {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        let items = s.history.items();
        let f = s.preview.render(items, None).clone();
        (s.hwnd, f)
    };

    let dir = dialog::resolve_save_dir(&settings.save_dir);
    let name = crate::output::default_filename(settings.format, crate::os::local_time());
    let picked = match dialog::save_image(Some(hwnd), Some(&dir), &name, settings.format) {
        Ok(v) => v,
        Err(e) => {
            dialog::error_box(
                Some(hwnd),
                st.state_save_fail_title,
                &fill(st.state_save_fail, "DETAIL", &e.to_string()),
            );
            apply_route(app, Route::Save(SaveOutcome::Failed));
            return;
        }
    };
    let Some(path) = picked else {
        // Save-As cancelled: the session stays exactly as it was.
        apply_route(app, Route::Save(SaveOutcome::Cancelled));
        return;
    };

    // Trust an explicit known extension; otherwise append the default one.
    let (path, fmt) = match path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(crate::config::OutputFormat::from_str_lossy)
    {
        Some(f) => (path, f),
        None => {
            let mut os = path.into_os_string();
            os.push(".");
            os.push(settings.format.extension());
            (std::path::PathBuf::from(os), settings.format)
        }
    };

    match crate::encode::encode_to_file(&path, &frame, fmt, settings.jpeg_quality) {
        Ok(()) => {
            if let Some(t) = app.tray.borrow().as_ref() {
                t.balloon(st.state_saved, &path.to_string_lossy(), false);
            }
            apply_route(app, Route::Save(SaveOutcome::Saved));
        }
        Err(e) => {
            dialog::error_box(
                Some(hwnd),
                st.state_save_fail_title,
                &fill(st.state_save_fail, "DETAIL", &e.to_string()),
            );
            apply_route(app, Route::Save(SaveOutcome::Failed));
        }
    }
}

/// Finalize the editor session exactly once (see [`crate::session::close_target`]).
fn close_session(app: &'static App) {
    let Some(hwnd) = app.editor.borrow().as_ref().map(|s| s.hwnd) else {
        return; // no record: already finalized
    };
    let alive = !hwnd.0.is_null() && unsafe { IsWindow(Some(hwnd)) }.as_bool();
    match session::close_target(true, alive) {
        session::CloseAction::Destroy => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        session::CloseAction::ClearStale => {
            app.text_edit_active.set(false);
            app.editor.take();
        }
        session::CloseAction::Nothing => {}
    }
}

/// Feed `route` to [`crate::session::plan`] for the editor session and
/// finalize it when the policy says so (mirrors
/// [`crate::overlay::apply_route`]).
pub fn apply_route(app: &'static App, route: Route) -> session::Plan {
    let open = app.editor.borrow().is_some();
    let plan = session::plan(route, open);
    if matches!(plan, session::Plan::CloseSession | session::Plan::Exit) {
        close_session(app);
    }
    plan
}

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

pub(crate) fn worth(sh: &Shape) -> bool {
    match sh {
        Shape::Line { a, b, .. } | Shape::Arrow { a, b, .. } | Shape::Highlight { a, b, .. } => {
            (a.0 - b.0).abs() >= 2 || (a.1 - b.1).abs() >= 2
        }
        Shape::Rect { rect, .. } | Shape::Mosaic { rect, .. } | Shape::Solid { rect, .. } => {
            rect.w >= 3 && rect.h >= 3
        }
        Shape::Pen { pts, .. } => pts.len() >= 2,
        Shape::Text { text, .. } => !text.trim().is_empty(),
    }
}

pub(crate) fn make_draft(
    tool: Tool,
    a: (i32, i32),
    b: (i32, i32),
    color: Rgba,
    wi: usize,
) -> Option<Shape> {
    let wi = wi.min(WIDTHS.len() - 1);
    let width = WIDTHS[wi];
    let rect = IRect::from_points(a.0, a.1, b.0, b.1);
    Some(match tool {
        Tool::Pen | Tool::Text => return None,
        Tool::Line => Shape::Line { a, b, color, width },
        Tool::Arrow => Shape::Arrow { a, b, color, width },
        Tool::Highlight => Shape::Highlight {
            a,
            b,
            color: Rgba::new(color.r, color.g, color.b, 110),
            width: width.saturating_mul(3),
        },
        Tool::Rect => Shape::Rect {
            rect,
            color,
            width,
            filled: true,
        },
        Tool::Box => Shape::Rect {
            rect,
            color,
            width,
            filled: false,
        },
        Tool::Mosaic => Shape::Mosaic {
            rect,
            block: BLOCKS[wi],
        },
        Tool::Solid => Shape::Solid {
            rect,
            color: Rgba::new(color.r, color.g, color.b, 255),
        },
    })
}

// ---------------------------------------------------------------------------
// Text tool (transient child EDIT, subclassed)
// ---------------------------------------------------------------------------

fn commit_any_text(app: &'static App) {
    let h = {
        let b = app.editor.borrow();
        b.as_ref().and_then(|s| s.text_edit.map(|(h, _)| h))
    };
    if let Some(h) = h {
        commit_text(app, h);
    }
}

fn begin_text(app: &'static App, dev: (i32, i32), img: (i32, i32)) {
    commit_any_text(app);

    let Some(parent) = app.editor.borrow().as_ref().map(|s| s.hwnd) else {
        return;
    };
    let size = app
        .editor
        .borrow()
        .as_ref()
        .map(|s| TEXT_SIZES[s.width_idx.min(TEXT_SIZES.len() - 1)])
        .unwrap_or(24);

    let font = unsafe {
        CreateFontW(
            -(size as i32 + 6),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            ANSI_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            w!("Segoe UI"),
        )
    };
    if font.is_invalid() {
        return;
    }

    let edit = unsafe {
        CreateWindowExW(
            Default::default(),
            w!("EDIT"),
            PCWSTR(std::ptr::null()),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            dev.0 - 2,
            dev.1 - 2,
            340,
            (size + 16) as i32,
            Some(parent),
            None,
            None,
            Some(app as *const App as *const core::ffi::c_void),
        )
    };
    let Ok(edit) = edit else {
        unsafe {
            let _ = DeleteObject(font.into());
        }
        return;
    };
    crate::win::a11y::set_name(edit, s().text_input_name);

    unsafe {
        SendMessageW(
            edit,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
        let _ = SetWindowSubclass(
            edit,
            Some(edit_proc),
            SUBCLASS_ID,
            app as *const App as usize,
        );
    }

    {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        s.text_font = Some(OwnedGdi(font));
        s.text_edit = Some((edit, img));
        s.status = crate::strings::s().text_prompt.to_string();
        s.tool = Tool::Text;
    }
    app.text_edit_active.set(true);
    unsafe {
        let _ = SetFocus(Some(edit));
    }
    if let Some(h) = app.editor_hwnd_opt() {
        invalidate_all(h);
    }
}

/// Read the edit box, destroy it, and (when non-empty) push the text shape.
pub fn commit_text(app: &App, edit: HWND) {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetWindowTextW(edit, &mut buf) };
    let text = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    let text = text.trim().to_string();

    let anchor = {
        let b = app.editor.borrow();
        b.as_ref()
            .and_then(|s| s.text_edit)
            .and_then(|(h, at)| (h == edit).then_some(at))
    };

    unsafe {
        let _ = RemoveWindowSubclass(edit, Some(edit_proc), SUBCLASS_ID);
        let _ = DestroyWindow(edit);
    }
    app.text_edit_active.set(false);

    let Some(anchor) = anchor else { return };
    let h = {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        s.text_edit = None;
        if !text.is_empty() {
            let shape = Shape::Text {
                at: anchor,
                text,
                color: s.color(),
                size: TEXT_SIZES[s.width_idx.min(TEXT_SIZES.len() - 1)],
            };
            s.history.push(shape);
            s.status.clear();
        }
        let (cw, ch) = (s.client_w, s.client_h);
        s.relayout(cw, ch);
        s.hwnd
    };
    unsafe {
        let _ = InvalidateRect(Some(h), None, false);
    }
}

/// Esc in the text box: discard the pending text.
pub fn cancel_text(app: &App, edit: HWND) {
    unsafe {
        let _ = RemoveWindowSubclass(edit, Some(edit_proc), SUBCLASS_ID);
        let _ = DestroyWindow(edit);
    }
    app.text_edit_active.set(false);
    let h = {
        let mut b = app.editor.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        s.text_edit = None;
        s.status.clear();
        s.hwnd
    };
    unsafe {
        let _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uid: usize,
    refdata: usize,
) -> LRESULT {
    let app = &*(refdata as *const App);
    match msg {
        WM_KEYDOWN => match (wparam.0 as u16) & 0xFF {
            VK_RETURN => {
                commit_text(app, hwnd);
                LRESULT(0)
            }
            VK_ESCAPE => {
                cancel_text(app, hwnd);
                LRESULT(0)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        },
        WM_KILLFOCUS => {
            commit_text(app, hwnd);
            LRESULT(0)
        }
        _ => DefSubclassProc(hwnd, msg, wparam, lparam),
    }
}
