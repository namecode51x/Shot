//! Privacy redaction primitives. These are the only operations that may
//! destroy source pixels; they are pure functions over `Frame` so the exact
//! exported result can be unit-tested without any UI.

use super::{Frame, IRect, Rgba};

pub const MIN_BLOCK: u32 = 2;
pub const MAX_BLOCK: u32 = 256;

/// Mosaic / pixelation.
///
/// The region is divided into a grid of `block x block` cells aligned to the
/// region origin; every pixel of a cell becomes the **mean** of the original
/// pixels of that cell (computed from the region-clipped cell only). The source
/// pixels are overwritten in place — nothing keeps the originals, and the alpha
/// of redacted pixels is forced opaque.
///
/// Pixelation reduces legibility but is **not** a guarantee of anonymity: on
/// low-contrast or highly patterned content some information may remain
/// inferable. Use [`redact::solid`] when the content must be unreadable for sure.
pub fn pixelate(img: &mut Frame, region: IRect, block: u32) {
    let block = block.clamp(MIN_BLOCK, MAX_BLOCK) as i32;
    let r = region.clamp_to(img.w as i32, img.h as i32);
    if r.is_empty() {
        return;
    }
    let w = img.w as usize;
    let mut y = r.y;
    while y < r.bottom() {
        let bh = block.min(r.bottom() - y);
        let mut x = r.x;
        while x < r.right() {
            let bw = block.min(r.right() - x);
            // Mean of the clipped cell.
            let mut sum = [0u64; 4];
            for yy in y..y + bh {
                let base = (yy as usize * w + x as usize) * 4;
                for xx in 0..bw as usize {
                    let i = base + xx * 4;
                    sum[0] += img.px[i] as u64;
                    sum[1] += img.px[i + 1] as u64;
                    sum[2] += img.px[i + 2] as u64;
                    sum[3] += img.px[i + 3] as u64;
                }
            }
            let n = (bw * bh) as u64;
            let avg = [
                ((sum[0] + n / 2) / n) as u8,
                ((sum[1] + n / 2) / n) as u8,
                ((sum[2] + n / 2) / n) as u8,
                ((sum[3] + n / 2) / n) as u8,
            ];
            for yy in y..y + bh {
                let base = (yy as usize * w + x as usize) * 4;
                for xx in 0..bw as usize {
                    let i = base + xx * 4;
                    img.px[i] = avg[0];
                    img.px[i + 1] = avg[1];
                    img.px[i + 2] = avg[2];
                    img.px[i + 3] = 255;
                }
            }
            x += bw;
        }
        y += bh;
    }
}

