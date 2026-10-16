//! Full-screen capture overlay: shows the frozen frame, lets the user drag,
//! move and resize the selection with 8 handles, and commits the selection
//! **on mouse release** — keeping the same window and transitioning in place
//! to the inline annotation editor ([`Phase::Editing`], [`edit`]) — so the
//! capture → annotate flow never leaves the overlay (no separate editor
//! window, no recapture, no hide/show). Enter and double-click still commit
//! (keyboard path); F = full screen, R = restart capture, Esc = cancel (in
//! editing: close/discard).
//!
//! The window covers the whole virtual screen at 1:1, so client coordinates
//! equal screen coordinates minus the virtual-screen origin. Every paint is
//! composed into an offscreen backbuffer (fill → image → chrome) and
//! presented with a single `BitBlt`, so partially painted intermediate states
//! are never visible on screen (the Phase A flicker root cause).

pub mod edit;
mod layout;

use crate::app::App;
use crate::capture;
use crate::error::{Error, Result};
use crate::image::{Frame, IRect};
use crate::os::VirtualScreen;
use crate::session::{self, Route};
use crate::strings::{fill, s};
use crate::theme;
use crate::win::gdi::{MemDc, OwnedGdi, frame_solid, round_face};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, CreateDIBSection, CreatePen,
    CreateSolidBrush, DIB_RGB_COLORS, DeleteObject, EndPaint, HALFTONE, HBITMAP, HDC, HGDIOBJ,
    IntersectClipRect, InvalidateRect, PAINTSTRUCT, PS_SOLID, Polygon, RestoreDC, SRCCOPY, SaveDC,
    SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchDIBits,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    IDC_CROSS, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, IsWindow,
    SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetCursor, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY,
    WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT, WM_SETCURSOR, WM_SETFOCUS, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

/// Accent COLORREF `0x00BBGGRR` (shared with the toolbar).
const ACCENT: u32 = theme::ACCENT_U32;

/// Hit tolerance beyond the visual handle.
const HIT: i32 = 7;

const MK_SHIFT_FL: i32 = 0x10;

const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const VK_F: u32 = 0x46;
const VK_R: u32 = 0x52;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;

#[derive(Clone, Copy)]
pub enum ODrag {
    /// Press outside the selection: rubber-band from `anchor` (screen coords).
    New {
        anchor: (i32, i32),
        prev: IRect,
        moved: bool,
    },
    /// Press inside the selection: offset from its top-left corner.
    Move { dx: i32, dy: i32 },
    /// Press on a handle: resize from the rectangle at drag start.
    Resize { handle: usize, orig: IRect },
}

/// Overlay phase: selection first, then in-place annotation editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Drag/resize the capture region.
    Selecting,
    /// Annotate the retained crop inside the same window.
    Editing,
}

/// Offscreen backbuffer the whole scene is composed into before a single
/// `BitBlt` presents it — no intermediate fill states ever reach the screen.
pub struct Backbuf {
    mem: MemDc,
    /// Owned so it is deleted after `mem` restores the stock object on drop.
    #[allow(dead_code)]
    bmp: OwnedGdi<HBITMAP>,
    prev: HGDIOBJ,
    w: i32,
    h: i32,
}

impl Backbuf {
    pub fn new(w: i32, h: i32) -> Option<Backbuf> {
        if w <= 0 || h <= 0 {
            return None;
        }
        unsafe {
            let mem = MemDc::new()?;
            let bmi = bitmapinfo(w, h);
            let mut bits = core::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(mem.0), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                return None;
            };
            if bits.is_null() {
                let _ = DeleteObject(bmp.into());
                return None;
            }
            let prev = SelectObject(mem.0, bmp.into());
            Some(Backbuf {
                mem,
                bmp: OwnedGdi(bmp),
                prev,
                w,
                h,
            })
        }
    }

    pub fn dc(&self) -> HDC {
        self.mem.0
    }

    pub fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }
}

impl Drop for Backbuf {
    fn drop(&mut self) {
        unsafe {
            // Deselect before the fields are deleted (DC first, then bitmap).
            let _ = SelectObject(self.mem.0, self.prev);
        }
    }
}

/// Live capture-overlay state, owned by [`crate::app::App`].
pub struct OverlaySession {
    pub hwnd: HWND,
    /// Captured frame of the whole virtual screen (BGRA, opaque).
    pub frame: Frame,
    /// Pre-dimmed copy used outside the selection.
    pub dim: Vec<u8>,
    pub vs: VirtualScreen,
    /// Current selection in screen coordinates.
    pub sel: IRect,
    pub drag: Option<ODrag>,
    pub phase: Phase,
    /// Inline editing state; `Some` iff [`Phase::Editing`].
    pub edit: Option<edit::EditSession>,
    /// Scene backbuffer (created once the window exists).
    pub buf: Option<Backbuf>,
}

fn full_rect(vs: &VirtualScreen) -> IRect {
    IRect::new(vs.x, vs.y, vs.w, vs.h)
}

/// One veil lookup table: `LUT[x] = (x * KEEP + bg * MIX) / 100`.
/// Built at compile time (no per-capture cost); the dim pass becomes three
/// table lookups per pixel instead of three multiply+divide pairs.
const fn veil_lut(bg: u8) -> [u8; 256] {
    let keep = theme::VEIL_KEEP_PCT as u32;
    let mix = 100 - keep;
    let mut t = [0u8; 256];
    let mut x = 0u32;
    while x < 256 {
        t[x as usize] = ((x * keep + bg as u32 * mix) / 100) as u8;
        x += 1;
    }
    t
}

