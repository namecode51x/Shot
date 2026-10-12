//! Image encoding on Linux: PNG through the `png` crate (pure Rust, no
//! system libraries), BMP through the core [`crate::output::build_bmp`]
//! (hand-rolled file header over the `CF_DIB` body), JPEG deliberately not
//! adopted in M1 — no encoder dependency was added without a runtime to
//! validate it (plan §7 discipline; documented in `docs/BUILD_LOG.md`).
//!
//! Mirrors the Windows `crate::encode` surface used by the save/clipboard
//! paths so the future UI port can switch with a `cfg`.

use crate::config::OutputFormat;
use crate::error::{Error, Result};
use crate::image::Frame;
use crate::platform::x11_logic;
use std::io::Write;
use std::path::Path;

/// Encode to PNG bytes in memory (clipboard `image/png` target, memory
/// round-trips). RGBA in, standard 8-bit RGBA PNG out.
pub fn encode_png_bytes(frame: &Frame) -> Result<Vec<u8>> {
    if frame.w == 0 || frame.h == 0 {
        return Err(Error::invalid("cannot encode a zero-sized image"));
    }
    let rgba = x11_logic::rgba_from_bgra(&frame.px)
        .ok_or_else(|| Error::invalid("frame buffer length mismatch"))?;
    let mut out = Vec::with_capacity(rgba.len() / 4 + 1024);
    {
        // Inner scope: the writer borrows `out` until it is dropped.
        let mut encoder = png::Encoder::new(&mut out, frame.w, frame.h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| Error::msg(format!("PNG header: {e}")))?;
        writer
            .write_image_data(&rgba)
            .map_err(|e| Error::msg(format!("PNG data: {e}")))?;
    }
    Ok(out)
}

/// Encode to a file. `quality` is accepted for signature parity with the
/// Windows adapter and ignored (PNG and BMP are lossless).
pub fn encode_to_file(
    path: &Path,
    frame: &Frame,
    format: OutputFormat,
    _quality: u8,
) -> Result<()> {
    if frame.w == 0 || frame.h == 0 {
        return Err(Error::invalid("cannot encode a zero-sized image"));
    }
    let bytes = match format {
        OutputFormat::Png => encode_png_bytes(frame)?,
        OutputFormat::Bmp => crate::output::build_bmp(frame)?,
        OutputFormat::Jpeg => {
            return Err(Error::msg(
                "JPEG output is not available on this build yet \
                 (no encoder dependency adopted in M1; see docs/BUILD_LOG.md)",
            ));
        }
    };
    let mut f = std::fs::File::create(path).map_err(|e| Error::io("create", e))?;
    f.write_all(&bytes).map_err(|e| Error::io("write", e))?;
    Ok(())
}
