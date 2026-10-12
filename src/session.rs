//! Capture-session close policy — the single place that decides whether a
//! route ends the session, keeps it open, or quits the app.
//!
//! Every route that can end a capture session is funnelled through [`plan`]:
//! toolbar **Copy**/**Save**/**Close**, `Ctrl+C`/`Ctrl+S`, `Esc`, `WM_CLOSE`,
//! the *Quick save* tray action and the tray **Exit** item. The rules:
//!
//! - **Copy**/**Save** end the session only after the work actually
//!   succeeded (image on the clipboard, bytes on disk). A clipboard error, a
//!   failed write or a cancelled Save-As keeps the session open — the caller
//!   shows the in-app error.
//! - An **explicit close** (Close button, `Esc`, `WM_CLOSE`) discards the
//!   session immediately: no confirmation dialog ever.
//! - `Esc` in the focused text box only cancels the draft
//!   ([`Route::TextCancel`]); the session itself survives until a later `Esc`
//!   with no text box focused.
//! - Tray **Exit** closes the session and then quits the application.
//!
//! [`close_target`] decides *how* the finalizer runs, so calling it again —
//! a double click, a late message, a second route — releases the window and
//! its child controls exactly once.

/// Outcome of a Save-As action, as produced by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The file dialog returned a path and the encode succeeded.
    Saved,
    /// The user cancelled the file dialog.
    Cancelled,
    /// The dialog failed or the file could not be written.
    Failed,
}

/// A route that may end the capture session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Clipboard copy finished (`ok` = the image really landed on the
    /// clipboard).
    Copy { ok: bool },
    /// Save-As finished with `outcome`.
    Save(SaveOutcome),
    /// Explicit close: toolbar Close button, `Esc`, or `WM_CLOSE`.
    Close,
    /// `Esc` while the text box has focus: the caller discards the draft;
    /// this route alone never touches the session.
    TextCancel,
    /// Tray menu Exit: close the session, then quit the application.
    Exit,
}

/// What the caller must do after [`plan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// Leave the session exactly as it is (the caller has shown the error,
    /// or the route had no session to act on).
    KeepOpen,
    /// Discard the open text draft; the session stays open.
    CancelTextDraft,
    /// Finalize the session now, without prompting and without moving focus.
    CloseSession,
    /// Finalize the session, then quit the application.
    Exit,
}

/// The session-close decision for `route`.
///
/// `session_open` is the caller's live record of whether a session exists;
/// with no session there is nothing to close (but [`Route::Exit`] still
/// quits, because leaving the app running is the point of that route).
pub fn plan(route: Route, session_open: bool) -> Plan {
    if !session_open && !matches!(route, Route::Exit) {
        return Plan::KeepOpen;
    }
    match route {
        Route::Copy { ok: true } => Plan::CloseSession,
        Route::Copy { ok: false } => Plan::KeepOpen,
        Route::Save(SaveOutcome::Saved) => Plan::CloseSession,
        Route::Save(SaveOutcome::Cancelled | SaveOutcome::Failed) => Plan::KeepOpen,
        Route::Close => Plan::CloseSession,
        Route::TextCancel => Plan::CancelTextDraft,
        Route::Exit => Plan::Exit,
    }
}

/// How the finalizer must run against a session record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// No record: the session was already finalized (or never opened).
    Nothing,
    /// The record owns a live window: destroy it (`WM_DESTROY` drops the
    /// record together with its child controls, fonts and backbuffer).
    Destroy,
    /// The record outlived its window: drop the record only.
    ClearStale,
}

/// The finalize step for a session record described by
/// `(session_open, window_alive)`.
///
/// This is what makes the finalizer idempotent: a second call sees
/// `(false, false)` and returns [`CloseAction::Nothing`].
pub fn close_target(session_open: bool, window_alive: bool) -> CloseAction {
    match (session_open, window_alive) {
        (false, _) => CloseAction::Nothing,
        (true, true) => CloseAction::Destroy,
        (true, false) => CloseAction::ClearStale,
    }
}

