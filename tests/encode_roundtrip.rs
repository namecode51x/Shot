//! Round-trip tests for WIC encode/decode.
//!
//! These exercise the exact code path the editor's Save command uses
//! (`encode_to_file`) and the clipboard PNG path (`encode_png_bytes`).
//!
//! WIC/COM only — compiled on Windows targets; the Linux build has its own
//! encoder behind `cfg(target_os = "linux")` (M1) with no round-trip suite
//! until a runtime exists.
#![cfg(windows)]

use nameshot::config::OutputFormat;
use nameshot::encode;
use nameshot::image::Frame;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

/// WIC requires COM to be initialized on the calling thread.
fn init_com() {
    unsafe {
        // S_FALSE when already initialized; RPC_E_CHANGED_MODE only if the
        // thread is already in a different apartment — both are fine here.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

fn sample_frame() -> Frame {
    let (w, h) = (64u32, 48u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.push(x as u8);
            px.push(y as u8);
            px.push(128);
            px.push(255);
        }
    }
    Frame::from_bgra(w, h, px).expect("valid sample frame")
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("nameshot_encode_test");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn encode_to_file_writes_nonempty_png() {
    init_com();
    let path = temp_path("roundtrip.png");
    let frame = sample_frame();
    encode::encode_to_file(&path, &frame, OutputFormat::Png, 100).expect("encode_to_file");
    let len = std::fs::metadata(&path).expect("file exists").len();
    assert!(len > 0, "PNG file is empty");
    let back = encode::decode_file(&path).expect("decode_file");
    assert_eq!(back.w, 64);
    assert_eq!(back.h, 48);
    assert_eq!(back.px, frame.px);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn encode_to_file_writes_nonempty_jpeg() {
    init_com();
    let path = temp_path("roundtrip.jpg");
    let frame = sample_frame();
    encode::encode_to_file(&path, &frame, OutputFormat::Jpeg, 85).expect("encode_to_file");
    let len = std::fs::metadata(&path).expect("file exists").len();
    assert!(len > 0, "JPEG file is empty");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn encode_to_file_writes_nonempty_bmp() {
    init_com();
    let path = temp_path("roundtrip.bmp");
    let frame = sample_frame();
    encode::encode_to_file(&path, &frame, OutputFormat::Bmp, 100).expect("encode_to_file");
    let len = std::fs::metadata(&path).expect("file exists").len();
    assert!(len > 0, "BMP file is empty");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn encode_png_bytes_round_trips() {
    init_com();
    let frame = sample_frame();
    let bytes = encode::encode_png_bytes(&frame).expect("encode_png_bytes");
    assert!(!bytes.is_empty(), "PNG buffer is empty");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG signature");
    let back = encode::decode_bytes(&bytes).expect("decode_bytes");
    assert_eq!(back.w, 64);
    assert_eq!(back.h, 48);
    assert_eq!(back.px, frame.px);
}
