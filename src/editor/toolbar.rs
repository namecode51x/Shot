//! Toolbar model: button layout, hit testing and paint code shared by
//! the overlay's rail/strip, the editor window and the owner-drawn buttons of
//! Settings/About.
//!
//! The toolbar is painted manually into the window's own DC — no child
//! controls — so the dark theme, focus rings and selection states stay under
//! our control and behave identically on every Windows build. All colors come
//! from [`crate::theme`]; buttons are icon-first with text reserved for the
//! document actions.

use crate::annotate::Tool;
use crate::image::Rgba;
use crate::strings::s;
use crate::theme;
use crate::win::dark;
use crate::win::gdi::{draw_label, fill_solid, round_face};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, Ellipse, GetStockObject, HDC, HFONT, NULL_BRUSH,
    PS_SOLID, Polyline, Rectangle, SelectObject,
};

/// Toolbar strip height in pixels (two rows).
pub const H: i32 = 76;

const MARGIN: i32 = 8;
const GAP: i32 = 4;
const GROUP: i32 = 14;
const ROW1_Y: i32 = 8;
const ROW1_H: i32 = 30;
const ROW2_Y: i32 = 44;
const ROW2_H: i32 = 26;
const MIN_W: i32 = 30;

/// Fixed width of an icon-only button in a row.
pub const ICON_W: i32 = 40;

/// Accent color as a palette-style `0x00BBGGRR` COLORREF constant.
pub const ACCENT_U32: u32 = theme::ACCENT_U32;

/// Everything a toolbar command needs to know about the current editor state.
#[derive(Debug, Clone, Copy)]
pub struct State {
    pub tool: Tool,
    pub color_idx: usize,
    pub width_idx: usize,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Tool(Tool),
    Undo,
    Redo,
    Copy,
    Save,
    Close,
    Color(usize),
    Width(usize),
    ZoomIn,
    ZoomOut,
    ZoomFit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Content {
    Text(&'static str),
    /// Icon-only content (rail tools, Undo/Redo); the tooltip chip carries
    /// the label, so no button loses its accessible name.
    Icon(Icon),
    Swatch(Rgba),
    Dot(u32),
}

/// Original line icons drawn with GDI primitives only — an 18 px box and a
/// 2 px stroke, no emoji, no copied artwork, no icon font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Pen,
    Line,
    Arrow,
    Rect,
    Box,
    Text,
    Highlight,
    Mosaic,
    Solid,
    Undo,
    Redo,
}

impl Icon {
    /// Icon for a drawing tool (rail / editor row 1).
    pub fn for_tool(t: Tool) -> Icon {
        match t {
            Tool::Pen => Icon::Pen,
            Tool::Line => Icon::Line,
            Tool::Arrow => Icon::Arrow,
            Tool::Rect => Icon::Rect,
            Tool::Box => Icon::Box,
            Tool::Text => Icon::Text,
            Tool::Highlight => Icon::Highlight,
            Tool::Mosaic => Icon::Mosaic,
            Tool::Solid => Icon::Solid,
        }
    }
}

fn poly(hdc: HDC, pts: &[POINT]) {
    if pts.len() >= 2 {
        unsafe {
            let _ = Polyline(hdc, pts);
        }
    }
}

/// Draw `icon` centered in `rect` at [`theme::ICON_PX`] with a
/// [`theme::ICON_STROKE`] stroke. Caller owns font selection (icons ignore it).
///
/// # Safety
/// `hdc` must be a valid device context for the duration of the call.
pub unsafe fn draw_icon(hdc: HDC, rect: &RECT, icon: Icon, color: u32) {
    let cx = (rect.left + rect.right) / 2;
    let cy = (rect.top + rect.bottom) / 2;
    let x0 = cx - theme::ICON_PX / 2;
    let y0 = cy - theme::ICON_PX / 2;
    let p = |a: i32, b: i32| POINT {
        x: x0 + a,
        y: y0 + b,
    };
    let cref = dark::colorref_u32(color);
    let sw = theme::ICON_STROKE;

    let pen = CreatePen(PS_SOLID, sw, cref);
    let prev_pen = SelectObject(hdc, pen.into());
    let brush = CreateSolidBrush(cref);
    let prev_brush = SelectObject(hdc, brush.into());
    let hollow = GetStockObject(NULL_BRUSH);

    match icon {
        Icon::Line => poly(hdc, &[p(3, 15), p(15, 3)]),
        Icon::Pen => {
            // Pencil: shaft plus nib band.
            poly(hdc, &[p(14, 3), p(6, 11)]);
            poly(hdc, &[p(4, 9), p(9, 14)]);
        }
        Icon::Arrow => {
            poly(hdc, &[p(3, 15), p(13, 5)]);
            poly(hdc, &[p(13, 11), p(13, 5), p(7, 5)]);
        }
        Icon::Text => {
            poly(hdc, &[p(4, 4), p(14, 4)]);
            poly(hdc, &[p(9, 4), p(9, 15)]);
        }
        Icon::Highlight => {
            // Text line plus a heavy marker bar underneath.
            poly(hdc, &[p(4, 6), p(14, 6)]);
            let thick = CreatePen(PS_SOLID, sw * 2, cref);
            let prev_thick = SelectObject(hdc, thick.into());
            poly(hdc, &[p(4, 13), p(14, 13)]);
            SelectObject(hdc, prev_thick);
            let _ = DeleteObject(thick.into());
        }
        Icon::Undo => {
            // Return arrow: riser on the right, head pointing left.
            poly(hdc, &[p(14, 14), p(14, 9), p(6, 9)]);
            poly(hdc, &[p(9, 6), p(6, 9), p(9, 12)]);
        }
        Icon::Redo => {
            poly(hdc, &[p(4, 14), p(4, 9), p(12, 9)]);
            poly(hdc, &[p(9, 6), p(12, 9), p(9, 12)]);
        }
        Icon::Box => {
            let prev = SelectObject(hdc, hollow);
            let _ = Rectangle(hdc, x0 + 3, y0 + 4, x0 + 15, y0 + 14);
            SelectObject(hdc, prev);
        }
        Icon::Rect => {
            let _ = Rectangle(hdc, x0 + 3, y0 + 6, x0 + 15, y0 + 12);
        }
        Icon::Solid => {
            let _ = Rectangle(hdc, x0 + 2, y0 + 2, x0 + 16, y0 + 16);
        }
        Icon::Mosaic => {
            // Checker: filled TL/BR, hollow TR/BL.
            let _ = Rectangle(hdc, x0 + 4, y0 + 4, x0 + 8, y0 + 8);
            let _ = Rectangle(hdc, x0 + 10, y0 + 10, x0 + 14, y0 + 14);
            let prev = SelectObject(hdc, hollow);
            let _ = Rectangle(hdc, x0 + 10, y0 + 4, x0 + 14, y0 + 8);
            let _ = Rectangle(hdc, x0 + 4, y0 + 10, x0 + 8, y0 + 14);
            SelectObject(hdc, prev);
        }
    }

    SelectObject(hdc, prev_brush);
    SelectObject(hdc, prev_pen);
    let _ = DeleteObject(brush.into());
    let _ = DeleteObject(pen.into());
}

/// Visual weight of a document action: Copy leads, Save supports, Close stays
/// quiet (see `docs/UI_REFRESH_SPEC.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    None,
    Primary,
    Secondary,
    Quiet,
}

