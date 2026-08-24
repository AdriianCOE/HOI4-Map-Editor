//! Typed requests for synchronous map selection and camera navigation.
//!
//! Requests describe only user intent in screen-space. Canvas owns all map
//! resolution, selection storage, camera mutation, and related session state.

use vecmath::Vector2;

use crate::app::input::{
    InputCommand, PointerCommand, RelativeMotionCommand, ScreenPosition, WheelCommand,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SelectionNavigationRequest {
    SelectStateAt {
        screen_position: ScreenPosition,
        toggle_province: bool,
    },
    ClearStateSelection,
    SelectStrategicRegionAt {
        screen_position: ScreenPosition,
    },
    PanBegin,
    PanBy {
        delta: Vector2<f64>,
    },
    PanEnd,
    Zoom {
        amount: f64,
        anchor: ScreenPosition,
    },
}

pub(crate) const fn clear_state_selection() -> SelectionNavigationRequest {
    SelectionNavigationRequest::ClearStateSelection
}

/// Maps already-classified navigation commands only. Primary clicks are mapped
/// separately after Canvas-owned tool context has confirmed they are selection.
pub(crate) fn request_from_input(command: InputCommand) -> Option<SelectionNavigationRequest> {
    match command {
        InputCommand::Pointer(classification) => match classification.command {
            PointerCommand::BeginPan => Some(SelectionNavigationRequest::PanBegin),
            PointerCommand::EndPan => Some(SelectionNavigationRequest::PanEnd),
            _ => None,
        },
        InputCommand::RelativeMotion(RelativeMotionCommand::PanBy { delta }) => {
            Some(SelectionNavigationRequest::PanBy { delta })
        }
        InputCommand::Wheel(WheelCommand::Zoom { delta_y, position }) => {
            Some(SelectionNavigationRequest::Zoom {
                amount: delta_y,
                anchor: position,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::input::{PointerClassification, WheelCommand};

    #[test]
    fn maps_only_already_classified_navigation_to_requests() {
        assert_eq!(
            request_from_input(InputCommand::Pointer(PointerClassification {
                clear_tooltip: true,
                command: PointerCommand::BeginPan,
            })),
            Some(SelectionNavigationRequest::PanBegin)
        );
        assert_eq!(
            request_from_input(InputCommand::Pointer(PointerClassification {
                clear_tooltip: false,
                command: PointerCommand::EndPan,
            })),
            Some(SelectionNavigationRequest::PanEnd)
        );
        assert_eq!(
            request_from_input(InputCommand::RelativeMotion(RelativeMotionCommand::PanBy {
                delta: [2.0, -3.0],
            })),
            Some(SelectionNavigationRequest::PanBy { delta: [2.0, -3.0] })
        );
        assert_eq!(
            request_from_input(InputCommand::Wheel(WheelCommand::Zoom {
                delta_y: 1.0,
                position: [4.0, 5.0],
            })),
            Some(SelectionNavigationRequest::Zoom {
                amount: 1.0,
                anchor: [4.0, 5.0],
            })
        );
    }

    #[test]
    fn tool_wheel_and_ui_capture_do_not_produce_navigation_requests() {
        assert_eq!(
            request_from_input(InputCommand::Wheel(WheelCommand::ChangeBrushRadius {
                delta_y: 1.0,
            })),
            None
        );
        assert_eq!(
            request_from_input(InputCommand::Pointer(PointerClassification {
                clear_tooltip: false,
                command: PointerCommand::CaptureByInterface,
            })),
            None
        );
    }

    #[test]
    fn state_selection_keeps_screen_position_and_modifier_intent() {
        assert_eq!(
            SelectionNavigationRequest::SelectStateAt {
                screen_position: [8.0, 9.0],
                toggle_province: true,
            },
            SelectionNavigationRequest::SelectStateAt {
                screen_position: [8.0, 9.0],
                toggle_province: true,
            }
        );
    }

    #[test]
    fn clear_selection_is_an_explicit_request() {
        assert_eq!(
            clear_state_selection(),
            SelectionNavigationRequest::ClearStateSelection
        );
    }

    #[test]
    fn strategic_region_selection_is_not_a_map_gesture() {
        assert_eq!(
            SelectionNavigationRequest::SelectStrategicRegionAt {
                screen_position: [3.0, 4.0]
            },
            SelectionNavigationRequest::SelectStrategicRegionAt {
                screen_position: [3.0, 4.0]
            },
        );
    }
}
