//! Image encoding/decoding through the Windows Imaging Component (WIC).
//!
//! WIC ships with Windows, so NameShot needs no third-party encoder crates and
//! no extra runtime. Only the **flattened** frame is ever handed to an encoder.

use crate::config::OutputFormat;
use crate::error::{Error, Result};
use crate::image::Frame;
use std::path::Path;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatBmp, GUID_ContainerFormatJpeg,
    GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, IWICBitmapDecoder,
    IWICBitmapFrameEncode, IWICBitmapSource, IWICFormatConverter, IWICImagingFactory, IWICStream,
    WICBitmapDitherType, WICBitmapEncoderNoCache, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::{
    CreateStreamOnHGlobal, GetHGlobalFromStream, IPropertyBag2,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IStream};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Variant::{VARIANT, VT_R4};
use windows::core::{GUID, PCWSTR};

fn container_guid(format: OutputFormat) -> GUID {
    match format {
        OutputFormat::Png => GUID_ContainerFormatPng,
        OutputFormat::Jpeg => GUID_ContainerFormatJpeg,
        OutputFormat::Bmp => GUID_ContainerFormatBmp,
    }
}

fn factory() -> Result<IWICImagingFactory> {
    // COM activation per Copy/Save is pure overhead on the UI thread, so the
    // factory lives for the process once created (same pattern as the app
    // fonts). Thread-local: WIC objects are apartment-bound and every encode
    // runs on the single UI thread.
    thread_local! {
        static WIC_FACTORY: std::cell::RefCell<Option<IWICImagingFactory>> =
            const { std::cell::RefCell::new(None) };
    }
    if let Some(fac) = WIC_FACTORY
        .try_with(|c| c.borrow().clone())
        .unwrap_or(None)
    {
        return Ok(fac);
    }
    let fac: IWICImagingFactory = unsafe {
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
    }
    .map_err(|e| Error::Win32(e).with_context("CoCreateInstance(WICImagingFactory)"))?;
    let _ = WIC_FACTORY.try_with(|c| *c.borrow_mut() = Some(fac.clone()));
    Ok(fac)
}

/// Encode `frame` into an already-initialized WIC stream.
fn encode_into(
    wicstream: &IStream,
    frame: &Frame,
    format: OutputFormat,
    quality: u8,
) -> Result<()> {
    if frame.w == 0 || frame.h == 0 {
        return Err(Error::invalid("cannot encode a zero-sized image"));
    }
    let fac = factory()?;
    let encoder = unsafe { fac.CreateEncoder(&container_guid(format), std::ptr::null()) }
        .map_err(|e| Error::Win32(e).with_context("WIC CreateEncoder"))?;
    unsafe {
        encoder
            .Initialize(wicstream, WICBitmapEncoderNoCache)
            .map_err(|e| Error::Win32(e).with_context("WIC encoder Initialize"))?
    };

    let mut frame_enc: Option<IWICBitmapFrameEncode> = None;
    let mut bag: Option<IPropertyBag2> = None;
    unsafe {
        encoder
            .CreateNewFrame(&mut frame_enc, &mut bag)
            .map_err(|e| Error::Win32(e).with_context("WIC CreateNewFrame"))?
    };
    let frame_enc = frame_enc.ok_or_else(|| Error::msg("WIC returned no frame encoder"))?;
    let bag = bag.ok_or_else(|| Error::msg("WIC returned no property bag"))?;

    // The frame must be initialized with the property bag from CreateNewFrame
    // before any other frame call; otherwise SetSize fails with
    // WINCODEC_ERR_PROPERTYNOTINITIALIZED (0x88982F0C) and the file stays empty.
    unsafe {
        frame_enc
            .Initialize(&bag)
            .map_err(|e| Error::Win32(e).with_context("WIC frame Initialize"))?
    };

    unsafe {
        frame_enc
            .SetSize(frame.w, frame.h)
            .map_err(|e| Error::Win32(e).with_context("WIC SetSize"))?
    };
    // Ask for 32bpp BGRA; WriteSource converts to whatever the container
    // needs (JPEG gets 24bpp, PNG keeps alpha), so the result stays correct
    // even if the encoder rewrote the pixel format.
    let mut pix_fmt = GUID_WICPixelFormat32bppBGRA;
    unsafe {
        frame_enc
            .SetPixelFormat(&mut pix_fmt)
            .map_err(|e| Error::Win32(e).with_context("WIC SetPixelFormat"))?
    };

    if format == OutputFormat::Jpeg {
        if let Err(e) = set_jpeg_quality(&bag, quality) {
            // Quality is a nice-to-have: fall back to the encoder default.
            let _ = e;
        }
    }

    let stride = frame
        .w
        .checked_mul(4)
        .ok_or_else(|| Error::invalid("stride overflow"))?;
    let bitmap = unsafe {
        fac.CreateBitmapFromMemory(
            frame.w,
            frame.h,
            &GUID_WICPixelFormat32bppBGRA,
            stride,
            &frame.px,
        )
    }
    .map_err(|e| Error::Win32(e).with_context("WIC CreateBitmapFromMemory"))?;

    unsafe {
        frame_enc
            .WriteSource(&bitmap, std::ptr::null())
            .map_err(|e| Error::Win32(e).with_context("WIC WriteSource"))?;
        frame_enc
            .Commit()
            .map_err(|e| Error::Win32(e).with_context("WIC frame Commit"))?;
        encoder
            .Commit()
            .map_err(|e| Error::Win32(e).with_context("WIC encoder Commit"))?;
    }
    Ok(())
}

