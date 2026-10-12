//! macOS platform layer. M0 ships compiling stubs only; the portable text
//! rasterizer arrives with M1 (`docs/CROSS_PLATFORM_PLAN.md`).

pub mod text {
    use crate::image::{Frame, Rgba};

    /// Not yet implemented on macOS (M1).
    pub fn draw_text(_img: &mut Frame, _at: (i32, i32), _text: &str, _color: Rgba, _size: u32) {
        unimplemented!("portable text rasterization arrives in M1")
    }
}
