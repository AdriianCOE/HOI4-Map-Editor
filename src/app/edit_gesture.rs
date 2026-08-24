//! Typed routing for primary map-edit gesture intent.
//!
//! This module deliberately represents only the input lifecycle.  It owns no
//! tool, map, session, history, or UI state; Canvas resolves the request using
//! its current tool and project context.

use crate::app::input::{CursorCommand, PointerCommand, ScreenPosition};
use crate::events::KeyMods;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GestureModifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl From<KeyMods> for GestureModifiers {
    fn from(modifiers: KeyMods) -> Self {
        Self {
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
        }
    }
}

/// A screen-space lifecycle request for a left-button map interaction.
///
/// `Begin` and `Continue` retain the modifiers captured with that input event.
/// Canvas alone interprets them for its active tool.  `End` intentionally has
/// no map-edit result: Canvas decides whether a tool-specific transaction ends,
/// commits, or is unaffected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MapGestureRequest {
    Begin {
        screen_position: ScreenPosition,
        modifiers: GestureModifiers,
    },
    Continue {
        screen_position: ScreenPosition,
        modifiers: GestureModifiers,
    },
    End,
}

pub(crate) fn request_from_pointer(
    command: PointerCommand,
    modifiers: KeyMods,
) -> Option<MapGestureRequest> {
    match command {
        PointerCommand::BeginPrimaryGesture { position } => Some(MapGestureRequest::Begin {
            screen_position: position,
            modifiers: modifiers.into(),
        }),
        PointerCommand::EndPrimaryGesture => Some(MapGestureRequest::End),
        _ => None,
    }
}

pub(crate) fn request_from_cursor(
    command: CursorCommand,
    modifiers: KeyMods,
) -> Option<MapGestureRequest> {
    match command {
        CursorCommand::UpdateInterface {
            position,
            primary_gesture_active: true,
        } => Some(MapGestureRequest::Continue {
            screen_position: position,
            modifiers: modifiers.into(),
        }),
        CursorCommand::CaptureByStateApplyDialog
        | CursorCommand::UpdateInterface {
            primary_gesture_active: false,
            ..
        } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modifiers() -> KeyMods {
        KeyMods {
            ctrl: true,
            shift: true,
            alt: true,
        }
    }

    #[test]
    fn primary_press_becomes_a_pure_begin_request() {
        assert_eq!(
            request_from_pointer(
                PointerCommand::BeginPrimaryGesture {
                    position: [8.0, 9.0],
                },
                modifiers(),
            ),
            Some(MapGestureRequest::Begin {
                screen_position: [8.0, 9.0],
                modifiers: GestureModifiers {
                    ctrl: true,
                    shift: true,
                    alt: true,
                },
            })
        );
    }

    #[test]
    fn active_motion_becomes_continue_and_idle_motion_does_not() {
        assert_eq!(
            request_from_cursor(
                CursorCommand::UpdateInterface {
                    position: [3.0, 4.0],
                    primary_gesture_active: true,
                },
                KeyMods::default(),
            ),
            Some(MapGestureRequest::Continue {
                screen_position: [3.0, 4.0],
                modifiers: GestureModifiers {
                    ctrl: false,
                    shift: false,
                    alt: false,
                },
            })
        );
        assert_eq!(
            request_from_cursor(
                CursorCommand::UpdateInterface {
                    position: [3.0, 4.0],
                    primary_gesture_active: false,
                },
                KeyMods::default(),
            ),
            None
        );
    }

    #[test]
    fn only_a_primary_release_becomes_end() {
        assert_eq!(
            request_from_pointer(PointerCommand::EndPrimaryGesture, KeyMods::default()),
            Some(MapGestureRequest::End)
        );
        assert_eq!(
            request_from_pointer(PointerCommand::ConsumePrimaryRelease, KeyMods::default()),
            None
        );
    }
}
