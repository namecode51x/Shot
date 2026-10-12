//! x11rb event → [`RawEvent`] glue (M1b): the mechanical field extraction
//! only — every *decision* (which button counts, root→frame offsets,
//! keycode comparison) happens in the tested
//! [`crate::platform::x11_logic::to_session`] policy. The field-extraction
//! tests below are gated to Linux (`#[cfg(all(test, target_os = "linux"))]`)
//! and run only where the x11rb dependency exists; the build host without a
//! Linux toolchain records them as compile-verified
//! (`cargo check --target x86_64-unknown-linux-gnu --tests`), and the
//! validation script (`scripts/linux_x11_validate.sh`) executes them.
//!
//! Button and motion events are read from `root_*`, not `event_*`: the
//! root position is window-independent, so it is irrelevant whether the
//! pointer grab delivered the event to the hidden owner window or to the
//! overlay.

use crate::platform::x11_logic::{RawEvent, RawKind};
use x11rb::protocol::Event;

/// Extract the session-relevant fields of one protocol event; `None` for
/// everything the session does not consume (exposures, clipboard traffic,
/// structure notifies — the driver routes those before calling this).
///
/// The `detail`/keycode filtering itself is *not* done here: `to_session`
/// owns that policy, so a wrong-button or wrong-key decision would be
/// caught by the unit suite rather than by review alone.
pub fn extract(ev: &Event) -> Option<RawEvent> {
    let raw = match ev {
        Event::ButtonPress(b) => RawEvent {
            kind: RawKind::Press,
            detail: b.detail,
            root_x: i32::from(b.root_x),
            root_y: i32::from(b.root_y),
        },
        Event::MotionNotify(m) => RawEvent {
            kind: RawKind::Move,
            detail: m.detail.into(),
            root_x: i32::from(m.root_x),
            root_y: i32::from(m.root_y),
        },
        Event::ButtonRelease(b) => RawEvent {
            kind: RawKind::Release,
            detail: b.detail,
            root_x: i32::from(b.root_x),
            root_y: i32::from(b.root_y),
        },
        Event::KeyPress(k) => RawEvent {
            kind: RawKind::Key,
            detail: k.detail,
            root_x: 0,
            root_y: 0,
        },
        _ => return None,
    };
    Some(raw)
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use crate::platform::x11_logic::{SessEvent, to_session};
    use x11rb::protocol::xproto::{
        ButtonPressEvent, ExposeEvent, KeyPressEvent, Motion, MotionNotifyEvent,
    };

    fn press(detail: u8, rx: i16, ry: i16, ex: i16, ey: i16) -> ButtonPressEvent {
        ButtonPressEvent {
            response_type: 4,
            detail,
            sequence: 0,
            time: 0,
            root: 1,
            event: 1,
            child: 0,
            root_x: rx,
            root_y: ry,
            event_x: ex,
            event_y: ey,
            state: 0u16.into(),
            same_screen: true,
        }
    }

    fn motion(rx: i16, ry: i16) -> MotionNotifyEvent {
        MotionNotifyEvent {
            response_type: 6,
            detail: Motion::NORMAL,
            sequence: 0,
            time: 0,
            root: 1,
            event: 1,
            child: 0,
            root_x: rx,
            root_y: ry,
            event_x: 0,
            event_y: 0,
            state: 0u16.into(),
            same_screen: true,
        }
    }

    fn key(code: u8) -> KeyPressEvent {
        KeyPressEvent {
            response_type: 2,
            detail: code,
            sequence: 0,
            time: 0,
            root: 1,
            event: 1,
            child: 0,
            root_x: 0,
            root_y: 0,
            event_x: 0,
            event_y: 0,
            state: 0u16.into(),
            same_screen: true,
        }
    }

    #[test]
    fn press_reads_root_coords_not_event_coords() {
        let ev = Event::ButtonPress(press(1, 100, 200, 50, 60));
        assert_eq!(
            extract(&ev),
            Some(RawEvent {
                kind: RawKind::Press,
                detail: 1,
                root_x: 100,
                root_y: 200
            })
        );
    }

    #[test]
    fn release_maps_fields() {
        let mut ev = Event::ButtonPress(press(1, 7, 8, 0, 0));
        if let Event::ButtonPress(ref mut b) = ev {
            b.response_type = 5;
        }
        assert_eq!(
            extract(&ev),
            Some(RawEvent {
                kind: RawKind::Release,
                detail: 1,
                root_x: 7,
                root_y: 8
            })
        );
    }

    #[test]
    fn non_primary_button_is_extracted_not_filtered() {
        // Filtering belongs to to_session; extraction must not decide.
        let ev = Event::ButtonPress(press(3, 9, 9, 0, 0));
        assert_eq!(
            extract(&ev),
            Some(RawEvent {
                kind: RawKind::Press,
                detail: 3,
                root_x: 9,
                root_y: 9
            })
        );
    }

    #[test]
    fn motion_maps_fields_and_detail() {
        let ev = Event::MotionNotify(motion(300, 400));
        let got = extract(&ev).expect("motion must extract");
        assert_eq!(got.kind, RawKind::Move);
        assert_eq!(got.detail, u8::from(Motion::NORMAL));
        assert_eq!((got.root_x, got.root_y), (300, 400));
    }

    #[test]
    fn keypress_maps_code_with_zero_coords() {
        let ev = Event::KeyPress(key(9));
        assert_eq!(
            extract(&ev),
            Some(RawEvent {
                kind: RawKind::Key,
                detail: 9,
                root_x: 0,
                root_y: 0
            })
        );
    }

    #[test]
    fn non_session_events_return_none() {
        let ev = Event::Expose(ExposeEvent {
            response_type: 12,
            sequence: 0,
            window: 1,
            x: 0,
            y: 0,
            width: 8,
            height: 8,
            count: 0,
        });
        assert_eq!(extract(&ev), None);
    }

    #[test]
    fn extracted_press_runs_through_policy_with_origin() {
        let ev = Event::ButtonPress(press(1, 300, 400, 0, 0));
        let raw = extract(&ev).unwrap();
        assert_eq!(
            to_session(&raw, None, 100, 200),
            Some(SessEvent::Press { x: 200, y: 200 })
        );
    }

    #[test]
    fn policy_rejects_non_primary_and_unresolved_keys() {
        let raw3 = extract(&Event::ButtonPress(press(3, 1, 2, 0, 0))).unwrap();
        assert_eq!(to_session(&raw3, None, 0, 0), None);
        let rawk = extract(&Event::KeyPress(key(9))).unwrap();
        assert_eq!(to_session(&rawk, None, 0, 0), None);
        assert_eq!(
            to_session(&rawk, Some((9, 36)), 0, 0),
            Some(SessEvent::Cancel)
        );
    }
}
