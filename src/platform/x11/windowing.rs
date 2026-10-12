//! X11 windowing primitives for the UI port (M1b): connection, the hidden
//! owner window that receives `XGrabKey` hits and clipboard events, a
//! blocking event pump, override-redirect windows for the selection
//! overlay, pointer grab for drags, crosshair cursor, and a one-shot
//! `PutImage` blit.
//!
//! Design carried over from the Windows build: **one connection, one hidden
//! window, blocking wait, no threads, no polling** — idle time sits in
//! `XNextEvent` at 0% CPU. The app/overlay/editor *state machines* are not
//! ported yet; this is only the platform layer they will sit on.
//!
//! Like every `x11` module in M1: compiled, never executed (no X11 session
//! exists on the build host), no runtime claims.

use super::clipboard::Clipboard;
use super::display;
use crate::error::{Error, Result};
use crate::image::Frame;
use crate::platform::x11_logic;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask,
    GrabMode, GrabStatus, ImageFormat, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

/// Cursor font glyph id for a crosshair (X11 core cursor font).
const CROSSHAIR_GLYPH: u16 = 34;

pub struct App {
    pub conn: RustConnection,
    /// X screen index this connection opened (from `x11rb::connect`).
    pub screen: usize,
    /// Hidden 1x1 `INPUT_ONLY` window: hotkey grabs, clipboard ownership
    /// and the event loop's home. `INPUT_ONLY` + `override_redirect` means
    /// it draws nothing and never appears in taskbars/window lists.
    pub hidden: Window,
    pub clipboard: Clipboard,
    cursor: Option<(
        x11rb::protocol::xproto::Font,
        x11rb::protocol::xproto::Cursor,
    )>,
}

impl App {
    /// Open the display and create the hidden owner window.
    pub fn connect() -> Result<App> {
        let (conn, screen) =
            x11rb::connect(None).map_err(|e| Error::msg(format!("cannot open X display: {e}")))?;
        let root = display::root_of(&conn, screen)?;
        let hidden = conn
            .generate_id()
            .map_err(|e| Error::msg(format!("X11 id: {e}")))?;
        let aux = CreateWindowAux::new()
            .override_redirect(Some(1u32))
            .event_mask(
                EventMask::KEY_PRESS
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::STRUCTURE_NOTIFY,
            );
        // depth 0 + visual 0 = CopyFromParent, mandatory for INPUT_ONLY.
        conn.create_window(
            0,
            hidden,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &aux,
        )
        .map_err(|e| Error::msg(format!("create hidden window: {e}")))?
        .check()
        .map_err(|e| Error::msg(format!("create hidden window: {e}")))?;
        conn.map_window(hidden)
            .map_err(|e| Error::msg(format!("map hidden window: {e}")))?
            .check()
            .map_err(|e| Error::msg(format!("map hidden window: {e}")))?;
        let clipboard = Clipboard::new(&conn, hidden)?;
        conn.flush()
            .map_err(|e| Error::msg(format!("X11 flush: {e}")))?;
        Ok(App {
            conn,
            screen,
            hidden,
            clipboard,
            cursor: None,
        })
    }

    /// Blocking wait for the next event — the idle state spends 0% CPU
    /// here (same blocking-`GetMessage` model as the Windows build).
    pub fn next_event(&self) -> Result<Event> {
        self.conn
            .wait_for_event()
            .map_err(|e| Error::msg(format!("X11 event loop: {e}")))
    }

    /// Hand an event to the clipboard state machine; `true` = consumed.
    pub fn pump_clipboard(&mut self, ev: &Event) -> Result<bool> {
        let App {
            conn, clipboard, ..
        } = self;
        clipboard.handle_event(conn, ev)
    }