/// Emphasis for a command's face (all surfaces share it).
pub fn emphasis(cmd: Cmd) -> Emphasis {
    match cmd {
        Cmd::Copy => Emphasis::Primary,
        Cmd::Save => Emphasis::Secondary,
        Cmd::Close => Emphasis::Quiet,
        _ => Emphasis::None,
    }
}

#[derive(Debug, Clone)]
pub struct Button {
    pub cmd: Cmd,
    pub rect: RECT,
    pub content: Content,
    pub selected: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct Toolbar {
    pub buttons: Vec<Button>,
    /// Full strip rectangle (below the caption, above the canvas).
    pub rect: RECT,
}

/// Visual style for the standalone owner-drawn push buttons (Settings/About).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceStyle {
    Normal,
    /// Accent-filled (OK / Apply / default action).
    Primary,
}

impl Toolbar {
    /// Lay out both rows for the given client width and editor state.
    pub fn layout(client_w: i32, st: State) -> Toolbar {
        let text = s();
        let tools: [(Tool, &'static str); 9] = [
            (Tool::Pen, text.tool_pen),
            (Tool::Line, text.tool_line),
            (Tool::Arrow, text.tool_arrow),
            (Tool::Rect, text.tool_rect),
            (Tool::Box, text.tool_box),
            (Tool::Text, text.tool_text),
            (Tool::Highlight, text.tool_highlight),
            (Tool::Mosaic, text.tool_mosaic),
            (Tool::Solid, text.tool_solid),
        ];

        let mut buttons: Vec<Button> = Vec::with_capacity(32);
        let client_w = client_w.max(320);

        // --- Row 1: tools + document actions, Close pinned to the right -----
        let close_w = text_w(text.tool_close).max(52);
        let close_x = client_w - MARGIN - close_w;
        let row1_avail = (close_x - GROUP - MARGIN).max(MIN_W * 4);

        let mut items: Vec<(Cmd, i32, Content, bool)> = Vec::new();
        for (t, _) in tools {
            items.push((
                Cmd::Tool(t),
                ICON_W,
                Content::Icon(Icon::for_tool(t)),
                false,
            ));
        }
        items.push((Cmd::Undo, ICON_W, Content::Icon(Icon::Undo), true));
        items.push((Cmd::Redo, ICON_W, Content::Icon(Icon::Redo), false));
        let actions: [(Cmd, &'static str); 2] =
            [(Cmd::Copy, text.tool_copy), (Cmd::Save, text.tool_save)];
        for (c, label) in actions {
            items.push((c, text_w(label).max(48), Content::Text(label), true));
        }

        let mut st1 = st;
        st1.color_idx = 0;
        st1.width_idx = 0;
        place(
            &items,
            MARGIN,
            ROW1_Y,
            ROW1_H,
            row1_avail,
            st1,
            &mut buttons,
        );

        // Close button (tab order last, drawn on row 1 right edge).
        buttons.push(Button {
            cmd: Cmd::Close,
            rect: RECT {
                left: close_x,
                top: ROW1_Y,
                right: close_x + close_w,
                bottom: ROW1_Y + ROW1_H,
            },
            content: Content::Text(text.tool_close),
            selected: false,
            enabled: true,
        });

        // --- Row 2: colors, widths, zoom ------------------------------------
        let mut items2: Vec<(Cmd, i32, Content, bool)> = Vec::new();
        for i in 0..crate::editor::PALETTE.len() {
            items2.push((
                Cmd::Color(i),
                ROW2_H,
                Content::Swatch(crate::editor::PALETTE[i]),
                i == 0,
            ));
        }
        for (i, w) in crate::editor::WIDTHS.iter().enumerate() {
            items2.push((Cmd::Width(i), 34, Content::Dot(*w), i == 0));
        }
        items2.push((Cmd::ZoomOut, 34, Content::Text("-"), true));
        items2.push((Cmd::ZoomIn, 34, Content::Text("+"), false));
        items2.push((
            Cmd::ZoomFit,
            text_w(text.tool_zoom_fit).max(46),
            Content::Text(text.tool_zoom_fit),
            false,
        ));

        place(
            &items2,
            MARGIN,
            ROW2_Y,
            ROW2_H,
            client_w - 2 * MARGIN,
            st,
            &mut buttons,
        );

        Toolbar {
            buttons,
            rect: RECT {
                left: 0,
                top: 0,
                right: client_w,
                bottom: H,
            },
        }
    }

    /// Index of the button containing the client point.
    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.buttons.iter().position(|b| {
            x >= b.rect.left && x < b.rect.right && y >= b.rect.top && y < b.rect.bottom
        })
    }
}

/// Desired width for a text label at the UI font size (approximation; the
/// layout shrinks uniformly when the window is narrow, labels ellipsize).
pub(crate) fn text_w(label: &str) -> i32 {
    (label.chars().count() as i32) * 8 + 20
}

/// Width a [`place`] call would take before any shrinking (same gap math).
pub(crate) fn ideal_width(items: &[(Cmd, i32, Content, bool)]) -> i32 {
    let n = items.len();
    if n == 0 {
        return 0;
    }
    let groups = items.iter().filter(|i| i.3).count().max(1);
    let gaps = GAP * (n as i32 - 1) + (GROUP - GAP) * groups as i32;
    items.iter().map(|i| i.1).sum::<i32>() + gaps
}

/// Place one row of buttons, shrinking widths uniformly when the content does
/// not fit `avail`. `pad_before` marks the first item of a logical group
/// (extra spacing instead of a normal gap). Returns the end x coordinate.
pub(crate) fn place(
    items: &[(Cmd, i32, Content, bool)],
    x0: i32,
    y: i32,
    h: i32,
    avail: i32,
    st: State,
    out: &mut Vec<Button>,
) -> i32 {
    let n = items.len();
    if n == 0 {
        return x0;
    }
    let groups = items.iter().filter(|i| i.3).count().max(1);
    let gaps = GAP * (n as i32 - 1) + (GROUP - GAP) * groups as i32;
    let sum: i32 = items.iter().map(|i| i.1).sum();

    let widths: Vec<i32> = if sum + gaps <= avail {
        items.iter().map(|i| i.1).collect()
    } else {
        let budget = (avail - gaps).max(n as i32 * 18);
        let mut w: Vec<i32> = items
            .iter()
            .map(|i| (((i.1 as i64 * budget as i64) / sum.max(1) as i64) as i32).max(18))
            .collect();
        let total: i32 = w.iter().sum::<i32>() + gaps;
        if total > avail {
            let each = ((avail - gaps) / n as i32).max(16);
            w = vec![each; n];
        }
        w
    };

    let mut x = x0;
    for (i, ((cmd, _, content, pad), w)) in items.iter().zip(widths.iter()).enumerate() {
        if i > 0 {
            x += if *pad { GROUP } else { GAP };
        }
        let selected = match cmd {
            Cmd::Tool(t) => *t == st.tool,
            Cmd::Color(c) => *c == st.color_idx,
            Cmd::Width(wi) => *wi == st.width_idx,
            _ => false,
        };
        let enabled = match cmd {
            Cmd::Undo => st.can_undo,
            Cmd::Redo => st.can_redo,
            _ => true,
        };
        out.push(Button {
            cmd: *cmd,
            rect: RECT {
                left: x,
                top: y,
                right: x + w,
                bottom: y + h,
            },
            content: *content,
            selected,
            enabled,
        });
        x += w;
    }
    x
}

/// Paint every button of a laid-out toolbar.
///
/// `font` is selected for the duration of the call and restored afterwards.
pub fn paint(
    hdc: HDC,
    tb: &Toolbar,
    hover: Option<usize>,
    focus: Option<usize>,
    kb: bool,
    font: HFONT,
) {
    unsafe {
        let prev = SelectObject(hdc, font.into());
        for (i, b) in tb.buttons.iter().enumerate() {
            let is_hover = hover == Some(i) && b.enabled;
            let is_focus = kb && focus == Some(i);
            paint_button(hdc, b, is_hover, is_focus);
        }
        SelectObject(hdc, prev);
    }
}

fn paint_button(hdc: HDC, b: &Button, hover: bool, focus: bool) {
    // Face: one rounded fill+border pass, then a shape cue for selection.
    // Clean look: rest faces are borderless (border == fill); the hairline
    // appears on hover, selection and disabled faces only.
    let (fill, border) = if !b.enabled {
        (theme::PRESSED_U32, theme::BORDER_U32)
    } else if b.selected {
        (
            ACCENT_U32,
            if focus {
                theme::ON_ACCENT_U32
            } else {
                ACCENT_U32
            },
        )
    } else {
        match emphasis(b.cmd) {
            Emphasis::Primary => {
                if hover {
                    (theme::ACCENT_HOVER_U32, theme::ACCENT_HOVER_U32)
                } else {
                    (ACCENT_U32, ACCENT_U32)
                }
            }
            Emphasis::Secondary => {
                if hover {
                    (theme::HOVER_U32, theme::BORDER_U32)
                } else {
                    (theme::ELEVATED_U32, theme::ELEVATED_U32)
                }
            }
            Emphasis::Quiet => {
                if hover {
                    (theme::ELEVATED_U32, theme::BORDER_U32)
                } else {
                    (theme::SURFACE_U32, theme::SURFACE_U32)
                }
            }
            Emphasis::None => {
                if hover {
                    (theme::HOVER_U32, theme::BORDER_U32)
                } else {
                    (theme::SURFACE_U32, theme::SURFACE_U32)
                }
            }
        }
    };
    round_face(hdc, &b.rect, fill, border);

    // Selection never rides on color alone: a 3px indicator bar is a shape.
    if b.selected && b.enabled {
        let bar = RECT {
            left: b.rect.left + 1,
            top: b.rect.top + 4,
            right: b.rect.left + 1 + theme::INDICATOR_W,
            bottom: b.rect.bottom - 4,
        };
        fill_solid(hdc, &bar, theme::ON_ACCENT_U32);
    }

    let fg = if !b.enabled {
        theme::FG_DISABLED_U32
    } else if b.selected {
        theme::ON_ACCENT_U32
    } else if hover || emphasis(b.cmd) != Emphasis::Quiet {
        theme::FG_U32
    } else {
        theme::FG_DIM_U32
    };

    match b.content {
        Content::Text(label) => {
            draw_label(
                hdc,
                b.rect,
                label,
                fg,
                windows::Win32::Graphics::Gdi::DT_CENTER
                    | windows::Win32::Graphics::Gdi::DT_VCENTER
                    | windows::Win32::Graphics::Gdi::DT_SINGLELINE,
            );
        }
        Content::Icon(icon) => unsafe {
            draw_icon(hdc, &b.rect, icon, fg);
        },
        Content::Swatch(c) => {
            let inset = 4;
            let inner = RECT {
                left: b.rect.left + inset,
                top: b.rect.top + inset,
                right: b.rect.right - inset,
                bottom: b.rect.bottom - inset,
            };
            fill_solid(
                hdc,
                &inner,
                (c.r as u32) | ((c.g as u32) << 8) | ((c.b as u32) << 16),
            );
            // Outline shape: on the accent fill the near-black edge reads;
            // on a quiet face the white edge reads (plus the indicator bar).
            let edge = if b.selected {
                theme::ON_ACCENT_U32
            } else {
                theme::FG_U32
            };
            crate::win::gdi::frame_solid(hdc, &inner, edge);
        }
        Content::Dot(d) => {
            let cx = (b.rect.left + b.rect.right) / 2;
            let cy = (b.rect.top + b.rect.bottom) / 2;
            let radius = (d as i32 / 2 + 2).clamp(3, 9);
            let cref = dark::colorref_u32(fg);
            unsafe {
                let br = CreateSolidBrush(cref);
                let pen = CreatePen(PS_SOLID, 1, cref);
                let pb = SelectObject(hdc, br.into());
                let pp = SelectObject(hdc, pen.into());
                let _ = Ellipse(hdc, cx - radius, cy - radius, cx + radius, cy + radius);
                SelectObject(hdc, pp);
                SelectObject(hdc, pb);
                let _ = DeleteObject(pen.into());
                let _ = DeleteObject(br.into());
            }
        }
    }
}

/// Paint a single owner-drawn push button face (used by Settings and About).
pub fn paint_face(
    hdc: HDC,
    rect: &RECT,
    label: &str,
    pressed: bool,
    enabled: bool,
    focus: bool,
    style: FaceStyle,
    font: HFONT,
) {
    let (fill, border) = if !enabled {
        (theme::PRESSED_U32, theme::PRESSED_U32)
    } else {
        match (style, pressed) {
            (FaceStyle::Primary, false) => (
                ACCENT_U32,
                if focus {
                    theme::ON_ACCENT_U32
                } else {
                    ACCENT_U32
                },
            ),
            (FaceStyle::Primary, true) => (theme::ACCENT_DOWN_U32, theme::ACCENT_DOWN_U32),
            // Clean look: borderless rest, accent hairline only on focus.
            (FaceStyle::Normal, false) => (
                theme::ELEVATED_U32,
                if focus {
                    ACCENT_U32
                } else {
                    theme::ELEVATED_U32
                },
            ),
            (FaceStyle::Normal, true) => (theme::PRESSED_U32, theme::PRESSED_U32),
        }
    };
    round_face(hdc, rect, fill, border);
    let color = if !enabled {
        theme::FG_DISABLED_U32
    } else if style == FaceStyle::Primary {
        theme::ON_ACCENT_U32
    } else {
        theme::FG_U32
    };
    unsafe {
        let prev = SelectObject(hdc, font.into());
        draw_label(
            hdc,
            *rect,
            label,
            color,
            windows::Win32::Graphics::Gdi::DT_CENTER
                | windows::Win32::Graphics::Gdi::DT_VCENTER
                | windows::Win32::Graphics::Gdi::DT_SINGLELINE,
        );
        SelectObject(hdc, prev);
    }
}

// ---------------------------------------------------------------------------
// Tests: layout invariants and icon glyphs (offscreen, no windows shown)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS,
    };

