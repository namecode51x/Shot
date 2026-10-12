//! Linux/X11 platform adapters (M1) plus the M1b session driver.
//!
//! Honest status: everything in this module compiles for
//! `x86_64-unknown-linux-gnu` but has **never been executed** — the build
//! host has no X11 session, no container runtime and no WSL, so no Linux
//! binary can run here. No X11 runtime support is claimed; the server-free
//! parts of these adapters (pixel conversion, geometry, keysym tables,
//! dialog argv, the M1b selection session and event-mapping policy) live in
//! [`crate::platform::x11_logic`] and are unit-tested by the regular
//! suite; `eventmap` here is only the field-extraction glue. See
//! `docs/CROSS_PLATFORM_PLAN.md` ("M1 result", "M1b result").

pub mod app;
pub mod capture;
pub mod clipboard;
pub mod dialog;
pub mod display;
pub mod encode;
pub mod eventmap;
pub mod hotkey;
pub mod text;
pub mod windowing;
