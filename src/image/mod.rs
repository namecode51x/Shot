//! OS-free raster image type and drawing primitives.
//! Pixel layout is **BGRA8, straight alpha** — the layout used by GDI DIBs,
//! the Windows clipboard and WIC, so no conversion happens at the boundaries.

pub mod redact;

pub const MAX_DIM: i32 = 32_767;
/// Upper bound for a single buffer: 256 megapixels (1 GiB of BGRA).
pub const MAX_PIXELS: usize = 268_435_456;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Rgba { r, g, b, a }
    }
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Rgba { r, g, b, a: 255 }
    }
    pub const fn bgra(self) -> [u8; 4] {
        [self.b, self.g, self.r, self.a]
    }
    pub const fn from_bgra(bgra: [u8; 4]) -> Self {
        Rgba {
            b: bgra[0],
            g: bgra[1],
            r: bgra[2],
            a: bgra[3],
        }
    }
}

/// Integer rectangle, x/y = top-left, w/h >= 0, bottom/right edges exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl IRect {
    pub const EMPTY: IRect = IRect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        IRect { x, y, w, h }
    }

    /// Rectangle covering both points inclusive (drag selection convention).
    pub fn from_points(ax: i32, ay: i32, bx: i32, by: i32) -> Self {
        let x0 = ax.min(bx);
        let y0 = ay.min(by);
        let x1 = ax.max(bx);
        let y1 = ay.max(by);
        IRect {
            x: x0,
            y: y0,
            w: x1 - x0 + 1,
            h: y1 - y0 + 1,
        }
    }

    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.w)
    }
    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h)
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn area(&self) -> usize {
        if self.is_empty() {
            0
        } else {
            (self.w as usize).saturating_mul(self.h as usize)
        }
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub fn intersect(&self, other: &IRect) -> IRect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 <= x0 || y1 <= y0 {
            IRect::EMPTY
        } else {
            IRect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }

    /// Clamp into a `width x height` image, returning `IRect::EMPTY` if no overlap.
    pub fn clamp_to(&self, width: i32, height: i32) -> IRect {
        self.intersect(&IRect::new(0, 0, width.max(0), height.max(0)))
    }

    pub fn expand(&self, by: i32) -> IRect {
        IRect::new(
            self.x.saturating_sub(by),
            self.y.saturating_sub(by),
            self.w.saturating_add(by.saturating_mul(2)),
            self.h.saturating_add(by.saturating_mul(2)),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    /// BGRA8, row-major, `len == w * h * 4`.
    pub px: Vec<u8>,
}

fn checked_len(w: u32, h: u32) -> Option<usize> {
    if w == 0 || h == 0 || w > MAX_DIM as u32 || h > MAX_DIM as u32 {
        return None;
    }
    let pixels = (w as usize).checked_mul(h as usize)?;
    if pixels > MAX_PIXELS {
        return None;
    }
    Some(pixels.checked_mul(4)?)
}

impl Frame {
    pub fn new(w: u32, h: u32) -> Option<Frame> {
        let len = checked_len(w, h)?;
        Some(Frame {
            w,
            h,
            px: vec![0u8; len],
        })
    }

    pub fn from_bgra(w: u32, h: u32, px: Vec<u8>) -> Option<Frame> {
        let len = checked_len(w, h)?;
        if px.len() != len {
            return None;
        }
        Some(Frame { w, h, px })
    }

    pub fn bounds(&self) -> IRect {
        IRect::new(0, 0, self.w as i32, self.h as i32)
    }

    #[inline]
    pub fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return None;
        }
        Some((y as usize * self.w as usize + x as usize) * 4)
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        let i = self.index(x, y)?;
        Some([self.px[i], self.px[i + 1], self.px[i + 2], self.px[i + 3]])
    }

    /// Blend one pixel with the source-over operator (out = src*a + dst*(1-a)).
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: Rgba) {
        let Some(i) = self.index(x, y) else { return };
        let a = c.a as u32;
        if a == 255 {
            self.px[i] = c.b;
            self.px[i + 1] = c.g;
            self.px[i + 2] = c.r;
            self.px[i + 3] = 255;
            return;
        }
        if a == 0 {
            return;
        }
        let inv = 255 - a;
        let b = (c.b as u32 * a + self.px[i] as u32 * inv + 127) / 255;
        let g = (c.g as u32 * a + self.px[i + 1] as u32 * inv + 127) / 255;
        let r = (c.r as u32 * a + self.px[i + 2] as u32 * inv + 127) / 255;
        self.px[i] = b as u8;
        self.px[i + 1] = g as u8;
        self.px[i + 2] = r as u8;
        self.px[i + 3] = 255;
    }

    pub fn fill_rect(&mut self, r: IRect, c: Rgba) {
        let r = r.clamp_to(self.w as i32, self.h as i32);
        if r.is_empty() || c.a == 0 {
            return;
        }
        if c.a == 255 {
            let row = [c.b, c.g, c.r, 255];
            for y in r.y..r.bottom() {
                let start = (y as usize * self.w as usize + r.x as usize) * 4;
                for x in 0..r.w as usize {
                    let i = start + x * 4;
                    self.px[i..i + 4].copy_from_slice(&row);
                }
            }
        } else {
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    self.blend(x, y, c);
                }
            }
        }
    }

    pub fn stroke_rect(&mut self, r: IRect, c: Rgba, width: u32) {
        let t = width.max(1) as i32;
        if r.is_empty() {
            return;
        }
        let x0 = r.x;
        let y0 = r.y;
        let x1 = r.right();
        let y1 = r.bottom();
        // top, bottom
        self.fill_rect(IRect::new(x0, y0, r.w, t), c);
        self.fill_rect(IRect::new(x0, y1 - t, r.w, t), c);
        // left, right (excluding corners already painted)
        self.fill_rect(IRect::new(x0, y0 + t, t, r.h - 2 * t), c);
        self.fill_rect(IRect::new(x1 - t, y0 + t, t, r.h - 2 * t), c);
    }

    /// Thick line: every pixel whose integer coordinate lies within
    /// `width / 2` of the segment. Geometry coordinates are integer pixel
    /// indices, so an axis-aligned line of width `w` covers exactly `w` rows
    /// (odd `w`) or `w ± 1` (even `w`).
    pub fn line(&mut self, ax: i32, ay: i32, bx: i32, by: i32, c: Rgba, width: u32) {
        let half = (width.max(1) as f32) * 0.5;
        let radius = half;
        let x0 = ax.min(bx) - radius.ceil() as i32 - 1;
        let x1 = ax.max(bx) + radius.ceil() as i32 + 1;
        let y0 = ay.min(by) - radius.ceil() as i32 - 1;
        let y1 = ay.max(by) + radius.ceil() as i32 + 1;
        let dx = (bx - ax) as f32;
        let dy = (by - ay) as f32;
        let len2 = dx * dx + dy * dy;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let px = x as f32;
                let py = y as f32;
                let t = if len2 <= f32::EPSILON {
                    0.0
                } else {
                    (((px - ax as f32) * dx + (py - ay as f32) * dy) / len2).clamp(0.0, 1.0)
                };
                let cx = ax as f32 + t * dx;
                let cy = ay as f32 + t * dy;
                let dist2 = (px - cx) * (px - cx) + (py - cy) * (py - cy);
                if dist2 <= radius * radius {
                    self.blend(x, y, c);
                }
            }
        }
    }

    pub fn polyline(&mut self, pts: &[(i32, i32)], c: Rgba, width: u32) {
        match pts.len() {
            0 => {}
            1 => {
                self.line(pts[0].0, pts[0].1, pts[0].0, pts[0].1, c, width.max(2));
            }
            _ => {
                for w in pts.windows(2) {
                    self.line(w[0].0, w[0].1, w[1].0, w[1].1, c, width);
                }
            }
        }
    }

    pub fn arrow(&mut self, ax: i32, ay: i32, bx: i32, by: i32, c: Rgba, width: u32) {
        let dx = (bx - ax) as f32;
        let dy = (by - ay) as f32;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1.0 {
            return;
        }
        let ux = dx / len;
        let uy = dy / len;
        let head = (width.max(1) as f32 * 3.0).clamp(9.0, len.max(9.0));
        let half_base = head * 0.45;
        let tipx = bx as f32;
        let tipy = by as f32;
        let backx = tipx - ux * head;
        let backy = tipy - uy * head;
        let px = -uy;
        let py = ux;
        let p1 = (backx + px * half_base, backy + py * half_base);
        let p2 = (backx - px * half_base, backy - py * half_base);
        self.line(
            ax,
            ay,
            (backx + ux * 1.0) as i32,
            (backy + uy * 1.0) as i32,
            c,
            width,
        );
        self.fill_triangle((tipx, tipy), p1, p2, c);
    }

    fn fill_triangle(&mut self, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), c: Rgba) {
        let minx = p0.0.min(p1.0).min(p2.0).floor() as i32;
        let maxx = p0.0.max(p1.0).max(p2.0).ceil() as i32;
        let miny = p0.1.min(p1.1).min(p2.1).floor() as i32;
        let maxy = p0.1.max(p1.1).max(p2.1).ceil() as i32;
        let area = edge(p0, p1, p2);
        if area.abs() < 1e-6 {
            return;
        }
        for y in miny..=maxy {
            for x in minx..=maxx {
                let q = (x as f32 + 0.5, y as f32 + 0.5);
                let s = edge(p0, p1, q);
                let t = edge(p1, p2, q);
                let u = edge(p2, p0, q);
                let inside = (area > 0.0 && s >= 0.0 && t >= 0.0 && u >= 0.0)
                    || (area < 0.0 && s <= 0.0 && t <= 0.0 && u <= 0.0);
                if inside {
                    self.blend(x, y, c);
                }
            }
        }
    }

    /// Crop a region. The rectangle must lie inside the image and be non-empty.
    pub fn crop(&self, r: IRect) -> Result<Frame, String> {
        let r = r.clamp_to(self.w as i32, self.h as i32);
        if r.is_empty() {
            return Err("crop region is empty".into());
        }
        let mut out = Vec::with_capacity(r.area() * 4);
        for y in r.y..r.bottom() {
            let start = (y as usize * self.w as usize + r.x as usize) * 4;
            let end = start + r.w as usize * 4;
            out.extend_from_slice(&self.px[start..end]);
        }
        Frame::from_bgra(r.w as u32, r.h as u32, out).ok_or_else(|| "cropped frame invalid".into())
    }

    /// Number of pixels whose bytes differ — used by tests and diagnostics.
    pub fn diff_pixels(&self, other: &Frame) -> usize {
        if self.w != other.w || self.h != other.h || self.px.len() != other.px.len() {
            return usize::MAX;
        }
        self.px
            .chunks_exact(4)
            .zip(other.px.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count()
    }
}