    fn state() -> State {
        State {
            tool: Tool::Pen,
            color_idx: 0,
            width_idx: 0,
            can_undo: false,
            can_redo: false,
        }
    }

    /// Render one icon into an 18x18 top-down DIB and return it as ASCII art.
    fn render_icon(icon: Icon) -> String {
        let n = theme::ICON_PX;
        unsafe {
            let mem = crate::win::gdi::MemDc::new().expect("memory DC");
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = n;
            bmi.bmiHeader.biHeight = -n;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(mem.0), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                .expect("DIB section");
            assert!(!bits.is_null());
            std::ptr::write_bytes(bits as *mut u8, 0, (n * n * 4) as usize);
            let prev = SelectObject(mem.0, bmp.into());
            draw_icon(
                mem.0,
                &RECT {
                    left: 0,
                    top: 0,
                    right: n,
                    bottom: n,
                },
                icon,
                0x00FF_FFFF,
            );
            SelectObject(mem.0, prev);
            let px = std::slice::from_raw_parts(bits as *const u8, (n * n * 4) as usize);
            let mut rows: Vec<String> = Vec::with_capacity(n as usize);
            for y in 0..n {
                let mut row = String::with_capacity(n as usize);
                for x in 0..n {
                    let i = ((y * n + x) * 4) as usize;
                    row.push(if px[i] | px[i + 1] | px[i + 2] != 0 {
                        '#'
                    } else {
                        '.'
                    });
                }
                rows.push(row);
            }
            let _ = DeleteObject(bmp.into());
            rows.join("\n")
        }
    }

