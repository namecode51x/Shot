//! One-shot whole-desktop capture via `GetImage` on the root window — the
//! X11 counterpart of the GDI `BitBlt` path, with the same one-shot-per-
//! session model: grab once when a capture starts, then treat the frame as
//! immutable. The root window spans the full virtual desktop, so one request
//! covers multi-monitor layouts (root coordinates have no negatives).
//!
//! Known limitations mirror the Windows ones: protected content (DRM,
//! locked screens) comes back black or missing and is not an error. Only
//! TrueColor/DirectColor visuals are supported (see `x11_logic`).

use crate::error::{Error, Result};
use crate::image::{Frame, MAX_DIM};
use crate::platform::x11_logic::{self, ImageFormat};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, Window};

use super::display;

pub struct Capture {
    /// Physical pixels of the whole virtual desktop.
    pub frame: Frame,
    /// Origin/size of the virtual desktop in root coordinates.
    pub virtual_screen: display::VirtualScreen,
}

/// Capture the virtual screen exactly once.
pub fn grab_virtual_screen<C: Connection>(conn: &C, screen: usize) -> Result<Capture> {
    let vs = display::virtual_screen(conn, screen)?;
    if !vs.is_valid() {
        return Err(Error::invalid(
            "virtual screen has no pixels (headless session?)",
        ));
    }
    if vs.w > MAX_DIM || vs.h > MAX_DIM {
        return Err(Error::invalid(format!(
            "virtual screen {}x{} exceeds the supported maximum",
            vs.w, vs.h
        )));
    }

    let root: Window = display::root_of(conn, screen)?;
    let fmt: ImageFormat = display::screen_image_format(conn, screen, vs.w as u32, vs.h as u32)?;

    // ALL_PLANES; the reply data is raw ZPixmap in the client byte order
    // announced at connection setup (captured in `fmt.big_endian`).
    let image = conn
        .get_image(
            x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
            root,
            vs.x as i16,
            vs.y as i16,
            vs.w as u16,
            vs.h as u16,
            u32::MAX,
        )
        .map_err(|e| Error::msg(format!("GetImage failed: {e}")))?
        .reply()
        .map_err(|e| Error::msg(format!("GetImage failed: {e}")))?;

    let px = x11_logic::bgra_from_zpixmap(&image.data, &fmt).ok_or_else(|| {
        Error::msg("unsupported screen pixel format (TrueColor/DirectColor visuals only)")
    })?;

    let frame = Frame::from_bgra(vs.w as u32, vs.h as u32, px)
        .ok_or_else(|| Error::msg("captured frame failed validation"))?;

    Ok(Capture {
        frame,
        virtual_screen: vs,
    })
}