#[inline]
fn edge(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (c.0 - a.0) * (b.1 - a.1) - (c.1 - a.1) * (b.0 - a.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_alloc_rejects_bad_dimensions() {
        assert!(Frame::new(0, 10).is_none());
        assert!(Frame::new(10, 0).is_none());
        assert!(Frame::new(40_000, 10).is_none());
        // Would overflow usize*4 on 32-bit, exceeds MAX_PIXELS on 64-bit.
        assert!(Frame::new(30_000, 30_000).is_none());
        let ok = Frame::new(3, 2);
        assert_eq!(ok.unwrap().px.len(), 24);
    }

    #[test]
    fn from_bgra_checks_length() {
        assert!(Frame::from_bgra(2, 2, vec![0u8; 16]).is_some());
        assert!(Frame::from_bgra(2, 2, vec![0u8; 15]).is_none());
    }

    #[test]
    fn rect_from_points_normalizes() {
        let r = IRect::from_points(10, 10, 4, 6);
        assert_eq!(r, IRect::new(4, 6, 7, 5));
        assert_eq!(r.right(), 11);
        assert_eq!(r.bottom(), 11);
        assert!(IRect::from_points(5, 5, 5, 5).contains(5, 5));
    }

    #[test]
    fn rect_intersect_and_clamp() {
        let a = IRect::new(0, 0, 10, 10);
        let b = IRect::new(8, 8, 10, 10);
        assert_eq!(a.intersect(&b), IRect::new(8, 8, 2, 2));
        assert!(a.intersect(&IRect::new(20, 20, 5, 5)).is_empty());
        let c = IRect::new(-5, -5, 8, 8);
        assert_eq!(c.clamp_to(100, 100), IRect::new(0, 0, 3, 3));
    }

    #[test]
    fn crop_is_bounds_checked_and_exact() {
        let mut f = Frame::new(4, 4).unwrap();
        for (i, v) in f.px.iter_mut().enumerate() {
            *v = (i % 251) as u8;
        }
        let c = f.crop(IRect::new(1, 1, 2, 2)).unwrap();
        assert_eq!((c.w, c.h), (2, 2));
        let expected_start = (1 * 4 + 1) * 4;
        assert_eq!(&c.px[0..4], &f.px[expected_start..expected_start + 4]);
        assert!(f.crop(IRect::new(3, 3, 5, 5)).is_ok()); // clamped
        assert!(f.crop(IRect::new(10, 10, 2, 2)).is_err());
    }

    #[test]
    fn blend_overwrites_opaque_and_respects_alpha() {
        let mut f = Frame::new(2, 1).unwrap();
        f.fill_rect(IRect::new(0, 0, 2, 1), Rgba::opaque(0, 0, 0));
        f.blend(0, 0, Rgba::opaque(255, 255, 255));
        assert_eq!(f.get(0, 0).unwrap(), [255, 255, 255, 255]);
        f.blend(1, 0, Rgba::new(255, 255, 255, 128));
        let p = f.get(1, 0).unwrap();
        // ~50% over black
        assert!((p[0] as i32 - 128).abs() <= 1, "{p:?}");
        assert_eq!(f.get(-1, 0), None);
    }

    #[test]
    fn line_and_rect_hit_expected_pixels() {
        let mut f = Frame::new(20, 20).unwrap();
        f.fill_rect(IRect::new(0, 0, 20, 20), Rgba::opaque(0, 0, 0));
        f.line(0, 10, 19, 10, Rgba::opaque(255, 0, 0), 3);
        assert_eq!(f.get(10, 10).unwrap()[2], 255); // red
        assert_eq!(f.get(10, 8).unwrap()[2], 0);
        f.fill_rect(IRect::new(2, 2, 4, 4), Rgba::opaque(0, 255, 0));
        assert_eq!(f.get(3, 3).unwrap()[1], 255);
        f.stroke_rect(IRect::new(5, 5, 6, 6), Rgba::opaque(0, 0, 255), 1);
        assert_eq!(f.get(5, 5).unwrap()[0], 255);
        assert_eq!(f.get(8, 8).unwrap()[0], 0); // interior untouched
    }

    #[test]
    fn arrow_draws_something_at_both_ends() {
        let mut f = Frame::new(50, 50).unwrap();
        f.arrow(5, 5, 40, 40, Rgba::opaque(255, 255, 255), 3);
        assert!(f.get(5, 5).unwrap()[0] > 0, "shaft start");
        // tip region must contain filled head pixels
        let head = f.crop(IRect::new(34, 34, 8, 8)).unwrap();
        let painted = head.px.chunks_exact(4).filter(|p| p[0] > 0).count();
        assert!(painted > 10, "arrow head pixels: {painted}");
    }
}