fn set_jpeg_quality(bag: &IPropertyBag2, quality: u8) -> Result<()> {
    let q = quality.clamp(1, 100) as f32 / 100.0;
    unsafe {
        let mut v: VARIANT = std::mem::zeroed();
        (*v.Anonymous.Anonymous).vt = VT_R4;
        (*v.Anonymous.Anonymous).Anonymous.fltVal = q;

        let mut name: Vec<u16> = "ImageQuality"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut prop: windows::Win32::System::Com::StructuredStorage::PROPBAG2 = std::mem::zeroed();
        prop.dwType = 0; // PROPBAG2_TYPE_DATA
        prop.vt = VT_R4;
        prop.pstrName = windows::core::PWSTR(name.as_mut_ptr());
        bag.Write(1, &prop, &v)
            .map_err(|e| Error::Win32(e).with_context("WIC ImageQuality"))?
    };
    Ok(())
}

/// Encode to a file with WIC (create/truncate through the stream itself).
pub fn encode_to_file(path: &Path, frame: &Frame, format: OutputFormat, quality: u8) -> Result<()> {
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    if wide.len() < 2 {
        return Err(Error::invalid("empty file path"));
    }
    let fac = factory()?;
    let stream: IWICStream = unsafe {
        fac.CreateStream()
            .map_err(|e| Error::Win32(e).with_context("WIC CreateStream"))?
    };
    unsafe {
        stream
            .InitializeFromFilename(
                PCWSTR(wide.as_ptr()),
                windows::Win32::Foundation::GENERIC_WRITE.0,
            )
            .map_err(|e| Error::Win32(e).with_context("WIC InitializeFromFilename"))?
    };
    encode_into(&stream, frame, format, quality)?;
    Ok(())
}

