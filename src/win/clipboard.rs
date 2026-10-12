//! Clipboard integration: puts the **flattened** image on the Windows
//! clipboard as `CF_DIB` (universal) plus a registered `PNG` format (keeps
//! alpha and is honored by modern apps). Ownership rules are explicit: the
//! global block is handed to the clipboard only after `SetClipboardData`
//! succeeds, otherwise it is freed.

use crate::encode::encode_png_bytes;
use crate::error::{Error, Result};
use crate::image::Frame;
use crate::output::build_dib;
use crate::win::gdi::GlobalMem;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Ole::CF_DIB;

/// `PNG` is a registered clipboard format; the id is stable per session.
pub fn png_clipboard_format() -> Result<u32> {
    unsafe { Ok(RegisterClipboardFormatW(w!("PNG"))) }
}
use windows::core::w;

struct ClipGuard;

impl Drop for ClipGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn set_block(format: u32, bytes: &[u8]) -> Result<()> {
    let mut mem = GlobalMem::alloc(bytes.len())
        .ok_or_else(|| Error::msg("GlobalAlloc failed for clipboard data"))?;
    if !mem.write(bytes) {
        return Err(Error::msg("could not write clipboard data"));
    }
    unsafe {
        SetClipboardData(format, Some(HANDLE(mem.raw().0)))
            .map_err(|e| Error::Win32(e).with_context("SetClipboardData"))?;
    }
    // Ownership transferred to the system.
    mem.leak();
    Ok(())
}

/// Copy `image` (already flattened and redacted) to the clipboard.
pub fn copy_image(hwnd: Option<windows::Win32::Foundation::HWND>, image: &Frame) -> Result<()> {
    let dib = build_dib(image)?;
    unsafe {
        OpenClipboard(hwnd).map_err(|e| Error::Win32(e).with_context("OpenClipboard"))?;
    }
    let _guard = ClipGuard;
    unsafe {
        EmptyClipboard().map_err(|e| Error::Win32(e).with_context("EmptyClipboard"))?;
    }
    set_block(CF_DIB.0 as u32, &dib)?;

    // Best effort: PNG keeps transparency; apps that do not know the format
    // still have CF_DIB above.
    if let Ok(bytes) = encode_png_bytes(image) {
        if let Ok(fmt) = png_clipboard_format() {
            let _ = set_block(fmt, &bytes);
        }
    }
    Ok(())
}

/// Copy a plain local file path (for "Copy local file path").
pub fn copy_text(hwnd: Option<windows::Win32::Foundation::HWND>, text: &str) -> Result<()> {
    let mut wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    if wide.len() < 2 {
        wide.push(0);
    }
    let bytes_len = wide.len() * 2;
    let mut mem = GlobalMem::alloc(bytes_len)
        .ok_or_else(|| Error::msg("GlobalAlloc failed for clipboard text"))?;
    if !mem.write(unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, bytes_len) }) {
        return Err(Error::msg("could not write clipboard text"));
    }
    unsafe {
        OpenClipboard(hwnd).map_err(|e| Error::Win32(e).with_context("OpenClipboard"))?;
    }
    let _guard = ClipGuard;
    unsafe {
        EmptyClipboard().map_err(|e| Error::Win32(e).with_context("EmptyClipboard"))?;
    }
    unsafe {
        SetClipboardData(
            windows::Win32::System::Ole::CF_UNICODETEXT.0 as u32,
            Some(HANDLE(mem.raw().0)),
        )
        .map_err(|e| Error::Win32(e).with_context("SetClipboardData"))?;
    }
    mem.leak();
    Ok(())
}
