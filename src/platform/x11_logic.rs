//! OS-free helpers behind the Linux/X11 adapters (M1, extended in M1b).
//!
//! This module is compiled on every target on purpose: the wire-protocol
//! adapters live behind `cfg(target_os = "linux")`, while everything that can
//! be decided without a server connection — pixel-format conversion, geometry,
//! keysym tables, dialog argv, the M1b selection session and the
//! event-mapping policy — lives here and is unit-tested by the regular suite
//! on every host. Nothing in this module talks to an X11 server: the raw
//! `x11rb` match that produces [`RawEvent`] is the thin, compile-verified
//! `x11/eventmap.rs` glue.

use crate::hotkey::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
use crate::image::{Frame, IRect, Rgba};
use crate::theme;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// XImage (ZPixmap) pixel conversion
// ---------------------------------------------------------------------------

/// Server-side description of a `ZPixmap` sample area, assembled from the
/// connection setup (`pixmap_formats`, image byte order) and the chosen
/// visual (colour masks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageFormat {
    pub width: u32,
    pub height: u32,
    /// Bits per pixel of one sample slot (8/16/24/32).
    pub bits_per_pixel: u8,
    /// Scanline padding in bits (X11 protocol allows 8 or 32).
    pub scanline_pad_bits: u8,
    /// `true` when the server sends the most-significant byte first.
    pub big_endian: bool,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
}

impl ImageFormat {
    /// Bytes per scanline: `width * bits_per_pixel` padded up to `scanline_pad`.
    pub fn stride(&self) -> Option<usize> {
        let bits = (self.width as usize).checked_mul(self.bits_per_pixel as usize)?;
        let pad = (self.scanline_pad_bits as usize).max(8);
        let padded = bits.checked_add(pad - 1)? / pad;
        padded.checked_mul(pad / 8)
    }

    /// Exact byte length of a `ZPixmap` block for this format.
    pub fn byte_len(&self) -> Option<usize> {
        self.stride()?.checked_mul(self.height as usize)
    }
}

