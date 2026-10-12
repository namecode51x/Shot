//! Output helpers that do not touch the OS directly: clipboard DIB layout,
//! safe filename generation, unique-path derivation. Everything here is pure
//! and unit-tested; the Win32 calls live in `win::clipboard` and the editor.

use crate::config::OutputFormat;
use crate::error::{Error, Result};
use crate::image::Frame;

pub const BITMAPINFOHEADER_SIZE: u32 = 40;
pub const BITMAPFILEHEADER_SIZE: u32 = 14;

/// Build a complete BMP file: 14-byte `BITMAPFILEHEADER` followed by the
/// `CF_DIB` body from [`build_dib`] (top-down — negative height, which the
/// BMP spec allows and mainstream decoders honour). Used by the Linux
/// encoder and the unit tests; compiled outside the Windows release binary
/// (`cfg`), which saves BMP through its own path.
#[cfg(any(test, not(windows)))]
pub fn build_bmp(frame: &Frame) -> Result<Vec<u8>> {
    let dib = build_dib(frame)?;
    let total = (BITMAPFILEHEADER_SIZE as usize)
        .checked_add(dib.len())
        .ok_or_else(|| Error::invalid("BMP size overflows"))?;
    if total > u32::MAX as usize {
        return Err(Error::invalid("BMP too large"));
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved 1
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved 2
    // Pixel data starts right after both headers.
    out.extend_from_slice(&(BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE).to_le_bytes());
    out.extend_from_slice(&dib);
    Ok(out)
}

/// Build a `CF_DIB` payload: BITMAPINFOHEADER (32bpp, top-down BI_RGB) followed
/// by the tightly packed BGRA rows of `frame`.
///
/// Sizes are checked before allocation; no padding is needed because each row
/// is already 4-byte aligned.
pub fn build_dib(frame: &Frame) -> Result<Vec<u8>> {
    let w = frame.w;
    let h = frame.h;
    if w == 0 || h == 0 {
        return Err(Error::invalid("image has zero size"));
    }
    let row_bytes = (w as usize)
        .checked_mul(4)
        .ok_or_else(|| Error::invalid("row size overflows"))?;
    let body = row_bytes
        .checked_mul(h as usize)
        .ok_or_else(|| Error::invalid("image size overflows"))?;
    if frame.px.len() != body {
        return Err(Error::invalid("frame buffer length mismatch"));
    }
    let mut out = Vec::with_capacity(BITMAPINFOHEADER_SIZE as usize + body);
    out.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    // Negative height => top-down rows, matching `frame.px` order.
    out.extend_from_slice(&(-(h as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bit count
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(body as u32).to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes()); // XPelsPerMeter
    out.extend_from_slice(&0i32.to_le_bytes()); // YPelsPerMeter
    out.extend_from_slice(&0u32.to_le_bytes()); // ClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // ClrImportant
    out.extend_from_slice(&frame.px);
    Ok(out)
}

/// `2026-10-06_153045` style timestamp label from a local time tuple.
pub fn timestamp_label(dt: (i32, u32, u32, u32, u32, u32)) -> String {
    let (y, mo, d, h, mi, s) = dt;
    format!("{y:04}-{mo:02}-{d:02}_{h:02}{mi:02}{s:02}")
}

/// Sanitize a base name for the filesystem: keep `[A-Za-z0-9._-]`, collapse
/// everything else to `_`, cap the length, and never return an empty name.
pub fn sanitize_base(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars().take(80) {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    while out.ends_with('.') || out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "screenshot".to_string()
    } else {
        out
    }
}

/// Default file name for a capture: `NameShot_<timestamp>.<ext>`.
pub fn default_filename(format: OutputFormat, dt: (i32, u32, u32, u32, u32, u32)) -> String {
    format!("NameShot_{}.{}", timestamp_label(dt), format.extension())
}

/// Given a desired path, return `path` when it does not exist, otherwise
/// `name (2).ext`, `name (3).ext`... up to `limit`. Never overwrites.
pub fn unique_path<F>(desired: &str, exists: F) -> String
where
    F: Fn(&str) -> bool,
{
    if !exists(desired) {
        return desired.to_string();
    }
    let (stem, ext) = match desired.rfind('.') {
        Some(i) if i > desired.rfind(['/', '\\']).unwrap_or(0) => (&desired[..i], &desired[i..]),
        _ => (desired, ""),
    };
    for n in 2..1000 {
        let candidate = format!("{stem} ({n}){ext}");
        if !exists(&candidate) {
            return candidate;
        }
    }
    // Pathological: fall back to a numeric suffix beyond the loop.
    format!("{stem} ({}){ext}", 1000 + std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32) -> Frame {
        let mut f = Frame::new(w, h).unwrap();
        for (i, v) in f.px.iter_mut().enumerate() {
            *v = (i % 250) as u8;
        }
        f.px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        f
    }

    #[test]
    fn dib_layout_is_exact() {
        let f = frame(3, 2);
        let dib = build_dib(&f).unwrap();
        assert_eq!(dib.len(), 40 + 3 * 2 * 4);
        assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 40);
        assert_eq!(i32::from_le_bytes(dib[4..8].try_into().unwrap()), 3);
        assert_eq!(
            i32::from_le_bytes(dib[8..12].try_into().unwrap()),
            -2,
            "top-down"
        );
        assert_eq!(u16::from_le_bytes(dib[12..14].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(dib[14..16].try_into().unwrap()), 32);
        assert_eq!(
            u32::from_le_bytes(dib[16..20].try_into().unwrap()),
            0,
            "BI_RGB"
        );
        assert_eq!(
            u32::from_le_bytes(dib[20..24].try_into().unwrap()),
            24,
            "biSizeImage"
        );
        // payload equals frame bytes exactly (BGRA, top-down)
        assert_eq!(&dib[40..], f.px.as_slice());
        // BGRA: first pixel blue channel first
        assert_eq!(dib[40], f.px[0]);
    }

    #[test]
    fn dib_rejects_bad_frames() {
        let f = frame(4, 4);
        let mut broken = f.clone();
        broken.px.truncate(10);
        assert!(build_dib(&broken).is_err());
        let empty = Frame::new(0, 1);
        if let Some(e) = empty {
            assert!(build_dib(&e).is_err());
        }
    }

    #[test]
    fn dib_round_trip_pixels() {
        let f = frame(5, 3);
        let dib = build_dib(&f).unwrap();
        let body = &dib[40..];
        let back = Frame::from_bgra(5, 3, body.to_vec()).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn timestamp_and_filename() {
        let dt = (2026, 10, 6, 15, 30, 45);
        assert_eq!(timestamp_label(dt), "2026-10-06_153045");
        assert_eq!(
            default_filename(OutputFormat::Png, dt),
            "NameShot_2026-10-06_153045.png"
        );
        assert_eq!(
            default_filename(OutputFormat::Jpeg, dt),
            "NameShot_2026-10-06_153045.jpg"
        );
    }

    #[test]
    fn sanitize_keeps_only_safe_chars() {
        assert_eq!(sanitize_base("report final"), "report_final");
        assert_eq!(sanitize_base("a/b\\c:d*e?f"), "a_b_c_d_e_f");
        assert_eq!(sanitize_base("..."), "screenshot");
        assert_eq!(sanitize_base(""), "screenshot");
        assert_eq!(sanitize_base("keep-1.2_3"), "keep-1.2_3");
        let long = "x".repeat(500);
        assert!(sanitize_base(&long).len() <= 80);
    }

    #[test]
    fn unique_path_never_repeats() {
        let taken = ["shot.png", "shot (2).png"];
        let got = unique_path("shot.png", |p| taken.contains(&p));
        assert_eq!(got, "shot (3).png");
        let got2 = unique_path("other.jpg", |p| taken.contains(&p));
        assert_eq!(got2, "other.jpg");
        // extension detection with dots in directories: only the desired path
        // exists, so the first candidate wins — proving the split happened at
        // the extension, not at the directory dot.
        let got3 = unique_path("C:\\a.b\\shot.png", |p| p == "C:\\a.b\\shot.png");
        assert_eq!(got3, "C:\\a.b\\shot (2).png");
    }

    #[test]
    fn bmp_file_layout_is_exact() {
        let f = frame(2, 3);
        let bmp = build_bmp(&f).unwrap();
        let dib = build_dib(&f).unwrap();
        assert_eq!(bmp.len(), 14 + dib.len());
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(bmp[2..6].try_into().unwrap()),
            bmp.len() as u32,
            "file size field"
        );
        assert_eq!(u16::from_le_bytes(bmp[6..8].try_into().unwrap()), 0);
        assert_eq!(u16::from_le_bytes(bmp[8..10].try_into().unwrap()), 0);
        assert_eq!(
            u32::from_le_bytes(bmp[10..14].try_into().unwrap()),
            54,
            "pixel offset"
        );
        // DIB body intact: negative height (top-down) and payload unchanged.
        // biHeight lives at DIB offset 8 → file offset 14 + 8 = 22.
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), -3);
        assert_eq!(&bmp[14..], dib.as_slice());
        assert_eq!(&bmp[54..], f.px.as_slice());
    }

    #[test]
    fn bmp_rejects_bad_frames() {
        let mut broken = frame(4, 4);
        broken.px.truncate(9);
        assert!(build_bmp(&broken).is_err());
    }
}
