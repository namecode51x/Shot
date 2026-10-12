//! Windows platform layer: re-exports the existing Win32 implementation
//! unchanged. M0 only builds the seam; no Windows code moves.

pub mod text {
    pub use crate::text::draw_text;
}