    /// Create an `override-redirect` top-level window for the selection
    /// overlay — the `WS_EX_TOOLWINDOW` analogue (no taskbar, no alt-tab,
    /// no WM decorations to fight over). The caller positions and resizes it
    /// with `configure_window` as the drag progresses.
    pub fn create_overlay(&self, x: i32, y: i32, w: u32, h: u32) -> Result<Window> {
        let root = display::root_of(&self.conn, self.screen)?;
        let (depth, visual) = display::root_depth_visual(&self.conn, self.screen)?;
        let black = self
            .conn
            .setup()
            .roots
            .get(self.screen)
            .map(|s| s.black_pixel)
            .unwrap_or(0);
        let id = self
            .conn
            .generate_id()
            .map_err(|e| Error::msg(format!("X11 id: {e}")))?;
        let aux = CreateWindowAux::new()
            .override_redirect(Some(1u32))
            .background_pixel(black)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::KEY_PRESS
                    | EventMask::STRUCTURE_NOTIFY,
            );
        let (x, y, w, h) = clamp_to_i16_u16(x, y, w, h);
        self.conn
            .create_window(
                depth,
                id,
                root,
                x,
                y,
                w,
                h,
                0,
                WindowClass::INPUT_OUTPUT,
                visual,
                &aux,
            )
            .map_err(|e| Error::msg(format!("create overlay window: {e}")))?
            .check()
            .map_err(|e| Error::msg(format!("create overlay window: {e}")))?;
        self.conn
            .map_window(id)
            .map_err(|e| Error::msg(format!("map overlay window: {e}")))?
            .check()
            .map_err(|e| Error::msg(format!("map overlay window: {e}")))?;
        Ok(id)
    }

    /// Move/resize an existing window (overlay drag updates).
    pub fn configure(&self, window: Window, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let (x, y, w, h) = clamp_to_i16_u16(x, y, w, h);
        self.conn
            .configure_window(
                window,
                &x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .x(x as i32)
                    .y(y as i32)
                    .width(w as u32)
                    .height(h as u32),
            )
            .map_err(|e| Error::msg(format!("configure_window: {e}")))?
            .check()
            .map_err(|e| Error::msg(format!("configure_window: {e}")))?;
        Ok(())
    }

    /// Grab the pointer for a selection drag: async modes so motion events
    /// stream without round-trips, no cursor swap (the crosshair is set on
    /// the window), no confinement.
    pub fn grab_pointer(&self) -> Result<()> {
        let reply = self
            .conn
            .grab_pointer(
                false,
                self.hidden,
                EventMask::POINTER_MOTION | EventMask::BUTTON_RELEASE | EventMask::BUTTON_PRESS,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                x11rb::CURRENT_TIME,
            )
            .map_err(|e| Error::msg(format!("GrabPointer: {e}")))?
            .reply()
            .map_err(|e| Error::msg(format!("GrabPointer: {e}")))?;
        if reply.status != GrabStatus::SUCCESS {
            return Err(Error::msg(
                "pointer grab refused (another client holds the pointer)",
            ));
        }
        Ok(())
    }

    /// Release the pointer grab (best-effort, like `UnregisterHotKey`).
    pub fn ungrab_pointer(&self) {
        let _ = self.conn.ungrab_pointer(x11rb::CURRENT_TIME);
    }

    /// Give the keyboard to `window` so session keys (Escape, Enter) reach
    /// the event loop: the overlay is `override-redirect`, so no window
    /// manager will ever focus it on its own. Best-effort semantics are not
    /// enough here — a failed focus means silently dead keys — so errors
    /// surface.
    pub fn set_input_focus(&self, window: Window) -> Result<()> {
        self.conn
            .set_input_focus(
                x11rb::protocol::xproto::InputFocus::POINTER_ROOT,
                window,
                x11rb::CURRENT_TIME,
            )
            .map_err(|e| Error::msg(format!("SetInputFocus: {e}")))?
            .check()
            .map_err(|e| Error::msg(format!("SetInputFocus: {e}")))?;
        Ok(())
    }

    /// Assign the crosshair cursor (cursor-font glyph 34, mask at 35 — the
    /// standard `glyph`/`glyph + 1` pairing of the X11 cursor font) to a
    /// window. The font/cursor pair is opened once and reused.
    pub fn set_crosshair(&mut self, window: Window) -> Result<()> {
        if self.cursor.is_none() {
            let fid = self
                .conn
                .generate_id()
                .map_err(|e| Error::msg(format!("X11 id: {e}")))?;
            self.conn
                .open_font(fid, b"cursor")
                .map_err(|e| Error::msg(format!("open cursor font: {e}")))?
                .check()
                .map_err(|e| Error::msg(format!("open cursor font: {e}")))?;
            let cid = self
                .conn
                .generate_id()
                .map_err(|e| Error::msg(format!("X11 id: {e}")))?;
            // Black shape on white, matching the default X cursor colours.
            self.conn
                .create_glyph_cursor(
                    cid,
                    fid,
                    fid,
                    CROSSHAIR_GLYPH,
                    CROSSHAIR_GLYPH + 1,
                    0,
                    0,
                    0,
                    u16::MAX,
                    u16::MAX,
                    u16::MAX,
                )
                .map_err(|e| Error::msg(format!("create cursor: {e}")))?
                .check()
                .map_err(|e| Error::msg(format!("create cursor: {e}")))?;
            self.cursor = Some((fid, cid));
        }
        let (_, cursor) = self.cursor.expect("cursor set above");
        self.conn
            .change_window_attributes(window, &ChangeWindowAttributesAux::new().cursor(cursor))
            .map_err(|e| Error::msg(format!("assign cursor: {e}")))?
            .check()
            .map_err(|e| Error::msg(format!("assign cursor: {e}")))?;
        Ok(())
    }

    /// Blit a finished frame into `window` at (0,0): one `PutImage` per
    /// refresh (refreshes are event-driven, never timed), GC created and
    /// freed with the blit because M1 has no repaint loop yet.
    pub fn put_frame(&self, window: Window, frame: &Frame) -> Result<()> {
        if frame.w == 0 || frame.h == 0 {
            return Err(Error::invalid("cannot blit a zero-sized frame"));
        }
        let (depth, _) = display::root_depth_visual(&self.conn, self.screen)?;
        let fmt = display::screen_image_format(&self.conn, self.screen, frame.w, frame.h)?;
        let data = x11_logic::zpixmap_from_bgra(&frame.px, &fmt)
            .ok_or_else(|| Error::invalid("frame buffer length mismatch"))?;
        let gc = self
            .conn
            .generate_id()
            .map_err(|e| Error::msg(format!("X11 id: {e}")))?;
        self.conn
            .create_gc(gc, window, &CreateGCAux::new().graphics_exposures(0))
            .map_err(|e| Error::msg(format!("create_gc: {e}")))?
            .check()
            .map_err(|e| Error::msg(format!("create_gc: {e}")))?;
        let result = self
            .conn
            .put_image(
                ImageFormat::Z_PIXMAP,
                window,
                gc,
                frame.w as u16,
                frame.h as u16,
                0,
                0,
                0,
                depth,
                &data,
            )
            .map_err(|e| Error::msg(format!("PutImage: {e}")))
            .and_then(|c| c.check().map_err(|e| Error::msg(format!("PutImage: {e}"))));
        let _ = self.conn.free_gc(gc);
        let _ = self.conn.flush();
        result?;
        Ok(())
    }

    /// Flush queued requests to the server.
    pub fn flush(&self) -> Result<()> {
        self.conn
            .flush()
            .map_err(|e| Error::msg(format!("X11 flush: {e}")))
    }
}

/// X11 wire limits: window coordinates are `INT16`, sizes `CARD16`.
fn clamp_to_i16_u16(x: i32, y: i32, w: u32, h: u32) -> (i16, i16, u16, u16) {
    (
        x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        w.min(u16::MAX as u32) as u16,
        h.min(u16::MAX as u32) as u16,
    )
}
