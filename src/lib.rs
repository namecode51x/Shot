//! NameShot — a local-first, lightweight regional screenshot tool for Windows.
//!
//! Crate layout:
//! - OS-free core: [`image`], [`annotate`], [`flatten`], [`config`],
//!   [`hotkey`], [`output`], [`session`], [`strings`], [`error`], [`theme`],
//!   [`platform`]
//! - Windows layer: [`win`], [`capture`], [`encode`], [`text`], [`os`]
//! - UI: [`app`], [`overlay`], [`editor`]
//!
//! Windows-only modules are `#[cfg(windows)]`. On other targets `overlay`
//! resolves to its OS-free layout planner so the core keeps compiling for
//! the port in progress (`docs/CROSS_PLATFORM_PLAN.md`).

// Win32 window-procedure callbacks are `unsafe fn` by ABI contract and their
// bodies are entirely unsafe (every line is a Win32 call), so wrapping each
// operation in its own `unsafe {}` would add noise without adding information.
#![allow(unsafe_op_in_unsafe_fn)]

pub mod annotate;
#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod capture;
pub mod config;
#[cfg(windows)]
pub mod editor;
#[cfg(windows)]
pub mod encode;
pub mod error;
pub mod flatten;
pub mod hotkey;
pub mod image;
#[cfg(windows)]
pub mod os;
pub mod output;
#[cfg(windows)]
pub mod overlay;
#[cfg(not(windows))]
#[path = "overlay/layout.rs"]
pub mod overlay;
pub mod platform;
pub mod session;
#[cfg(windows)]
pub mod settings_ui;
pub mod strings;
#[cfg(windows)]
pub mod text;
pub mod theme;
#[cfg(windows)]
pub mod win;

/// Program entry: returns the process exit code.
#[cfg(windows)]
pub fn run() -> i32 {
    app::run()
}

/// Linux entry (M1b): the X11 session driver. Compile-verified only — no
/// X11 session exists on the build host, so it has never run here and no
/// runtime support is claimed (`docs/BUILD_LOG.md` "M1b").
#[cfg(target_os = "linux")]
pub fn run() -> i32 {
    platform::app::run()
}

/// Not ported (macOS and other targets): M1 shipped Linux/X11 adapters and
/// M1b their event loop; neither has ever run on a server
/// (`docs/CROSS_PLATFORM_PLAN.md`).
#[cfg(not(any(windows, target_os = "linux")))]
pub fn run() -> i32 {
    eprintln!(
        "nameshot: UI not ported on this platform yet (M1 added Linux/X11 adapters only; see docs/CROSS_PLATFORM_PLAN.md)"
    );
    1
}
