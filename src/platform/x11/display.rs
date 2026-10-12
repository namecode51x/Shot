//! Screen geometry, visual formats and DPI through RandR / X resources.
//!
//! X11 coordinates are root-relative with no negative origins (unlike the
//! Win32 virtual screen), so `VirtualScreen` is kept as the same four-int
//! shape the capture path already uses.

use crate::error::{Error, Result};
use crate::image::IRect;
use crate::platform::x11_logic::ImageFormat;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, ImageOrder, Visualid};

/// Physical monitor bounds in root coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Monitor {
    pub bounds: IRect,
    pub primary: bool,
}

/// Bounding box of all monitors; same shape as the Win32
/// `os::VirtualScreen` the capture adapter mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Root window id of an X screen index.
pub fn root_of<C: Connection>(conn: &C, screen: usize) -> Result<x11rb::protocol::xproto::Window> {
    conn.setup()
        .roots
        .get(screen)
        .map(|s| s.root)
        .ok_or_else(|| Error::msg("X11: screen index out of range"))
}

/// `(depth, visual)` of the root window — `CreateWindow` needs both.
pub fn root_depth_visual<C: Connection>(conn: &C, screen: usize) -> Result<(u8, Visualid)> {
    let setup = conn.setup();
    let scr = setup
        .roots
        .get(screen)
        .ok_or_else(|| Error::msg("X11: screen index out of range"))?;
    let root_visual = scr.root_visual;
    let depth = scr
        .allowed_depths
        .iter()
        .find(|d| d.visuals.iter().any(|v| v.visual_id == root_visual))
        .map(|d| d.depth)
        .ok_or_else(|| Error::msg("X11: root visual missing from allowed depths"))?;
    Ok((depth, root_visual))
}

/// Connected monitors via RandR 1.5, active set; falls back to the root
/// window's own geometry when the server has no RandR (or pre-1.5).
/// Multi-X-screen setups: the given screen index is the one queried; other
/// screens are not unioned (documented limitation).
pub fn monitors<C: Connection>(conn: &C, screen: usize) -> Result<Vec<Monitor>> {
    let root = root_of(conn, screen)?;
    if let Ok(cookie) = conn.randr_get_monitors(root, true)
        && let Ok(reply) = cookie.reply()
        && !reply.monitors.is_empty()
    {
        return Ok(reply
            .monitors
            .iter()
            .map(|m| Monitor {
                bounds: IRect::new(m.x as i32, m.y as i32, m.width as i32, m.height as i32),
                primary: m.primary,
            })
            .collect());
    }
    // RandR unavailable → the screen itself is the one monitor.
    let scr = conn
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| Error::msg("X11: screen index out of range"))?;
    Ok(vec![Monitor {
        bounds: IRect::new(
            0,
            0,
            scr.width_in_pixels as i32,
            scr.height_in_pixels as i32,
        ),
        primary: true,
    }])
}

/// Bounding box of all monitors (the capture region).
pub fn virtual_screen<C: Connection>(conn: &C, screen: usize) -> Result<VirtualScreen> {
    let ms = monitors(conn, screen)?;
    let rects: Vec<IRect> = ms.iter().map(|m| m.bounds).collect();
    let u = crate::platform::x11_logic::union_bounds(&rects)
        .ok_or_else(|| Error::invalid("no monitors reported (headless session?)"))?;
    Ok(VirtualScreen {
        x: u.x,
        y: u.y,
        w: u.w,
        h: u.h,
    })
}

/// Server image format for the root visual: sample layout from the root
/// window's visual (colour masks) plus the connection setup (`pixmap_formats`
/// for bits-per-pixel/scanline pad, image byte order).
pub fn screen_image_format<C: Connection>(
    conn: &C,
    screen: usize,
    width: u32,
    height: u32,
) -> Result<ImageFormat> {
    let setup = conn.setup();
    let (depth, root_visual) = root_depth_visual(conn, screen)?;
    let vis = setup
        .roots
        .get(screen)
        .and_then(|s| {
            s.allowed_depths.iter().find_map(|d| {
                d.visuals
                    .iter()
                    .find(|v| v.visual_id == root_visual)
                    .map(|v| (depth, v.red_mask, v.green_mask, v.blue_mask))
            })
        })
        .ok_or_else(|| Error::msg("X11: root visual not found"))?;
    let fmt = setup
        .pixmap_formats
        .iter()
        .find(|f| f.depth == vis.0)
        .ok_or_else(|| Error::msg("X11: no pixmap format for root depth"))?;
    Ok(ImageFormat {
        width,
        height,
        bits_per_pixel: fmt.bits_per_pixel,
        scanline_pad_bits: fmt.scanline_pad,
        big_endian: setup.image_byte_order == ImageOrder::MSB_FIRST,
        red_mask: vis.1,
        green_mask: vis.2,
        blue_mask: vis.3,
    })
}

/// Effective DPI from the root `RESOURCE_MANAGER` (`Xft.dpi`), default 96.
/// Read on demand — no caching yet because no UI startup path exists on
/// X11 in M1.
pub fn dpi<C: Connection>(conn: &C, root: x11rb::protocol::xproto::Window) -> u16 {
    let property: Atom = AtomEnum::RESOURCE_MANAGER.into();
    let r = conn
        .get_property(false, root, property, AtomEnum::STRING, 0, u32::MAX)
        .ok()
        .and_then(|c| c.reply().ok());
    let text = r.and_then(|reply| String::from_utf8(reply.value).ok());
    text.as_deref()
        .and_then(crate::platform::x11_logic::parse_xft_dpi)
        .unwrap_or(96)
}
