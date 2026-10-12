//! Redaction-correctness tests for the export path.
//!
//! These implement the acceptance criteria written in `docs/ARCHITECTURE.md`
//! ("Redaction correctness model"): mosaic equals block averages and keeps no
//! original per-pixel values, solid is fully opaque, the flattened output
//! differs from the source only inside redaction regions, and an encode →
//! decode round-trip equals the flattened buffer byte-for-byte.
//!
//! The encode/decode round-trip uses WIC — Windows targets only (see
//! `encode_roundtrip.rs` for the same rationale).
#![cfg(windows)]

use nameshot::annotate::Shape;
use nameshot::config::OutputFormat;
use nameshot::encode;
use nameshot::flatten::flatten;
use nameshot::image::{Frame, IRect, Rgba};
use std::collections::HashSet;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

/// WIC requires COM to be initialized on the calling thread.
fn init_com() {
    unsafe {
        // S_FALSE when already initialized; RPC_E_CHANGED_MODE only if the
        // thread is already in a different apartment — both are fine here.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

/// Synthetic source: every colour channel is **even**, laid out as a 4x4 grid
/// where each cell holds `base, base+2, …, base+30`. The cell mean is therefore
/// `base + 15` (odd) and can never equal any original pixel value.
fn sample_source(w: u32, h: u32, alpha: u8) -> Frame {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (bx, by) = (x / 4, y / 4);
            let k = (y % 4) * 4 + (x % 4);
            let b = 2 * ((bx * 7 + by * 11) % 96) + 2 * k;
            let g = 2 * ((bx * 13 + by * 3) % 96) + 2 * k;
            let r = 2 * ((bx * 5 + by * 17) % 96) + 2 * k;
            px.extend_from_slice(&[b as u8, g as u8, r as u8, alpha]);
        }
    }
    Frame::from_bgra(w, h, px).expect("sample frame")
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("nameshot_redaction_test");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn mosaic_output_equals_block_means_and_contains_no_original_values() {
    let src = sample_source(32, 24, 255);
    let region = IRect::new(0, 0, 16, 12);
    let out = flatten(
        &src,
        &[Shape::Mosaic {
            rect: region,
            block: 4,
        }],
    );

    let mut original_values = HashSet::new();
    let mut output_values = HashSet::new();
    for y in 0..24 {
        for x in 0..32 {
            let s = src.get(x, y).unwrap();
            let o = out.get(x, y).unwrap();
            if region.contains(x, y) {
                let (bx, by) = ((x / 4) as u32, (y / 4) as u32);
                let base_b = 2 * ((bx * 7 + by * 11) % 96);
                let base_g = 2 * ((bx * 13 + by * 3) % 96);
                let base_r = 2 * ((bx * 5 + by * 17) % 96);
                assert_eq!(
                    o,
                    [
                        (base_b + 15) as u8,
                        (base_g + 15) as u8,
                        (base_r + 15) as u8,
                        255
                    ],
                    "mosaic pixel {x},{y} must equal its block mean and be opaque"
                );
                for k in 0..3 {
                    original_values.insert(s[k]);
                    output_values.insert(o[k]);
                }
            } else {
                assert_eq!(s, o, "pixel {x},{y} outside the region must be untouched");
            }
        }
    }
    let overlap: Vec<u8> = original_values
        .intersection(&output_values)
        .cloned()
        .collect();
    assert!(
        overlap.is_empty(),
        "redacted region must contain no original colour values, found {overlap:?}"
    );
}

#[test]
fn solid_output_is_fully_opaque() {
    let src = sample_source(16, 16, 17);
    assert_eq!(
        src.get(0, 0).unwrap()[3],
        17,
        "source alpha must not be 255"
    );
    let region = IRect::new(4, 4, 6, 6);
    let out = flatten(
        &src,
        &[Shape::Solid {
            rect: region,
            color: Rgba::new(10, 20, 30, 42),
        }],
    );
    for y in 4..10 {
        for x in 4..10 {
            let p = out.get(x, y).unwrap();
            assert_eq!(
                p,
                [30, 20, 10, 255],
                "redacted pixel {x},{y} must be opaque"
            );
            assert_eq!(p[3], 255, "alpha of pixel {x},{y} must be forced to 255");
        }
    }
    for y in 0..16 {
        for x in 0..16 {
            if !region.contains(x, y) {
                assert_eq!(
                    out.get(x, y),
                    src.get(x, y),
                    "pixel {x},{y} outside the region"
                );
            }
        }
    }
}

#[test]
fn flatten_differs_from_source_only_inside_redaction_regions() {
    let src = sample_source(32, 24, 255);
    let mosaic = IRect::new(0, 0, 16, 12);
    let solid = IRect::new(16, 12, 8, 6);
    let out = flatten(
        &src,
        &[
            Shape::Mosaic {
                rect: mosaic,
                block: 4,
            },
            Shape::Solid {
                rect: solid,
                color: Rgba::new(1, 3, 5, 90),
            },
        ],
    );
    let mut diffs = 0usize;
    for y in 0..24 {
        for x in 0..32 {
            let inside = mosaic.contains(x, y) || solid.contains(x, y);
            let same = src.get(x, y) == out.get(x, y);
            assert_eq!(same, !inside, "pixel {x},{y}: inside={inside} same={same}");
            if !same {
                diffs += 1;
            }
        }
    }
    assert_eq!(
        diffs,
        mosaic.area() + solid.area(),
        "every region pixel must differ"
    );
    assert_eq!(src.diff_pixels(&out), mosaic.area() + solid.area());
}

#[test]
fn encode_decode_roundtrip_equals_flattened_buffer() {
    init_com();
    let src = sample_source(40, 30, 255);
    let items = vec![
        Shape::Rect {
            rect: IRect::new(2, 2, 10, 8),
            color: Rgba::opaque(255, 0, 0),
            width: 2,
            filled: true,
        },
        Shape::Pen {
            pts: vec![(0, 0), (39, 29), (10, 25)],
            color: Rgba::opaque(0, 255, 255),
            width: 3,
        },
        Shape::Mosaic {
            rect: IRect::new(8, 8, 16, 12),
            block: 4,
        },
        Shape::Solid {
            rect: IRect::new(24, 4, 8, 8),
            color: Rgba::new(200, 100, 50, 99),
        },
    ];
    let flat = flatten(&src, &items);
    assert_ne!(flat, src, "redactions must change the flattened output");
    let path = temp_path("redaction_roundtrip.png");
    encode::encode_to_file(&path, &flat, OutputFormat::Png, 100).expect("encode_to_file");
    let decoded = encode::decode_file(&path).expect("decode_file");
    let _ = std::fs::remove_file(&path);
    assert_eq!(decoded.w, flat.w);
    assert_eq!(decoded.h, flat.h);
    assert_eq!(
        decoded, flat,
        "PNG round-trip must equal the flattened buffer byte-for-byte"
    );
}
