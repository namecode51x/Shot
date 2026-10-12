//! Portable greyscale text rasterization (plan §6.1): `ab_glyph` outlines
//! blended into the frame with the annotation colour.
//!
//! Deliberate deltas vs the Windows GDI path, all unvalidated until an X11
//! session exists (plan §11.3 quality spike): greyscale AA instead of
//! ClearType, system fonts discovered by scanning standard directories
//! instead of Segoe UI/fontconfig, and a slightly different top-anchoring
//! metric. Like the Windows path this returns silently when no font is
//! available — a missing font never crashes the app.

use crate::image::{Frame, Rgba};
use crate::platform::x11_logic;
use ab_glyph::{Font as _, ScaleFont as _};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Directories scanned (recursively, depth-capped) for the default font at
/// first use — startup stays free of font work.
const FONT_DIRS: [&str; 2] = ["/usr/share/fonts", "/usr/local/share/fonts"];

static FONT_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

fn scan_fonts(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_fonts(&path, out, depth + 1);
        } else if x11_logic::is_font_file(&path) {
            out.push(path);
        }
    }
}

fn discover_font() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for dir in FONT_DIRS {
        scan_fonts(Path::new(dir), &mut candidates, 0);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        scan_fonts(&home.join(".fonts"), &mut candidates, 0);
        scan_fonts(&home.join(".local/share/fonts"), &mut candidates, 0);
    }
    x11_logic::pick_font_path(&candidates)
}

/// Draw `text` with its top-left anchor at `at` (image coordinates).
/// Out-of-frame pixels are clipped by the blend operation, same as GDI.
pub fn draw_text(img: &mut Frame, at: (i32, i32), text: &str, color: Rgba, size: u32) {
    if text.is_empty() || size == 0 {
        return;
    }
    let size_px = size.clamp(6, 400);
    let Some(path) = FONT_PATH.get_or_init(discover_font).clone() else {
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let Ok(font) = ab_glyph::FontArc::try_from_vec(bytes) else {
        return;
    };

    let scale = ab_glyph::PxScale::from(size_px as f32);
    let scaled = font.as_scaled(scale);
    // Top anchor ≈ GDI's DrawText rect: baseline one ascent below the top.
    let baseline = scaled.ascent();
    let mut pen_x = 0.0f32;
    for ch in text.chars().take(4096) {
        let glyph_id = font.glyph_id(ch);
        let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(pen_x, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let a = (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
                if a > 0 {
                    img.blend(
                        at.0 + bounds.min.x as i32 + gx as i32,
                        at.1 + bounds.min.y as i32 + gy as i32,
                        Rgba {
                            r: color.r,
                            g: color.g,
                            b: color.b,
                            a,
                        },
                    );
                }
            });
        }
        pen_x += scaled.h_advance(glyph_id);
    }
}