/// Encode to PNG bytes in memory (used for the clipboard `PNG` format).
pub fn encode_png_bytes(frame: &Frame) -> Result<Vec<u8>> {
    unsafe {
        let stream = CreateStreamOnHGlobal(
            windows::Win32::Foundation::HGLOBAL(std::ptr::null_mut()),
            true,
        )
        .map_err(|e| Error::Win32(e).with_context("CreateStreamOnHGlobal"))?;
        // Any IStream will do for the encoder — the HGLOBAL stream is one, and
        // it lets us read the bytes back through GetHGlobalFromStream.
        encode_into(&stream, frame, OutputFormat::Png, 100)?;

        let hglobal = GetHGlobalFromStream(&stream)
            .map_err(|e| Error::Win32(e).with_context("GetHGlobalFromStream"))?;
        let len = GlobalSize(hglobal);
        if len == 0 {
            return Err(Error::msg("PNG buffer is empty"));
        }
        let p = GlobalLock(hglobal) as *const u8;
        if p.is_null() {
            return Err(Error::msg("GlobalLock failed for PNG buffer"));
        }
        let bytes = std::slice::from_raw_parts(p, len).to_vec();
        let _ = GlobalUnlock(hglobal);
        // `stream` (and with it the HGLOBAL) is released here.
        Ok(bytes)
    }
}

fn decoder_from_file(path: &Path) -> Result<IWICBitmapDecoder> {
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let fac = factory()?;
    unsafe {
        fac.CreateDecoderFromFilename(
            PCWSTR(wide.as_ptr()),
            None,
            windows::Win32::Foundation::GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
        .map_err(|e| Error::Win32(e).with_context("WIC CreateDecoderFromFilename"))
    }
}

fn decoder_from_bytes(bytes: &[u8]) -> Result<IWICBitmapDecoder> {
    let fac = factory()?;
    unsafe {
        // A real WIC stream (from the WIC factory) implements IWICStream and
        // can wrap caller memory directly; the HGLOBAL shell stream cannot.
        let wic: IWICStream = fac
            .CreateStream()
            .map_err(|e| Error::Win32(e).with_context("WIC CreateStream"))?;
        wic.InitializeFromMemory(bytes)
            .map_err(|e| Error::Win32(e).with_context("IWICStream::InitializeFromMemory"))?;
        fac.CreateDecoderFromStream(&wic, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .map_err(|e| Error::Win32(e).with_context("WIC CreateDecoderFromStream"))
    }
}

/// Decode from a file (used by tests and diagnostics).
pub fn decode_file(path: &Path) -> Result<Frame> {
    let decoder = decoder_from_file(path)?;
    decode_with_size(&decoder)
}

/// Decode from in-memory bytes (e.g. a PNG that went through the clipboard).
pub fn decode_bytes(bytes: &[u8]) -> Result<Frame> {
    let decoder = decoder_from_bytes(bytes)?;
    decode_with_size(&decoder)
}

fn decode_with_size(decoder: &IWICBitmapDecoder) -> Result<Frame> {
    let source: IWICBitmapSource = unsafe {
        decoder
            .GetFrame(0)
            .map_err(|e| Error::Win32(e).with_context("WIC GetFrame"))?
            .into()
    };
    let mut w = 0u32;
    let mut h = 0u32;
    unsafe {
        source
            .GetSize(&mut w, &mut h)
            .map_err(|e| Error::Win32(e).with_context("WIC GetSize"))?
    };
    if w == 0 || h == 0 {
        return Err(Error::invalid("decoded image has zero size"));
    }
    let conv: IWICFormatConverter = unsafe {
        factory()?
            .CreateFormatConverter()
            .map_err(|e| Error::Win32(e).with_context("WIC CreateFormatConverter"))?
    };
    unsafe {
        conv.Initialize(
            &source,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherType(0),
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .map_err(|e| Error::Win32(e).with_context("WIC converter Initialize"))?
    };
    let stride = (w as usize)
        .checked_mul(4)
        .ok_or_else(|| Error::invalid("stride overflow"))?;
    let len = stride
        .checked_mul(h as usize)
        .ok_or_else(|| Error::invalid("size overflow"))?;
    let mut px = vec![0u8; len];
    unsafe {
        conv.CopyPixels(std::ptr::null(), stride as u32, &mut px)
            .map_err(|e| Error::Win32(e).with_context("WIC CopyPixels"))?
    };
    Frame::from_bgra(w, h, px).ok_or_else(|| Error::invalid("decoded frame invalid"))
}