    #[test]
    fn every_icon_draws_a_visible_unique_glyph() {
        let icons = [
            Icon::Pen,
            Icon::Line,
            Icon::Arrow,
            Icon::Rect,
            Icon::Box,
            Icon::Text,
            Icon::Highlight,
            Icon::Mosaic,
            Icon::Solid,
            Icon::Undo,
            Icon::Redo,
        ];
        let mut rendered: Vec<(Icon, String)> = Vec::new();
        for icon in icons {
            let art = render_icon(icon);
            println!("--- {icon:?} ---\n{art}");
            let lit = art.matches('#').count();
            assert!(lit >= 8, "{icon:?} drew only {lit} pixels");
            rendered.push((icon, art));
        }
        for (i, (a, aa)) in rendered.iter().enumerate() {
            for (b, ab) in &rendered[i + 1..] {
                assert_ne!(aa, ab, "{a:?} and {b:?} render identically");
            }
        }
    }

    #[test]
    fn document_actions_carry_the_expected_emphasis() {
        assert_eq!(emphasis(Cmd::Copy), Emphasis::Primary);
        assert_eq!(emphasis(Cmd::Save), Emphasis::Secondary);
        assert_eq!(emphasis(Cmd::Close), Emphasis::Quiet);
        assert_eq!(emphasis(Cmd::Undo), Emphasis::None);
        assert_eq!(emphasis(Cmd::Tool(Tool::Pen)), Emphasis::None);
        assert_eq!(emphasis(Cmd::Color(0)), Emphasis::None);
    }

