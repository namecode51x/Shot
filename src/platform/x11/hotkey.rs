//! Global hotkey registration through passive `XGrabKey` grabs on the app's
//! hidden window — the X11 counterpart of `RegisterHotKey`, with the same
//! policy as the Win32 adapter: registration happens once at startup and on
//! explicit settings changes, never in a polling loop; conflicts are
//! reported, not fought over.
//!
//! Keycode resolution depends on the server's current keymap (the keysym →
//! keycode step is per-layout), and each grab is taken four times — bare,
//! CapsLock, NumLock, both — so lock keys never mute the hotkey.

use crate::error::{Error, Result};
use crate::hotkey::HotkeySpec;
use crate::platform::x11_logic;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask, Window};

fn keycode_for<C: Connection>(conn: &C, spec: &HotkeySpec) -> Result<u8> {
    spec.validate().map_err(|e| Error::invalid(e.message()))?;
    let keysym = x11_logic::x_keysym_for_vk(spec.vk)
        .ok_or_else(|| Error::invalid("that key cannot be a global X11 hotkey"))?;
    keycode_for_keysym(conn, keysym)
}

/// First keycode whose keymap entry produces `keysym`. X11 has no
/// keysym→keycode request: fetch the server's keymap once and scan it
/// (first/last keycode come from the connection setup). Shared with the
/// M1b session driver, which resolves Escape/Enter this way.
pub fn keycode_for_keysym<C: Connection>(conn: &C, keysym: u32) -> Result<u8> {
    let setup = conn.setup();
    let min = setup.min_keycode;
    let max = setup.max_keycode;
    let count = (u16::from(max).saturating_sub(u16::from(min)) + 1).min(255) as u8;
    let reply = conn
        .get_keyboard_mapping(min, count)
        .map_err(|e| Error::msg(format!("GetKeyboardMapping: {e}")))?
        .reply()
        .map_err(|e| Error::msg(format!("GetKeyboardMapping: {e}")))?;
    let per = reply.keysyms_per_keycode as usize;
    for (i, &ks) in reply.keysyms.iter().enumerate() {
        if ks == keysym {
            let Some(offset) = i.checked_div(per) else {
                break;
            };
            if offset < count as usize {
                return Ok(min + offset as u8);
            }
        }
    }
    Err(Error::invalid(
        "the key is not mapped by the current keyboard layout",
    ))
}

/// Grab `spec` on `window` (the hidden app window). A `BadAccess` reply
/// means another client already owns the combination — surfaced through
/// [`is_conflict`] exactly like Win32 error 1409.
pub fn register<C: Connection>(conn: &C, window: Window, spec: &HotkeySpec) -> Result<()> {
    let keycode = keycode_for(conn, spec)?;
    let base = x11_logic::x_modifier_mask(spec.modifiers);
    for mods in x11_logic::grab_variant_modifiers(base) {
        let mask = ModMask::from(mods as u16);
        let cookie = conn
            .grab_key(
                false,
                window,
                mask,
                keycode,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .map_err(|e| Error::msg(format!("XGrabKey: {e}")))?;
        cookie.check().map_err(|e| {
            let msg = format!("XGrabKey: {e}");
            if msg.contains("BadAccess") {
                Error::msg(format!(
                    "{msg} (key already grabbed by another application)"
                ))
            } else {
                Error::msg(msg)
            }
        })?;
    }
    Ok(())
}

/// Release every variant taken by [`register`]. Best-effort: an already
/// gone grab is not an error (mirrors `UnregisterHotKey`'s ignore-failure).
pub fn unregister<C: Connection>(conn: &C, window: Window, spec: &HotkeySpec) {
    let Ok(keycode) = keycode_for(conn, spec) else {
        return;
    };
    let base = x11_logic::x_modifier_mask(spec.modifiers);
    for mods in x11_logic::grab_variant_modifiers(base) {
        // Best-effort like `UnregisterHotKey`: a gone grab is not an error.
        let _ = conn.ungrab_key(keycode, window, ModMask::from(mods as u16));
    }
}

/// True when the failure means "someone else owns this key".
pub fn is_conflict(err: &Error) -> bool {
    x11_logic::grab_error_is_conflict(err)
}