/// Uniform near-black veil: blend each pixel toward [`theme::BG`] keeping
/// [`theme::VEIL_KEEP_PCT`] of it (alpha forced opaque; SRCCOPY ignores it).
/// One O(n) pass per session open/restart — same cost class as the old
/// `>>= 1` half-brightness dim, but LUT-driven (no mul/div per pixel).
fn dim_frame(frame: &Frame) -> Vec<u8> {
    // BG is (r, g, b); frame bytes are BGRA.
    static LUT_B: [u8; 256] = veil_lut(theme::BG.2);
    static LUT_G: [u8; 256] = veil_lut(theme::BG.1);
    static LUT_R: [u8; 256] = veil_lut(theme::BG.0);
    let mut v = frame.px.clone();
    let (chunks, _) = v.as_chunks_mut::<4>();
    for c in chunks {
        c[0] = LUT_B[c[0] as usize];
        c[1] = LUT_G[c[1] as usize];
        c[2] = LUT_R[c[2] as usize];
        c[3] = 255;
    }
    v
}

#[cfg(test)]
mod veil_tests {
    use super::*;

    #[test]
    fn lut_matches_the_blend_formula() {
        let keep = theme::VEIL_KEEP_PCT as u32;
        let mix = 100 - keep;
        for (bg, lut) in [
            (theme::BG.2, veil_lut(theme::BG.2)),
            (theme::BG.1, veil_lut(theme::BG.1)),
            (theme::BG.0, veil_lut(theme::BG.0)),
        ] {
            for x in 0..256u32 {
                assert_eq!(lut[x as usize], ((x * keep + bg as u32 * mix) / 100) as u8);
            }
        }
    }
}

/// Clamp `v` into `[lo, hi]` (returns `lo` for an inverted range).
fn cl(v: i32, lo: i32, hi: i32) -> i32 {
    if hi < lo { lo } else { v.max(lo).min(hi) }
}

/// Translate `sel` so it stays inside `vs` (size preserved).
fn move_rect(sel: IRect, nx: i32, ny: i32, vs: &VirtualScreen) -> IRect {
    let x = cl(nx, vs.x, (vs.x + vs.w - sel.w).max(vs.x));
    let y = cl(ny, vs.y, (vs.y + vs.h - sel.h).max(vs.y));
    IRect::new(x, y, sel.w, sel.h)
}

/// Rubber-band rectangle between two screen points, clamped to `vs`.
fn band_rect(a: (i32, i32), b: (i32, i32), vs: &VirtualScreen) -> IRect {
    let r = IRect::from_points(a.0, a.1, b.0, b.1);
    let l = cl(r.x, vs.x, vs.x + vs.w - 1);
    let t = cl(r.y, vs.y, vs.y + vs.h - 1);
    let rr = cl(r.right(), l + 1, vs.x + vs.w);
    let bb = cl(r.bottom(), t + 1, vs.y + vs.h);
    IRect::new(l, t, rr - l, bb - t)
}

/// Sticky "this rubber-band actually moved" flag: once the band left its
/// starting rectangle, the drag counts as moved even if it ends back at the
/// start (a full-screen band must still commit on release).
fn band_moved(was_moved: bool, prev: IRect, sel: IRect) -> bool {
    was_moved || sel != prev
}

/// Resize `orig` by dragging `handle` to the cursor, staying inside `vs`.
fn resize_rect(handle: usize, orig: IRect, cx: i32, cy: i32, vs: &VirtualScreen) -> IRect {
    let l = orig.x;
    let t = orig.y;
    let r = orig.right();
    let b = orig.bottom();
    let vr = vs.x + vs.w;
    let vb = vs.y + vs.h;
    match handle {
        0 => {
            let x2 = cl(cx, vs.x, r - 1);
            let y2 = cl(cy, vs.y, b - 1);
            IRect::new(x2, y2, r - x2, b - y2)
        }
        1 => {
            let t2 = cl(cy, vs.y, b - 1);
            IRect::new(l, t2, r - l, b - t2)
        }
        2 => {
            let t2 = cl(cy, vs.y, b - 1);
            let r2 = cl(cx + 1, l + 1, vr);
            IRect::new(l, t2, r2 - l, b - t2)
        }
        3 => {
            let r2 = cl(cx + 1, l + 1, vr);
            IRect::new(l, t, r2 - l, b - t)
        }
        4 => {
            let r2 = cl(cx + 1, l + 1, vr);
            let b2 = cl(cy + 1, t + 1, vb);
            IRect::new(l, t, r2 - l, b2 - t)
        }
        5 => {
            let b2 = cl(cy + 1, t + 1, vb);
            IRect::new(l, t, r - l, b2 - t)
        }
        6 => {
            let l2 = cl(cx, vs.x, r - 1);
            let b2 = cl(cy + 1, t + 1, vb);
            IRect::new(l2, t, r - l2, b2 - t)
        }
        _ => {
            let l2 = cl(cx, vs.x, r - 1);
            IRect::new(l2, t, r - l2, b - t)
        }
    }
}

/// Center points of the 8 handles (client/screen space):
/// NW, N, NE, E, SE, S, SW, W.
fn handle_centers(r: IRect) -> [(i32, i32); 8] {
    let x0 = r.x;
    let y0 = r.y;
    let x1 = r.x + r.w - 1;
    let y1 = r.y + r.h - 1;
    let mx = r.x + r.w / 2;
    let my = r.y + r.h / 2;
    [
        (x0, y0),
        (mx, y0),
        (x1, y0),
        (x1, my),
        (x1, y1),
        (mx, y1),
        (x0, y1),
        (x0, my),
    ]
}

fn hit_handle(cx: i32, cy: i32, sel: IRect) -> Option<usize> {
    handle_centers(sel)
        .into_iter()
        .position(|(x, y)| (cx - x).abs() <= HIT && (cy - y).abs() <= HIT)
}

fn handle_cursor(handle: usize) -> PCWSTR {
    match handle {
        0 | 4 => IDC_SIZENWSE,
        2 | 6 => IDC_SIZENESW,
        1 | 5 => IDC_SIZENS,
        _ => IDC_SIZEWE,
    }
}

