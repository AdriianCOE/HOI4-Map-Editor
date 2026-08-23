//! Pure classification of application input.
//!
//! This module deliberately owns neither `App` nor `Canvas`.  It turns a small
//! snapshot of capture and workspace state plus raw input into explicit routing
//! commands.  App executes application requests and forwards map intent to the
//! existing Canvas/tool methods, which remain the owners of map mutation,
//! selection, camera state, and history transactions.

use graphics::Viewport;
use piston::input::{Key, MouseButton};
use vecmath::Vector2;

use crate::app::map_layers::WorkspaceMode;
use crate::app::project::{LassoSelectionMode, MapViewMode};
use crate::events::KeyMods;

pub(crate) type ScreenPosition = Vector2<f64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputContext {
    pub has_canvas: bool,
    pub preferences_dialog_open: bool,
    pub state_apply_dialog_open: bool,
    pub inspector_picker_open: bool,
    pub inspector_search_focused: bool,
    pub property_editor_open: bool,
    pub editing_locked: bool,
    pub workspace: Option<WorkspaceMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawKeyEvent {
    pub key: Key,
    pub pressed: bool,
    pub mods: KeyMods,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum InputCommand {
    Keyboard(KeyboardCommand),
    Pointer(PointerClassification),
    Cursor(CursorCommand),
    RelativeMotion(RelativeMotionCommand),
    Wheel(WheelCommand),
    FileDrop(FileDropCommand),
    Viewport(ViewportCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyboardCommand {
    Preferences { key: Key, mods: KeyMods },
    StateApplyDialog(StateApplyDialogKeyCommand),
    InspectorPicker(InspectorPickerKeyCommand),
    InspectorSearch(InspectorSearchKeyCommand),
    PropertyEditor(PropertyEditorKeyCommand),
    EditingLocked(EditingLockedKeyCommand),
    FocusMapSearch,
    SetWorkspace(WorkspaceMode),
    SetAlertsVisible(bool),
    Application(ApplicationCommand),
    Map(MapKeyboardCommand),
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StateApplyDialogKeyCommand {
    Backspace,
    Close,
    Captured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectorPickerKeyCommand {
    Cancel,
    Backspace,
    MovePrevious,
    MoveNext,
    PagePrevious,
    PageNext,
    First,
    Last,
    Confirm,
    Captured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectorSearchKeyCommand {
    Cancel,
    Backspace,
    MovePrevious,
    MoveNext,
    Select { additive: bool },
    Captured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PropertyEditorKeyCommand {
    Cancel,
    NextField { reverse: bool },
    Backspace,
    ClearField,
    Apply,
    SelectAll,
    Captured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditingLockedKeyCommand {
    Save,
    Cancel,
    ReportLocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplicationCommand {
    OpenProject { archive: bool },
    ExportProvinceMap { archive: bool },
    Save,
    RevealMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolShortcut {
    BrushOrBucket,
    Lasso {
        selection_mode: Option<LassoSelectionMode>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapKeyboardCommand {
    Undo,
    Redo,
    DiscardStateEdits,
    MoveSelectedProvinces,
    UnassignSelectedProvinces,
    CycleToolBrush {
        shift: bool,
    },
    CancelTool,
    ConfirmTool,
    ActivateHoveredStateFill,
    CalculateCoastalProvinces,
    CalculateRecolorMap,
    DisplayProblems,
    ToggleProblemsOverlay,
    CycleBrushMask,
    ResetCamera,
    SetPaintAreaTool,
    ToolShortcut(ToolShortcut),
    ChangeMapView(MapViewMode),
    CycleProvinceLabels,
    #[cfg(any(debug_assertions, feature = "debug-mode"))]
    CycleDeveloperDiagnostics,
}

pub(crate) fn classify_key(raw: RawKeyEvent, context: InputContext) -> InputCommand {
    let command = if raw.pressed && context.preferences_dialog_open {
        KeyboardCommand::Preferences {
            key: raw.key,
            mods: raw.mods,
        }
    } else if raw.pressed && context.state_apply_dialog_open {
        KeyboardCommand::StateApplyDialog(match raw.key {
            Key::Backspace => StateApplyDialogKeyCommand::Backspace,
            Key::Escape => StateApplyDialogKeyCommand::Close,
            _ => StateApplyDialogKeyCommand::Captured,
        })
    } else if raw.pressed && context.inspector_picker_open {
        KeyboardCommand::InspectorPicker(match raw.key {
            Key::Escape => InspectorPickerKeyCommand::Cancel,
            Key::Backspace => InspectorPickerKeyCommand::Backspace,
            Key::Up => InspectorPickerKeyCommand::MovePrevious,
            Key::Down => InspectorPickerKeyCommand::MoveNext,
            Key::PageUp => InspectorPickerKeyCommand::PagePrevious,
            Key::PageDown => InspectorPickerKeyCommand::PageNext,
            Key::Home => InspectorPickerKeyCommand::First,
            Key::End => InspectorPickerKeyCommand::Last,
            Key::Return => InspectorPickerKeyCommand::Confirm,
            _ => InspectorPickerKeyCommand::Captured,
        })
    } else if raw.pressed && context.inspector_search_focused {
        KeyboardCommand::InspectorSearch(match raw.key {
            Key::Escape => InspectorSearchKeyCommand::Cancel,
            Key::Backspace => InspectorSearchKeyCommand::Backspace,
            Key::Up => InspectorSearchKeyCommand::MovePrevious,
            Key::Down => InspectorSearchKeyCommand::MoveNext,
            Key::Return => InspectorSearchKeyCommand::Select {
                additive: raw.mods.ctrl,
            },
            _ => InspectorSearchKeyCommand::Captured,
        })
    } else if raw.pressed && context.property_editor_open {
        KeyboardCommand::PropertyEditor(match raw.key {
            Key::Escape => PropertyEditorKeyCommand::Cancel,
            Key::Tab => PropertyEditorKeyCommand::NextField {
                reverse: raw.mods.shift,
            },
            Key::Backspace => PropertyEditorKeyCommand::Backspace,
            Key::Delete => PropertyEditorKeyCommand::ClearField,
            Key::Return => PropertyEditorKeyCommand::Apply,
            Key::A if raw.mods.ctrl => PropertyEditorKeyCommand::SelectAll,
            _ => PropertyEditorKeyCommand::Captured,
        })
    } else if raw.pressed && context.editing_locked {
        KeyboardCommand::EditingLocked(if raw.key == Key::S && raw.mods.ctrl && !raw.mods.shift {
            EditingLockedKeyCommand::Save
        } else if raw.key == Key::Escape {
            EditingLockedKeyCommand::Cancel
        } else {
            EditingLockedKeyCommand::ReportLocked
        })
    } else if raw.pressed && raw.key == Key::F && raw.mods.ctrl {
        KeyboardCommand::FocusMapSearch
    } else if raw.pressed
        && context
            .workspace
            .and_then(|current| workspace_shortcut(raw.key, raw.mods, current))
            .is_some()
    {
        KeyboardCommand::SetWorkspace(
            context
                .workspace
                .and_then(|current| workspace_shortcut(raw.key, raw.mods, current))
                .expect("workspace shortcut was checked above"),
        )
    } else if raw.key == Key::Tab {
        KeyboardCommand::SetAlertsVisible(raw.pressed)
    } else if raw.pressed && raw.key == Key::O && raw.mods.ctrl {
        KeyboardCommand::Application(ApplicationCommand::OpenProject {
            archive: raw.mods.alt,
        })
    } else if raw.pressed
        && context.has_canvas
        && raw.key == Key::S
        && raw.mods.ctrl
        && raw.mods.shift
    {
        KeyboardCommand::Application(ApplicationCommand::ExportProvinceMap {
            archive: raw.mods.alt,
        })
    } else if raw.pressed && context.has_canvas && raw.key == Key::S && raw.mods.ctrl {
        KeyboardCommand::Application(ApplicationCommand::Save)
    } else if raw.pressed
        && context.has_canvas
        && raw.key == Key::R
        && raw.mods.ctrl
        && raw.mods.alt
    {
        KeyboardCommand::Application(ApplicationCommand::RevealMap)
    } else if raw.pressed && context.has_canvas {
        classify_map_key(raw).unwrap_or(KeyboardCommand::Ignore)
    } else {
        KeyboardCommand::Ignore
    };
    InputCommand::Keyboard(command)
}

fn classify_map_key(raw: RawKeyEvent) -> Option<KeyboardCommand> {
    let command = match raw.key {
        Key::Z if raw.mods.ctrl => MapKeyboardCommand::Undo,
        Key::Y if raw.mods.ctrl => MapKeyboardCommand::Redo,
        Key::D if raw.mods.ctrl && raw.mods.shift => MapKeyboardCommand::DiscardStateEdits,
        Key::M if !raw.mods.shift => MapKeyboardCommand::MoveSelectedProvinces,
        Key::Delete => MapKeyboardCommand::UnassignSelectedProvinces,
        Key::Space => MapKeyboardCommand::CycleToolBrush {
            shift: raw.mods.shift,
        },
        Key::Escape => MapKeyboardCommand::CancelTool,
        Key::Return => MapKeyboardCommand::ConfirmTool,
        Key::F => MapKeyboardCommand::ActivateHoveredStateFill,
        Key::C if raw.mods.shift => MapKeyboardCommand::CalculateCoastalProvinces,
        Key::R if raw.mods.shift => MapKeyboardCommand::CalculateRecolorMap,
        Key::P if raw.mods.shift => MapKeyboardCommand::DisplayProblems,
        Key::O if raw.mods.shift => MapKeyboardCommand::ToggleProblemsOverlay,
        Key::M if raw.mods.shift => MapKeyboardCommand::CycleBrushMask,
        Key::H => MapKeyboardCommand::ResetCamera,
        Key::A => MapKeyboardCommand::SetPaintAreaTool,
        Key::B => MapKeyboardCommand::ToolShortcut(ToolShortcut::BrushOrBucket),
        Key::L => MapKeyboardCommand::ToolShortcut(ToolShortcut::Lasso {
            selection_mode: lasso_mode_from_mods(raw.mods),
        }),
        Key::D1 => MapKeyboardCommand::ChangeMapView(MapViewMode::ProvinceColors),
        Key::D2 => MapKeyboardCommand::ChangeMapView(MapViewMode::ProvinceTypes),
        Key::D3 => MapKeyboardCommand::ChangeMapView(MapViewMode::Terrain),
        Key::D4 => MapKeyboardCommand::ChangeMapView(MapViewMode::Continents),
        Key::D5 => MapKeyboardCommand::ChangeMapView(MapViewMode::Coastal),
        Key::D6 => MapKeyboardCommand::ChangeMapView(MapViewMode::States),
        Key::D7 => MapKeyboardCommand::ChangeMapView(MapViewMode::Political),
        Key::D8 => MapKeyboardCommand::ChangeMapView(MapViewMode::States),
        Key::D9 => MapKeyboardCommand::CycleProvinceLabels,
        #[cfg(any(debug_assertions, feature = "debug-mode"))]
        Key::F3 => MapKeyboardCommand::CycleDeveloperDiagnostics,
        _ => return None,
    };
    Some(KeyboardCommand::Map(command))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RawPointerEvent {
    pub button: MouseButton,
    pub pressed: bool,
    pub position: ScreenPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PointerContext {
    pub preferences_dialog_open: bool,
    pub state_apply_dialog_open: bool,
    pub inspector_picker_open: bool,
    pub left_press_consumed: bool,
    pub has_interface: bool,
    pub has_canvas: bool,
    pub map_contains_cursor: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PointerClassification {
    pub clear_tooltip: bool,
    pub command: PointerCommand,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PointerCommand {
    PreferencesPrimaryClick { position: ScreenPosition },
    ConsumePrimaryRelease,
    StateApplyDialogClick { position: ScreenPosition },
    CaptureByStateApplyDialog,
    InspectorPickerPrimaryGesture { position: ScreenPosition },
    RoutePrimaryClick { position: ScreenPosition },
    BeginPrimaryGesture { position: ScreenPosition },
    CaptureByInterface,
    EndPrimaryGesture,
    BeginPan,
    EndPan,
    PickBrush { position: ScreenPosition },
    Ignore,
}

pub(crate) fn classify_pointer(raw: RawPointerEvent, context: PointerContext) -> InputCommand {
    let command = if context.preferences_dialog_open {
        if raw.button == MouseButton::Left && raw.pressed {
            PointerCommand::PreferencesPrimaryClick {
                position: raw.position,
            }
        } else {
            PointerCommand::Ignore
        }
    } else if raw.button == MouseButton::Left && !raw.pressed && context.left_press_consumed {
        PointerCommand::ConsumePrimaryRelease
    } else if raw.pressed && raw.button == MouseButton::Left && context.state_apply_dialog_open {
        PointerCommand::StateApplyDialogClick {
            position: raw.position,
        }
    } else if context.state_apply_dialog_open {
        PointerCommand::CaptureByStateApplyDialog
    } else if raw.pressed && raw.button == MouseButton::Left && context.inspector_picker_open {
        PointerCommand::InspectorPickerPrimaryGesture {
            position: raw.position,
        }
    } else if raw.pressed && raw.button == MouseButton::Left && context.has_interface {
        PointerCommand::RoutePrimaryClick {
            position: raw.position,
        }
    } else if !raw.pressed
        && raw.button == MouseButton::Left
        && context.has_interface
        && context.has_canvas
    {
        PointerCommand::EndPrimaryGesture
    } else if raw.pressed
        && raw.button == MouseButton::Right
        && context.has_canvas
        && context.map_contains_cursor
    {
        PointerCommand::BeginPan
    } else if !raw.pressed
        && raw.button == MouseButton::Right
        && context.has_interface
        && context.has_canvas
    {
        PointerCommand::EndPan
    } else if raw.pressed
        && raw.button == MouseButton::Middle
        && context.has_canvas
        && context.map_contains_cursor
    {
        PointerCommand::PickBrush {
            position: raw.position,
        }
    } else {
        PointerCommand::Ignore
    };
    InputCommand::Pointer(PointerClassification {
        clear_tooltip: raw.pressed && !context.preferences_dialog_open,
        command,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrimaryClickOutcome {
    InterfaceButton,
    Map,
    CapturedByInterface,
}

pub(crate) fn classify_primary_click(
    outcome: PrimaryClickOutcome,
    position: ScreenPosition,
) -> PointerCommand {
    match outcome {
        PrimaryClickOutcome::InterfaceButton => PointerCommand::CaptureByInterface,
        PrimaryClickOutcome::Map => PointerCommand::BeginPrimaryGesture { position },
        PrimaryClickOutcome::CapturedByInterface => PointerCommand::CaptureByInterface,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CursorContext {
    pub state_apply_dialog_open: bool,
    pub state_brush_stroking: bool,
    pub province_paint_drag_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CursorCommand {
    CaptureByStateApplyDialog,
    UpdateInterface {
        position: ScreenPosition,
        map_gesture: Option<MapGestureCommand>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MapGestureCommand {
    ContinueStateBrush { position: ScreenPosition },
    ContinueProvincePaint { position: ScreenPosition },
}

pub(crate) fn classify_cursor(position: ScreenPosition, context: CursorContext) -> InputCommand {
    let command = if context.state_apply_dialog_open {
        CursorCommand::CaptureByStateApplyDialog
    } else {
        let map_gesture = if context.state_brush_stroking {
            Some(MapGestureCommand::ContinueStateBrush { position })
        } else if context.province_paint_drag_active {
            Some(MapGestureCommand::ContinueProvincePaint { position })
        } else {
            None
        };
        CursorCommand::UpdateInterface {
            position,
            map_gesture,
        }
    };
    InputCommand::Cursor(command)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RelativeMotionCommand {
    PanBy { delta: Vector2<f64> },
}

pub(crate) fn classify_relative_motion(delta: Vector2<f64>) -> InputCommand {
    InputCommand::RelativeMotion(RelativeMotionCommand::PanBy { delta })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RawWheelEvent {
    pub delta_y: f64,
    pub position: ScreenPosition,
    pub mods: KeyMods,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WheelContext {
    pub validation_results_scrolled: bool,
    pub inspector_scrolled: bool,
    pub state_lasso_active: bool,
    pub map_contains_cursor: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum WheelCommand {
    CapturedByUi,
    ChangeBrushRadius {
        delta_y: f64,
    },
    Zoom {
        delta_y: f64,
        position: ScreenPosition,
    },
    Ignore,
}

pub(crate) fn classify_wheel(raw: RawWheelEvent, context: WheelContext) -> InputCommand {
    let command = if context.validation_results_scrolled {
        WheelCommand::CapturedByUi
    } else if !context.inspector_scrolled && raw.mods.shift && !context.state_lasso_active {
        WheelCommand::ChangeBrushRadius {
            delta_y: raw.delta_y,
        }
    } else if context.map_contains_cursor {
        WheelCommand::Zoom {
            delta_y: raw.delta_y,
            position: raw.position,
        }
    } else {
        WheelCommand::Ignore
    };
    InputCommand::Wheel(command)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileDropCommand {
    OpenProject,
}

pub(crate) fn classify_file_drop() -> InputCommand {
    InputCommand::FileDrop(FileDropCommand::OpenProject)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ViewportCommand {
    RebuildInterface { viewport: Viewport },
}

pub(crate) fn classify_viewport(viewport: Viewport) -> InputCommand {
    InputCommand::Viewport(ViewportCommand::RebuildInterface { viewport })
}

pub(crate) fn lasso_mode_from_mods(mods: KeyMods) -> Option<LassoSelectionMode> {
    if mods.alt {
        Some(LassoSelectionMode::Remove)
    } else if mods.shift {
        Some(LassoSelectionMode::Add)
    } else {
        None
    }
}

fn workspace_shortcut(key: Key, mods: KeyMods, current: WorkspaceMode) -> Option<WorkspaceMode> {
    if !mods.ctrl || mods.alt {
        return None;
    }
    match key {
        Key::D1 => Some(WorkspaceMode::Provinces),
        Key::D2 => Some(WorkspaceMode::States),
        Key::Tab => Some(current.next()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: InputContext = InputContext {
        has_canvas: true,
        preferences_dialog_open: false,
        state_apply_dialog_open: false,
        inspector_picker_open: false,
        inspector_search_focused: false,
        property_editor_open: false,
        editing_locked: false,
        workspace: Some(WorkspaceMode::Provinces),
    };

    fn key(key: Key, mods: KeyMods) -> RawKeyEvent {
        RawKeyEvent {
            key,
            pressed: true,
            mods,
        }
    }

    #[test]
    fn captured_keyboard_precedes_map_shortcuts() {
        let context = InputContext {
            property_editor_open: true,
            ..EMPTY
        };
        assert_eq!(
            classify_key(
                key(
                    Key::S,
                    KeyMods {
                        ctrl: true,
                        ..KeyMods::default()
                    }
                ),
                context
            ),
            InputCommand::Keyboard(KeyboardCommand::PropertyEditor(
                PropertyEditorKeyCommand::Captured
            ))
        );
    }

    #[test]
    fn map_click_requires_ui_to_defer_it() {
        let position = [40.0, 50.0];
        assert_eq!(
            classify_primary_click(PrimaryClickOutcome::Map, position),
            PointerCommand::BeginPrimaryGesture { position }
        );
        assert_eq!(
            classify_primary_click(PrimaryClickOutcome::CapturedByInterface, position),
            PointerCommand::CaptureByInterface
        );
    }

    #[test]
    fn pointer_classification_preserves_primary_secondary_and_middle_routes() {
        let context = PointerContext {
            preferences_dialog_open: false,
            state_apply_dialog_open: false,
            inspector_picker_open: false,
            left_press_consumed: false,
            has_interface: true,
            has_canvas: true,
            map_contains_cursor: true,
        };
        let position = [40.0, 50.0];
        assert!(matches!(
            classify_pointer(
                RawPointerEvent {
                    button: MouseButton::Left,
                    pressed: true,
                    position
                },
                context
            ),
            InputCommand::Pointer(PointerClassification {
                command: PointerCommand::RoutePrimaryClick { .. },
                ..
            })
        ));
        assert!(matches!(
            classify_pointer(
                RawPointerEvent {
                    button: MouseButton::Right,
                    pressed: true,
                    position
                },
                context
            ),
            InputCommand::Pointer(PointerClassification {
                command: PointerCommand::BeginPan,
                ..
            })
        ));
        assert!(matches!(
            classify_pointer(
                RawPointerEvent {
                    button: MouseButton::Middle,
                    pressed: true,
                    position
                },
                context
            ),
            InputCommand::Pointer(PointerClassification {
                command: PointerCommand::PickBrush { .. },
                ..
            })
        ));
    }

    #[test]
    fn release_is_not_recaptured_after_a_consumed_primary_press() {
        let context = PointerContext {
            left_press_consumed: true,
            ..PointerContext {
                preferences_dialog_open: false,
                state_apply_dialog_open: false,
                inspector_picker_open: false,
                left_press_consumed: false,
                has_interface: true,
                has_canvas: true,
                map_contains_cursor: true,
            }
        };
        assert!(matches!(
            classify_pointer(
                RawPointerEvent {
                    button: MouseButton::Left,
                    pressed: false,
                    position: [0.0, 0.0]
                },
                context
            ),
            InputCommand::Pointer(PointerClassification {
                command: PointerCommand::ConsumePrimaryRelease,
                ..
            })
        ));
    }

    #[test]
    fn cursor_motion_only_classifies_active_gestures() {
        assert_eq!(
            classify_cursor(
                [3.0, 4.0],
                CursorContext {
                    state_apply_dialog_open: false,
                    state_brush_stroking: false,
                    province_paint_drag_active: false,
                }
            ),
            InputCommand::Cursor(CursorCommand::UpdateInterface {
                position: [3.0, 4.0],
                map_gesture: None,
            })
        );
        assert!(matches!(
            classify_cursor(
                [3.0, 4.0],
                CursorContext {
                    state_apply_dialog_open: false,
                    state_brush_stroking: true,
                    province_paint_drag_active: true,
                }
            ),
            InputCommand::Cursor(CursorCommand::UpdateInterface {
                position: [3.0, 4.0],
                map_gesture: Some(MapGestureCommand::ContinueStateBrush { .. })
            })
        ));
    }

    #[test]
    fn relative_motion_is_a_camera_intent_without_canvas_state() {
        assert_eq!(
            classify_relative_motion([2.0, -3.0]),
            InputCommand::RelativeMotion(RelativeMotionCommand::PanBy { delta: [2.0, -3.0] })
        );
    }

    #[test]
    fn wheel_preserves_shift_radius_and_plain_zoom_behavior() {
        let map_context = WheelContext {
            validation_results_scrolled: false,
            inspector_scrolled: false,
            state_lasso_active: false,
            map_contains_cursor: true,
        };
        assert!(matches!(
            classify_wheel(
                RawWheelEvent {
                    delta_y: 1.0,
                    position: [1.0, 2.0],
                    mods: KeyMods::default()
                },
                map_context
            ),
            InputCommand::Wheel(WheelCommand::Zoom { .. })
        ));
        assert!(matches!(
            classify_wheel(
                RawWheelEvent {
                    delta_y: 1.0,
                    position: [1.0, 2.0],
                    mods: KeyMods {
                        shift: true,
                        ..KeyMods::default()
                    },
                },
                map_context
            ),
            InputCommand::Wheel(WheelCommand::ChangeBrushRadius { .. })
        ));
        assert!(matches!(
            classify_wheel(
                RawWheelEvent {
                    delta_y: 1.0,
                    position: [1.0, 2.0],
                    mods: KeyMods {
                        shift: true,
                        ..KeyMods::default()
                    },
                },
                WheelContext {
                    state_lasso_active: true,
                    ..map_context
                }
            ),
            InputCommand::Wheel(WheelCommand::Zoom { .. })
        ));
        assert!(matches!(
            classify_wheel(
                RawWheelEvent {
                    delta_y: 1.0,
                    position: [1.0, 2.0],
                    mods: KeyMods::default(),
                },
                WheelContext {
                    inspector_scrolled: true,
                    ..map_context
                }
            ),
            InputCommand::Wheel(WheelCommand::Zoom { .. })
        ));
    }

    #[test]
    fn save_undo_redo_tool_and_view_shortcuts_keep_their_existing_bindings() {
        let ctrl = KeyMods {
            ctrl: true,
            ..KeyMods::default()
        };
        assert_eq!(
            classify_key(key(Key::S, ctrl), EMPTY),
            InputCommand::Keyboard(KeyboardCommand::Application(ApplicationCommand::Save))
        );
        assert!(matches!(
            classify_key(key(Key::Z, ctrl), EMPTY),
            InputCommand::Keyboard(KeyboardCommand::Map(MapKeyboardCommand::Undo))
        ));
        assert!(matches!(
            classify_key(key(Key::Y, ctrl), EMPTY),
            InputCommand::Keyboard(KeyboardCommand::Map(MapKeyboardCommand::Redo))
        ));
        assert!(matches!(
            classify_key(key(Key::B, KeyMods::default()), EMPTY),
            InputCommand::Keyboard(KeyboardCommand::Map(MapKeyboardCommand::ToolShortcut(
                ToolShortcut::BrushOrBucket
            )))
        ));
        assert!(matches!(
            classify_key(key(Key::D7, KeyMods::default()), EMPTY),
            InputCommand::Keyboard(KeyboardCommand::Map(MapKeyboardCommand::ChangeMapView(
                MapViewMode::Political
            )))
        ));
    }

    #[test]
    fn workspace_shortcuts_keep_plain_view_shortcuts_available() {
        let ctrl = KeyMods {
            ctrl: true,
            ..KeyMods::default()
        };
        assert_eq!(
            classify_key(key(Key::D1, ctrl), EMPTY),
            InputCommand::Keyboard(KeyboardCommand::SetWorkspace(WorkspaceMode::Provinces))
        );
        assert!(matches!(
            classify_key(key(Key::D1, KeyMods::default()), EMPTY),
            InputCommand::Keyboard(KeyboardCommand::Map(MapKeyboardCommand::ChangeMapView(
                MapViewMode::ProvinceColors
            )))
        ));
    }

    #[test]
    fn map_view_shortcuts_keep_the_existing_canonical_mapping() {
        let expected = [
            (Key::D1, MapViewMode::ProvinceColors),
            (Key::D2, MapViewMode::ProvinceTypes),
            (Key::D3, MapViewMode::Terrain),
            (Key::D4, MapViewMode::Continents),
            (Key::D5, MapViewMode::Coastal),
            (Key::D6, MapViewMode::States),
            (Key::D7, MapViewMode::Political),
            (Key::D8, MapViewMode::States),
        ];
        for (event_key, view) in expected {
            assert_eq!(
                classify_key(key(event_key, KeyMods::default()), EMPTY),
                InputCommand::Keyboard(KeyboardCommand::Map(MapKeyboardCommand::ChangeMapView(
                    view
                )))
            );
        }
    }

    #[test]
    fn tool_and_lasso_modifier_shortcuts_remain_intent_only() {
        assert!(matches!(
            classify_key(key(Key::A, KeyMods::default()), EMPTY),
            InputCommand::Keyboard(KeyboardCommand::Map(MapKeyboardCommand::SetPaintAreaTool))
        ));
        assert!(matches!(
            classify_key(
                key(
                    Key::L,
                    KeyMods {
                        alt: true,
                        ..KeyMods::default()
                    }
                ),
                EMPTY
            ),
            InputCommand::Keyboard(KeyboardCommand::Map(MapKeyboardCommand::ToolShortcut(
                ToolShortcut::Lasso {
                    selection_mode: Some(LassoSelectionMode::Remove)
                }
            )))
        ));
        assert!(matches!(
            classify_key(
                key(
                    Key::Space,
                    KeyMods {
                        shift: true,
                        ..KeyMods::default()
                    }
                ),
                EMPTY
            ),
            InputCommand::Keyboard(KeyboardCommand::Map(MapKeyboardCommand::CycleToolBrush {
                shift: true
            }))
        ));
    }

    #[test]
    fn save_and_state_apply_modals_capture_their_existing_keyboard_routes() {
        assert_eq!(
            classify_key(
                key(Key::Escape, KeyMods::default()),
                InputContext {
                    editing_locked: true,
                    ..EMPTY
                }
            ),
            InputCommand::Keyboard(KeyboardCommand::EditingLocked(
                EditingLockedKeyCommand::Cancel
            ))
        );
        assert_eq!(
            classify_key(
                key(Key::Backspace, KeyMods::default()),
                InputContext {
                    state_apply_dialog_open: true,
                    ..EMPTY
                }
            ),
            InputCommand::Keyboard(KeyboardCommand::StateApplyDialog(
                StateApplyDialogKeyCommand::Backspace
            ))
        );
    }

    #[test]
    fn modal_pointer_capture_and_file_drop_are_explicit() {
        let context = PointerContext {
            preferences_dialog_open: false,
            state_apply_dialog_open: true,
            inspector_picker_open: false,
            left_press_consumed: false,
            has_interface: true,
            has_canvas: true,
            map_contains_cursor: true,
        };
        assert!(matches!(
            classify_pointer(
                RawPointerEvent {
                    button: MouseButton::Right,
                    pressed: true,
                    position: [0.0, 0.0]
                },
                context
            ),
            InputCommand::Pointer(PointerClassification {
                command: PointerCommand::CaptureByStateApplyDialog,
                ..
            })
        ));
        assert_eq!(
            classify_file_drop(),
            InputCommand::FileDrop(FileDropCommand::OpenProject)
        );
    }
}
