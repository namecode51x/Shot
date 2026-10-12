//! Operating-system seam. The OS-free core reaches OS services through
//! this module; `windows.rs` re-exports the existing Win32 layer, while
//! `linux.rs` holds the X11 adapters (M1) and `macos.rs` the current stubs
//! (see `docs/CROSS_PLATFORM_PLAN.md`).

#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
#[path = "linux.rs"]
mod imp;

/// Pure helpers behind the X11 adapters. Compiled for tests and non-Windows
/// targets: the unit tests run in the regular suite on any host, while the
/// Windows release binary stays byte-identical to M0 (the module's items
/// would otherwise shift rustc's codegen-unit partitioning). The
/// wire-protocol side stays behind `cfg(target_os = "linux")` inside
/// `linux.rs` — including the thin `x11/eventmap.rs` glue that produces the
/// [`x11_logic::RawEvent`]s the tested mapping policy consumes.
#[cfg(any(test, not(windows)))]
#[path = "x11_logic.rs"]
pub mod x11_logic;

/// Rasterization of annotation text into a frame.
pub use imp::text;

/// X11 adapters (Linux only): capture, encode, clipboard, hotkey, dialogs,
/// display geometry, windowing primitives, and the M1b session driver.
#[cfg(target_os = "linux")]
pub use imp::{app, capture, clipboard, dialog, display, encode, hotkey, windowing};