/// Outward unit step for a selection handle (NW, N, NE, E, SE, S, SW, W).
fn handle_dir(handle: usize) -> (i32, i32) {
    match handle {
        0 => (-1, -1),
        1 => (0, -1),
        2 => (1, -1),
        3 => (1, 0),
        4 => (1, 1),
        5 => (0, 1),
        6 => (-1, 1),
        _ => (-1, 0),
    }
}

/// Draw one Lightshot-style resize arrow at `(cx, cy)` pointing along the
/// outward unit step `(dx, dy)`: tip `reach` px out, base `half` px wide.
/// Accent fill with a near-black edge so it reads over any capture content.
///
/// # Safety
/// `hdc` must be a valid device context for the duration of the call.
unsafe fn draw_resize_arrow(
    hdc: HDC,
    cx: i32,
    cy: i32,
    dx: i32,
    dy: i32,
    reach: i32,
    half: i32,
) {
    let tx = cx + dx * reach;
    let ty = cy + dy * reach;
    let bx = cx - dx * 3;
    let by = cy - dy * 3;
    let (px, py) = (-dy, dx);
    let pts = [
        POINT { x: tx, y: ty },
        POINT {
            x: bx + px * half,
            y: by + py * half,
        },
        POINT {
            x: bx - px * half,
            y: by - py * half,
        },
    ];
    let pen = CreatePen(
        PS_SOLID,
        1,
        crate::win::dark::colorref_u32(theme::SEL_OUTLINE_U32),
    );
    let brush = CreateSolidBrush(crate::win::dark::colorref_u32(ACCENT));
    let prev_pen = SelectObject(hdc, pen.into());
    let prev_brush = SelectObject(hdc, brush.into());
    let _ = Polygon(hdc, &pts);
    SelectObject(hdc, prev_pen);
    SelectObject(hdc, prev_brush);
    let _ = DeleteObject(pen.into());
    let _ = DeleteObject(brush.into());
}

fn union(a: IRect, b: IRect) -> IRect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = a.right().max(b.right());
    let y1 = a.bottom().max(b.bottom());
    IRect::new(x0, y0, x1 - x0, y1 - y0)
}

/// Invalidate the union of two screen rects (plus chrome room) on `hwnd`.
fn invalidate_region(hwnd: HWND, vs: &VirtualScreen, a: IRect, b: IRect) {
    // 32px covers the selection frame, handles and the size chip.
    let r = union(a, b).expand(32);
    let local = r.intersect(&IRect::new(vs.x, vs.y, vs.w, vs.h));
    if local.is_empty() {
        return;
    }
    let rc = RECT {
        left: local.x - vs.x,
        top: local.y - vs.y,
        right: local.x - vs.x + local.w,
        bottom: local.y - vs.y + local.h,
    };
    unsafe {
        let _ = InvalidateRect(Some(hwnd), Some(&raw const rc), false);
    }
}

fn key_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

#[inline]
fn mx(lp: LPARAM) -> i32 {
    (lp.0 as u16 as i16) as i32
}
#[inline]
fn my(lp: LPARAM) -> i32 {
    (((lp.0 >> 16) & 0xFFFF) as u16 as i16) as i32
}

/// Capture the screen and open the overlay. Errors propagate to the caller,
/// which shows `capture_fail_msg`.
pub fn open(app: &'static App) -> Result<HWND> {
    let cap = capture::capture_virtual_screen()?;
    let vs = cap.virtual_screen;
    let dim = dim_frame(&cap.frame);

    let session = OverlaySession {
        hwnd: HWND::default(),
        frame: cap.frame,
        dim,
        vs,
        sel: full_rect(&vs),
        drag: None,
        phase: Phase::Selecting,
        edit: None,
        buf: None,
    };
    app.overlay.replace(Some(session));

    let title = crate::win::wide(s().app_name);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("NameShot.Overlay"),
            PCWSTR(title.as_ptr()),
            WS_POPUP,
            vs.x,
            vs.y,
            vs.w,
            vs.h,
            None,
            None,
            Some(crate::win::gdi::this_module()),
            Some(app as *const App as *const core::ffi::c_void),
        )
        .map_err(|e| {
            app.overlay.take();
            Error::Win32(e).with_context("CreateWindowExW(overlay)")
        })?
    };

    if let Some(s) = app.overlay.borrow_mut().as_mut() {
        s.hwnd = hwnd;
        let (w, h) = (s.vs.w, s.vs.h);
        s.buf = Backbuf::new(w, h);
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
    Ok(hwnd)
}

/// Focus an already-open overlay instead of stacking a second capture.
pub fn focus_existing(app: &App) -> Option<HWND> {
    let h = app.overlay.borrow().as_ref().map(|s| s.hwnd)?;
    unsafe {
        let _ = SetForegroundWindow(h);
    }
    Some(h)
}

// ---------------------------------------------------------------------------
// Session finalization (the close policy lives in crate::session)
// ---------------------------------------------------------------------------

/// Finalize the capture session exactly once.
///
/// Runs [`crate::session::close_target`] against the live record: a session
/// with a window is destroyed (its `WM_DESTROY` drops the record together
/// with the text box, fonts and backbuffer), a record whose window is
/// already gone is just cleared, and with no record at all nothing happens —
/// so any route may call this twice, or after somebody else already closed
/// the session. Never prompts, never moves focus.
pub fn close_session(app: &'static App) {
    let Some(hwnd) = app.overlay.borrow().as_ref().map(|s| s.hwnd) else {
        return; // no record: already finalized
    };
    let alive = !hwnd.0.is_null() && unsafe { IsWindow(Some(hwnd)) }.as_bool();
    match session::close_target(true, alive) {
        session::CloseAction::Destroy => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        session::CloseAction::ClearStale => {
            app.text_edit_active.set(false);
            app.overlay.take();
        }
        session::CloseAction::Nothing => {}
    }
}

