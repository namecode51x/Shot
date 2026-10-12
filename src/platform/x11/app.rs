//! M1b session driver: the X11 event loop that stitches the M1 adapters
//! (capture, windowing, clipboard pump, dialogs, encoding) together into
//! the selecting phase of a capture session — grab once, veil the screen,
//! drag a band, release to commit, Escape to cancel, then save through a
//! zenity dialog.
//!
//! Honest status: **compiled, never executed.** No X11 session exists on
//! the build host (no WSL, no X server, no way to launch an ELF binary —
//! see `docs/BUILD_LOG.md` "M1b" for the exact checks), so this driver has
//! never run and no runtime support is claimed. What *is* tested is the
//! decision core it drives: [`crate::platform::x11_logic::SelectSession`]
//! and [`crate::platform::x11_logic::eventmap`] run in the regular unit
//! suite on every host.
//!
//! Deliberately out of scope for M1b (event-loop phase only): move/resize
//! handles, editing/annotation, hotkey registration, clipboard-after-save,
//! and multi-monitor capture per monitor. Each is called out in
//! `docs/CROSS_PLATFORM_PLAN.md`.

use super::capture;
use super::dialog;
use super::encode;
use super::eventmap;
use super::hotkey;
use super::windowing::App;
use crate::config::OutputFormat;
use crate::error::{Error, Result};
use crate::hotkey::vk;
use crate::image::{Frame, IRect};
use crate::platform::x11_logic::{self, SelectSession, SessOutcome};
use x11rb::connection::Connection;

/// Session entry point with the same exit-code contract as the Windows
/// `run()`: `0` = the user finished (saved, cancelled the picker, or
/// pressed Escape), `1` = something failed (reported on stderr).
pub fn run() -> i32 {
    match session() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("nameshot: X11 session failed: {e}");
            1
        }
    }
}

fn session() -> Result<i32> {
    let mut app = App::connect()?;
    let shot = capture::grab_virtual_screen(&app.conn, app.screen)?;
    let vs = shot.virtual_screen;
    let crisp: Frame = shot.frame;
    let veil = x11_logic::veil_frame(&crisp);
    // Everything below works in frame coordinates (0,0 = top-left of the
    // captured virtual screen); `eventmap` shifts root coordinates over.
    let bounds = IRect::new(0, 0, crisp.w as i32, crisp.h as i32);

    let overlay = app.create_overlay(vs.x, vs.y, vs.w as u32, vs.h as u32)?;
    app.put_frame(overlay, &x11_logic::compose_select(&veil, &crisp, bounds))?;
    app.set_crosshair(overlay)?;
    app.set_input_focus(overlay)?;
    app.grab_pointer()?;

    let keys = resolve_keys(&app.conn);
    let mut sess = SelectSession::new(bounds);
    loop {
        let ev = app.next_event()?;
        if app.pump_clipboard(&ev)? {
            continue;
        }
        let Some(raw) = eventmap::extract(&ev) else {
            continue;
        };
        let Some(input) = x11_logic::to_session(&raw, keys, vs.x, vs.y) else {
            continue;
        };
        match sess.handle(input) {
            SessOutcome::None => {}
            SessOutcome::Redraw(sel) => {
                app.put_frame(overlay, &x11_logic::compose_select(&veil, &crisp, sel))?;
            }
            SessOutcome::Commit(sel) => {
                app.ungrab_pointer();
                return commit(&crisp, sel);
            }
            SessOutcome::Cancelled => {
                app.ungrab_pointer();
                return Ok(0);
            }
        }
    }
}

/// Escape/Enter keycodes from the server keymap. `None` = the keyboard
/// layout does not map them (or the query failed): the session still runs
/// on pointer input alone, which is the honest degradation — the driver
/// never invents a keycode.
fn resolve_keys<C: Connection>(conn: &C) -> Option<(u8, u8)> {
    let escape = x11_logic::x_keysym_for_vk(vk::ESCAPE)?;
    let confirm = x11_logic::x_keysym_for_vk(vk::RETURN)?;
    Some((
        hotkey::keycode_for_keysym(conn, escape).ok()?,
        hotkey::keycode_for_keysym(conn, confirm).ok()?,
    ))
}

/// Commit path: crop the band and offer it to the user. Cancelling the
/// picker is a user decision, not an error (exit 0 either way).
fn commit(crisp: &Frame, sel: IRect) -> Result<i32> {
    let cropped = crisp.crop(sel).map_err(Error::invalid)?;
    let default = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("nameshot.png");
    let Some(path) = dialog::save_as("Save screenshot", &default)? else {
        return Ok(0);
    };
    encode::encode_to_file(&path, &cropped, OutputFormat::Png, 100)?;
    Ok(0)
}