    #[test]
    fn layout_is_inside_bounds_with_icon_tools_and_text_actions() {
        let w = 1024;
        let tb = Toolbar::layout(w, state());
        for b in &tb.buttons {
            assert!(
                b.rect.left >= 0 && b.rect.right <= w,
                "button outside width: {b:?}"
            );
            assert!(
                b.rect.top >= 0 && b.rect.bottom <= tb.rect.bottom,
                "button below strip: {b:?}"
            );
            assert!(
                b.rect.right > b.rect.left && b.rect.bottom > b.rect.top,
                "empty rect: {b:?}"
            );
        }
        let tools: Vec<_> = tb
            .buttons
            .iter()
            .filter(|b| matches!(b.cmd, Cmd::Tool(_)))
            .collect();
        assert_eq!(tools.len(), 9);
        for t in tools {
            assert!(
                matches!(t.content, Content::Icon(_)),
                "tool must be icon-only"
            );
            assert_eq!(t.rect.right - t.rect.left, ICON_W, "tool button width");
        }
        for (cmd, icon) in [(Cmd::Undo, Icon::Undo), (Cmd::Redo, Icon::Redo)] {
            let b = tb.buttons.iter().find(|b| b.cmd == cmd).expect("button");
            assert_eq!(b.content, Content::Icon(icon));
        }
        for cmd in [Cmd::Copy, Cmd::Save, Cmd::Close] {
            let b = tb.buttons.iter().find(|b| b.cmd == cmd).expect("button");
            assert!(
                matches!(b.content, Content::Text(_)),
                "actions stay labelled"
            );
        }
        let close = tb.buttons.iter().find(|b| b.cmd == Cmd::Close).unwrap();
        assert_eq!(close.rect.right, w - MARGIN, "Close pinned to the right");
    }

    #[test]
    fn narrow_layout_still_places_every_button() {
        let tb = Toolbar::layout(340, state());
        assert_eq!(
            tb.buttons.len(),
            Toolbar::layout(1600, state()).buttons.len()
        );
        for b in &tb.buttons {
            assert!(b.rect.right <= 340 && b.rect.right > b.rect.left);
        }
    }
}
