//! Linux platform layer.
//!
//! On `target_os = "linux"` this dispatches to the X11 adapter set in
//! [`x11`] (M1: compile-checked only — no X11 session exists on the build
//! host, so nothing here has run and no runtime support is claimed; see
//! `docs/CROSS_PLATFORM_PLAN.md`). Any other non-Windows, non-macOS target
//! keeps the M0 text stub.

#[cfg(target_os = "linux")]
#[path = "x11/mod.rs"]
mod x11;

#[cfg(target_os = "linux")]
pub use x11::*;

#[cfg(not(target_os = "linux"))]
pub mod text {
    use crate::image::{Frame, Rgba};

    /// Not yet implemented on this target (M1 covers Linux/X11 only).
    pub fn draw_text(_img: &mut Frame, _at: (i32, i32), _text: &str, _color: Rgba, _size: u32) {
        unimplemented!("portable text rasterization is only implemented for Linux/X11 (M1)")
    }
}