// ---------------------------------------------------------------------------
// Tests: the policy state machine (no windows, no UI)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: bool = true;
    const CLOSED: bool = false;

    #[test]
    fn copy_closes_only_after_a_successful_clipboard_write() {
        assert_eq!(plan(Route::Copy { ok: true }, OPEN), Plan::CloseSession);
        assert_eq!(plan(Route::Copy { ok: false }, OPEN), Plan::KeepOpen);
        // With no session open there is nothing to close either way.
        assert_eq!(plan(Route::Copy { ok: true }, CLOSED), Plan::KeepOpen);
        assert_eq!(plan(Route::Copy { ok: false }, CLOSED), Plan::KeepOpen);
    }

    #[test]
    fn save_closes_only_after_the_file_was_written() {
        assert_eq!(
            plan(Route::Save(SaveOutcome::Saved), OPEN),
            Plan::CloseSession
        );
        assert_eq!(
            plan(Route::Save(SaveOutcome::Cancelled), OPEN),
            Plan::KeepOpen
        );
        assert_eq!(plan(Route::Save(SaveOutcome::Failed), OPEN), Plan::KeepOpen);
        assert_eq!(
            plan(Route::Save(SaveOutcome::Saved), CLOSED),
            Plan::KeepOpen
        );
    }

    #[test]
    fn explicit_close_discards_immediately_and_only_with_a_session() {
        // Close button, Esc and WM_CLOSE all land here: never a prompt.
        assert_eq!(plan(Route::Close, OPEN), Plan::CloseSession);
        assert_eq!(plan(Route::Close, CLOSED), Plan::KeepOpen);
    }

    #[test]
    fn text_box_escape_cancels_the_draft_and_never_the_session() {
        assert_eq!(plan(Route::TextCancel, OPEN), Plan::CancelTextDraft);
        assert_eq!(plan(Route::TextCancel, CLOSED), Plan::KeepOpen);
    }

    #[test]
    fn exit_quits_with_and_without_an_open_session() {
        assert_eq!(plan(Route::Exit, OPEN), Plan::Exit);
        assert_eq!(plan(Route::Exit, CLOSED), Plan::Exit);
    }

    #[test]
    fn no_failure_or_cancel_route_ever_closes_or_exits() {
        let routes = [
            Route::Copy { ok: false },
            Route::Save(SaveOutcome::Cancelled),
            Route::Save(SaveOutcome::Failed),
            Route::TextCancel,
        ];
        for route in routes {
            for open in [OPEN, CLOSED] {
                let p = plan(route, open);
                assert!(
                    matches!(p, Plan::KeepOpen | Plan::CancelTextDraft),
                    "{route:?} (open={open}) must not close, got {p:?}"
                );
            }
        }
    }

    #[test]
    fn close_target_is_the_idempotency_table() {
        assert_eq!(close_target(false, false), CloseAction::Nothing);
        assert_eq!(close_target(false, true), CloseAction::Nothing);
        assert_eq!(close_target(true, true), CloseAction::Destroy);
        assert_eq!(close_target(true, false), CloseAction::ClearStale);
    }

    #[test]
    fn a_successful_route_finalizes_the_session_exactly_once() {
        // Open session, live window (what the production records hold).
        let mut open = true;
        let mut alive = true;
        let mut destroys = 0;

        // Route: Copy succeeded -> CloseSession -> destroy the window; the
        // window procedure's WM_DESTROY drops the session record.
        assert_eq!(plan(Route::Copy { ok: true }, open), Plan::CloseSession);
        match close_target(open, alive) {
            CloseAction::Destroy => {
                destroys += 1;
                open = false;
                alive = false;
            }
            other => panic!("expected Destroy for a live session, got {other:?}"),
        }

        // A duplicate route (double click, late message) must be a no-op.
        assert_eq!(plan(Route::Close, open), Plan::KeepOpen);
        assert_eq!(close_target(open, alive), CloseAction::Nothing);

        // A record whose window died without WM_DESTROY is cleared, never
        // "destroyed" a second time.
        assert_eq!(close_target(true, false), CloseAction::ClearStale);

        assert_eq!(destroys, 1, "the window must be released exactly once");
        assert!(!open, "the record must be gone");
    }

    #[test]
    fn full_route_matrix_never_leaves_the_policy_undefined() {
        let routes = [
            Route::Copy { ok: true },
            Route::Copy { ok: false },
            Route::Save(SaveOutcome::Saved),
            Route::Save(SaveOutcome::Cancelled),
            Route::Save(SaveOutcome::Failed),
            Route::Close,
            Route::TextCancel,
            Route::Exit,
        ];
        for route in routes {
            for open in [OPEN, CLOSED] {
                let p = plan(route, open);
                assert!(
                    matches!(
                        p,
                        Plan::KeepOpen | Plan::CancelTextDraft | Plan::CloseSession | Plan::Exit
                    ),
                    "{route:?} (open={open}) produced an unknown plan"
                );
                // Only the two "close now" outcomes may drive a finalizer —
                // and Exit keeps its meaning with no session, because
                // quitting is the point of that route (see `plan`).
                if matches!(p, Plan::CloseSession | Plan::Exit) {
                    assert!(
                        open || matches!(route, Route::Exit),
                        "{route:?} closed a session that was not open"
                    );
                }
            }
        }
    }
}