/// Solid opaque redaction: the region becomes a single flat colour
/// (alpha forced to 255). No original pixel survives, whatever the content.
pub fn solid(img: &mut Frame, region: IRect, color: Rgba) {
    let r = region.clamp_to(img.w as i32, img.h as i32);
    if r.is_empty() {
        return;
    }
    img.fill_rect(r, Rgba::new(color.r, color.g, color.b, 255));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn striped(w: u32, h: u32) -> Frame {
        // Every pixel has a distinct-ish value so nothing can survive "by chance".
        let mut f = Frame::new(w, h).unwrap();
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let v = ((x * 7 + y * 131) % 251) as u8;
                let i = f.index(x, y).unwrap();
                f.px[i] = v;
                f.px[i + 1] = (v as u16 + 17).rem_euclid(256) as u8;
                f.px[i + 2] = (v as u16 + 91).rem_euclid(256) as u8;
                f.px[i + 3] = 255;
            }
        }
        f
    }

    #[test]
    fn pixelate_transforms_every_pixel_in_region_to_block_mean() {
        let src = striped(32, 32);
        let mut out = src.clone();
        let region = IRect::new(4, 4, 17, 13);
        pixelate(&mut out, region, 4);
        let clamped = region.clamp_to(32, 32);
        // Every pixel in the region must equal its block's mean and be opaque.
        for y in clamped.y..clamped.bottom() {
            for x in clamped.x..clamped.right() {
                let bx = clamped.x + ((x - clamped.x) / 4) * 4;
                let by = clamped.y + ((y - clamped.y) / 4) * 4;
                let bw = 4.min(clamped.right() - bx);
                let bh = 4.min(clamped.bottom() - by);
                let mut sum = [0u64; 4];
                for yy in by..by + bh {
                    for xx in bx..bx + bw {
                        let p = src.get(xx, yy).unwrap();
                        for k in 0..4 {
                            sum[k] += p[k] as u64;
                        }
                    }
                }
                let n = (bw * bh) as u64;
                let expected: Vec<u8> = sum.iter().map(|s| ((s + n / 2) / n) as u8).collect();
                let got = out.get(x, y).unwrap();
                assert_eq!(&got[0..3], &expected[0..3], "pixel {x},{y}");
                assert_eq!(got[3], 255);
            }
        }
        // Pixels outside the region are untouched; inside, a pixel may keep
        // its value when it already equals the block mean (asserted above).
        assert!(src.diff_pixels(&out) <= clamped.area());
        let untouched = out.crop(IRect::new(0, 0, 32, 4)).unwrap();
        let orig = src.crop(IRect::new(0, 0, 32, 4)).unwrap();
        assert_eq!(untouched, orig);
    }

    #[test]
    fn pixelate_reduces_distinct_values_inside_region() {
        let src = striped(24, 24);
        let mut out = src.clone();
        let region = IRect::new(0, 0, 24, 24);
        let before: std::collections::HashSet<[u8; 3]> = (0..24)
            .flat_map(|y| (0..24).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = src.get(x, y).unwrap();
                [p[0], p[1], p[2]]
            })
            .collect();
        pixelate(&mut out, region, 8);
        let after: std::collections::HashSet<[u8; 3]> = (0..24)
            .flat_map(|y| (0..24).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = out.get(x, y).unwrap();
                [p[0], p[1], p[2]]
            })
            .collect();
        assert!(
            after.len() < before.len(),
            "distinct {} -> {}",
            before.len(),
            after.len()
        );
        assert_eq!(after.len(), 9, "3x3 grid of 8px blocks");
    }

    #[test]
    fn pixelate_ignores_empty_or_out_of_bounds_regions() {
        let src = striped(8, 8);
        let mut out = src.clone();
        pixelate(&mut out, IRect::new(3, 3, 0, 5), 4);
        pixelate(&mut out, IRect::new(100, 100, 4, 4), 4);
        pixelate(&mut out, IRect::new(-5, -5, 3, 3), 4);
        assert_eq!(src, out);
        // Partially out-of-bounds regions only touch the visible part.
        pixelate(&mut out, IRect::new(6, 6, 8, 8), 4);
        assert!(src.diff_pixels(&out) <= 4);
    }

    #[test]
    fn solid_redaction_is_fully_opaque_and_covers_region() {
        let src = striped(16, 16);
        let mut out = src.clone();
        solid(&mut out, IRect::new(2, 2, 6, 6), Rgba::new(12, 34, 56, 78));
        for y in 2..8 {
            for x in 2..8 {
                // Frame::get is BGRA: b=56, g=34, r=12, a forced to 255.
                assert_eq!(out.get(x, y).unwrap(), [56, 34, 12, 255]);
            }
        }
        assert_eq!(src.diff_pixels(&out), 36);
        // nothing outside changed
        assert_eq!(out.get(0, 0), src.get(0, 0));
    }

    #[test]
    fn block_size_clamped() {
        let src = striped(8, 8);
        let mut out = src.clone();
        pixelate(&mut out, IRect::new(0, 0, 8, 8), 1); // clamped up to MIN_BLOCK
        let vals: std::collections::HashSet<[u8; 3]> = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = out.get(x, y).unwrap();
                [p[0], p[1], p[2]]
            })
            .collect();
        assert_eq!(vals.len(), 16, "2x2 grid of 2px blocks");
    }
}
