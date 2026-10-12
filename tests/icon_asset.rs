//! Asset contract for the shipped Windows icon.
//!
//! `assets/icon.ico` is generated from `assets/icon.png` by
//! `assets/make_icon.ps1` and embedded into `nameshot.exe` as RT_ICON id 1 by
//! `build.rs`. This test locks the shape of that contract: 7 square,
//! PNG-compressed, 32 bpp RGBA frames (16..256) and nothing else in the file.

use std::fs;
use std::path::PathBuf;

const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

fn asset(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join(name)
}

fn u16le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], 0, 0])
}

fn u32le(b: &[u8]) -> usize {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize
}

#[test]
fn icon_ico_is_seven_png_rgba_frames() {
    let bytes = fs::read(asset("icon.ico")).expect("read assets/icon.ico");

    // ICONDIR header: reserved = 0, type = 1 (icon), then the frame count.
    assert_eq!(u16le(&bytes[0..2]), 0, "ICONDIR reserved");
    assert_eq!(u16le(&bytes[2..4]), 1, "ICONDIR type (icon)");
    assert_eq!(u16le(&bytes[4..6]), SIZES.len() as u32, "frame count");

    for (i, &dim) in SIZES.iter().enumerate() {
        let e = &bytes[6 + i * 16..6 + (i + 1) * 16];
        let stored = if dim == 256 { 0usize } else { dim as usize };
        assert_eq!(e[0] as usize, stored, "frame {i} width byte");
        assert_eq!(e[1] as usize, stored, "frame {i} height byte");
        assert_eq!(u16le(&e[4..6]), 1, "frame {i} planes");
        assert_eq!(u16le(&e[6..8]), 32, "frame {i} bpp");

        let len = u32le(&e[8..12]);
        let off = u32le(&e[12..16]);
        let data = &bytes[off..off + len];

        assert_eq!(data[..8], PNG_SIGNATURE, "frame {i} PNG signature");
        assert_eq!(&data[12..16], b"IHDR", "frame {i} IHDR");
        let iw = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let ih = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        assert_eq!(iw, dim, "frame {i} IHDR width");
        assert_eq!(ih, dim, "frame {i} IHDR height");
        assert_eq!(data[24], 8, "frame {i} PNG bit depth");
        assert_eq!(data[25], 6, "frame {i} PNG color type (RGBA)");

        // No overlap: each frame starts exactly where the previous ends.
        let mut expected_off = 6 + SIZES.len() * 16;
        for j in 0..i {
            let prev = &bytes[6 + j * 16..6 + (j + 1) * 16];
            expected_off += u32le(&prev[8..12]);
        }
        assert_eq!(off, expected_off, "frame {i} offset is contiguous");
    }

    // Last frame ends exactly at EOF: no trailing bytes.
    let last = &bytes[6 + (SIZES.len() - 1) * 16..6 + SIZES.len() * 16];
    assert_eq!(
        u32le(&last[12..16]) + u32le(&last[8..12]),
        bytes.len(),
        "file ends after the last frame"
    );
}

#[test]
fn icon_source_png_exists_and_is_square() {
    let bytes = fs::read(asset("icon.png")).expect("read assets/icon.png");
    assert_eq!(bytes[..8], PNG_SIGNATURE, "source PNG signature");
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    assert_eq!(w, h, "source artwork must be square");
    assert!(w >= 256, "source artwork must cover the largest frame");
}
