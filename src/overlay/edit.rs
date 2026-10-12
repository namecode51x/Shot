//! Inline annotation editing inside the capture overlay.
//!
//! Entered in place from [`super::confirm`] (no window transition, no
//! recapture): the selection crop becomes the immutable source of a
//! [`Preview`], and annotations live in crop coordinates (1:1, no zoom/pan).
//! Chrome is a vertical tool rail plus a horizontal action strip, both placed
//! edge-aware around the selection. Only invalidates the regions that can
//! actually change: picking a tool repaints the chrome, never the canvas —
//! together with the backbuffer present in [`super::paint`] this removes the
//! Phase A flicker.

use super::{OverlaySession, Phase, blit, key_down};
use crate::annotate::{History, Shape, Tool};
use crate::app::App;
use crate::editor::toolbar::{self, Button, Cmd, Content, State, Toolbar};
use crate::flatten::Preview;
use crate::image::{Frame, IRect, Rgba};
use crate::session::{self, Plan, Route, SaveOutcome};
use crate::strings::s;
use crate::theme;
use crate::win::dialog;
use crate::win::gdi::{OwnedGdi, draw_label, fill_solid, frame_solid, round_face};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ANSI_CHARSET, ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DT_END_ELLIPSIS, DT_LEFT,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, FW_NORMAL, HFONT, IntersectClipRect, InvalidateRect,
    OUT_DEFAULT_PRECIS, RestoreDC, SaveDC, SelectObject,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, ES_AUTOHSCROLL, GetWindowTextW, IDC_ARROW, IDC_CROSS,
    IDC_IBEAM, SendMessageW, WINDOW_STYLE, WM_KEYDOWN, WM_KILLFOCUS, WM_SETFONT, WS_BORDER,
    WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

