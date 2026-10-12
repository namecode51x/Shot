//! One-shot desktop capture of the whole virtual screen.
//!
//! Strategy: a single GDI `BitBlt` from the desktop DC into a 32-bit top-down
//! DIB section. This covers multi-monitor layouts (including negative monitor
//! origins) in one call, needs no D3D device, no runtime service and no
//! repeated polling — the frame is grabbed once when a capture session starts
//! and then treated as immutable.
//!
//! Known limitations (documented in the README): content that is protected
//! (DRM video, secure desktop / UAC / login screens) cannot be captured from a
//! normal process and may appear black or be missing; those surfaces are not
//! an error condition and never crash the app.

use crate::error::{Error, Result};
use crate::image::Frame;
use crate::os::{self, VirtualScreen};
use crate::win::gdi::{MemDc, ScreenDc, SelectedObject};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateDIBSection, DIB_RGB_COLORS, ROP_CODE,
    SRCCOPY,
};

pub struct Capture {
    /// Physical pixels of the whole virtual desktop.
    pub frame: Frame,
    /// Origin/size of the virtual desktop in screen coordinates.
    pub virtual_screen: VirtualScreen,
}

/// Capture the virtual screen exactly once.
pub fn capture_virtual_screen() -> Result<Capture> {
    let vs = os::virtual_screen();
    if !vs.is_valid() {
        return Err(Error::invalid(
            "virtual screen has no pixels (headless session?)",
        ));
    }
    if vs.w > crate::image::MAX_DIM || vs.h > crate::image::MAX_DIM {
        return Err(Error::invalid(format!(
            "virtual screen {}x{} exceeds the supported maximum",
            vs.w, vs.h
        )));
    }

    let screen = ScreenDc::desktop().ok_or_else(|| Error::msg("GetDC(NULL) failed"))?;
    let mem = MemDc::new().ok_or_else(|| Error::msg("CreateCompatibleDC failed"))?;

    let w = vs.w;
    let h = vs.h;
    let stride = (w as usize)
        .checked_mul(4)
        .ok_or_else(|| Error::invalid("stride overflow"))?;
    let size_image = stride
        .checked_mul(h as usize)
        .ok_or_else(|| Error::invalid("image size overflow"))?;

    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            // Negative height => top-down, matching our Frame layout.
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: windows::Win32::Graphics::Gdi::BI_RGB.0,
            biSizeImage: size_image as u32,
            ..unsafe { std::mem::zeroed() }
        },
        bmiColors: [unsafe { std::mem::zeroed() }],
    };

    let dib =
        unsafe { CreateDIBSection(Some(screen.hdc()), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) }
            .map_err(|e| Error::Win32(e).with_context("CreateDIBSection"))?;

    if bits.is_null() {
        unsafe {
            use windows::Win32::Graphics::Gdi::DeleteObject;
            let _ = DeleteObject(dib.into());
        }
        return Err(Error::msg("CreateDIBSection returned a null pointer"));
    }

    // Copy out of the DIB section before it is deleted: one allocation,
    // one memcpy straight into the frame buffer.
    let mut px = Vec::with_capacity(size_image);
    {
        let _selected = unsafe { SelectedObject::select(mem.0, dib.into()) }
            .ok_or_else(|| Error::msg("SelectObject failed"))?;
        // SRCCOPY | CAPTUREBLT: includes layered windows on the desktop.
        let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
        unsafe { BitBlt(mem.0, 0, 0, w, h, Some(screen.hdc()), vs.x, vs.y, rop) }
            .map_err(|e| Error::Win32(e).with_context("BitBlt"))?;

        unsafe {
            let src = std::slice::from_raw_parts(bits as *const u8, size_image);
            let dst = px.as_mut_ptr();
            std::ptr::copy_nonoverlapping(src.as_ptr(), dst, size_image);
            px.set_len(size_image);
        }
        // SelectedObject::drop restores the previous object and deletes `dib`.
    }

    // The display driver leaves the alpha channel undefined (usually 0).
    // Force opaque pixels so PNG export and blending behave predictably.
    // One 32-bit OR per pixel (BGRA in memory is 0xAARRGGBB little-endian,
    // so the top byte is alpha): 4x fewer stores than the byte loop, and it
    // auto-vectorizes. Length is a multiple of 4 by construction.
    let words = px.len() / 4;
    let up = px.as_mut_ptr() as *mut u32;
    for i in 0..words {
        unsafe {
            *up.add(i) |= 0xFF00_0000;
        }
    }

    let frame = Frame::from_bgra(w as u32, h as u32, px)
        .ok_or_else(|| Error::msg("captured frame failed validation"))?;

    Ok(Capture {
        frame,
        virtual_screen: vs,
    })
}
