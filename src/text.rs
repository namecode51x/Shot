//! Text annotation rasterization. Text is rendered by GDI (ClearType-capable
//! system fonts) into a scratch DIB, converted to coverage, and blended into
//! the frame with the chosen color — so text ends up **inside** the flattened
//! bitmap like every other annotation, never as an overlay layer.

use crate::image::{Frame, Rgba};
use crate::win::gdi::{MemDc, SelectedObject};
use std::cell::RefCell;
use std::collections::HashMap;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    ANSI_CHARSET, ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS,
    CreateDIBSection, CreateFontW, DIB_RGB_COLORS, DT_CALCRECT, DT_NOCLIP, DT_NOPREFIX,
    DT_SINGLELINE, DrawTextW, FW_NORMAL, HFONT, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::core::w;

thread_local! {
    /// Process-lifetime HFONTs keyed by pixel height (UI thread only, like the
    /// app fonts). Every undo/redo replay re-rasters each text shape, and font
    /// creation dominates small-text cost — so fonts are created once and
    /// never deleted, exactly like the startup fonts.
    static FONT_CACHE: RefCell<HashMap<i32, HFONT>> = RefCell::new(HashMap::new());
}

/// Process-lifetime HFONT for `size_px`, created on first use.
fn cached_font(size_px: i32) -> Option<HFONT> {
    FONT_CACHE
        .try_with(|cache| {
            let mut cache = cache.borrow_mut();
            if let Some(&font) = cache.get(&size_px) {
                return Some(font);
            }
            let font = unsafe {
                CreateFontW(
                    -size_px,
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    ANSI_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    ANTIALIASED_QUALITY,
                    0,
                    w!("Segoe UI"),
                )
            };
            if font.is_invalid() {
                return None;
            }
            cache.insert(size_px, font);
            Some(font)
        })
        .unwrap_or(None)
}

/// Draw `text` with its top-left anchor at `at` (image coordinates).
/// Out-of-frame pixels are clipped by the blend operation.
pub fn draw_text(img: &mut Frame, at: (i32, i32), text: &str, color: Rgba, size: u32) {
    if text.is_empty() || size == 0 {
        return;
    }
    let size_px = size.clamp(6, 400) as i32;
    let Some(mem) = MemDc::new() else { return };

    let mut wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    if wide.len() > 4096 {
        wide.truncate(4096);
        wide.push(0);
    }

    unsafe {
        // 1. Measure with DT_CALCRECT (rect width/height become the extent).
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: 32_000,
            bottom: size_px * 3,
        };
        DrawTextW(
            mem.0,
            &mut wide,
            &mut rc,
            DT_SINGLELINE | DT_CALCRECT | DT_NOPREFIX,
        );
        let text_w = (rc.right - rc.left).clamp(1, 30_000);
        let text_h = (rc.bottom - rc.top).clamp(1, size_px * 4);

        // 2. Scratch DIB, top-down 32bpp, zero-filled.
        let dw = (text_w + 4) as u32;
        let dh = (text_h + 4) as u32;
        let stride = (dw as usize) * 4;
        let total = stride * dh as usize;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: dw as i32,
                biHeight: -(dh as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: total as u32,
                ..std::mem::zeroed()
            },
            bmiColors: [std::mem::zeroed()],
        };
        let dib = match CreateDIBSection(Some(mem.0), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(_) => return,
        };
        if bits.is_null() {
            use windows::Win32::Graphics::Gdi::DeleteObject;
            let _ = DeleteObject(dib.into());
            return;
        }
        std::ptr::write_bytes(bits as *mut u8, 0, total);
        let sel_bmp = match SelectedObject::select(mem.0, dib.into()) {
            Some(s) => s,
            None => {
                use windows::Win32::Graphics::Gdi::DeleteObject;
                let _ = DeleteObject(dib.into());
                return;
            }
        };

        // 3. Font + transparent background, white text => coverage map.
        // Cached process-lifetime font: selected raw (never through the
        // deleting guard) and restored afterwards, never deleted.
        let Some(font) = cached_font(size_px) else {
            drop(sel_bmp);
            return;
        };
        let prev_font = SelectObject(mem.0, font.into());
        if prev_font.is_invalid() {
            drop(sel_bmp);
            return;
        }

        SetBkMode(mem.0, TRANSPARENT);
        SetTextColor(mem.0, windows::Win32::Foundation::COLORREF(0x00FF_FFFF));

        let mut rc_draw = RECT {
            left: 2,
            top: 2,
            right: 2 + text_w,
            bottom: 2 + text_h,
        };
        DrawTextW(
            mem.0,
            &mut wide,
            &mut rc_draw,
            DT_SINGLELINE | DT_NOCLIP | DT_NOPREFIX,
        );

        // 4. Blend coverage into the frame with the annotation color.
        let src = std::slice::from_raw_parts(bits as *const u8, total);
        for y in 0..dh as i32 {
            let row = y as usize * stride;
            for x in 0..dw as i32 {
                let i = row + x as usize * 4;
                let coverage = {
                    let b = src[i] as u32;
                    let g = src[i + 1] as u32;
                    let r = src[i + 2] as u32;
                    ((b + g + r) / 3) as u8
                };
                if coverage == 0 {
                    continue;
                }
                img.blend(
                    at.0 + x,
                    at.1 + y,
                    Rgba {
                        r: color.r,
                        g: color.g,
                        b: color.b,
                        a: coverage,
                    },
                );
            }
        }

        SelectObject(mem.0, prev_font);
        drop(sel_bmp);
    }
}