impl From<super::layout::Rect> for RECT {
    fn from(r: super::layout::Rect) -> RECT {
        RECT {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

impl From<RECT> for super::layout::Rect {
    fn from(r: RECT) -> super::layout::Rect {
        super::layout::Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

/// Width of the vertical tool rail.
const RAIL_W: i32 = theme::RAIL_W;
/// Margin around chrome elements.
const PAD: i32 = theme::CHROME_PAD;
/// Vertical gap between rail buttons.
const RAIL_GAP: i32 = theme::RAIL_GAP;
/// Action-strip height.
const STRIP_H: i32 = theme::STRIP_H;
/// Narrow-strip (two stacked rows) metrics: 4px top pad + 30px colors row +
/// 4px gap + 30px actions row + 4px bottom pad == [`theme::STRIP_H_NARROW`].
const STRIP_ROW_PAD: i32 = 4;
const STRIP_ROW_GAP: i32 = 4;
const STRIP_ROW_H: i32 = (theme::STRIP_H_NARROW - 2 * STRIP_ROW_PAD - STRIP_ROW_GAP) / 2;
/// Height of the gap reserved for the status chip next to the strip.
const CHIP_GAP: i32 = 30;
const CHIP_H: i32 = theme::CHIP_H;

const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const VK_C: u32 = 0x43;
const VK_S: u32 = 0x53;
const VK_Y: u32 = 0x59;
const VK_Z: u32 = 0x5A;
const VK_CONTROL: u32 = 0x11;
const VK_SHIFT: i32 = 0x10;
/// `VK_RETURN`/`VK_ESCAPE` as seen by the subclassed EDIT control.
const KEY_RETURN: u16 = 0x0D;
const KEY_ESCAPE: u16 = 0x1B;
const SUBCLASS_ID: usize = 0x454F; // 'EO'

const TEXT_SIZES: [u32; 3] = crate::editor::TEXT_SIZES;

/// What a pure state change requires to be redrawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redraw {
    /// Rail/strip/status only — the canvas is untouched.
    Chrome,
    /// The document (canvas) changed; the strip may also change (enabled
    /// undo/redo) and a stale status message must be cleared.
    Document,
}

/// In-flight annotation drag (no pan/zoom: coordinates are crop pixels), or
/// an in-editor crop-window resize (coordinates are client pixels).
#[derive(Debug, Clone)]
pub enum Drag {
    Shape { anchor: (i32, i32) },
    Pen { pts: Vec<(i32, i32)> },
    /// Border-handle drag: `orig` is the canvas at drag start. Every move
    /// re-cuts from `orig` at the cursor and `recrop_to` applies the delta
    /// against the current canvas, so annotations track the photo.
    ResizeCanvas { handle: usize, orig: RECT },
}

/// Inline editing state living inside [`OverlaySession`].
pub struct EditSession {
    /// Current capture crop (re-cut from [`Self::full`] on every crop move).
    pub preview: Preview,
    /// Full capture behind the crop: growing the crop window re-cuts from
    /// here, so enlarging never invents pixels. Client coordinates equal
    /// full-frame coordinates (both relative to the virtual-screen origin).
    pub full: Frame,
    pub history: History,
    pub tool: Tool,
    pub color_idx: usize,
    pub width_idx: usize,
    pub draft: Option<Shape>,
    pub drag: Option<Drag>,
    /// Transient message (copied/saved/text prompt/secure-surface warning);
    /// empty shows the image dimensions instead.
    pub status: String,
    /// The current [`Self::status`] is an error (chip paints error state and
    /// keeps it until the next status change).
    pub status_err: bool,
    /// Tooltip for the hovered chrome button, shown in the status chip.
    /// Empty when no button is hovered.
    pub hover_tip: String,
    /// Open Text-tool edit: child HWND + crop-space anchor.
    pub text_edit: Option<(HWND, (i32, i32))>,
    text_font: Option<OwnedGdi<HFONT>>,
    /// Rail + strip buttons in one list (absolute client rects).
    pub bar: Toolbar,
    pub rail: RECT,
    pub strip: RECT,
    pub status_rect: RECT,
    pub hover: Option<usize>,
    pub focus: usize,
    pub kb: bool,
    /// Crop area in client coordinates (adjustable with the border arrows;
    /// the chrome is re-placed around it on every move).
    pub canvas: RECT,
    client_w: i32,
    client_h: i32,
    /// Available work area in client coordinates (selected monitor's
    /// `rcWork`): chrome stays inside it even when the selection hangs
    /// over the taskbar.
    work: RECT,
}

fn in_rect(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Smallest crop window the editor keeps (matches the click-noise threshold
/// a rubber-band must exceed to commit).
const MIN_CANVAS: i32 = crate::theme::SPACE_XS;

/// Resize `orig` by dragging `handle` to the client point, clamped to the
/// `fw`x`fh` full capture with a [`MIN_CANVAS`] floor. Cursor points map to
/// exclusive edges exactly like the selecting phase (`+1` on right/bottom).
fn resize_canvas_rect(handle: usize, orig: RECT, x: i32, y: i32, fw: i32, fh: i32) -> RECT {
    let l = orig.left;
    let t = orig.top;
    let r = orig.right;
    let b = orig.bottom;
    let (nl, nt, nr, nb) = match handle {
        0 => (
            x.clamp(0, r - MIN_CANVAS),
            y.clamp(0, b - MIN_CANVAS),
            r,
            b,
        ),
        1 => (l, y.clamp(0, b - MIN_CANVAS), r, b),
        2 => (
            l,
            y.clamp(0, b - MIN_CANVAS),
            (x + 1).clamp(l + MIN_CANVAS, fw),
            b,
        ),
        3 => (l, t, (x + 1).clamp(l + MIN_CANVAS, fw), b),
        4 => (
            l,
            t,
            (x + 1).clamp(l + MIN_CANVAS, fw),
            (y + 1).clamp(t + MIN_CANVAS, fh),
        ),
        5 => (l, t, r, (y + 1).clamp(t + MIN_CANVAS, fh)),
        6 => (
            x.clamp(0, r - MIN_CANVAS),
            t,
            r,
            (y + 1).clamp(t + MIN_CANVAS, fh),
        ),
        _ => (x.clamp(0, r - MIN_CANVAS), t, r, b),
    };
    RECT {
        left: nl,
        top: nt,
        right: nr,
        bottom: nb,
    }
}

/// Border-handle hit test around `canvas` (same centers and tolerance as the
/// selecting phase: NW, N, NE, E, SE, S, SW, W).
fn hit_canvas_handle(canvas: &RECT, x: i32, y: i32) -> Option<usize> {
    let r = IRect::new(
        canvas.left,
        canvas.top,
        canvas.right - canvas.left,
        canvas.bottom - canvas.top,
    );
    super::handle_centers(r)
        .into_iter()
        .position(|(hx, hy)| (x - hx).abs() <= super::HIT && (y - hy).abs() <= super::HIT)
}

fn union_rect(a: &RECT, b: &RECT) -> RECT {
    RECT {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

impl EditSession {
    /// Build the editing state for a fresh crop. Pure: creates no windows,
    /// captures nothing, and leaves the overlay untouched. `full` is the
    /// capture the crop was cut from; later crop-window moves re-cut from
    /// it so growing the canvas reveals real pixels.
    pub fn new(
        crop: Frame,
        full: Frame,
        canvas: RECT,
        client: (i32, i32),
        work: RECT,
        secure: bool,
    ) -> EditSession {
        let mut e = EditSession {
            preview: Preview::new(crop),
            full,
            history: History::new(),
            tool: Tool::Pen,
            color_idx: 2,
            width_idx: 1,
            draft: None,
            drag: None,
            status: if secure {
                s().secure_surface_msg.to_string()
            } else {
                String::new()
            },
            status_err: false,
            hover_tip: String::new(),
            text_edit: None,
            text_font: None,
            bar: Toolbar {
                buttons: Vec::new(),
                rect: RECT::default(),
            },
            rail: RECT::default(),
            strip: RECT::default(),
            status_rect: RECT::default(),
            hover: None,
            focus: 0,
            kb: false,
            canvas,
            client_w: client.0,
            client_h: client.1,
            work,
        };
        e.rebuild_bar();
        e
    }

    pub fn state(&self) -> State {
        State {
            tool: self.tool,
            color_idx: self.color_idx,
            width_idx: self.width_idx,
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
        }
    }

    /// Show a transient status message (`err` = error state on the chip).
    pub fn set_status(&mut self, msg: String, err: bool) {
        self.status = msg;
        self.status_err = err;
    }

    /// Clear the status chip back to the image dimensions.
    pub fn clear_status(&mut self) {
        self.status.clear();
        self.status_err = false;
    }

    fn color(&self) -> Rgba {
        crate::editor::PALETTE[self.color_idx.min(crate::editor::PALETTE.len() - 1)]
    }

    fn stroke_w(&self) -> u32 {
        crate::editor::WIDTHS[self.width_idx.min(crate::editor::WIDTHS.len() - 1)]
    }

    fn text_size(&self) -> u32 {
        TEXT_SIZES[self.width_idx.min(TEXT_SIZES.len() - 1)]
    }

    /// Client point → crop-space point (1:1).
    pub fn to_img(&self, x: i32, y: i32) -> (i32, i32) {
        (x - self.canvas.left, y - self.canvas.top)
    }

    /// Move the crop window to `canvas` (client coordinates, already clamped
    /// to the full capture). Re-cuts the source, shifts every annotation so
    /// it stays glued to the same photo pixels, drops the draft, clears the
    /// status back to the dimensions chip, and re-places the chrome.
    /// Returns false when nothing changed (or the window is degenerate).
    fn recrop_to(&mut self, canvas: RECT) -> bool {
        if canvas.left == self.canvas.left
            && canvas.top == self.canvas.top
            && canvas.right == self.canvas.right
            && canvas.bottom == self.canvas.bottom
        {
            return false;
        }
        let win = IRect::new(
            canvas.left,
            canvas.top,
            canvas.right - canvas.left,
            canvas.bottom - canvas.top,
        );
        let Ok(crop) = self.full.crop(win) else {
            return false;
        };
        self.history.translate_all(
            canvas.left - self.canvas.left,
            canvas.top - self.canvas.top,
        );
        // The committed render cache describes the old pixels: replay all.
        self.preview = Preview::new(crop);
        self.canvas = canvas;
        self.draft = None;
        self.clear_status();
        self.rebuild_bar();
        true
    }

    /// Rebuild the rail and the action strip for the current state.
    ///
    /// Geometry comes from the pure [`super::layout::plan`], which measures
    /// the chrome, generates Expanded/Compact/EdgeFallback candidates and
    /// keeps only placements that stay inside the client with the rail,
    /// strip and status chip pairwise disjoint (never painted through each
    /// other, never covering the selection when any alternative exists).
    /// This function only stacks the buttons into the chosen rectangles.
    fn rebuild_bar(&mut self) {
        let st = self.state();
        let text = s();
        let mut buttons: Vec<Button> = Vec::new();

        // --- Rail: nine tools, vertical -----------------------------------
        let tools: [Tool; 9] = [
            Tool::Pen,
            Tool::Line,
            Tool::Arrow,
            Tool::Rect,
            Tool::Box,
            Tool::Text,
            Tool::Highlight,
            Tool::Mosaic,
            Tool::Solid,
        ];
        let n = tools.len() as i32;
        let avail = (self.client_h - 2 * PAD).max(n * 20 + RAIL_GAP * (n - 1));
        let th_max = ((avail - RAIL_GAP * (n - 1)) / n).clamp(20, 38);

        // --- Strip items: undo/redo icons, colors, widths, copy/save/close -
        // (single-row order and grouping preserved exactly for wide shots)
        let mut items: Vec<(Cmd, i32, Content, bool)> = Vec::new();
        items.push((
            Cmd::Undo,
            toolbar::ICON_W,
            Content::Icon(toolbar::Icon::Undo),
            true,
        ));
        items.push((
            Cmd::Redo,
            toolbar::ICON_W,
            Content::Icon(toolbar::Icon::Redo),
            false,
        ));
        for i in 0..crate::editor::PALETTE.len() {
            items.push((
                Cmd::Color(i),
                28,
                Content::Swatch(crate::editor::PALETTE[i]),
                i == 0,
            ));
        }
        for (i, w) in crate::editor::WIDTHS.iter().enumerate() {
            items.push((Cmd::Width(i), 32, Content::Dot(*w), i == 0));
        }
        items.push((
            Cmd::Copy,
            toolbar::text_w(text.tool_copy).max(46),
            Content::Text(text.tool_copy),
            true,
        ));
        items.push((
            Cmd::Save,
            toolbar::text_w(text.tool_save).max(46),
            Content::Text(text.tool_save),
            false,
        ));
        items.push((
            Cmd::Close,
            toolbar::text_w(text.tool_close).max(52),
            Content::Text(text.tool_close),
            false,
        ));
        // Narrow captures (the reported small-shot deformation): a 686 px
        // single row dwarfs the shot, so colors go on a top row of their
        // own with the document actions + widths below (~414 px wide).
        let row_top: Vec<(Cmd, i32, Content, bool)> = items
            .iter()
            .filter(|(c, _, _, _)| matches!(c, Cmd::Color(_)))
            .cloned()
            .collect();
        let row_bottom: Vec<(Cmd, i32, Content, bool)> = items
            .iter()
            .filter(|(c, _, _, _)| !matches!(c, Cmd::Color(_)))
            .cloned()
            .collect();
        let single_ideal = toolbar::ideal_width(&items);
        let narrow_ideal =
            toolbar::ideal_width(&row_top).max(toolbar::ideal_width(&row_bottom));
        let narrow = (self.canvas.right - self.canvas.left) < single_ideal + 2 * PAD;
        let (strip_ideal, strip_h) = if narrow {
            (narrow_ideal, theme::STRIP_H_NARROW)
        } else {
            (single_ideal, STRIP_H)
        };

        // --- Adaptive chrome placement (pure) ------------------------------
        let lay = super::layout::plan(&super::layout::Measures {
            canvas: self.canvas.into(),
            client: (self.client_w, self.client_h),
            work: self.work.into(),
            rail_w: RAIL_W,
            rail_items: n,
            rail_gap: RAIL_GAP,
            th_max,
            th_min: 20,
            th_floor: 14,
            strip_ideal,
            strip_h,
            chip_w: 340.min((self.client_w - 2 * PAD).max(80)),
            chip_h: CHIP_H,
            pad: PAD,
            chip_gap: CHIP_GAP,
            margin: 4,
        });

        let mut y = lay.rail.top;
        for t in tools {
            buttons.push(Button {
                cmd: Cmd::Tool(t),
                rect: RECT {
                    left: lay.rail.left,
                    top: y,
                    right: lay.rail.left + RAIL_W,
                    bottom: y + lay.th,
                },
                content: Content::Icon(toolbar::Icon::for_tool(t)),
                selected: t == st.tool,
                enabled: true,
            });
            y += lay.th + lay.rail_gap;
        }
        self.rail = lay.rail.into();

        let avail_w = (lay.strip.right - lay.strip.left - 2 * PAD).max(120);
        if narrow {
            toolbar::place(
                &row_top,
                lay.strip.left + PAD,
                lay.strip.top + STRIP_ROW_PAD,
                STRIP_ROW_H,
                avail_w,
                st,
                &mut buttons,
            );
            toolbar::place(
                &row_bottom,
                lay.strip.left + PAD,
                lay.strip.top + STRIP_ROW_PAD + STRIP_ROW_H + STRIP_ROW_GAP,
                STRIP_ROW_H,
                avail_w,
                st,
                &mut buttons,
            );
        } else {
            let bh = STRIP_H - 8;
            toolbar::place(
                &items,
                lay.strip.left + PAD,
                lay.strip.top + 4,
                bh,
                avail_w,
                st,
                &mut buttons,
            );
        }
        self.strip = lay.strip.into();
        self.status_rect = lay.chip.into();
        self.bar = Toolbar {
            buttons,
            rect: union_rect(&self.rail, &self.strip),
        };
    }

    /// Apply a pure state change. Returns what must be redrawn.
    ///
    /// Deliberately side-effect free (no windows, no capture, no dialogs) so
    /// the "tool selection never disturbs the document" invariant is unit
    /// testable.
    pub fn apply(&mut self, cmd: Cmd) -> Option<Redraw> {
        match cmd {
            Cmd::Tool(t) => {
                self.tool = t;
                self.rebuild_bar();
                Some(Redraw::Chrome)
            }
            Cmd::Color(i) => {
                self.color_idx = i.min(crate::editor::PALETTE.len() - 1);
                self.rebuild_bar();
                Some(Redraw::Chrome)
            }
            Cmd::Width(i) => {
                self.width_idx = i.min(crate::editor::WIDTHS.len() - 1);
                self.rebuild_bar();
                Some(Redraw::Chrome)
            }
            Cmd::Undo => {
                if self.history.undo() {
                    self.draft = None;
                    self.clear_status();
                    self.rebuild_bar();
                    Some(Redraw::Document)
                } else {
                    None
                }
            }
            Cmd::Redo => {
                if self.history.redo() {
                    self.draft = None;
                    self.clear_status();
                    self.rebuild_bar();
                    Some(Redraw::Document)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Finish an in-flight drag at `at` (crop-space). Returns `true` when a
    /// shape was committed to the history.
    pub fn finish(&mut self, drag: Drag, at: (i32, i32)) -> bool {
        let mut pushed = false;
        match drag {
            Drag::Pen { pts } => {
                if pts.len() >= 2 {
                    let shape = Shape::Pen {
                        pts,
                        color: self.color(),
                        width: self.stroke_w(),
                    };
                    self.history.push(shape);
                    self.clear_status();
                    pushed = true;
                }
            }
            Drag::Shape { anchor } => {
                let shape =
                    crate::editor::make_draft(self.tool, anchor, at, self.color(), self.width_idx);
                if let Some(shape) = shape
                    && crate::editor::worth(&shape)
                {
                    self.history.push(shape);
                    self.clear_status();
                    pushed = true;
                }
            }
            // Crop-window resizes apply live (see on_move); nothing to commit.
            Drag::ResizeCanvas { .. } => {}
        }
        self.draft = None;
        if pushed {
            self.rebuild_bar();
        }
        pushed
    }

    /// The pending text EDIT lost keyboard focus (its window was deactivated
    /// or focus moved within/outside NameShot). The draft must survive
    /// untouched: no commit, no cancel, no focus stealing — explicit actions
    /// (Enter, Escape, toolbar and action commands) decide the text's fate.
    pub fn on_text_focus_lost(&mut self) {
        // Deliberately empty: committing or destroying the box here is
        // exactly the silent-draft-loss bug. See commit/cancel_pending_text.
    }

    /// Explicit-commit path: close the pending slot and push the text
    /// verbatim (intentional leading/trailing spaces preserved) as a shape
    /// anchored where the box stood. An all-whitespace box counts as empty
    /// and just closes (Enter on nothing changes no history).
    pub fn commit_pending_text(&mut self, text: &str) {
        let Some((_, anchor)) = self.text_edit.take() else {
            return;
        };
        self.clear_status();
        if text.trim().is_empty() {
            return;
        }
        let shape = Shape::Text {
            at: anchor,
            text: text.to_string(),
            color: self.color(),
            size: self.text_size(),
        };
        self.history.push(shape);
        self.rebuild_bar();
    }

    /// Explicit-cancel path (Esc): drop the slot without touching history.
    pub fn cancel_pending_text(&mut self) {
        self.text_edit = None;
        self.clear_status();
    }

    /// The frame that Copy/Save export: production flatten path, no draft.
    pub fn output_frame(&mut self) -> Frame {
        let items = self.history.items();
        self.preview.render(items, None).clone()
    }
}

// ---------------------------------------------------------------------------
// Invalidation helpers (region-limited)
// ---------------------------------------------------------------------------

fn inv(app: &App, r: RECT) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let h = app.overlay.borrow().as_ref().map(|s| s.hwnd);
    if let Some(h) = h {
        unsafe {
            let _ = InvalidateRect(Some(h), Some(&raw const r), false);
        }
    }
}

pub fn invalidate_canvas(app: &App) {
    let r = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .map(|e| e.canvas);
    if let Some(r) = r {
        inv(app, r);
    }
}

pub fn invalidate_bar(app: &App) {
    let rects = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref().map(|e| (e.rail, e.strip)));
    if let Some((rail, strip)) = rects {
        inv(app, rail);
        inv(app, strip);
    }
}

fn invalidate_status(app: &App) {
    let r = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .map(|e| e.status_rect);
    if let Some(r) = r {
        inv(app, r);
    }
}

fn invalidate_btn(app: &App, idx: usize) {
    let r = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .and_then(|e| e.bar.buttons.get(idx).map(|b| b.rect));
    if let Some(r) = r {
        inv(app, r);
    }
}

/// Invalidate the bounding union of two rects (old + new position of a chrome
/// element that moved), expanded by `pad` for overdraw (resize arrows reach
/// 10px beyond the canvas edge, rounded faces 1px). Without the union the
/// backbuffer keeps the old pixels and every move leaves a ghost trail — the
/// smeared arrows/strip in the resize bug report.
fn inv_union(app: &App, a: RECT, b: RECT, pad: i32) {
    let r = RECT {
        left: a.left.min(b.left) - pad,
        top: a.top.min(b.top) - pad,
        right: a.right.max(b.right) + pad,
        bottom: a.bottom.max(b.bottom) + pad,
    };
    inv(app, r);
}

#[cfg(test)]
mod inv_tests {
    use super::*;

    #[test]
    fn union_covers_old_and_new_chrome() {
        let old = RECT {
            left: 100,
            top: 100,
            right: 300,
            bottom: 200,
        };
        let new = RECT {
            left: 100,
            top: 100,
            right: 600,
            bottom: 200,
        };
        let u = RECT {
            left: old.left.min(new.left) - 2,
            top: old.top.min(new.top) - 2,
            right: old.right.max(new.right) + 2,
            bottom: old.bottom.max(new.bottom) + 2,
        };
        assert!(u.left <= 98 && u.right >= 602);
        assert!(u.left <= old.left && u.right >= old.right);
        assert!(u.left <= new.left && u.right >= new.right);
    }
}

fn invalidate_redraw(app: &App, redraw: Redraw) {
    match redraw {
        Redraw::Chrome => {
            invalidate_bar(app);
            invalidate_status(app);
        }
        Redraw::Document => {
            invalidate_canvas(app);
            invalidate_bar(app);
            invalidate_status(app);
        }
    }
}

// ---------------------------------------------------------------------------
// Commands (side-effecting; called from input handlers)
// ---------------------------------------------------------------------------

fn overlay_hwnd(app: &App) -> Option<HWND> {
    app.overlay.borrow().as_ref().map(|s| s.hwnd)
}

/// Tooltip text for a chrome button: the visible label plus a short hint
/// (shortcuts are listed only where they are actually wired up). Shown in
/// the status chip while the button is hovered — the overlay deliberately
/// has no floating tooltip windows (each hover invalidates one small rect).
fn tip_for(cmd: Cmd) -> String {
    let st = s();
    match cmd {
        Cmd::Tool(t) => {
            let (label, hint) = match t {
                Tool::Pen => (st.tool_pen, st.tip_pen),
                Tool::Line => (st.tool_line, st.tip_line),
                Tool::Arrow => (st.tool_arrow, st.tip_arrow),
                Tool::Rect => (st.tool_rect, st.tip_rect),
                Tool::Box => (st.tool_box, st.tip_box),
                Tool::Text => (st.tool_text, st.tip_text),
                Tool::Highlight => (st.tool_highlight, st.tip_mark),
                Tool::Mosaic => (st.tool_mosaic, st.tip_mosaic),
                Tool::Solid => (st.tool_solid, st.tip_solid),
            };
            format!("{}: {}", label, hint)
        }
        Cmd::Color(i) => {
            let name = st.color_names.get(i).copied().unwrap_or("");
            format!("{}: {}", st.tool_color, name)
        }
        Cmd::Width(i) => {
            let w = crate::editor::WIDTHS.get(i).copied().unwrap_or(0);
            format!("{}: {} px", st.tip_width, w)
        }
        Cmd::Undo => st.tip_undo.to_string(),
        Cmd::Redo => st.tip_redo.to_string(),
        Cmd::Copy => st.tip_copy.to_string(),
        Cmd::Save => st.tip_save.to_string(),
        Cmd::Close => st.tip_close.to_string(),
        _ => String::new(),
    }
}

/// Dispatch a chrome command. Never holds the session borrow across a call
/// that can pump messages (dialogs, clipboard waits).
pub fn command(app: &'static App, cmd: Cmd) {
    match cmd {
        Cmd::Copy => {
            // Explicit action: pending text must land in the exported image.
            let _ = commit_any_text(app);
            do_copy(app);
            return;
        }
        Cmd::Save => {
            let _ = commit_any_text(app);
            do_save(app);
            return;
        }
        Cmd::Close => {
            // Explicit close: discard immediately, no confirmation.
            super::apply_route(app, Route::Close);
            return;
        }
        Cmd::Tool(_) | Cmd::Color(_) | Cmd::Width(_) => {
            // A pending text edit commits first (it invalidates the canvas
            // itself); the tool change below must not repaint the canvas.
            let _ = commit_any_text(app);
        }
        _ => {}
    }
    let redraw = {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        e.apply(cmd)
    };
    if let Some(redraw) = redraw {
        invalidate_redraw(app, redraw);
    }
}

fn do_copy(app: &'static App) {
    let st = s();
    let (hwnd, frame) = {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        (ov.hwnd, e.output_frame())
    };
    match crate::win::clipboard::copy_image(Some(hwnd), &frame) {
        Ok(()) => {
            // Success ends the session: the status chip dies with the
            // window, so the confirmation moves to the tray balloon.
            if let Some(t) = app.tray.borrow().as_ref() {
                t.balloon(st.state_copied, st.app_name, false);
            }
            super::apply_route(app, Route::Copy { ok: true });
        }
        Err(e) => {
            // Failure keeps the session open and lands on the status chip
            // (error state) instead of interrupting with a modal box.
            set_status_error(
                app,
                format!("{}: {}", st.state_copy_fail_title, first_line(&e)),
            );
            super::apply_route(app, Route::Copy { ok: false });
        }
    }
}

/// First line of an error, trimmed — one chip line stays one line.
fn first_line(e: &impl std::fmt::Display) -> String {
    e.to_string()
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Put an error message on the status chip and repaint just that region.
fn set_status_error(app: &App, msg: String) {
    if let Some(e) = app
        .overlay
        .borrow_mut()
        .as_mut()
        .and_then(|s| s.edit.as_mut())
    {
        e.set_status(msg, true);
    }
    invalidate_status(app);
}

fn do_save(app: &'static App) {
    let st = s();
    let settings = app.settings.borrow().clone();
    let (hwnd, frame) = {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        (ov.hwnd, e.output_frame())
    };

    let dir = dialog::resolve_save_dir(&settings.save_dir);
    let name = crate::output::default_filename(settings.format, crate::os::local_time());
    let picked = match dialog::save_image(Some(hwnd), Some(&dir), &name, settings.format) {
        Ok(v) => v,
        Err(e) => {
            set_status_error(
                app,
                format!("{}: {}", st.state_save_fail_title, first_line(&e)),
            );
            super::apply_route(app, Route::Save(SaveOutcome::Failed));
            return;
        }
    };
    let Some(path) = picked else {
        // Save-As cancelled: the session stays exactly as it was.
        super::apply_route(app, Route::Save(SaveOutcome::Cancelled));
        return;
    };

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
            // Written bytes are the proof: announce on the tray, close.
            if let Some(t) = app.tray.borrow().as_ref() {
                t.balloon(st.state_saved, &path.to_string_lossy(), false);
            }
            super::apply_route(app, Route::Save(SaveOutcome::Saved));
        }
        Err(e) => {
            set_status_error(
                app,
                format!("{}: {}", st.state_save_fail_title, first_line(&e)),
            );
            super::apply_route(app, Route::Save(SaveOutcome::Failed));
        }
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

pub fn on_down(app: &'static App, hwnd: HWND, x: i32, y: i32) {
    let hit = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .and_then(|e| e.bar.hit(x, y));
    if let Some(idx) = hit {
        commit_any_text(app);
        let cmd = {
            let mut b = app.overlay.borrow_mut();
            let Some(ov) = b.as_mut() else { return };
            let Some(e) = ov.edit.as_mut() else { return };
            e.focus = idx;
            e.kb = true;
            e.hover = Some(idx);
            e.bar.buttons[idx].cmd
        };
        unsafe {
            let _ = SetFocus(Some(hwnd));
        }
        invalidate_btn(app, idx);
        command(app, cmd);
        return;
    }

    let over_canvas = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .map(|e| in_rect(&e.canvas, x, y))
        .unwrap_or(false);

    // Border arrows first: resizing the crop window beats annotating.
    // A pending text edit commits first so its anchor stays valid.
    let handle = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .and_then(|e| hit_canvas_handle(&e.canvas, x, y));
    if let Some(handle) = handle {
        commit_any_text(app);
        {
            let mut b = app.overlay.borrow_mut();
            let Some(ov) = b.as_mut() else { return };
            let Some(e) = ov.edit.as_mut() else { return };
            e.drag = Some(Drag::ResizeCanvas {
                handle,
                orig: e.canvas,
            });
            e.draft = None;
        }
        unsafe {
            SetCapture(hwnd);
        }
        return;
    }

    if !over_canvas {
        return;
    }

    commit_any_text(app);

    let (img, tool) = {
        let b = app.overlay.borrow();
        let Some(ov) = b.as_ref() else { return };
        let Some(e) = ov.edit.as_ref() else { return };
        (e.to_img(x, y), e.tool)
    };

    if tool == Tool::Text {
        begin_text(app, (x, y), img);
        return;
    }

    {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        e.drag = Some(match tool {
            Tool::Pen => Drag::Pen { pts: vec![img] },
            _ => Drag::Shape { anchor: img },
        });
        e.draft = None;
    }
    unsafe {
        SetCapture(hwnd);
    }
}

pub fn on_move(app: &'static App, _hwnd: HWND, x: i32, y: i32) {
    // Hover feedback on the rail/strip only (never the canvas).
    let hover = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .and_then(|e| e.bar.hit(x, y));
    let prev = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .and_then(|e| e.hover);
    if prev != hover {
        let tip = hover
            .and_then(|i| {
                app.overlay
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.edit.as_ref())
                    .and_then(|e| e.bar.buttons.get(i).map(|b| tip_for(b.cmd)))
            })
            .unwrap_or_default();
        let mut tip_changed = false;
        if let Some(e) = app
            .overlay
            .borrow_mut()
            .as_mut()
            .and_then(|s| s.edit.as_mut())
        {
            e.hover = hover;
            tip_changed = e.hover_tip != tip;
            e.hover_tip = tip;
        }
        if let Some(i) = prev {
            invalidate_btn(app, i);
        }
        if let Some(i) = hover {
            invalidate_btn(app, i);
        }
        if tip_changed {
            invalidate_status(app);
        }
    }

    let dragging = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .map(|e| e.drag.is_some())
        .unwrap_or(false);
    if !dragging {
        return;
    }

    // Crop-window resize: re-cut live so the photo follows the arrows.
    let resizing = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .and_then(|e| match &e.drag {
            Some(Drag::ResizeCanvas { handle, orig }) => Some((*handle, *orig)),
            _ => None,
        });
    if let Some((handle, orig)) = resizing {
        let (fw, fh, old_canvas, old_rail, old_strip, old_status) = {
            let b = app.overlay.borrow();
            let Some(ov) = b.as_ref() else { return };
            let Some(e) = ov.edit.as_ref() else { return };
            (
                e.full.w as i32,
                e.full.h as i32,
                e.canvas,
                e.rail,
                e.strip,
                e.status_rect,
            )
        };
        let new = resize_canvas_rect(handle, orig, x, y, fw, fh);
        let applied = {
            let mut b = app.overlay.borrow_mut();
            let Some(ov) = b.as_mut() else { return };
            let Some(e) = ov.edit.as_mut() else { return };
            e.recrop_to(new)
        };
        if applied {
            let (new_canvas, new_rail, new_strip, new_status) = {
                let b = app.overlay.borrow();
                let Some(ov) = b.as_ref() else { return };
                let Some(e) = ov.edit.as_ref() else { return };
                (e.canvas, e.rail, e.strip, e.status_rect)
            };
            // The canvas AND every chrome panel moved: repaint the union of
            // old + new for each. Invalidating only the new rects leaves the
            // old rail/strip/arrows in the persistent backbuffer — the ghost
            // smear in the report. Canvas gets arrow room (12px), panels 2px.
            inv_union(app, old_canvas, new_canvas, 12);
            inv_union(app, old_rail, new_rail, 2);
            inv_union(app, old_strip, new_strip, 2);
            inv_union(app, old_status, new_status, 2);
        }
        return;
    }

    let img = {
        let b = app.overlay.borrow();
        let Some(ov) = b.as_ref() else { return };
        let Some(e) = ov.edit.as_ref() else { return };
        e.to_img(x, y)
    };

    // Repaint only the union of the previous and current draft bounds
    // (image space → client space, padded, clipped to the canvas): a stroke
    // repaints O(draft) pixels instead of the whole crop, matching what
    // Preview::render refreshes. Degenerate drags repaint nothing at all.
    let dirty: Option<RECT> = {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        let old = e.draft.as_ref().map(|d| d.bounds());
        match e.drag.as_mut() {
            Some(Drag::Shape { anchor }) => {
                let anchor = *anchor;
                let (tool, color, wi) = (e.tool, e.color(), e.width_idx);
                e.draft = crate::editor::make_draft(tool, anchor, img, color, wi);
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
                    let (color, w) = (e.color(), e.stroke_w());
                    e.draft = Some(Shape::Pen {
                        pts,
                        color,
                        width: w,
                    });
                }
            }
            // Handled with an early return above (live re-cut per move).
            Some(Drag::ResizeCanvas { .. }) => {}
            None => {}
        }
        let new = e.draft.as_ref().map(|d| d.bounds());
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
            let r = r.expand(2);
            let c = &e.canvas;
            RECT {
                left: (c.left + r.x).max(c.left),
                top: (c.top + r.y).max(c.top),
                right: (c.left + r.right()).min(c.right),
                bottom: (c.top + r.bottom()).min(c.bottom),
            }
        })
        .filter(|r| r.right > r.left && r.bottom > r.top)
    };
    if let Some(r) = dirty {
        inv(app, r);
    }
}

pub fn on_up(app: &'static App, x: i32, y: i32) {
    let drag = {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        e.drag.take()
    };
    let Some(drag) = drag else { return };
    unsafe {
        let _ = ReleaseCapture();
    }
    if matches!(drag, Drag::ResizeCanvas { .. }) {
        // Applied live on every move; one full repaint so the rail/strip
        // settle around the final window with no ghost pixels left behind
        // (a single full frame is negligible once per drag).
        if let Some(h) = app.overlay.borrow().as_ref().map(|s| s.hwnd) {
            unsafe {
                let _ = InvalidateRect(Some(h), None, false);
            }
        }
        return;
    }
    let changed = {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        let img = e.to_img(x, y);
        e.finish(drag, img)
    };
    invalidate_canvas(app);
    invalidate_status(app);
    if changed {
        invalidate_bar(app);
    }
}

pub fn on_key(app: &'static App, vk: u32) {
    let ctrl = key_down(VK_CONTROL as i32);
    let shift = key_down(VK_SHIFT);
    match vk {
        VK_ESCAPE => {
            let _ = super::apply_route(app, Route::Close);
        }
        VK_C if ctrl => command(app, Cmd::Copy),
        VK_S if ctrl => command(app, Cmd::Save),
        VK_Z if ctrl => command(app, if shift { Cmd::Redo } else { Cmd::Undo }),
        VK_Y if ctrl => command(app, Cmd::Redo),
        VK_TAB => {
            let (old, n) = {
                let b = app.overlay.borrow();
                let Some(ov) = b.as_ref() else { return };
                let Some(e) = ov.edit.as_ref() else { return };
                (e.focus, e.bar.buttons.len())
            };
            if n == 0 {
                return;
            }
            {
                let mut b = app.overlay.borrow_mut();
                if let Some(e) = b.as_mut().and_then(|s| s.edit.as_mut()) {
                    e.focus = if shift {
                        (e.focus + n - 1) % n
                    } else {
                        (e.focus + 1) % n
                    };
                    e.kb = true;
                }
            }
            invalidate_btn(app, old);
            invalidate_btn(app, e_focus(app));
        }
        VK_RETURN => {
            let cmd = {
                let b = app.overlay.borrow();
                let Some(ov) = b.as_ref() else { return };
                let Some(e) = ov.edit.as_ref() else { return };
                e.bar.buttons.get(e.focus).map(|b| b.cmd)
            };
            if let Some(cmd) = cmd {
                command(app, cmd);
            }
        }
        _ => {}
    }
}

fn e_focus(app: &App) -> usize {
    app.overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .map(|e| e.focus)
        .unwrap_or(0)
}

pub fn clear_hover(app: &App) {
    let prev = app
        .overlay
        .borrow()
        .as_ref()
        .and_then(|s| s.edit.as_ref())
        .and_then(|e| e.hover);
    if let Some(i) = prev {
        if let Some(e) = app
            .overlay
            .borrow_mut()
            .as_mut()
            .and_then(|s| s.edit.as_mut())
        {
            e.hover = None;
        }
        invalidate_btn(app, i);
    }
}

pub fn cursor_for(app: &App, x: i32, y: i32) -> PCWSTR {
    let info = app.overlay.borrow().as_ref().and_then(|s| {
        s.edit.as_ref().map(|e| {
            let over_bar = e.bar.hit(x, y).is_some();
            let handle = hit_canvas_handle(&e.canvas, x, y);
            (over_bar, handle, in_rect(&e.canvas, x, y), e.tool)
        })
    });
    let Some((over_bar, handle, over_canvas, tool)) = info else {
        return IDC_CROSS;
    };
    if over_bar {
        return IDC_ARROW;
    }
    if let Some(h) = handle {
        return super::handle_cursor(h);
    }
    if over_canvas {
        return match tool {
            Tool::Text => IDC_IBEAM,
            _ => IDC_CROSS,
        };
    }
    IDC_CROSS
}

// ---------------------------------------------------------------------------
// Painting (called from super::compose after the dimmed backdrop)
// ---------------------------------------------------------------------------

pub(super) unsafe fn compose(
    ov: &mut OverlaySession,
    hdc: windows::Win32::Graphics::Gdi::HDC,
    f_ui: HFONT,
    f_small: HFONT,
) {
    debug_assert_eq!(ov.phase, Phase::Editing);
    let Some(e) = ov.edit.as_mut() else { return };
    let c = e.canvas;

    if c.right > c.left && c.bottom > c.top {
        let saved = SaveDC(hdc);
        let _ = IntersectClipRect(hdc, c.left, c.top, c.right, c.bottom);
        fill_solid(hdc, &c, crate::win::dark::CANVAS);
        let (hist, prev, draft) = (&e.history, &mut e.preview, &e.draft);
        let frame = prev.render(hist.items(), draft.as_ref());
        blit(
            hdc,
            &frame.px,
            frame.w as i32,
            frame.h as i32,
            c.left,
            c.top,
            c.right - c.left,
            c.bottom - c.top,
        );
        let _ = RestoreDC(hdc, saved);

        // Photo edges, drawn outside the canvas clip so they are always
        // visible: 1px near-black outside + a crisp 2px accent inside, the
        // same treatment as the selecting phase.
        let outer = RECT {
            left: c.left - 1,
            top: c.top - 1,
            right: c.right + 1,
            bottom: c.bottom + 1,
        };
        frame_solid(hdc, &outer, theme::SEL_OUTLINE_U32);
        frame_solid(hdc, &c, theme::ACCENT_U32);
        let inner = RECT {
            left: c.left + 1,
            top: c.top + 1,
            right: c.right - 1,
            bottom: c.bottom - 1,
        };
        frame_solid(hdc, &inner, theme::ACCENT_U32);

        // Resize arrows on the photo borders (functional: drag to re-cut
        // the crop window; annotations stay glued to their pixels).
        let (reach, half) = if (c.right - c.left).min(c.bottom - c.top) < 120 {
            (6, 4)
        } else {
            (10, 7)
        };
        for (i, (hx, hy)) in super::handle_centers(IRect::new(
            c.left,
            c.top,
            c.right - c.left,
            c.bottom - c.top,
        ))
        .into_iter()
        .enumerate()
        {
            let (dx, dy) = super::handle_dir(i);
            super::draw_resize_arrow(hdc, hx, hy, dx, dy, reach, half);
        }
    }

    // Chrome panels: rail and strip sit on one restrained surface.
    round_face(hdc, &e.rail, theme::SURFACE_U32, theme::BORDER_U32);
    round_face(hdc, &e.strip, theme::SURFACE_U32, theme::BORDER_U32);

    toolbar::paint(hdc, &e.bar, e.hover, Some(e.focus), e.kb, f_ui);

    // Status chip: hovered-button tooltip, else transient message, else dims.
    let show_error = e.status_err && e.hover_tip.is_empty() && !e.status.is_empty();
    let chip_text = if !e.hover_tip.is_empty() {
        e.hover_tip.clone()
    } else if e.status.is_empty() {
        format!("{} x {}", e.preview.source().w, e.preview.source().h)
    } else {
        e.status.clone()
    };
    let (chip_fill, chip_border, chip_fg) = if show_error {
        (theme::ELEVATED_U32, theme::ERROR_U32, theme::ERROR_U32)
    } else {
        (theme::ELEVATED_U32, theme::BORDER_U32, theme::FG_DIM_U32)
    };
    round_face(hdc, &e.status_rect, chip_fill, chip_border);
    let prev_font = SelectObject(hdc, f_small.into());
    draw_label(
        hdc,
        RECT {
            left: e.status_rect.left + 8,
            top: e.status_rect.top,
            right: e.status_rect.right - 8,
            bottom: e.status_rect.bottom,
        },
        &chip_text,
        chip_fg,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    SelectObject(hdc, prev_font);
}

// ---------------------------------------------------------------------------
// Text tool (transient child EDIT, subclassed — same pattern as the editor)
// ---------------------------------------------------------------------------

/// Explicit-action commit: when a text box is open, read it, destroy it and
/// push its text (see [`EditSession::commit_pending_text`]). Deliberately
/// used only from deliberate actions (toolbar commands, close, quick save,
/// exit) — never from focus changes.
pub(crate) fn commit_any_text(app: &'static App) -> bool {
    let h = {
        let b = app.overlay.borrow();
        b.as_ref()
            .and_then(|s| s.edit.as_ref())
            .and_then(|e| e.text_edit.map(|(h, _)| h))
    };
    if let Some(h) = h {
        commit_text(app, h);
        true
    } else {
        false
    }
}

fn begin_text(app: &'static App, dev: (i32, i32), img: (i32, i32)) {
    commit_any_text(app);

    let Some(parent) = overlay_hwnd(app) else {
        return;
    };
    let size = {
        let b = app.overlay.borrow();
        b.as_ref()
            .and_then(|s| s.edit.as_ref())
            .map(|e| e.text_size())
            .unwrap_or(24)
    };

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
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        e.text_font = Some(OwnedGdi(font));
        e.text_edit = Some((edit, img));
        e.set_status(s().text_prompt.to_string(), false);
        e.tool = Tool::Text;
        e.rebuild_bar();
    }
    app.text_edit_active.set(true);
    unsafe {
        let _ = SetFocus(Some(edit));
    }
    invalidate_canvas(app);
    invalidate_bar(app);
    invalidate_status(app);
}

/// Read the edit box, destroy it, and (when non-empty) push the text shape.
fn commit_text(app: &App, edit: HWND) {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetWindowTextW(edit, &mut buf) };
    // Verbatim: leading/trailing spaces are intentional content; only an
    // all-whitespace box counts as empty (commit_pending_text decides).
    let text = String::from_utf16_lossy(&buf[..n.max(0) as usize]);

    let matches = {
        let b = app.overlay.borrow();
        b.as_ref()
            .and_then(|s| s.edit.as_ref())
            .and_then(|e| e.text_edit)
            .map(|(h, _)| h == edit)
            .unwrap_or(false)
    };

    unsafe {
        let _ = RemoveWindowSubclass(edit, Some(edit_proc), SUBCLASS_ID);
        let _ = DestroyWindow(edit);
    }
    app.text_edit_active.set(false);

    if !matches {
        return;
    }
    {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        e.commit_pending_text(&text);
    }
    invalidate_canvas(app);
    invalidate_status(app);
    invalidate_bar(app);
}

/// Esc in the text box: discard the pending text.
fn cancel_text(app: &App, edit: HWND) {
    unsafe {
        let _ = RemoveWindowSubclass(edit, Some(edit_proc), SUBCLASS_ID);
        let _ = DestroyWindow(edit);
    }
    app.text_edit_active.set(false);
    {
        let mut b = app.overlay.borrow_mut();
        let Some(ov) = b.as_mut() else { return };
        let Some(e) = ov.edit.as_mut() else { return };
        e.cancel_pending_text();
    }
    invalidate_canvas(app);
    invalidate_status(app);
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
            KEY_RETURN => {
                commit_text(app, hwnd);
                LRESULT(0)
            }
            KEY_ESCAPE => {
                // Esc in the box discards the draft only: the close policy
                // keeps the session open for this route, so only a later
                // Esc (with the box gone) reaches the overlay and closes.
                let open = app.overlay.borrow().is_some();
                if session::plan(Route::TextCancel, open) == Plan::CancelTextDraft {
                    cancel_text(app, hwnd);
                }
                LRESULT(0)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        },
        WM_KILLFOCUS => {
            // The draft survives focus leaving the box (external window or
            // another NameShot control): no commit, no destroy, no focus
            // stealing back. Explicit actions decide the text's fate.
            {
                let mut b = app.overlay.borrow_mut();
                if let Some(e) = b.as_mut().and_then(|s| s.edit.as_mut()) {
                    e.on_text_focus_lost();
                }
            }
            // The control outlives the focus loss now: forward to the
            // default EDIT proc so its internal state stays consistent.
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
        _ => DefSubclassProc(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Tests: the inline editor state machine (no windows, no capture)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::IRect;

    fn session() -> EditSession {
        let f = Frame::new(80, 60).unwrap();
        EditSession::new(
            f,
            Frame::new(1920, 1200).unwrap(),
            RECT {
                left: 100,
                top: 50,
                right: 180,
                bottom: 110,
            },
            (1920, 1200),
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1200,
            },
            false,
        )
    }

    fn source_bytes(e: &EditSession) -> Vec<u8> {
        e.preview.source().px.clone()
    }

    #[test]
    fn narrow_canvas_gets_a_two_row_strip_and_wide_canvas_keeps_one() {
        // The 80x60 session canvas is narrower than the single-row strip:
        // colors on the top row, document actions below, nothing lost.
        let narrow = session();
        assert_eq!(
            narrow.strip.bottom - narrow.strip.top,
            theme::STRIP_H_NARROW,
            "narrow capture must use the two-row strip"
        );
        let strip_btns: Vec<_> = narrow
            .bar
            .buttons
            .iter()
            .filter(|b| !matches!(b.cmd, Cmd::Tool(_)))
            .collect();
        assert_eq!(
            strip_btns.len(),
            16,
            "no control may be lost in two-row mode"
        );
        for b in &strip_btns {
            assert!(
                b.rect.top >= narrow.strip.top && b.rect.bottom <= narrow.strip.bottom,
                "strip button outside the panel: {b:?}"
            );
        }
        let color_bottom = strip_btns
            .iter()
            .filter(|b| matches!(b.cmd, Cmd::Color(_)))
            .map(|b| b.rect.bottom)
            .max()
            .unwrap();
        let action_top = strip_btns
            .iter()
            .filter(|b| matches!(b.cmd, Cmd::Copy))
            .map(|b| b.rect.top)
            .min()
            .unwrap();
        assert!(
            color_bottom <= action_top,
            "colors must sit on the top row, actions below"
        );

        // A 1300 px wide canvas keeps the unchanged single row.
        let f = Frame::new(1300, 800).unwrap();
        let wide = EditSession::new(
            f,
            Frame::new(1920, 1200).unwrap(),
            RECT {
                left: 300,
                top: 100,
                right: 1600,
                bottom: 900,
            },
            (1920, 1200),
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1200,
            },
            false,
        );
        assert_eq!(
            wide.strip.bottom - wide.strip.top,
            theme::STRIP_H,
            "wide capture must keep the single-row strip"
        );
    }

    #[test]
    fn resize_canvas_rect_clamps_to_the_frame_and_the_minimum() {
        let orig = RECT {
            left: 100,
            top: 100,
            right: 300,
            bottom: 250,
        };
        // SE grows (cursor maps to the exclusive edge).
        assert_eq!(
            resize_canvas_rect(4, orig, 400, 350, 1920, 1200),
            RECT {
                left: 100,
                top: 100,
                right: 401,
                bottom: 351
            }
        );
        // NW shrinks down to the 4px floor, never inverts.
        assert_eq!(
            resize_canvas_rect(0, orig, 298, 248, 1920, 1200),
            RECT {
                left: 296,
                top: 246,
                right: 300,
                bottom: 250
            }
        );
        // Clamped to the full capture on every side.
        assert_eq!(
            resize_canvas_rect(4, orig, 5000, 5000, 1920, 1200),
            RECT {
                left: 100,
                top: 100,
                right: 1920,
                bottom: 1200
            }
        );
        assert_eq!(
            resize_canvas_rect(0, orig, -500, -500, 1920, 1200),
            RECT {
                left: 0,
                top: 0,
                right: 300,
                bottom: 250
            }
        );
        // N edge only moves the top.
        assert_eq!(
            resize_canvas_rect(1, orig, 999, 200, 1920, 1200),
            RECT {
                left: 100,
                top: 200,
                right: 300,
                bottom: 250
            }
        );
    }

    #[test]
    fn recrop_grows_from_the_full_capture_and_keeps_annotations_glued() {
        // Full frame with a white right half so growth is detectable.
        let mut full = Frame::new(200, 150).unwrap();
        full.fill_rect(IRect::new(100, 0, 100, 150), Rgba::opaque(255, 255, 255));
        let crop = full.crop(IRect::new(10, 10, 80, 60)).unwrap();
        let work = RECT {
            left: 0,
            top: 0,
            right: 200,
            bottom: 150,
        };
        let mut e = EditSession::new(
            crop,
            full,
            RECT {
                left: 10,
                top: 10,
                right: 90,
                bottom: 70,
            },
            (200, 150),
            work,
            false,
        );
        e.history.push(Shape::Line {
            a: (0, 0),
            b: (10, 10),
            color: Rgba::opaque(255, 0, 0),
            width: 2,
        });
        // Same window: no-op.
        assert!(!e.recrop_to(RECT {
            left: 10,
            top: 10,
            right: 90,
            bottom: 70
        }));

        // Grow right into the white half: new pixels are real, the line
        // stays at its crop-relative coordinates (left edge did not move).
        assert!(e.recrop_to(RECT {
            left: 10,
            top: 10,
            right: 130,
            bottom: 70
        }));
        assert_eq!((e.preview.source().w, e.preview.source().h), (120, 60));
        assert_eq!(
            e.preview.source().get(119, 0).unwrap(),
            [255, 255, 255, 255]
        );
        assert_eq!(e.history.len(), 1);

        // Slide the window right: annotations shift with their pixels.
        assert!(e.recrop_to(RECT {
            left: 30,
            top: 10,
            right: 130,
            bottom: 70
        }));
        assert_eq!(
            e.history.items(),
            &[Shape::Line {
                a: (20, 0),
                b: (30, 10),
                color: Rgba::opaque(255, 0, 0),
                width: 2
            }]
        );

        // Shrink onto a 10x10 corner: the line leaves the window, rendering
        // must clip (never panic) and export the new dimensions.
        assert!(e.recrop_to(RECT {
            left: 30,
            top: 10,
            right: 40,
            bottom: 20
        }));
        let out = e.output_frame();
        assert_eq!((out.w, out.h), (10, 10));
    }

    #[test]
    fn tool_selection_never_touches_source_or_history() {
        let mut e = session();
        e.history.push(Shape::Line {
            a: (0, 0),
            b: (10, 10),
            color: Rgba::opaque(255, 0, 0),
            width: 2,
        });
        let src = source_bytes(&e);
        let len = e.history.len();
        for t in [
            Tool::Line,
            Tool::Arrow,
            Tool::Rect,
            Tool::Box,
            Tool::Text,
            Tool::Highlight,
            Tool::Mosaic,
            Tool::Solid,
            Tool::Pen,
        ] {
            let redraw = e.apply(Cmd::Tool(t));
            assert_eq!(redraw, Some(Redraw::Chrome), "tool {t:?}");
        }
        assert_eq!(source_bytes(&e), src, "source bitmap must be immutable");
        assert_eq!(e.history.len(), len, "history must be untouched");
        assert_eq!(e.tool, Tool::Pen);
        assert!(e.draft.is_none() && e.drag.is_none());
    }

    #[test]
    fn color_and_width_selection_are_chrome_only() {
        let mut e = session();
        let src = source_bytes(&e);
        for i in 0..crate::editor::PALETTE.len() {
            assert_eq!(e.apply(Cmd::Color(i)), Some(Redraw::Chrome));
        }
        for i in 0..crate::editor::WIDTHS.len() {
            assert_eq!(e.apply(Cmd::Width(i)), Some(Redraw::Chrome));
        }
        assert_eq!(source_bytes(&e), src);
        assert_eq!(e.history.len(), 0);
    }

    #[test]
    fn canvas_geometry_survives_command_application() {
        let mut e = session();
        let canvas = RECT { ..e.canvas };
        let rail = RECT { ..e.rail };
        e.apply(Cmd::Tool(Tool::Mosaic));
        e.apply(Cmd::Color(5));
        assert_eq!(e.canvas, canvas, "crop rect must stay fixed");
        assert_eq!(e.rail, rail, "tool rail must stay fixed");
    }

    #[test]
    fn output_frame_is_the_production_flatten_result() {
        let mut e = session();
        let items = vec![
            Shape::Line {
                a: (1, 1),
                b: (30, 20),
                color: Rgba::opaque(10, 200, 30),
                width: 3,
            },
            Shape::Mosaic {
                rect: IRect::new(4, 4, 16, 16),
                block: 4,
            },
        ];
        for it in &items {
            e.history.push(it.clone());
        }
        let out = e.output_frame();
        let expected = crate::flatten::flatten(e.preview.source(), e.history.items());
        assert_eq!(out, expected);
        // The source itself is never mutated by rendering.
        assert_eq!(e.preview.source(), &Frame::new(80, 60).unwrap());
    }

    #[test]
    fn undo_redo_via_apply_redraws_document() {
        let mut e = session();
        e.history.push(Shape::Line {
            a: (0, 0),
            b: (5, 5),
            color: Rgba::opaque(1, 2, 3),
            width: 2,
        });
        assert_eq!(e.apply(Cmd::Undo), Some(Redraw::Document));
        assert_eq!(e.history.len(), 0);
        assert_eq!(e.apply(Cmd::Redo), Some(Redraw::Document));
        assert_eq!(e.history.len(), 1);
        // Nothing to undo on an empty document.
        e.apply(Cmd::Undo);
        assert_eq!(e.apply(Cmd::Undo), None);
    }

    #[test]
    fn drag_finish_commits_shapes() {
        let mut e = session();
        // Pen drag: two points are enough.
        let pushed = e.finish(
            Drag::Pen {
                pts: vec![(5, 5), (12, 14)],
            },
            (12, 14),
        );
        assert!(pushed);
        assert_eq!(e.history.len(), 1);
        // Shape drags use a shape tool: degenerate (same point) is dropped.
        e.apply(Cmd::Tool(Tool::Box));
        let pushed = e.finish(Drag::Shape { anchor: (10, 10) }, (10, 10));
        assert!(!pushed);
        assert_eq!(e.history.len(), 1);
        // A real box drag commits.
        let pushed = e.finish(Drag::Shape { anchor: (10, 10) }, (40, 40));
        assert!(pushed);
        assert_eq!(e.history.len(), 2);
        assert!(e.draft.is_none() && e.drag.is_none());
    }

    #[test]
    fn new_session_is_selecting_neutral() {
        let e = session();
        assert!(e.history.is_empty());
        assert_eq!(e.tool, Tool::Pen);
        assert_eq!(e.color_idx, 2);
        assert_eq!(e.width_idx, 1);
        assert!(e.text_edit.is_none() && e.drag.is_none());
        // Rail holds all nine tools, the strip all document actions.
        let tools = e
            .bar
            .buttons
            .iter()
            .filter(|b| matches!(b.cmd, Cmd::Tool(_)))
            .count();
        assert_eq!(tools, 9);
        for c in [Cmd::Undo, Cmd::Redo, Cmd::Copy, Cmd::Save, Cmd::Close] {
            assert!(e.bar.buttons.iter().any(|b| b.cmd == c), "missing {c:?}");
        }
    }

    #[test]
    fn tips_cover_every_button_with_shortcuts_only_where_wired() {
        // Shortcuts must exist in on_key before a tip may advertise them.
        for (cmd, key) in [
            (Cmd::Undo, "Ctrl+Z"),
            (Cmd::Redo, "Ctrl+Y"),
            (Cmd::Copy, "Ctrl+C"),
            (Cmd::Save, "Ctrl+S"),
            (Cmd::Close, "Esc"),
        ] {
            let tip = tip_for(cmd);
            assert!(
                tip.contains(key),
                "tip for {cmd:?} must mention {key}: {tip}"
            );
        }
        // Every tool/color/width button gets a non-empty, labelled tip.
        for t in [
            Tool::Pen,
            Tool::Line,
            Tool::Arrow,
            Tool::Rect,
            Tool::Box,
            Tool::Text,
            Tool::Highlight,
            Tool::Mosaic,
            Tool::Solid,
        ] {
            let tip = tip_for(Cmd::Tool(t));
            assert!(
                !tip.is_empty() && tip.contains(':'),
                "tool tip for {t:?}: {tip}"
            );
        }
        assert_eq!(tip_for(Cmd::Color(2)), "Color: Red");
        assert_eq!(tip_for(Cmd::Width(1)), "Stroke width: 4 px");
    }

    // --- Text-edit lifecycle: focus-loss survival + explicit commit/cancel --

    /// Placeholder handle for session-level tests (no window is created).
    fn fake_hwnd(tag: usize) -> HWND {
        HWND(tag as *mut _)
    }

    #[test]
    fn external_focus_loss_preserves_the_pending_draft() {
        let mut e = session();
        let h = fake_hwnd(0x1234);
        e.text_edit = Some((h, (10, 20)));
        e.status = "Type, Enter to place, Esc to cancel".to_string();

        e.on_text_focus_lost();

        // Nothing was committed, cancelled or cleared: the box slot, its
        // anchor and the prompt all survive an external WM_KILLFOCUS.
        assert_eq!(e.text_edit, Some((h, (10, 20))));
        assert_eq!(e.status, "Type, Enter to place, Esc to cancel");
        assert_eq!(e.history.len(), 0);
    }

    #[test]
    fn app_owned_focus_transition_keeps_the_draft_until_explicit_commit() {
        let mut e = session();
        let h = fake_hwnd(0x2345);
        e.text_edit = Some((h, (5, 6)));
        e.status = "Type, Enter to place, Esc to cancel".to_string();

        // Focus leaves the box (external steal, then a NameShot control
        // taking over): the draft stays until an explicit action.
        e.on_text_focus_lost();
        assert_eq!(e.text_edit, Some((h, (5, 6))));
        e.on_text_focus_lost();
        assert_eq!(e.text_edit, Some((h, (5, 6))));

        // The toolbar/action commit consumes the slot; the tool switch that
        // follows it never loses the committed shape.
        e.commit_pending_text("Hola");
        assert!(e.text_edit.is_none());
        let _ = e.apply(Cmd::Tool(Tool::Pen));
        assert_eq!(e.history.len(), 1);
        match &e.history.items()[0] {
            Shape::Text { at, text, .. } => {
                assert_eq!(*at, (5, 6));
                assert_eq!(text, "Hola");
            }
            _ => panic!("expected the committed text shape"),
        }
    }

    #[test]
    fn explicit_commit_preserves_spaces_and_whitespace_only_box_closes_free() {
        let mut e = session();
        e.text_edit = Some((fake_hwnd(0x3456), (3, 4)));
        e.status = "Type, Enter to place, Esc to cancel".to_string();
        e.commit_pending_text("  Hola  ");
        assert_eq!(e.history.len(), 1);
        match &e.history.items()[0] {
            Shape::Text { at, text, .. } => {
                assert_eq!(*at, (3, 4));
                // Intentional leading/trailing spaces survive the commit.
                assert_eq!(text, "  Hola  ");
            }
            _ => panic!("expected the committed text shape"),
        }
        assert!(e.text_edit.is_none());
        assert!(e.status.is_empty());

        // Enter on an all-whitespace box just closes the slot.
        e.text_edit = Some((fake_hwnd(0x4567), (30, 40)));
        e.status = "Type, Enter to place, Esc to cancel".to_string();
        e.commit_pending_text("   ");
        assert!(e.text_edit.is_none());
        assert!(e.status.is_empty());
        assert_eq!(e.history.len(), 1);

        // Commit with no open box is a free no-op.
        e.commit_pending_text("ghost");
        assert_eq!(e.history.len(), 1);
    }

    #[test]
    fn cancel_discards_the_draft_without_touching_history() {
        let mut e = session();
        e.history.push(Shape::Line {
            a: (0, 0),
            b: (4, 4),
            color: Rgba::opaque(255, 0, 0),
            width: 2,
        });
        let committed = e.history.len();
        e.text_edit = Some((fake_hwnd(0x5678), (1, 2)));
        e.status = "Type, Enter to place, Esc to cancel".to_string();

        e.cancel_pending_text();

        assert!(e.text_edit.is_none());
        assert!(e.status.is_empty());
        assert_eq!(e.history.len(), committed);
    }

    #[test]
    fn committed_text_renders_into_the_exported_frame() {
        let mut e = session();
        e.text_edit = Some((fake_hwnd(0x6789), (8, 6)));
        e.status = "Type, Enter to place, Esc to cancel".to_string();

        e.commit_pending_text("Hola");

        assert!(e.text_edit.is_none());
        assert!(e.status.is_empty());
        assert_eq!(e.history.len(), 1);
        // The export is the production flatten of the committed history...
        let out = e.output_frame();
        let expected = crate::flatten::flatten(e.preview.source(), e.history.items());
        assert_eq!(out, expected);
        // ...and the GDI-rendered glyphs actually landed inside the bitmap.
        assert!(
            e.preview.source().diff_pixels(&out) > 0,
            "committed text must change exported pixels"
        );
        // The source capture is never mutated by rendering or export.
        assert_eq!(e.preview.source(), &Frame::new(80, 60).unwrap());
    }

    #[test]
    fn solid_redaction_covers_committed_text_in_the_exported_frame() {
        let mut e = session();
        e.text_edit = Some((fake_hwnd(0x789A), (8, 6)));
        e.commit_pending_text("SECRETO");
        let before = e.output_frame();
        assert!(
            e.preview.source().diff_pixels(&before) > 0,
            "precondition: text is visible in the export"
        );

        // Redact everything, then export through the same Copy/Save path.
        e.history.push(Shape::Solid {
            rect: IRect::new(0, 0, 80, 60),
            color: Rgba::opaque(7, 7, 7),
        });
        let out = e.output_frame();

        for y in 0..60 {
            for x in 0..80 {
                // BGRA: flat opaque colour, no glyph pixel survives.
                assert_eq!(out.get(x, y).unwrap(), [7, 7, 7, 255], "pixel {x},{y}");
            }
        }
        assert_eq!(e.preview.source(), &Frame::new(80, 60).unwrap());
    }

    #[test]
    fn rail_and_strip_are_icon_first_and_edge_aware() {
        let crop = Frame::new(80, 60).unwrap();
        let client = (1200, 800);
        let work = RECT {
            left: 0,
            top: 0,
            right: 1200,
            bottom: 800,
        };

        // Canvas with room on the left: rail sits left of it.
        let e = EditSession::new(
            crop.clone(),
            Frame::new(1200, 800).unwrap(),
            RECT {
                left: 400,
                top: 200,
                right: 900,
                bottom: 500,
            },
            client,
            work,
            false,
        );
        assert!(
            e.rail.right <= 400,
            "rail must use the free left side: {:?}",
            e.rail
        );
        // Every tool button is icon-only.
        for b in &e.bar.buttons {
            if matches!(b.cmd, Cmd::Tool(_)) {
                assert!(matches!(b.content, Content::Icon(_)), "tools are icon-only");
            }
            if b.cmd == Cmd::Undo || b.cmd == Cmd::Redo {
                assert!(
                    matches!(b.content, Content::Icon(_)),
                    "undo/redo are icon-only"
                );
            }
        }
        // Strip prefers below the selection and the chip docks beside it.
        assert!(
            e.strip.top >= 500,
            "strip below the selection: {:?}",
            e.strip
        );
        assert_eq!(
            e.status_rect.bottom,
            e.strip.top - 4,
            "chip docks above the strip"
        );
        for r in [e.rail, e.strip, e.status_rect] {
            assert!(r.left >= 0 && r.right <= client.0 && r.top >= 0 && r.bottom <= client.1);
        }

        // Canvas flush left (no room for the rail): it flips to the right.
        let e = EditSession::new(
            crop.clone(),
            Frame::new(1200, 800).unwrap(),
            RECT {
                left: 20,
                top: 20,
                right: 400,
                bottom: 400,
            },
            client,
            work,
            false,
        );
        assert!(e.rail.left >= 400, "rail must flip right: {:?}", e.rail);

        // Canvas flush bottom: strip flips above the selection.
        let e = EditSession::new(
            crop,
            Frame::new(1200, 800).unwrap(),
            RECT {
                left: 300,
                top: 400,
                right: 700,
                bottom: 790,
            },
            client,
            work,
            false,
        );
        assert!(e.strip.bottom <= 400, "strip flips above: {:?}", e.strip);
        assert_eq!(
            e.status_rect.top,
            e.strip.bottom + 4,
            "chip docks below the strip"
        );
        assert!(e.status_rect.bottom <= 800);
    }

    #[test]
    fn chrome_never_enters_the_taskbar_band_of_the_work_area() {
        // Production wiring: a low selection (scenario A) on a 1920x1200
        // monitor whose taskbar owns y 1160..1200. The layout must plan
        // every chrome rect above the taskbar while the selection itself
        // may still reach into that band.
        let crop = Frame::new(80, 60).unwrap();
        let full = Frame::new(1920, 1200).unwrap();
        let client = (1920, 1200);
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1160,
        };
        for canvas in [
            RECT {
                left: 1570,
                top: 910,
                right: 1900,
                bottom: 1117,
            },
            RECT {
                left: 1640,
                top: 990,
                right: 1900,
                bottom: 1150,
            },
        ] {
            let e = EditSession::new(crop.clone(), full.clone(), canvas, client, work, false);
            for (name, r) in [
                ("rail", e.rail),
                ("strip", e.strip),
                ("chip", e.status_rect),
            ] {
                if r.right <= r.left {
                    continue; // empty chip is legal
                }
                assert!(r.bottom <= 1160, "{name} crossed the taskbar: {r:?}");
                assert!(r.bottom > r.top, "{name} degenerate: {r:?}");
            }
        }
    }

    #[test]
    fn error_status_lives_until_the_next_status_change() {
        let mut e = session();
        assert!(!e.status_err, "starts neutral");
        e.set_status("Copy failed: clipboard locked".to_string(), true);
        assert!(e.status_err && !e.status.is_empty());
        // Any later status change clears the error with it.
        e.set_status("Type, Enter to place, Esc to cancel".to_string(), false);
        assert!(!e.status_err);
        e.set_status("Copy failed: again".to_string(), true);
        e.clear_status();
        assert!(!e.status_err && e.status.is_empty());
    }
}
