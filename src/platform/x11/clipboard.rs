//! `CLIPBOARD` selection ownership and transfer — the X11 clipboard
//! protocol (ICCCM).
//!
//! Ownership model mirrors Windows `SetClipboardData`: `copy_*` stores the
//! payload locally and takes selection ownership; the bytes only leave the
//! process when another client asks, via `SelectionRequest` events that
//! [`Clipboard::handle_event`] serves from the normal event loop. Payloads
//! are small (a PNG of the selection), so transfers are synchronous and the
//! idle model (block in `XNextEvent`, zero CPU) is unaffected.
//!
//! Targets served in M1: `TARGETS`, `UTF8_STRING` (text) and `image/png`
//! (images). `STRING` (latin-1) and DIB-equivalents are not offered —
//! receivers get UTF-8 or PNG, both accepted everywhere modern.

use super::encode::encode_png_bytes;
use crate::error::{Error, Result};
use crate::image::Frame;
use std::collections::HashMap;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, EventMask, PropMode, SELECTION_NOTIFY_EVENT,
    SelectionNotifyEvent, Window,
};
use x11rb::wrapper::ConnectionExt as _;

pub struct Clipboard {
    window: Window,
    selection: Atom,
    targets_atom: Atom,
    utf8_atom: Atom,
    image_png_atom: Atom,
    /// Current payload: target atom → bytes.
    data: HashMap<Atom, Vec<u8>>,
}

impl Clipboard {
    /// Intern the atoms this clipboard speaks. Called once per app window.
    pub fn new<C: Connection>(conn: &C, window: Window) -> Result<Clipboard> {
        let intern = |name: &'static str| -> Result<Atom> {
            conn.intern_atom(false, name.as_bytes())
                .map_err(|e| Error::msg(format!("InternAtom({name}): {e}")))?
                .reply()
                .map(|r| r.atom)
                .map_err(|e| Error::msg(format!("InternAtom({name}): {e}")))
        };
        Ok(Clipboard {
            window,
            selection: intern("CLIPBOARD")?,
            targets_atom: intern("TARGETS")?,
            utf8_atom: intern("UTF8_STRING")?,
            image_png_atom: intern("image/png")?,
            data: HashMap::new(),
        })
    }

    /// Put `frame` on the clipboard as PNG (alpha preserved) and take
    /// ownership. Returns an error when ownership was refused — the caller
    /// reports it like any other copy failure.
    pub fn copy_image<C: Connection>(&mut self, conn: &C, frame: &Frame) -> Result<()> {
        let png = encode_png_bytes(frame)?;
        let mut data = HashMap::new();
        data.insert(self.image_png_atom, png);
        self.own(conn, data)
    }

    /// Put plain UTF-8 text on the clipboard and take ownership.
    pub fn copy_text<C: Connection>(&mut self, conn: &C, text: &str) -> Result<()> {
        let mut data = HashMap::new();
        data.insert(self.utf8_atom, text.as_bytes().to_vec());
        self.own(conn, data)
    }

    fn own<C: Connection>(&mut self, conn: &C, data: HashMap<Atom, Vec<u8>>) -> Result<()> {
        if data.is_empty() {
            return Err(Error::invalid("clipboard payload is empty"));
        }
        // Fire-and-forget: the verification read below is a round-trip and
        // therefore also guarantees this request has been processed.
        let _ = conn.set_selection_owner(self.window, self.selection, x11rb::CURRENT_TIME);
        // Verify: another client may have raced us for ownership.
        let reply = conn
            .get_selection_owner(self.selection)
            .map_err(|e| Error::msg(format!("GetSelectionOwner: {e}")))?
            .reply()
            .map_err(|e| Error::msg(format!("GetSelectionOwner: {e}")))?;
        if reply.owner != self.window {
            return Err(Error::msg("CLIPBOARD ownership was refused by the server"));
        }
        self.data = data;
        Ok(())
    }

    /// Serve one event from the app loop. Returns `true` when the event was
    /// a clipboard event (so the caller can skip further dispatch).
    pub fn handle_event<C: Connection>(&mut self, conn: &C, ev: &Event) -> Result<bool> {
        match ev {
            Event::SelectionRequest(req) if req.selection == self.selection => {
                self.serve(conn, req)?;
                Ok(true)
            }
            Event::SelectionClear(cl) if cl.selection == self.selection => {
                // Another client took the clipboard; drop our payload.
                self.data.clear();
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Answer a `SelectionRequest`: data goes over as a property on the
    /// requestor's window (synchronous transfer — our payloads are small),
    /// then a `SelectionNotify` completes the handshake. An unknown target
    /// is refused with property `NONE`, which is the protocol's "don't have
    /// it" answer, not an application error.
    fn serve<C: Connection>(
        &self,
        conn: &C,
        req: &x11rb::protocol::xproto::SelectionRequestEvent,
    ) -> Result<()> {
        // ICCCM: clients that pass property 0 mean "use the target atom".
        let property = if req.property == x11rb::NONE {
            req.target
        } else {
            req.property
        };
        let mut notify_property = x11rb::NONE;
        if req.target == self.targets_atom {
            let mut atoms: Vec<Atom> = self.data.keys().copied().collect();
            atoms.push(self.targets_atom);
            atoms.sort_unstable();
            if conn
                .change_property32(
                    PropMode::REPLACE,
                    req.requestor,
                    property,
                    AtomEnum::ATOM,
                    &atoms,
                )
                .ok()
                .and_then(|c| c.check().ok())
                .is_some()
            {
                notify_property = property;
            }
        } else if let Some(payload) = self.data.get(&req.target)
            && conn
                .change_property8(
                    PropMode::REPLACE,
                    req.requestor,
                    property,
                    req.target,
                    payload,
                )
                .ok()
                .and_then(|c| c.check().ok())
                .is_some()
        {
            notify_property = property;
        }
        // Always answer the requestor (empty property = refused target).
        let _ = conn.send_event(
            false,
            req.requestor,
            EventMask::NO_EVENT,
            SelectionNotifyEvent {
                response_type: SELECTION_NOTIFY_EVENT,
                sequence: 0,
                time: req.time,
                requestor: req.requestor,
                selection: req.selection,
                target: req.target,
                property: notify_property,
            },
        );
        let _ = conn.flush();
        Ok(())
    }
}