/// `(shift, bits)` of a contiguous channel mask; `None` when the mask is
/// zero or not a contiguous run of bits (anything else is not a visual a
/// screenshot tool can sample).
fn channel_layout(mask: u32) -> Option<(u32, u32)> {
    if mask == 0 {
        return None;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones();
    if bits < 32 && (mask >> shift) != (1u32 << bits) - 1 {
        return None;
    }
    Some((shift, bits))
}

/// Extract one channel from a pixel value and scale it to 0..=255 with
/// rounded bit replication (`max = 2^bits - 1`).
fn scale_channel(raw: u32, mask: u32) -> u8 {
    let Some((shift, bits)) = channel_layout(mask) else {
        return 0;
    };
    let v = ((raw & mask) >> shift) as u64;
    let max = if bits >= 32 {
        u32::MAX as u64
    } else {
        (1u64 << bits) - 1
    };
    ((v * 255 + max / 2) / max) as u8
}

/// Inverse of [`scale_channel`]: quantise an 8-bit channel into `mask`.
fn quantise_channel(value: u8, mask: u32) -> u32 {
    let Some((shift, bits)) = channel_layout(mask) else {
        return 0;
    };
    let max = if bits >= 32 {
        u32::MAX as u64
    } else {
        (1u64 << bits) - 1
    };
    let v = (value as u64 * max + 127) / 255;
    ((v as u32) << shift) & mask
}

fn read_uint(p: &[u8], big_endian: bool) -> u32 {
    if big_endian {
        let mut v = 0u32;
        for &b in p {
            v = (v << 8) | b as u32;
        }
        v
    } else {
        let mut v = 0u32;
        for (i, &b) in p.iter().enumerate() {
            v |= (b as u32) << (8 * i);
        }
        v
    }
}

fn write_uint(v: u32, out: &mut [u8], big_endian: bool) {
    if big_endian {
        let n = out.len();
        for (i, b) in out.iter_mut().enumerate() {
            *b = (v >> (8 * (n - 1 - i))) as u8;
        }
    } else {
        for (i, b) in out.iter_mut().enumerate() {
            *b = (v >> (8 * i)) as u8;
        }
    }
}

/// Convert raw `ZPixmap` data into a top-down, tightly packed BGRA buffer
/// with **opaque alpha** — the layout of [`crate::image::Frame`], matching
/// what the Windows GDI capture produces (alpha is forced to 255 because
/// screen contents have no meaningful transparency). `None` when the buffer
/// length or the format is not a valid X11 TrueColor/DirectColor layout.
///
/// Only TrueColor/DirectColor visuals are supported: an 8-bit PseudoColor
/// visual has no colour masks and returns `None` (the capture adapter turns
/// that into a clear error instead of garbage pixels).
pub fn bgra_from_zpixmap(data: &[u8], fmt: &ImageFormat) -> Option<Vec<u8>> {
    if fmt.width == 0 || fmt.height == 0 {
        return None;
    }
    if !matches!(fmt.bits_per_pixel, 8 | 16 | 24 | 32) {
        return None;
    }
    if !matches!(fmt.scanline_pad_bits, 8 | 32) {
        return None;
    }
    channel_layout(fmt.red_mask)?;
    channel_layout(fmt.green_mask)?;
    channel_layout(fmt.blue_mask)?;
    let stride = fmt.stride()?;
    if data.len() != fmt.byte_len()? {
        return None;
    }
    let w = fmt.width as usize;
    let bpp = fmt.bits_per_pixel as usize / 8;
    let pixels = w.checked_mul(fmt.height as usize)?.checked_mul(4)?;
    let mut out = vec![0u8; pixels];
    for y in 0..fmt.height as usize {
        let row = &data[y * stride..y * stride + w * bpp];
        for x in 0..w {
            let v = read_uint(&row[x * bpp..x * bpp + bpp], fmt.big_endian);
            let i = (y * w + x) * 4;
            out[i] = scale_channel(v, fmt.blue_mask);
            out[i + 1] = scale_channel(v, fmt.green_mask);
            out[i + 2] = scale_channel(v, fmt.red_mask);
            out[i + 3] = 255;
        }
    }
    Some(out)
}

/// Inverse of [`bgra_from_zpixmap`]: frame pixels (top-down BGRA) into a
/// padded `ZPixmap` block for `PutImage`. `px` must be `width * height * 4`
/// bytes and `fmt` must describe the same dimensions.
pub fn zpixmap_from_bgra(px: &[u8], fmt: &ImageFormat) -> Option<Vec<u8>> {
    if fmt.width == 0 || fmt.height == 0 {
        return None;
    }
    if !matches!(fmt.bits_per_pixel, 8 | 16 | 24 | 32) {
        return None;
    }
    channel_layout(fmt.red_mask)?;
    channel_layout(fmt.green_mask)?;
    channel_layout(fmt.blue_mask)?;
    let w = fmt.width as usize;
    let need = w.checked_mul(fmt.height as usize)?.checked_mul(4)?;
    if px.len() != need {
        return None;
    }
    let stride = fmt.stride()?;
    let bpp = fmt.bits_per_pixel as usize / 8;
    let mut out = vec![0u8; stride * fmt.height as usize];
    for y in 0..fmt.height as usize {
        let base = y * stride;
        for x in 0..w {
            let s = (y * w + x) * 4;
            let r = quantise_channel(px[s + 2], fmt.red_mask);
            let g = quantise_channel(px[s + 1], fmt.green_mask);
            let b = quantise_channel(px[s], fmt.blue_mask);
            let v = r | g | b;
            write_uint(
                v,
                &mut out[base + x * bpp..base + x * bpp + bpp],
                fmt.big_endian,
            );
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Smallest rectangle covering every non-empty input rectangle; `None` when
/// the input holds no pixels (the "invalid virtual screen" rule).
pub fn union_bounds(rects: &[IRect]) -> Option<IRect> {
    let mut x0 = i32::MAX;
    let mut y0 = i32::MAX;
    let mut x1 = i32::MIN;
    let mut y1 = i32::MIN;
    let mut any = false;
    for r in rects {
        if r.is_empty() {
            continue;
        }
        any = true;
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.right());
        y1 = y1.max(r.bottom());
    }
    if !any {
        return None;
    }
    Some(IRect::new(x0, y0, x1 - x0, y1 - y0))
}

// ---------------------------------------------------------------------------
// X resources / DPI
// ---------------------------------------------------------------------------

/// Parse `Xft.dpi` out of a root-window `RESOURCE_MANAGER` string.
/// Returns `None` when the entry is absent or outside 72..=384; the caller
/// falls back to 96.
pub fn parse_xft_dpi(resource_manager: &str) -> Option<u16> {
    for line in resource_manager.lines() {
        if let Some((key, value)) = line.split_once(':')
            && key.trim() == "Xft.dpi"
        {
            let dpi: f64 = value.trim().parse().ok()?;
            if (72.0..=384.0).contains(&dpi) {
                return Some(dpi.round() as u16);
            }
            return None;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Hotkey translation (Win32 `MOD_*`/VK → X11 masks/keysyms)
// ---------------------------------------------------------------------------

pub const SHIFT_MASK: u32 = 1 << 0;
pub const LOCK_MASK: u32 = 1 << 1;
pub const CONTROL_MASK: u32 = 1 << 2;
pub const MOD1_MASK: u32 = 1 << 3;
pub const MOD2_MASK: u32 = 1 << 4;
pub const MOD4_MASK: u32 = 1 << 6;

/// X11 keysym (`X11/keysymdef.h` values) for the Win32 virtual-key codes
/// NameShot accepts; `None` when the key cannot be a global X11 grab
/// (modifier keys themselves, media keys the parser never produces).
pub fn x_keysym_for_vk(vk: u16) -> Option<u32> {
    use crate::hotkey::vk;
    Some(match vk {
        0x41..=0x5A => 0x0061 + (vk - 0x41) as u32, // X11 letter keysyms are lowercase
        0x30..=0x39 => 0x0030 + (vk - 0x30) as u32,
        0x70..=0x7B => 0xFFBE + (vk - 0x70) as u32, // F1..F12
        vk::BACK => 0xFF08,
        vk::TAB => 0xFF09,
        vk::RETURN => 0xFF0D,
        vk::PAUSE => 0xFF13,
        vk::CAPITAL => 0xFFE5,
        vk::ESCAPE => 0xFF1B,
        vk::SPACE => 0x0020,
        vk::PRIOR => 0xFF55,
        vk::NEXT => 0xFF56,
        vk::END => 0xFF57,
        vk::HOME => 0xFF50,
        vk::LEFT => 0xFF51,
        vk::UP => 0xFF52,
        vk::RIGHT => 0xFF53,
        vk::DOWN => 0xFF54,
        vk::INSERT => 0xFF63,
        vk::DELETE => 0xFFFF,
        vk::PRINTSCREEN => 0xFF61,
        vk::NUMLOCK => 0xFF7F,
        vk::SCROLL => 0xFF14,
        vk::OEM_3 => 0x0060,
        0xBD => 0x002D,
        0xBB => 0x003D,
        0xDB => 0x005B,
        0xDD => 0x005D,
        0xDC => 0x005C,
        0xBA => 0x003B,
        0xDE => 0x0027,
        0xBC => 0x002C,
        0xBE => 0x002E,
        0xBF => 0x002F,
        _ => return None,
    })
}

/// `HotkeySpec::modifiers` (Win32 `MOD_*` values) → X11 modifier mask.
/// Convention: Alt = Mod1, Super/Win = Mod4 — what every mainstream desktop
/// uses; NumLock is Mod2 and is handled by [`grab_variant_modifiers`].
pub fn x_modifier_mask(modifiers: u8) -> u32 {
    let mut m = 0;
    if modifiers & MOD_SHIFT != 0 {
        m |= SHIFT_MASK;
    }
    if modifiers & MOD_CONTROL != 0 {
        m |= CONTROL_MASK;
    }
    if modifiers & MOD_ALT != 0 {
        m |= MOD1_MASK;
    }
    if modifiers & MOD_WIN != 0 {
        m |= MOD4_MASK;
    }
    m
}

/// The four passive-grab variants of `base`: with and without CapsLock
/// (`LockMask`) and NumLock (`Mod2Mask`). Grabbing only the bare combination
/// is the classic `XGrabKey` pitfall — the hotkey silently dies as soon as a
/// lock key is on.
pub fn grab_variant_modifiers(base: u32) -> [u32; 4] {
    [
        base,
        base | LOCK_MASK,
        base | MOD2_MASK,
        base | LOCK_MASK | MOD2_MASK,
    ]
}

/// `true` when a `XGrabKey` failure means "someone else owns this key"
/// (`BadAccess`), i.e. the `ERROR_HOTKEY_ALREADY_REGISTERED` analogue of the
/// Win32 adapter. Anything else (bad window, bad keycode) is a real error.
pub fn grab_error_is_conflict(err: &crate::error::Error) -> bool {
    match err {
        crate::error::Error::Msg(m) => m.contains("BadAccess"),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Font discovery ranking (no fontconfig)
// ---------------------------------------------------------------------------

/// `true` for file extensions [`crate::platform`] scans for.
pub fn is_font_file(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()), Some("ttf" | "otf"))
}

/// First candidate whose file name matches a well-known default UI sans
/// font, else the first TrueType/OpenType candidate, else `None`. Pure
/// ranking: the adapter only supplies the directory scan results.
pub fn pick_font_path(candidates: &[PathBuf]) -> Option<PathBuf> {
    const PREFERRED: [&str; 7] = [
        "DejaVuSans.ttf",
        "DejaVuSans-Book.ttf",
        "LiberationSans-Regular.ttf",
        "NotoSans-Regular.ttf",
        "FreeSans.ttf",
        "Ubuntu-R.ttf",
        "arial.ttf",
    ];
    for pref in PREFERRED {
        if let Some(p) = candidates
            .iter()
            .find(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(pref)))
        {
            return Some(p.clone());
        }
    }
    candidates.iter().find(|p| is_font_file(p)).cloned()
}

// ---------------------------------------------------------------------------
// Dialog argv (fallback save/folder/message dialogs)
// ---------------------------------------------------------------------------

/// argv for a `zenity` save dialog; spaces stay inside single entries (the
/// caller only execs the vector).
pub fn zenity_save_args(title: &str, default: &Path) -> Vec<String> {
    vec![
        "--file-selection".into(),
        "--save".into(),
        "--confirm-overwrite".into(),
        format!("--title={title}"),
        format!("--filename={}", default.display()),
    ]
}

/// argv for a `zenity` folder picker.
pub fn zenity_dir_args(title: &str, default: &Path) -> Vec<String> {
    vec![
        "--file-selection".into(),
        "--directory".into(),
        format!("--title={title}"),
        format!("--filename={}", default.display()),
    ]
}

/// argv for a `zenity` message box (`--error` for failures, else `--info`).
pub fn zenity_msg_args(title: &str, text: &str, error: bool) -> Vec<String> {
    vec![
        if error { "--error" } else { "--info" }.into(),
        format!("--title={title}"),
        format!("--text={text}"),
    ]
}

// ---------------------------------------------------------------------------
// Channel-order swap for encoders
// ---------------------------------------------------------------------------

/// BGRA → RGBA reordering for encoders that want RGBA (the `png` crate);
/// `None` when the length is not a whole number of pixels.
pub fn rgba_from_bgra(px: &[u8]) -> Option<Vec<u8>> {
    if !px.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(px.len());
    for p in px.as_chunks::<4>().0 {
        out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Selection session (M1b): the pure decision core of the X11 event loop
// ---------------------------------------------------------------------------

/// Input fed to [`SelectSession`], in frame coordinates (the captured
/// virtual screen's own pixel space — `eventmap` subtracts the screen
/// origin before delivery).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessEvent {
    /// Primary button pressed: a rubber-band drag starts.
    Press { x: i32, y: i32 },
    /// Pointer moved.
    Move { x: i32, y: i32 },
    /// Primary button released: the drag finishes.
    Release { x: i32, y: i32 },
    /// Enter: commit whatever is selected now (Windows `confirm`).
    Confirm,
    /// Escape: quit without saving.
    Cancel,
}

/// What the driver should do after feeding an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessOutcome {
    /// Nothing visible changed.
    None,
    /// Redraw the scene: the selection is now this rectangle.
    Redraw(IRect),
    /// The band is done: crop this region and offer to save it.
    Commit(IRect),
    /// Quit the session without saving.
    Cancelled,
}

/// In-progress rubber band, mirroring the Windows `ODrag::New` shape
/// (`prev` = selection captured at press; `moved` is sticky — see
/// [`band_moved`]). Move/resize of an existing selection is not part of
/// M1b (plan: event-loop phase only).
#[derive(Debug, Clone, Copy)]
enum SessDrag {
    New {
        anchor: (i32, i32),
        prev: IRect,
        moved: bool,
    },
}

/// The selecting phase as a pure state machine: full-screen selection to
/// start, drag to band, release commits when the band really moved and is
/// at least [`theme::SPACE_XS`] px in both dimensions, a plain click or a
/// sub-threshold slip reverts, Enter commits, Escape cancels. Decision
/// logic mirrors the Windows overlay (`src/overlay/mod.rs`:
/// `on_down`/`on_move`/`on_up`/`release_action`/`on_key`); unit-tested
/// here because the X11 driver itself can only be compile-checked.
pub struct SelectSession {
    /// Frame-coordinate bounds of the captured screen.
    vs: IRect,
    sel: IRect,
    drag: Option<SessDrag>,
}

impl SelectSession {
    /// Start a session over `vs` with the whole screen selected.
    pub fn new(vs: IRect) -> Self {
        SelectSession {
            vs,
            sel: vs,
            drag: None,
        }
    }

    /// Current selection (initially the full screen).
    pub fn selection(&self) -> IRect {
        self.sel
    }

    /// Feed one input event and get the instruction for the driver.
    pub fn handle(&mut self, ev: SessEvent) -> SessOutcome {
        match ev {
            SessEvent::Press { x, y } => {
                self.drag = Some(SessDrag::New {
                    anchor: (x, y),
                    prev: self.sel,
                    moved: false,
                });
                SessOutcome::None
            }
            SessEvent::Move { x, y } => {
                let Some(SessDrag::New {
                    anchor,
                    prev,
                    moved,
                }) = &mut self.drag
                else {
                    return SessOutcome::None;
                };
                let anchor = *anchor;
                let prev = *prev;
                self.sel = band_rect(anchor, (x, y), &self.vs);
                *moved = band_moved(*moved, prev, self.sel);
                SessOutcome::Redraw(self.sel)
            }
            SessEvent::Release { .. } => {
                let Some(SessDrag::New { prev, moved, .. }) = self.drag.take() else {
                    return SessOutcome::None;
                };
                if moved && self.sel.w >= theme::SPACE_XS && self.sel.h >= theme::SPACE_XS {
                    SessOutcome::Commit(self.sel)
                } else {
                    // Click noise: restore what was selected before the drag.
                    self.sel = prev;
                    SessOutcome::Redraw(prev)
                }
            }
            SessEvent::Confirm => SessOutcome::Commit(self.sel),
            SessEvent::Cancel => {
                self.drag = None;
                SessOutcome::Cancelled
            }
        }
    }
}

/// `v` clamped into `[lo, hi]` (returns `lo` for an inverted range).
fn cl(v: i32, lo: i32, hi: i32) -> i32 {
    if hi < lo { lo } else { v.max(lo).min(hi) }
}

/// Rubber-band rectangle between two points, clamped to `vs` with at least
/// one pixel — mirror of the Windows overlay's `band_rect`.
fn band_rect(a: (i32, i32), b: (i32, i32), vs: &IRect) -> IRect {
    let r = IRect::from_points(a.0, a.1, b.0, b.1);
    let l = cl(r.x, vs.x, vs.x + vs.w - 1);
    let t = cl(r.y, vs.y, vs.y + vs.h - 1);
    let rr = cl(r.right(), l + 1, vs.x + vs.w);
    let bb = cl(r.bottom(), t + 1, vs.y + vs.h);
    IRect::new(l, t, rr - l, bb - t)
}

/// Sticky "this rubber-band actually moved" flag: once the band left its
/// starting rectangle, the drag counts as moved even if it ends back at the
/// start (a full-screen band must still commit on release). Mirror of the
/// Windows `band_moved`.
fn band_moved(was_moved: bool, prev: IRect, sel: IRect) -> bool {
    was_moved || sel != prev
}

// ---------------------------------------------------------------------------
// Selecting-phase scene composition (M1b, pure): veil + crisp selection +
// border. Mirror of the Windows overlay's `compose` minus the resize
// handles and size chip (not part of M1b).
// ---------------------------------------------------------------------------

/// Veiled backdrop: every channel blended toward [`theme::BG`] keeping
/// [`theme::VEIL_KEEP_PCT`] of it, alpha forced opaque — the math of the
/// Windows overlay's `dim_frame`.
pub fn veil_frame(frame: &Frame) -> Frame {
    let mut v = frame.px.clone();
    let keep = theme::VEIL_KEEP_PCT as u32;
    let mix = 100 - keep;
    let (br, bg, bb) = theme::BG;
    for c in v.as_chunks_mut::<4>().0 {
        c[0] = ((c[0] as u32 * keep + bb as u32 * mix) / 100) as u8;
        c[1] = ((c[1] as u32 * keep + bg as u32 * mix) / 100) as u8;
        c[2] = ((c[2] as u32 * keep + br as u32 * mix) / 100) as u8;
        c[3] = 255;
    }
    // The length is unchanged, so validation cannot fail; fall back to the
    // input rather than panicking if it somehow does.
    match Frame::from_bgra(frame.w, frame.h, v) {
        Some(f) => f,
        None => frame.clone(),
    }
}

/// Compose the selecting-phase scene: the veil over the whole screen, the
/// crisp capture inside `sel`, then the border — 1px near-black outside
/// plus the 2px accent band, exactly as the Windows `compose` draws it
/// (`frame_solid(outer, SEL_OUTLINE)` then two `frame_solid(.., ACCENT)`).
/// Returns a new frame (the inputs are untouched).
pub fn compose_select(veil: &Frame, crisp: &Frame, sel: IRect) -> Frame {
    let mut f = veil.clone();
    let r = sel.clamp_to(crisp.w as i32, crisp.h as i32);
    if !r.is_empty() && r.clamp_to(f.w as i32, f.h as i32) == r {
        for y in r.y..r.bottom() {
            let src = (y as usize * crisp.w as usize + r.x as usize) * 4;
            let dst = (y as usize * f.w as usize + r.x as usize) * 4;
            let len = r.w as usize * 4;
            f.px[dst..dst + len].copy_from_slice(&crisp.px[src..src + len]);
        }
    }
    let black = Rgba::opaque(
        theme::SEL_OUTLINE.0,
        theme::SEL_OUTLINE.1,
        theme::SEL_OUTLINE.2,
    );
    let accent = Rgba::opaque(theme::ACCENT.0, theme::ACCENT.1, theme::ACCENT.2);
    f.stroke_rect(sel.expand(1), black, 1);
    f.stroke_rect(sel, accent, 1);
    f.stroke_rect(
        IRect::new(sel.x + 1, sel.y + 1, sel.w - 2, sel.h - 2),
        accent,
        1,
    );
    f
}

// ---------------------------------------------------------------------------
// Event mapping (M1b): wire event → session input, as a pure function.
//
// The x11rb match itself lives in `x11/eventmap.rs` (compile-verified on
// Linux only); everything decided *from* the extracted fields — button
// filtering, root→frame offsets, keycode comparison — is here so it runs in
// the regular unit suite on every host.
// ---------------------------------------------------------------------------

/// Event classes the wire layer can recognize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    Press,
    Move,
    Release,
    Key,
}

/// One protocol event reduced to the fields the mapping decides on.
/// `root_x`/`root_y` are root coordinates (frame origin not yet removed);
/// `detail` is the button number or keycode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEvent {
    pub kind: RawKind,
    pub detail: u8,
    pub root_x: i32,
    pub root_y: i32,
}

/// Translate a raw event into session input.
///
/// * `keys` — `(escape, confirm)` keycodes resolved from the server
///   keymap; `None` means the keyboard shortcuts are unavailable (the
///   pointer path still works, the driver never invents a keycode).
/// * `ox`, `oy` — virtual-screen origin in root coordinates: root positions
///   are shifted into frame coordinates, so it does not matter which
///   window the server delivered the event to.
///
/// Only the primary button drags; other buttons, and keys other than
/// Escape/Enter, map to `None` (ignored, not errors).
pub fn to_session(raw: &RawEvent, keys: Option<(u8, u8)>, ox: i32, oy: i32) -> Option<SessEvent> {
    match raw.kind {
        RawKind::Press if raw.detail == 1 => Some(SessEvent::Press {
            x: raw.root_x - ox,
            y: raw.root_y - oy,
        }),
        RawKind::Move => Some(SessEvent::Move {
            x: raw.root_x - ox,
            y: raw.root_y - oy,
        }),
        RawKind::Release if raw.detail == 1 => Some(SessEvent::Release {
            x: raw.root_x - ox,
            y: raw.root_y - oy,
        }),
        RawKind::Key => {
            let (escape, confirm) = keys?;
            match raw.detail {
                d if d == escape => Some(SessEvent::Cancel),
                d if d == confirm => Some(SessEvent::Confirm),
                _ => None,
            }
        }
        RawKind::Press | RawKind::Release => None,
    }
}

// ---------------------------------------------------------------------------
// Tests — these run in the regular (Windows) suite; the X11 adapters above
// them have never executed anywhere.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard 32bpp little-endian TrueColor layout (masks 0xFF0000 /
    /// 0xFF00 / 0xFF) as every mainstream X server sends it.
    fn fmt32(width: u32, height: u32) -> ImageFormat {
        ImageFormat {
            width,
            height,
            bits_per_pixel: 32,
            scanline_pad_bits: 32,
            big_endian: false,
            red_mask: 0x00FF_0000,
            green_mask: 0x0000_FF00,
            blue_mask: 0x0000_00FF,
        }
    }

    #[test]
    fn zpixmap_32bpp_le_converts_to_bgra() {
        // One pixel: value 0x00RRGGBB in little-endian bytes = [B, G, R, pad].
        let data = [0x11u8, 0x22, 0x33, 0x00];
        let out = bgra_from_zpixmap(&data, &fmt32(1, 1)).unwrap();
        assert_eq!(out, vec![0x11, 0x22, 0x33, 255], "BGRA + forced alpha");
    }

    #[test]
    fn zpixmap_32bpp_big_endian_converts() {
        let fmt = ImageFormat {
            big_endian: true,
            ..fmt32(1, 1)
        };
        // Same value as the little-endian test (0x00332211), sent MSB first.
        let data = [0x00u8, 0x33, 0x22, 0x11];
        let out = bgra_from_zpixmap(&data, &fmt).unwrap();
        assert_eq!(
            out,
            vec![0x11, 0x22, 0x33, 255],
            "byte order only, value identical"
        );
    }

    #[test]
    fn zpixmap_16bpp_565_scales_channels() {
        let fmt = ImageFormat {
            width: 1,
            height: 1,
            bits_per_pixel: 16,
            scanline_pad_bits: 32,
            big_endian: false,
            red_mask: 0xF800,
            green_mask: 0x07E0,
            blue_mask: 0x001F,
        };
        // red=31 (max), green=0, blue=0 → 0xF800 little-endian, padded up to
        // the 32-bit scanline (4 bytes total).
        let data = [0x00u8, 0xF8, 0x00, 0x00];
        let out = bgra_from_zpixmap(&data, &fmt).unwrap();
        assert_eq!(out, vec![0, 0, 255, 255]);
        // Mid green: green field = 32 of 63 → round(32 * 255 / 63) = 130.
        let data = [0x00u8, 0x04, 0x00, 0x00]; // 0x0400 = green bits 10..5 set to 32
        let out = bgra_from_zpixmap(&data, &fmt).unwrap();
        assert_eq!(out[1], 130, "green mid-scale");
    }

    #[test]
    fn zpixmap_24bpp_row_is_padded() {
        let fmt = ImageFormat {
            bits_per_pixel: 24,
            ..fmt32(1, 1)
        };
        // stride = ((1*24 + 31) / 32) * 4 = 4 bytes (1 pixel + 1 pad byte).
        assert_eq!(fmt.stride(), Some(4));
        assert_eq!(fmt.byte_len(), Some(4));
        let data = [0x11u8, 0x22, 0x33, 0xEE];
        let out = bgra_from_zpixmap(&data, &fmt).unwrap();
        assert_eq!(out, vec![0x11, 0x22, 0x33, 255]);
        // Two pixels: contiguous, with the scanline pad at the END of the
        // row: stride = ((2*24 + 31) / 32) * 4 = 8.
        let fmt2 = ImageFormat { width: 2, ..fmt };
        assert_eq!(fmt2.stride(), Some(8));
        let data2 = [1u8, 2, 3, 4, 5, 6, 0, 0];
        let out2 = bgra_from_zpixmap(&data2, &fmt2).unwrap();
        assert_eq!(out2.len(), 8);
        assert_eq!(&out2[0..3], &[1, 2, 3]);
        assert_eq!(&out2[4..7], &[4, 5, 6]);
    }

    #[test]
    fn zpixmap_rejects_bad_input() {
        // Wrong buffer length.
        assert!(bgra_from_zpixmap(&[0u8; 3], &fmt32(1, 1)).is_none());
        // Zero masks (PseudoColor visual) are unsupported.
        let bad = ImageFormat {
            red_mask: 0,
            ..fmt32(1, 1)
        };
        assert!(bgra_from_zpixmap(&[0u8; 4], &bad).is_none());
        // Non-contiguous mask.
        let bad = ImageFormat {
            blue_mask: 0x0000_00FF & 0x5,
            ..fmt32(1, 1)
        };
        assert!(bgra_from_zpixmap(&[0u8; 4], &bad).is_none());
        // Unsupported depth.
        let bad = ImageFormat {
            bits_per_pixel: 4,
            ..fmt32(1, 1)
        };
        assert!(bgra_from_zpixmap(&[0u8; 4], &bad).is_none());
        // Zero size.
        let bad = fmt32(0, 1);
        assert!(bgra_from_zpixmap(&[], &bad).is_none());
        // Pad value other than 8/32.
        let bad = ImageFormat {
            scanline_pad_bits: 16,
            ..fmt32(1, 1)
        };
        assert!(bgra_from_zpixmap(&[0u8; 4], &bad).is_none());
    }

    #[test]
    fn bgra_zpixmap_roundtrip_32bpp() {
        let fmt = fmt32(3, 2);
        let frame: Vec<u8> = (0..24).map(|i| i as u8).collect();
        let wire = zpixmap_from_bgra(&frame, &fmt).unwrap();
        assert_eq!(wire.len(), 24);
        let back = bgra_from_zpixmap(&wire, &fmt).unwrap();
        // Alpha is forced to 255 on the way back; other bytes must match.
        for (src, dst) in frame
            .as_chunks::<4>()
            .0
            .iter()
            .zip(back.as_chunks::<4>().0.iter())
        {
            assert_eq!(&src[0..3], &dst[0..3]);
            assert_eq!(dst[3], 255);
        }
    }

    #[test]
    fn union_bounds_covers_all_and_skips_empty() {
        let a = IRect::new(0, 0, 1920, 1080);
        let b = IRect::new(1920, 200, 1000, 800);
        let u = union_bounds(&[a, b]).unwrap();
        assert_eq!(u, IRect::new(0, 0, 2920, 1080));
        // Negative origins (X11 would not produce them, but the math is general).
        let u = union_bounds(&[IRect::new(-50, -10, 100, 10), IRect::new(0, 0, 10, 10)]).unwrap();
        assert_eq!(u, IRect::new(-50, -10, 100, 20));
        // Empty inputs only → None.
        assert_eq!(union_bounds(&[]), None);
        assert_eq!(union_bounds(&[IRect::EMPTY]), None);
    }

    #[test]
    fn xft_dpi_parsing_is_tolerant() {
        assert_eq!(parse_xft_dpi("Xft.dpi:\t96\n"), Some(96));
        assert_eq!(
            parse_xft_dpi("*fallbackDpi: 100\nXft.dpi: 144.0\n"),
            Some(144)
        );
        assert_eq!(parse_xft_dpi("Xft.dpi: 100.5"), Some(101));
        assert_eq!(parse_xft_dpi("Xft.dpi: banana"), None);
        assert_eq!(parse_xft_dpi("Xft.dpi: 10"), None, "outside 72..=384");
        assert_eq!(parse_xft_dpi("some.other.resource: 1\n"), None);
        assert_eq!(parse_xft_dpi(""), None);
    }

    #[test]
    fn vk_to_keysym_covers_the_parseable_set() {
        use crate::hotkey::{parse, vk};
        assert_eq!(x_keysym_for_vk(vk::PRINTSCREEN), Some(0xFF61));
        assert_eq!(x_keysym_for_vk(vk::F1), Some(0xFFBE));
        assert_eq!(x_keysym_for_vk(vk::F1 + 11), Some(0xFFC9), "F12");
        assert_eq!(x_keysym_for_vk(vk::A), Some(0x0061), "lowercase keysym");
        assert_eq!(x_keysym_for_vk(vk::A + 18), Some(0x0073), "'s'");
        assert_eq!(x_keysym_for_vk(vk::N0), Some(0x0030));
        assert_eq!(x_keysym_for_vk(vk::OEM_3), Some(0x0060), "grave");
        assert_eq!(x_keysym_for_vk(vk::SPACE), Some(0x0020));
        assert_eq!(x_keysym_for_vk(vk::LEFT), Some(0xFF51));
        assert_eq!(x_keysym_for_vk(vk::SHIFT), None, "modifier keys never grab");
        // Every hotkey the parser accepts (bare function/media keys, or any
        // key with modifiers) must translate to a keysym.
        for text in [
            "PrintScreen",
            "Ctrl+S",
            "Ctrl+Alt+PrtSc",
            "Shift+F5",
            "Win+PrintScreen",
            "Ctrl+Shift+9",
            "Alt+PageDown",
            "Ctrl+`",
        ] {
            let spec = parse(text).unwrap();
            assert!(
                x_keysym_for_vk(spec.vk).is_some(),
                "{text} (vk {:#x}) has no keysym",
                spec.vk
            );
        }
    }

    #[test]
    fn modifier_masks_follow_desktop_conventions() {
        use crate::hotkey::{MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, parse};
        assert_eq!(x_modifier_mask(MOD_CONTROL), CONTROL_MASK);
        assert_eq!(x_modifier_mask(MOD_ALT), MOD1_MASK);
        assert_eq!(x_modifier_mask(MOD_WIN), MOD4_MASK);
        assert_eq!(x_modifier_mask(MOD_SHIFT), SHIFT_MASK);
        let all = MOD_CONTROL | MOD_ALT | MOD_WIN | MOD_SHIFT;
        assert_eq!(
            x_modifier_mask(all),
            CONTROL_MASK | MOD1_MASK | MOD4_MASK | SHIFT_MASK
        );
        assert_eq!(
            x_modifier_mask(parse("Ctrl+Shift+S").unwrap().modifiers),
            CONTROL_MASK | SHIFT_MASK
        );
        assert_eq!(x_modifier_mask(0), 0, "bare PrintScreen grabs with no mods");
    }

    #[test]
    fn grab_variants_cover_lock_keys() {
        let base = x_modifier_mask(crate::hotkey::MOD_CONTROL);
        let v = grab_variant_modifiers(base);
        assert_eq!(v[0], base);
        assert_eq!(v[1], base | LOCK_MASK, "CapsLock on");
        assert_eq!(v[2], base | MOD2_MASK, "NumLock on");
        assert_eq!(v[3], base | LOCK_MASK | MOD2_MASK, "both");
        let mut sorted = v.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 4, "all four variants are distinct");
    }

    #[test]
    fn conflict_detection_matches_bad_access_only() {
        use crate::error::Error;
        assert!(grab_error_is_conflict(&Error::msg(
            "XGrabKey: BadAccess (key already grabbed by another application)"
        )));
        assert!(!grab_error_is_conflict(&Error::msg("XGrabKey: BadWindow")));
        assert!(!grab_error_is_conflict(&Error::invalid("no keycode")));
        assert!(!grab_error_is_conflict(&Error::io(
            "write",
            std::io::Error::other("x")
        )));
    }

    #[test]
    fn font_ranking_prefers_known_sans_fonts() {
        let mk = |s: &str| PathBuf::from(s);
        let found = vec![
            mk("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
            mk("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"),
        ];
        assert_eq!(
            pick_font_path(&found).unwrap(),
            mk("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"),
            "regular beats bold via the preferred list order"
        );
        // Case-insensitive exact file-name match.
        let found = vec![mk("/x/dejavusans.ttf")];
        assert!(pick_font_path(&found).is_some());
        // No preferred name → first ttf/otf, skipping non-fonts.
        let found = vec![mk("/x/README.txt"), mk("/x/Custom.otf")];
        assert_eq!(pick_font_path(&found).unwrap(), mk("/x/Custom.otf"));
        // Nothing usable → None.
        assert_eq!(pick_font_path(&[mk("/x/README.txt")]), None);
        assert_eq!(pick_font_path(&[]), None);
    }

    #[test]
    fn zenity_argv_keeps_spaces_in_one_argument() {
        let args = zenity_save_args("Save as", Path::new("/home/u/My Shots/pic.png"));
        assert_eq!(args[0], "--file-selection");
        assert!(args.contains(&"--save".to_string()));
        assert!(args.contains(&"--title=Save as".to_string()));
        assert!(args.contains(&"--filename=/home/u/My Shots/pic.png".to_string()));
        assert_eq!(args.len(), 5);

        let args = zenity_dir_args("Folder", Path::new("/tmp"));
        assert!(args.contains(&"--directory".to_string()));

        let info = zenity_msg_args("NameShot", "Copy failed", false);
        assert_eq!(info[0], "--info");
        let err = zenity_msg_args("NameShot", "Copy failed", true);
        assert_eq!(err[0], "--error");
    }

    #[test]
    fn rgba_swap_is_exact_and_checked() {
        let px = [1u8, 2, 3, 4, 10, 20, 30, 40];
        assert_eq!(
            rgba_from_bgra(&px).unwrap(),
            vec![3, 2, 1, 4, 30, 20, 10, 40]
        );
        assert_eq!(rgba_from_bgra(&[1, 2, 3]), None);
        assert_eq!(rgba_from_bgra(&[]), Some(vec![]));
    }

    // -----------------------------------------------------------------------
    // M1b: selection session — mirrors the Windows overlay's release policy
    // -----------------------------------------------------------------------

    /// 100x100 frame-coordinate screen.
    fn screen() -> IRect {
        IRect::new(0, 0, 100, 100)
    }

    #[test]
    fn session_starts_with_the_full_screen_selected() {
        let mut s = SelectSession::new(screen());
        assert_eq!(s.selection(), screen());
        assert_eq!(s.handle(SessEvent::Press { x: 5, y: 5 }), SessOutcome::None);
    }

    #[test]
    fn dragged_band_commits_on_release() {
        let mut s = SelectSession::new(screen());
        s.handle(SessEvent::Press { x: 60, y: 70 });
        assert_eq!(
            s.handle(SessEvent::Move { x: 20, y: 30 }),
            SessOutcome::Redraw(IRect::new(20, 30, 41, 41))
        );
        assert_eq!(
            s.handle(SessEvent::Release { x: 20, y: 30 }),
            SessOutcome::Commit(IRect::new(20, 30, 41, 41))
        );
        assert_eq!(s.selection(), IRect::new(20, 30, 41, 41));
    }

    #[test]
    fn drag_from_any_corner_normalizes_the_band() {
        let mut s = SelectSession::new(screen());
        s.handle(SessEvent::Press { x: 10, y: 10 });
        s.handle(SessEvent::Move { x: 50, y: 50 });
        let down = s.handle(SessEvent::Release { x: 50, y: 50 });
        assert_eq!(down, SessOutcome::Commit(IRect::new(10, 10, 41, 41)));

        let mut s = SelectSession::new(screen());
        s.handle(SessEvent::Press { x: 50, y: 50 });
        s.handle(SessEvent::Move { x: 10, y: 10 });
        let up = s.handle(SessEvent::Release { x: 10, y: 10 });
        assert_eq!(up, SessOutcome::Commit(IRect::new(10, 10, 41, 41)));
    }

    #[test]
    fn plain_click_reverts_instead_of_committing() {
        let mut s = SelectSession::new(screen());
        s.handle(SessEvent::Press { x: 40, y: 40 });
        assert_eq!(
            s.handle(SessEvent::Release { x: 40, y: 40 }),
            SessOutcome::Redraw(screen())
        );
        assert_eq!(s.selection(), screen());
    }

    #[test]
    fn sub_threshold_band_is_click_noise() {
        // theme::SPACE_XS = 4: a 3x3 slip reverts, the band at exactly 4x4
        // commits (same boundary as the Windows `release_action` tests).
        let mut s = SelectSession::new(screen());
        s.handle(SessEvent::Press { x: 0, y: 0 });
        s.handle(SessEvent::Move { x: 2, y: 2 });
        assert_eq!(
            s.handle(SessEvent::Release { x: 2, y: 2 }),
            SessOutcome::Redraw(screen())
        );

        let mut s = SelectSession::new(screen());
        s.handle(SessEvent::Press { x: 0, y: 0 });
        s.handle(SessEvent::Move { x: 3, y: 3 });
        assert_eq!(
            s.handle(SessEvent::Release { x: 3, y: 3 }),
            SessOutcome::Commit(IRect::new(0, 0, 4, 4))
        );
    }

    #[test]
    fn moved_flag_is_sticky_even_back_to_the_same_band() {
        // First drag leaves a 50x50 selection; the second drag leaves it and
        // returns exactly to it — the release must commit (sticky `moved`),
        // exactly like the Windows full-screen-band case.
        let mut s = SelectSession::new(screen());
        s.handle(SessEvent::Press { x: 10, y: 10 });
        s.handle(SessEvent::Move { x: 60, y: 60 });
        s.handle(SessEvent::Release { x: 60, y: 60 });
        let committed = IRect::new(10, 10, 51, 51);
        assert_eq!(s.selection(), committed);

        s.handle(SessEvent::Press { x: 10, y: 10 });
        s.handle(SessEvent::Move { x: 70, y: 70 });
        assert_eq!(
            s.handle(SessEvent::Move { x: 60, y: 60 }),
            SessOutcome::Redraw(committed)
        );
        assert_eq!(
            s.handle(SessEvent::Release { x: 60, y: 60 }),
            SessOutcome::Commit(committed)
        );
    }

    #[test]
    fn band_is_clamped_to_the_screen() {
        let mut s = SelectSession::new(screen());
        s.handle(SessEvent::Press { x: 10, y: 10 });
        assert_eq!(
            s.handle(SessEvent::Move {
                x: 10_000,
                y: 10_000
            }),
            SessOutcome::Redraw(IRect::new(10, 10, 90, 90))
        );
        assert_eq!(
            s.handle(SessEvent::Move { x: -500, y: -500 }),
            SessOutcome::Redraw(IRect::new(0, 0, 11, 11))
        );
    }

    #[test]
    fn input_without_an_active_drag_is_ignored() {
        let mut s = SelectSession::new(screen());
        assert_eq!(s.handle(SessEvent::Move { x: 5, y: 5 }), SessOutcome::None);
        assert_eq!(
            s.handle(SessEvent::Release { x: 5, y: 5 }),
            SessOutcome::None
        );
        assert_eq!(s.selection(), screen());
    }

    #[test]
    fn enter_commits_and_escape_cancels() {
        let mut s = SelectSession::new(screen());
        assert_eq!(s.handle(SessEvent::Confirm), SessOutcome::Commit(screen()));

        s.handle(SessEvent::Press { x: 5, y: 5 });
        s.handle(SessEvent::Move { x: 50, y: 50 });
        assert_eq!(
            s.handle(SessEvent::Confirm),
            SessOutcome::Commit(IRect::new(5, 5, 46, 46))
        );

        let mut s = SelectSession::new(screen());
        s.handle(SessEvent::Press { x: 5, y: 5 });
        assert_eq!(s.handle(SessEvent::Cancel), SessOutcome::Cancelled);
        assert_eq!(s.selection(), screen(), "cancel must not keep a stale band");
        // The drag is gone too: a later release is a no-op.
        assert_eq!(
            s.handle(SessEvent::Release { x: 5, y: 5 }),
            SessOutcome::None
        );
    }

    // -----------------------------------------------------------------------
    // M1b: veil and scene composition (mirror of the Windows dim/compose)
    // -----------------------------------------------------------------------

    #[test]
    fn veil_matches_the_windows_dim_math() {
        let mut f = Frame::new(2, 1).unwrap();
        // BGRA: [B=255, G=128, R=64, A=0] and a black pixel with A=255.
        f.px = vec![255, 128, 64, 0, 0, 0, 0, 255];
        let original = f.px.clone();

        let v = veil_frame(&f);
        assert_eq!(f.px, original, "veil_frame must not touch its input");

        let keep = i32::from(theme::VEIL_KEEP_PCT);
        let mix = 100 - keep;
        let (br, bg, bb) = theme::BG;
        let want = |base: i32, tint: i32| ((base * keep + tint * mix) / 100) as u8;
        assert_eq!(
            v.px,
            vec![
                want(255, i32::from(bb)),
                want(128, i32::from(bg)),
                want(64, i32::from(br)),
                255,
                want(0, i32::from(bb)),
                want(0, i32::from(bg)),
                want(0, i32::from(br)),
                255,
            ],
            "alpha must be forced opaque (the Windows SRCCOPY path ignores it)"
        );
    }

    #[test]
    fn compose_shows_crisp_inside_veil_outside_accent_on_the_border() {
        let mut crisp = Frame::new(8, 8).unwrap();
        for (i, c) in crisp.px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let i = i as u8;
            c.copy_from_slice(&[i, i, i, 255]);
        }
        let mut veil = Frame::new(8, 8).unwrap();
        veil.fill_rect(IRect::new(0, 0, 8, 8), Rgba::opaque(255, 255, 255));
        let veil_before = veil.px.clone();
        let crisp_before = crisp.px.clone();

        let sel = IRect::new(2, 2, 5, 5);
        let scene = compose_select(&veil, &crisp, sel);
        assert_eq!(veil.px, veil_before, "compose must not touch its inputs");
        assert_eq!(crisp.px, crisp_before, "compose must not touch its inputs");

        // Border cell: accent (the 2px band covers sel's own boundary).
        let accent = Rgba::opaque(theme::ACCENT.0, theme::ACCENT.1, theme::ACCENT.2);
        assert_eq!(scene.get(2, 2).unwrap(), accent.bgra());
        // The 1px near-black line sits one pixel outside the selection.
        let outline = Rgba::opaque(
            theme::SEL_OUTLINE.0,
            theme::SEL_OUTLINE.1,
            theme::SEL_OUTLINE.2,
        );
        assert_eq!(scene.get(1, 1).unwrap(), outline.bgra());
        // Interior cell beyond the 2px band: the crisp capture.
        assert_eq!(scene.get(4, 4).unwrap(), crisp.get(4, 4).unwrap());
        // Outside the outline: the untouched veil.
        assert_eq!(scene.get(0, 0).unwrap(), [255, 255, 255, 255]);
    }

    #[test]
    fn compose_with_full_screen_selection_never_panics_on_the_edge() {
        // The 1px outer outline sits one pixel outside the frame; fill must
        // clip instead of panicking (this is the scene the driver shows at
        // startup, before any drag).
        let crisp = Frame::new(4, 4).unwrap();
        let veil = veil_frame(&crisp);
        let scene = compose_select(&veil, &crisp, IRect::new(0, 0, 4, 4));
        assert_eq!(scene.w, 4);
        assert_eq!(scene.h, 4);
    }

    // -----------------------------------------------------------------------
    // M1b: event mapping policy (the x11rb glue in x11/eventmap.rs only
    // extracts these fields; all decisions are asserted here)
    // -----------------------------------------------------------------------

    /// Screen origin at (10, 5): root (12, 9) is frame pixel (2, 4).
    const OX: i32 = 10;
    const OY: i32 = 5;

    fn raw(kind: RawKind, detail: u8, rx: i32, ry: i32) -> RawEvent {
        RawEvent {
            kind,
            detail,
            root_x: rx,
            root_y: ry,
        }
    }

    #[test]
    fn primary_press_maps_to_press_in_frame_coordinates() {
        assert_eq!(
            to_session(&raw(RawKind::Press, 1, 12, 9), None, OX, OY),
            Some(SessEvent::Press { x: 2, y: 4 })
        );
    }

    #[test]
    fn motion_maps_to_move_from_root_coordinates() {
        assert_eq!(
            to_session(&raw(RawKind::Move, 0, 100, 50), None, OX, OY),
            Some(SessEvent::Move { x: 90, y: 45 })
        );
    }

    #[test]
    fn only_the_primary_button_drags() {
        assert_eq!(
            to_session(&raw(RawKind::Release, 1, 12, 9), None, OX, OY),
            Some(SessEvent::Release { x: 2, y: 4 })
        );
        assert_eq!(
            to_session(&raw(RawKind::Press, 3, 12, 9), None, OX, OY),
            None
        );
        assert_eq!(
            to_session(&raw(RawKind::Release, 2, 12, 9), None, OX, OY),
            None
        );
    }

    #[test]
    fn keys_map_escape_and_enter_only() {
        let keys = Some((9u8, 36u8));
        assert_eq!(
            to_session(&raw(RawKind::Key, 9, 0, 0), keys, 0, 0),
            Some(SessEvent::Cancel)
        );
        assert_eq!(
            to_session(&raw(RawKind::Key, 36, 0, 0), keys, 0, 0),
            Some(SessEvent::Confirm)
        );
        assert_eq!(to_session(&raw(RawKind::Key, 42, 0, 0), keys, 0, 0), None);
        // No keymap: the keyboard is simply unavailable, not an error —
        // the driver never invents a keycode.
        assert_eq!(to_session(&raw(RawKind::Key, 9, 0, 0), None, 0, 0), None);
    }
}