/// Feed `route` to [`crate::session::plan`] and finalize the session when
/// the policy says so. Returns the plan the caller received:
/// [`session::Plan::Exit`] means the caller (tray Exit) must now quit, and
/// [`session::Plan::CancelTextDraft`] is left to the text box's own caller.
pub fn apply_route(app: &'static App, route: Route) -> session::Plan {
    let open = app.overlay.borrow().is_some();
    let plan = session::plan(route, open);
    if matches!(plan, session::Plan::CloseSession | session::Plan::Exit) {
        close_session(app);
    }
    plan
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
        WM_CLOSE => {
            // Explicit close: the session is discarded immediately (the
            // policy in crate::session never prompts).
            apply_route(app, Route::Close);
            LRESULT(0)
        }
        WM_PAINT => {
            paint(app, hwnd);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let cur = cursor_for(app, mx(lparam), my(lparam));
            SetCursor(Some(get_cursor(cur)));
            LRESULT(1)
        }
        WM_LBUTTONDOWN => {
            if phase_of(app) == Phase::Editing {
                edit::on_down(app, hwnd, mx(lparam), my(lparam));
            } else {
                on_down(app, hwnd, mx(lparam), my(lparam));
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            track_leave(hwnd);
            if phase_of(app) == Phase::Editing {
                edit::on_move(app, hwnd, mx(lparam), my(lparam));
            } else {
                on_move(app, hwnd, mx(lparam), my(lparam));
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            edit::clear_hover(app);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if phase_of(app) == Phase::Editing {
                edit::on_up(app, mx(lparam), my(lparam));
            } else {
                on_up(app, hwnd);
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            // Capture was lost (Alt+Tab, dialog...): drop any in-flight drag
            // so the session never sticks with SetCapture semantics.
            let editing = phase_of(app) == Phase::Editing;
            let active = {
                let b = app.overlay.borrow();
                b.as_ref()
                    .map(|s| {
                        if editing {
                            s.edit.as_ref().is_some_and(|e| e.drag.is_some())
                        } else {
                            s.drag.is_some()
                        }
                    })
                    .unwrap_or(false)
            };
            if active {
                if let Some(s) = app.overlay.borrow_mut().as_mut() {
                    s.drag = None;
                    if let Some(e) = s.edit.as_mut() {
                        e.drag = None;
                        e.draft = None;
                    }
                }
                if editing {
                    edit::invalidate_canvas(app);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            let inside = app
                .overlay
                .borrow()
                .as_ref()
                .map(|s| {
                    let c = to_client(s.sel, &s.vs);
                    c.contains(mx(lparam), my(lparam))
                })
                .unwrap_or(false);
            if inside && phase_of(app) == Phase::Selecting {
                confirm(app, hwnd);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if phase_of(app) == Phase::Editing {
                edit::on_key(app, wparam.0 as u32 & 0xFF);
            } else {
                on_key(app, hwnd, wparam.0 as u32 & 0xFF);
            }
            LRESULT(0)
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            let has_focus = msg == WM_SETFOCUS;
            let mut b = app.overlay.borrow_mut();
            if let Some(e) = b.as_mut().and_then(|s| s.edit.as_mut()) {
                e.kb = has_focus;
            }
            drop(b);
            if phase_of(app) == Phase::Editing {
                edit::invalidate_bar(app);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let owns = app
                .overlay
                .borrow()
                .as_ref()
                .map(|s| s.hwnd == hwnd)
                .unwrap_or(false);
            if owns {
                app.text_edit_active.set(false);
                app.overlay.take();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn phase_of(app: &App) -> Phase {
    app.overlay
        .borrow()
        .as_ref()
        .map(|s| s.phase)
        .unwrap_or(Phase::Selecting)
}

fn track_leave(hwnd: HWND) {
    unsafe {
        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        let _ = TrackMouseEvent(&mut tme);
    }
}

fn to_client(r: IRect, vs: &VirtualScreen) -> IRect {
    IRect::new(r.x - vs.x, r.y - vs.y, r.w, r.h)
}

/// Translate a work area from screen coordinates into overlay-client
/// coordinates. Pure arithmetic: keeps the taskbar-excluded `rcWork` bands
/// and negative monitor origins intact (see tests), so [`layout::plan`]
/// receives bounds in the same space as the selection.
fn work_to_client(w: RECT, vs: &VirtualScreen) -> RECT {
    let r = to_client(
        IRect::new(w.left, w.top, w.right - w.left, w.bottom - w.top),
        vs,
    );
    RECT {
        left: r.x,
        top: r.y,
        right: r.x + r.w,
        bottom: r.y + r.h,
    }
}

fn get_cursor(idc: PCWSTR) -> windows::Win32::UI::WindowsAndMessaging::HCURSOR {
    crate::win::gdi::system_cursor(idc)
}

fn cursor_for(app: &App, x: i32, y: i32) -> PCWSTR {
    let info = app
        .overlay
        .borrow()
        .as_ref()
        .map(|s| (s.phase, to_client(s.sel, &s.vs), s.drag.is_some()));
    let Some((phase, sel, dragging)) = info else {
        return IDC_CROSS;
    };
    if phase == Phase::Editing {
        return edit::cursor_for(app, x, y);
    }
    if let Some(h) = hit_handle(x, y, sel) {
        return handle_cursor(h);
    }
    if dragging || sel.contains(x, y) {
        return IDC_SIZEALL;
    }
    IDC_CROSS
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn on_down(app: &'static App, hwnd: HWND, x: i32, y: i32) {
    let drag = {
        let b = app.overlay.borrow();
        let Some(s) = b.as_ref() else { return };
        let vs = s.vs;
        let sx = x + vs.x;
        let sy = y + vs.y;
        let c = to_client(s.sel, &vs);
        if let Some(h) = hit_handle(x, y, c) {
            ODrag::Resize {
                handle: h,
                orig: s.sel,
            }
        } else if c.contains(x, y) && s.sel != full_rect(&vs) {
            ODrag::Move {
                dx: sx - s.sel.x,
                dy: sy - s.sel.y,
            }
        } else {
            ODrag::New {
                anchor: (sx, sy),
                prev: s.sel,
                moved: false,
            }
        }
    };
    if let Some(d) = app.overlay.borrow_mut().as_mut() {
        d.drag = Some(drag);
    }
    unsafe {
        SetCapture(hwnd);
    }
}

fn on_move(app: &'static App, hwnd: HWND, x: i32, y: i32) {
    let updated = {
        let mut b = app.overlay.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        let vs = s.vs;
        let sx = x + vs.x;
        let sy = y + vs.y;
        let old = s.sel;
        let mut changed = false;
        match &mut s.drag {
            Some(ODrag::New {
                anchor,
                prev,
                moved,
            }) => {
                let anchor = *anchor;
                let prev = *prev;
                s.sel = band_rect(anchor, (sx, sy), &vs);
                // Sticky: any movement during the drag counts, even when the
                // band ends back at its starting rectangle (full-screen band).
                *moved = band_moved(*moved, prev, s.sel);
                changed = true;
            }
            Some(ODrag::Move { dx, dy }) => {
                let (dx, dy) = (*dx, *dy);
                s.sel = move_rect(s.sel, sx - dx, sy - dy, &vs);
                changed = true;
            }
            Some(ODrag::Resize { handle, orig }) => {
                let (handle, orig) = (*handle, *orig);
                s.sel = resize_rect(handle, orig, sx, sy, &vs);
                changed = true;
            }
            None => {}
        }
        (changed, old, s.sel, vs)
    };
    let (changed, old, new, vs) = updated;
    if changed && (old.x != new.x || old.y != new.y || old.w != new.w || old.h != new.h) {
        invalidate_region(hwnd, &vs, old, new);
    }
}

/// What a finished drag does on mouse release in the selecting phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseAction {
    /// Rubber-band done: crop and enter editing without waiting for Enter.
    Commit,
    /// Click noise (no movement, or a band below 4x4): restore the previous
    /// selection and stay in the selecting phase.
    Revert,
    /// Move/resize adjustment: only the drag ends, the phase stays.
    Stay,
}

/// Selection-on-mouse-release policy. A rubber-band that really moved and is
/// at least [`theme::SPACE_XS`] px in both dimensions commits on release (a
/// slip smaller than that swallows into the previous selection instead of
/// dropping into editing); plain clicks and move/resize releases never
/// commit. `confirm` still runs on Enter and double-click for keyboard users.
fn release_action(drag: ODrag, sel: IRect) -> ReleaseAction {
    match drag {
        ODrag::New { moved, .. } => {
            if moved && sel.w >= theme::SPACE_XS && sel.h >= theme::SPACE_XS {
                ReleaseAction::Commit
            } else {
                ReleaseAction::Revert
            }
        }
        ODrag::Move { .. } | ODrag::Resize { .. } => ReleaseAction::Stay,
    }
}

fn on_up(app: &'static App, hwnd: HWND) {
    let drag = {
        let mut b = app.overlay.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        s.drag.take()
    };
    let Some(drag) = drag else { return };
    unsafe {
        let _ = ReleaseCapture();
    }
    let sel = {
        let b = app.overlay.borrow();
        let Some(s) = b.as_ref() else { return };
        s.sel
    };
    match release_action(drag, sel) {
        // Selection-on-mouse-release: the band becomes the crop and the
        // overlay moves straight into the editing phase.
        ReleaseAction::Commit => confirm(app, hwnd),
        ReleaseAction::Revert => {
            if let ODrag::New { prev, .. } = drag {
                let target = {
                    let mut b = app.overlay.borrow_mut();
                    let Some(s) = b.as_mut() else { return };
                    s.sel = prev;
                    (sel, prev, s.vs, s.hwnd)
                };
                // Invalidate the union: the band drawn during the drag and
                // the restored previous selection.
                invalidate_region(target.3, &target.2, target.0, target.1);
            }
        }
        ReleaseAction::Stay => {}
    }
}

fn on_key(app: &'static App, hwnd: HWND, vk: u32) {
    match vk {
        VK_RETURN => confirm(app, hwnd),
        VK_ESCAPE => {
            let _ = apply_route(app, Route::Close);
        }
        VK_F => quick_full_screen(app, hwnd),
        VK_R => restart(app, hwnd),
        VK_LEFT..=VK_DOWN => nudge(app, vk),
        _ => {}
    }
}

/// Quick capture: the whole screen straight to the clipboard, then close
/// the session and confirm with a tray balloon. No dialog, no verification.
fn quick_full_screen(app: &'static App, hwnd: HWND) {
    let frame = {
        let b = app.overlay.borrow();
        let Some(s) = b.as_ref() else { return };
        s.frame.clone()
    };
    match crate::win::clipboard::copy_image(Some(hwnd), &frame) {
        Ok(()) => {
            // The balloon goes up before the session closes so the
            // confirmation appears immediately instead of after the window
            // is gone.
            if let Some(t) = app.tray.borrow().as_ref() {
                t.balloon(s().state_copied, s().app_name, false);
            }
            let _ = apply_route(app, Route::Close);
        }
        Err(e) => {
            // Keep the session open so the user can still edit or retry.
            crate::win::dialog::warn_box(
                Some(hwnd),
                s().state_copy_fail_title,
                &format!("{}: {}", s().state_copy_fail_title, e),
            );
        }
    }
}

fn nudge(app: &'static App, vk: u32) {
    let step = if key_down(MK_SHIFT_FL) { 10 } else { 1 };
    let (dx, dy) = match vk {
        VK_LEFT => (-step, 0),
        VK_RIGHT => (step, 0),
        VK_UP => (0, -step),
        _ => (0, step),
    };
    let target = {
        let mut b = app.overlay.borrow_mut();
        let Some(s) = b.as_mut() else { return };
        let old = s.sel;
        let vs = s.vs;
        s.sel = move_rect(s.sel, old.x + dx, old.y + dy, &vs);
        (old, s.sel, vs, s.hwnd)
    };
    invalidate_region(target.3, &target.2, target.0, target.1);
}

fn restart(app: &'static App, hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    let fresh = capture::capture_virtual_screen();
    match fresh {
        Ok(cap) => {
            let vs = cap.virtual_screen;
            {
                let mut b = app.overlay.borrow_mut();
                if let Some(s) = b.as_mut() {
                    s.frame = cap.frame;
                    s.dim = dim_frame(&s.frame);
                    s.vs = vs;
                    s.sel = full_rect(&vs);
                    s.drag = None;
                    // The virtual screen may have changed size while the
                    // overlay was open: size the backbuffer to the new one.
                    s.buf = Backbuf::new(vs.w, vs.h);
                }
            }
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    vs.x,
                    vs.y,
                    vs.w,
                    vs.h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(Some(hwnd));
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        Err(e) => {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(Some(hwnd));
            }
            let st = s();
            crate::win::dialog::warn_box(
                Some(hwnd),
                st.capture_fail_title,
                &fill(st.capture_fail_msg, "DETAIL", &e.to_string()),
            );
        }
    }
}

/// Crop the selection out of the frame and transition the overlay in place
/// to [`Phase::Editing`]. The window is not destroyed, hidden or recreated
/// and the capture is not repeated: the crop becomes the working source of
/// the inline editor, backed by the retained full capture so the crop
/// window stays adjustable with the border arrows.
fn confirm(app: &'static App, hwnd: HWND) {
    let (cropped, full) = {
        let b = app.overlay.borrow();
        let Some(ov) = b.as_ref() else { return };
        if ov.phase == Phase::Editing {
            return;
        }
        if ov.sel.is_empty() || ov.sel.w < 1 || ov.sel.h < 1 {
            let st = s();
            drop(b);
            crate::win::dialog::warn_box(Some(hwnd), st.app_name, st.capture_empty_msg);
            return;
        }
        let local = IRect::new(ov.sel.x - ov.vs.x, ov.sel.y - ov.vs.y, ov.sel.w, ov.sel.h)
            .clamp_to(ov.frame.w as i32, ov.frame.h as i32);
        if local.is_empty() {
            let st = s();
            drop(b);
            crate::win::dialog::warn_box(Some(hwnd), st.app_name, st.capture_empty_msg);
            return;
        }
        match ov.frame.crop(local) {
            Ok(f) => (f, ov.frame.clone()),
            Err(msg) => {
                drop(b);
                crate::win::dialog::warn_box(Some(hwnd), s().capture_fail_title, &msg);
                return;
            }
        }
    };
    let secure = crate::editor::looks_secure(&cropped);
    {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let canvas = to_client(ov.sel, &ov.vs);
        let canvas = RECT {
            left: canvas.x,
            top: canvas.y,
            right: canvas.x + canvas.w,
            bottom: canvas.y + canvas.h,
        };
        let client = (ov.vs.w, ov.vs.h);
        let sel_screen = RECT {
            left: ov.sel.x,
            top: ov.sel.y,
            right: ov.sel.x + ov.sel.w,
            bottom: ov.sel.y + ov.sel.h,
        };
        let work = work_to_client(crate::os::work_area_for(&sel_screen), &ov.vs);
        ov.drag = None;
        ov.edit = Some(edit::EditSession::new(
            cropped, full, canvas, client, work, secure,
        ));
        ov.phase = Phase::Editing;
    }
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

fn bitmapinfo(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
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

unsafe fn blit(dc: HDC, px: &[u8], w: i32, h: i32, dx: i32, dy: i32, dw: i32, dh: i32) {
    if w <= 0 || h <= 0 || dw <= 0 || dh <= 0 {
        return;
    }
    let bmi = bitmapinfo(w, h);
    let _ = StretchDIBits(
        dc,
        dx,
        dy,
        dw,
        dh,
        0,
        0,
        w,
        h,
        Some(px.as_ptr() as *const core::ffi::c_void),
        &bmi,
        DIB_RGB_COLORS,
        SRCCOPY,
    );
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
    let cw = client.right - client.left;
    let ch = client.bottom - client.top;
    let rc = ps.rcPaint;
    let f_ui = app.font_ui.0;
    let f_small = app.font_small.0;

    // Compose the full scene (fill → image → chrome) into the offscreen
    // backbuffer, clipped to the invalid region, then present it with a
    // single BitBlt: intermediate states never reach the screen.
    let mut b = app.overlay.borrow_mut();
    if let Some(s) = b.as_mut() {
        let buf = s.buf.as_ref().map(|b| b.dc());
        match buf {
            Some(mem) => {
                let saved = SaveDC(mem);
                let _ = IntersectClipRect(mem, rc.left, rc.top, rc.right, rc.bottom);
                compose(s, mem, cw, ch, f_ui, f_small);
                let _ = RestoreDC(mem, saved);
                let _ = BitBlt(
                    hdc,
                    rc.left,
                    rc.top,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                    Some(mem),
                    rc.left,
                    rc.top,
                    SRCCOPY,
                );
            }
            None => {
                // Fallback (backbuffer creation failed): compose directly,
                // still clipped to the invalid region.
                let saved = SaveDC(hdc);
                let _ = IntersectClipRect(hdc, rc.left, rc.top, rc.right, rc.bottom);
                compose(s, hdc, cw, ch, f_ui, f_small);
                let _ = RestoreDC(hdc, saved);
            }
        }
    }
    let _ = EndPaint(hwnd, &ps);
}

/// Draw the whole scene for `phase` into `hdc` (already clipped by the
/// caller). Touches `preview` incrementally, hence `&mut`.
unsafe fn compose(
    s: &mut OverlaySession,
    hdc: HDC,
    cw: i32,
    ch: i32,
    f_ui: windows::Win32::Graphics::Gdi::HFONT,
    f_small: windows::Win32::Graphics::Gdi::HFONT,
) {
    // Dimmed backdrop (both phases keep the captured desktop as context).
    if !s.dim.is_empty() {
        blit(
            hdc,
            &s.dim,
            s.frame.w as i32,
            s.frame.h as i32,
            0,
            0,
            cw,
            ch,
        );
    }
    if s.phase == Phase::Editing {
        edit::compose(s, hdc, f_ui, f_small);
        return;
    }

    let c = to_client(s.sel, &s.vs);
    if !c.is_empty() {
        let saved = SaveDC(hdc);
        let _ = IntersectClipRect(hdc, c.x, c.y, c.x + c.w, c.y + c.h);
        if (cw - s.frame.w as i32).abs() > 1 {
            let _ = SetStretchBltMode(hdc, HALFTONE);
            let _ = SetBrushOrgEx(hdc, 0, 0, None);
        }
        blit(
            hdc,
            &s.frame.px,
            s.frame.w as i32,
            s.frame.h as i32,
            0,
            0,
            cw,
            ch,
        );
        let _ = RestoreDC(hdc, saved);

        // Border: 1px near-black outside + a crisp 2px accent inside.
        let outer = RECT {
            left: c.x - 1,
            top: c.y - 1,
            right: c.x + c.w + 1,
            bottom: c.y + c.h + 1,
        };
        frame_solid(hdc, &outer, theme::SEL_OUTLINE_U32);
        let rc = RECT {
            left: c.x,
            top: c.y,
            right: c.x + c.w,
            bottom: c.y + c.h,
        };
        frame_solid(hdc, &rc, ACCENT);
        let inner = RECT {
            left: c.x + 1,
            top: c.y + 1,
            right: c.x + c.w - 1,
            bottom: c.y + c.h - 1,
        };
        frame_solid(hdc, &inner, ACCENT);

        // Resize handles: Lightshot-style outward arrows on all 8 border
        // points (corners aim diagonally, edges perpendicular). Shape, not
        // just color, marks them as resize targets (hit tolerance HIT, and
        // each handle already switches to its directional resize cursor).
        // Small selections get smaller arrows so neighbors do not collide.
        let (reach, half) = if c.w.min(c.h) < 120 { (6, 4) } else { (10, 7) };
        for (i, (hx, hy)) in handle_centers(c).into_iter().enumerate() {
            let (dx, dy) = handle_dir(i);
            draw_resize_arrow(hdc, hx, hy, dx, dy, reach, half);
        }

        // Size chip: dimensions at the selection's left edge, below it;
        // flips above near the bottom of the screen.
        let dims = format!("{} x {}", s.sel.w, s.sel.h);
        let chip_w = (dims.chars().count() as i32) * 7 + 16;
        let chip_x = c.x.clamp(
            theme::CHROME_PAD,
            (cw - chip_w - theme::CHROME_PAD).max(theme::CHROME_PAD),
        );
        let chip_y =
            if c.bottom() + theme::SELECT_CHIP_GAP + theme::CHIP_H <= ch - theme::CHROME_PAD {
                c.bottom() + theme::SELECT_CHIP_GAP
            } else {
                (c.y - theme::SELECT_CHIP_GAP - theme::CHIP_H).max(theme::CHROME_PAD)
            };
        let chip = RECT {
            left: chip_x,
            top: chip_y,
            right: chip_x + chip_w,
            bottom: chip_y + theme::CHIP_H,
        };
        round_face(hdc, &chip, theme::ELEVATED_U32, theme::BORDER_U32);
        let prev_font = SelectObject(hdc, f_small.into());
        crate::win::gdi::draw_label(
            hdc,
            RECT {
                left: chip.left + 8,
                top: chip.top,
                right: chip.right - 8,
                bottom: chip.bottom,
            },
            &dims,
            theme::FG_DIM_U32,
            windows::Win32::Graphics::Gdi::DT_LEFT
                | windows::Win32::Graphics::Gdi::DT_VCENTER
                | windows::Win32::Graphics::Gdi::DT_SINGLELINE
                | windows::Win32::Graphics::Gdi::DT_END_ELLIPSIS,
        );
        SelectObject(hdc, prev_font);
    }

    // Hint strip (two lines) pinned near the bottom center.
    let hw = (cw - 24).min(560);
    let hx = (cw - hw) / 2;
    let hy = (ch - 64).max(8);
    let strip = RECT {
        left: hx,
        top: hy,
        right: hx + hw,
        bottom: hy + 48,
    };
    round_face(hdc, &strip, theme::SURFACE_U32, theme::BORDER_U32);
    let st = crate::strings::s();
    let prev_font = SelectObject(hdc, f_ui.into());
    crate::win::gdi::draw_label(
        hdc,
        RECT {
            left: hx + 10,
            top: hy + 5,
            right: hx + hw - 10,
            bottom: hy + 27,
        },
        st.capture_hint,
        theme::FG_U32,
        windows::Win32::Graphics::Gdi::DT_CENTER
            | windows::Win32::Graphics::Gdi::DT_SINGLELINE
            | windows::Win32::Graphics::Gdi::DT_END_ELLIPSIS,
    );
    SelectObject(hdc, f_small.into());
    crate::win::gdi::draw_label(
        hdc,
        RECT {
            left: hx + 10,
            top: hy + 27,
            right: hx + hw - 10,
            bottom: hy + 45,
        },
        st.capture_hint_keys,
        theme::FG_DIM_U32,
        windows::Win32::Graphics::Gdi::DT_CENTER
            | windows::Win32::Graphics::Gdi::DT_SINGLELINE
            | windows::Win32::Graphics::Gdi::DT_END_ELLIPSIS,
    );
    SelectObject(hdc, prev_font);
}

// ---------------------------------------------------------------------------
// Tests: veil math and chrome placement (pure, no window shown)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn veil_blends_toward_theme_bg_and_stays_opaque() {
        let mut f = Frame::new(2, 1).unwrap();
        // BGRA: pixel 1 (B=255, G=128, R=64), pixel 2 black.
        f.px = vec![255, 128, 64, 0, 0, 0, 0, 255];
        let original = f.px.clone();
        let dim = dim_frame(&f);
        assert_eq!(f.px, original, "dim_frame must not touch the crisp frame");

        let keep = theme::VEIL_KEEP_PCT as i32;
        let mix = 100 - keep;
        let (br, bg, bb) = (theme::BG.0 as i32, theme::BG.1 as i32, theme::BG.2 as i32);
        let want = |c: i32, bgc: i32| ((c * keep + bgc * mix) / 100) as u8;
        assert_eq!(dim[0], want(255, bb), "pixel 1 blue");
        assert_eq!(dim[1], want(128, bg), "pixel 1 green");
        assert_eq!(dim[2], want(64, br), "pixel 1 red");
        assert_eq!(dim[3], 255, "opaque");
        assert_eq!(dim[4], want(0, bb), "pixel 2 blue");
        assert_eq!(dim[5], want(0, bg), "pixel 2 green");
        assert_eq!(dim[6], want(0, br), "pixel 2 red");
        assert_eq!(dim[7], 255, "opaque");
        // A mid gray stays mid gray-ish: strictly dimmer than the source.
        assert!(
            dim[1] < 128 && dim[1] > theme::BG.1,
            "restrained, not crushed"
        );
    }

    #[test]
    fn chrome_placeholders_match_theme_geometry() {
        // The tested coordinate model lives in the tokens (see spec section 6).
        assert_eq!(theme::RAIL_W, 60);
        assert_eq!(theme::CHROME_PAD, 8);
        assert_eq!(theme::RAIL_GAP, 4);
        assert_eq!(theme::STRIP_H, 46);
        assert_eq!(theme::CHIP_H, 22);
    }

    #[test]
    fn work_area_maps_from_screen_to_client_with_negative_monitor_origins() {
        // Dual 1920x1200 monitors: the virtual origin sits on the right
        // monitor, so the left one has negative screen coordinates.
        let vs = VirtualScreen {
            x: -1920,
            y: 0,
            w: 3840,
            h: 1200,
        };

        // Right monitor: taskbar band excluded (bottom 1160, not 1200).
        let right = work_to_client(
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1160,
            },
            &vs,
        );
        assert_eq!(
            right,
            RECT {
                left: 1920,
                top: 0,
                right: 3840,
                bottom: 1160,
            },
            "right monitor's rcWork must translate, not clamp"
        );

        // Left monitor (negative screen origin) maps onto the client origin.
        let left = work_to_client(
            RECT {
                left: -1920,
                top: 0,
                right: 0,
                bottom: 1160,
            },
            &vs,
        );
        assert_eq!(
            left,
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1160,
            }
        );

        // The taskbar-excluded height survives the conversion unchanged.
        assert_eq!(right.bottom - right.top, 1160);
        assert_eq!(left.bottom - left.top, 1160);
    }

    #[test]
    fn mouse_release_commits_a_real_rubber_band_only() {
        let prev = IRect::new(0, 0, 1920, 1200);
        let band = IRect::new(200, 250, 1300, 700);
        // Selection-on-mouse-release: a moved band of sane size commits.
        let d = ODrag::New {
            anchor: (200, 250),
            prev,
            moved: true,
        };
        assert_eq!(release_action(d, band), ReleaseAction::Commit);
        // A click without movement restores the previous selection.
        let d = ODrag::New {
            anchor: (200, 250),
            prev,
            moved: false,
        };
        assert_eq!(release_action(d, prev), ReleaseAction::Revert);
        // A sub-threshold band is click noise, not an edit request...
        let d = ODrag::New {
            anchor: (10, 10),
            prev,
            moved: true,
        };
        assert_eq!(
            release_action(d, IRect::new(10, 10, 3, 3)),
            ReleaseAction::Revert
        );
        // ...while the smallest band at the threshold commits.
        let d = ODrag::New {
            anchor: (10, 10),
            prev,
            moved: true,
        };
        assert_eq!(
            release_action(d, IRect::new(10, 10, 4, 4)),
            ReleaseAction::Commit
        );
        // Move/resize releases are adjustments: never leave the phase.
        assert_eq!(
            release_action(ODrag::Move { dx: 5, dy: 5 }, band),
            ReleaseAction::Stay
        );
        assert_eq!(
            release_action(
                ODrag::Resize {
                    handle: 0,
                    orig: band
                },
                band
            ),
            ReleaseAction::Stay
        );
    }

    #[test]
    fn band_movement_is_sticky_across_a_return_to_the_start() {
        let prev = IRect::new(0, 0, 1920, 1200);
        let mid = IRect::new(100, 100, 800, 600);
        // One-way movement marks the drag as moved...
        assert!(band_moved(false, prev, mid));
        // ...and a band that returns to its start rectangle stays marked, so
        // a full-screen drag still commits on release.
        assert!(band_moved(true, mid, prev));
        // A plain click never marks it.
        assert!(!band_moved(false, prev, prev));
    }
}
