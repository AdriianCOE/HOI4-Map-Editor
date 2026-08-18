pub mod alerts;
pub mod canvas;
pub mod format;
pub mod inspector;
pub mod inspector_controls;
pub mod interface;
pub mod map;
pub mod map_layers;
pub mod project;
pub mod state;

use defy::Contextualize;
use glutin::window::CursorIcon;
use graphics::context::Context;
use graphics::{Transformed, Viewport};
use opengl_graphics::{Filter, GlGraphics, TextureSettings};
use piston::input::{Key, MouseButton};
use vecmath::Vector2;

use self::alerts::Alerts;
use self::canvas::{Canvas, InspectorExternalRequest, StateApplyDialogAction, ToolMode, ViewMode};
use self::interface::{ButtonId, Interface, StateActionAvailability, get_interface};
use self::map::ProvinceSaveMode;
use self::map_layers::WorkspaceMode;
use self::project::{
    Hoi4Project, LassoSelectionMode, MapViewMode, ProjectPathError, ProjectPaths,
    ProvinceInclusionMode, StateBrushMode, StateFillMode,
};
use crate::config::{ConfigIssue, FileFingerprint, GlobalConfig, ProjectConfig, SaveConfigError};
use crate::error::Error;
use crate::events::{EventHandler, KeyMods};
use crate::font::{self, FONT_SIZE};
use crate::util::files::{IntoLocation, Location};

use std::env;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
enum PreferencesDialog {
    Global {
        original: GlobalConfig,
        draft: GlobalConfig,
        fingerprint: Option<FileFingerprint>,
        selected: usize,
    },
    Project {
        root: PathBuf,
        draft: ProjectConfig,
        fingerprint: Option<FileFingerprint>,
        selected: usize,
    },
}

pub mod colors {
    use graphics::types::Color as DrawColor;

    pub const BLACK: DrawColor = [0.0, 0.0, 0.0, 1.0];
    pub const WHITE: DrawColor = [1.0, 1.0, 1.0, 1.0];
    pub const WHITE_T: DrawColor = [1.0, 1.0, 1.0, 0.25];
    pub const WHITE_TT: DrawColor = [1.0, 1.0, 1.0, 0.015625];
    pub const PROBLEM: DrawColor = [0.875, 0.0, 0.0, 1.0];
    pub const WARNING: DrawColor = [0.875, 0.5, 0.0, 1.0];
    pub const NEUTRAL: DrawColor = [0.25, 0.25, 0.25, 1.0];
    pub const OVERLAY_T: DrawColor = [0.0, 0.0, 0.0, 0.5];

    pub const ADJ_LAND: DrawColor = [0.2, 0.6, 1.0 / 3.0, 1.0];
    pub const ADJ_SEA: DrawColor = [0.2, 1.0 / 3.0, 0.6, 1.0];
    pub const ADJ_IMPASSABLE: DrawColor = [0.0, 0.0, 0.0, 1.0];

    const fn color_inactive(value: u16) -> DrawColor {
        let v = value as f32 / 256.0;
        [v, v, v, 1.0]
    }

    const fn color_active(value: u16) -> DrawColor {
        let v = value as f32 / 256.0;
        [v, v, v * 2.0, 1.0]
    }

    pub const BUTTON: DrawColor = color_inactive(48);
    pub const BUTTON_ACTIVE: DrawColor = color_active(48 + 16);
    pub const BUTTON_HOVER: DrawColor = color_inactive(96);
    pub const BUTTON_HOVER_ACTIVE: DrawColor = color_active(96 + 16);

    pub const BUTTON_TOOLBAR: DrawColor = color_inactive(32);
    pub const BUTTON_TOOLBAR_ACTIVE: DrawColor = color_active(32 + 16);
    pub const BUTTON_TOOLBAR_HOVER: DrawColor = color_inactive(80);
    pub const BUTTON_TOOLBAR_HOVER_ACTIVE: DrawColor = color_active(80 + 16);
}

pub type FontGlyphCache = font::MultiFontGlyphCache<'static>;

pub struct App {
    pub canvas: Option<Canvas>,
    pub alerts: Alerts,
    pub glyph_cache: FontGlyphCache,
    pub interface: Option<Interface>,
    pub painting: bool,
    left_press_consumed: bool,
    global_config: GlobalConfig,
    global_config_fingerprint: Option<FileFingerprint>,
    global_config_issue: Option<ConfigIssue>,
    preferences_dialog: Option<PreferencesDialog>,
    viewport: Option<Viewport>,
}

impl EventHandler for App {
    fn new(_gl: &mut GlGraphics) -> Self {
        let texture_settings = TextureSettings::new().filter(Filter::Nearest);
        let mut glyph_cache = font::get_glyph_cache(texture_settings);
        glyph_cache.preload_printable_ascii(font::FONT_SIZE);

        let loaded = GlobalConfig::load().ok();
        let global_config = loaded
            .as_ref()
            .map(|loaded| loaded.value.clone())
            .unwrap_or_default();
        crate::localization::set_language(&global_config.language);
        App {
            canvas: None,
            alerts: Alerts::new(5.0),
            glyph_cache,
            interface: None,
            painting: false,
            left_press_consumed: false,
            global_config,
            global_config_fingerprint: loaded
                .as_ref()
                .and_then(|loaded| loaded.fingerprint.clone()),
            global_config_issue: loaded.and_then(|loaded| loaded.issue),
            preferences_dialog: None,
            viewport: None,
        }
    }

    fn on_init(&mut self) {
        if let Some(issue) = self.global_config_issue.take() {
            self.alerts.push(Err(format!(
                "Global configuration could not be loaded. Defaults are being used for this session. The original file was not modified.\n{issue}"
            )));
        }
        if let Some(path) = std::env::args().nth(1) {
            self.raw_open_map_at(path);
        } else if self.global_config.open_last_project
            && let Some(path) = self.global_config.last_project.clone()
        {
            if path.exists() {
                self.raw_open_map_at(path);
            } else {
                self.alerts.push(Err(
                    "The last project no longer exists. Open another HOI4 mod to replace it.",
                ));
            }
        } else {
            #[cfg(any(debug_assertions, feature = "debug-mode"))]
            self.raw_open_map_at("./test_map.zip");
            #[cfg(not(any(debug_assertions, feature = "debug-mode")))]
            self.alerts.push(Ok(
                "HOI4 Map Editor is a standalone tool; no HOI4 playset is required. Open or drag a mod root. Use Help > User Guide for setup help and Help > Open Logs Folder for diagnostics.",
            ));
        };
    }

    fn on_render(&mut self, ctx: Context, cursor_pos: Option<Vector2<f64>>, gl: &mut GlGraphics) {
        let Some(viewport) = ctx.viewport else { return };
        self.viewport = Some(viewport);
        let ictx = self.get_interface_draw_context();
        let inspector_width = self
            .canvas
            .as_ref()
            .map(Canvas::inspector_reserved_width)
            .unwrap_or(0.0);
        let interactive_cursor = self
            .canvas
            .as_ref()
            .is_none_or(|canvas| !canvas.state_apply_dialog_is_open())
            .then_some(cursor_pos)
            .flatten();
        let interface = get_interface(&mut self.interface, viewport);
        interface.set_inspector_width(inspector_width);
        interface.set_tooltip_delay_ms(self.global_config.tooltip_delay_ms);
        graphics::clear(colors::NEUTRAL, gl);

        if let Some(canvas) = &mut self.canvas {
            canvas.draw(
                ctx,
                interface,
                &mut self.glyph_cache,
                interactive_cursor,
                gl,
            );
        };

        self.alerts.draw(ctx, interface, &mut self.glyph_cache, gl);
        interface.draw(ctx, ictx, interactive_cursor, &mut self.glyph_cache, gl);
        if let Some(canvas) = self.canvas.as_ref() {
            canvas.draw_state_apply_dialog(ctx, interface, &mut self.glyph_cache, gl);
        }
        if let Some(dialog) = self.preferences_dialog.as_ref() {
            draw_preferences_dialog(ctx, dialog, &mut self.glyph_cache, gl);
        }
    }

    fn on_update(&mut self, dt: f32) {
        if !self.alerts.is_active() {
            self.alerts.tick(dt);
        };
        if let Some(interface) = self.interface.as_mut() {
            interface.tick(dt);
        }
        if self
            .canvas
            .as_mut()
            .is_some_and(Canvas::take_state_apply_ready_for_confirmation)
        {
            self.action_save_map();
        }
    }

    fn on_key(&mut self, key: Key, state: bool, mods: KeyMods, cursor_pos: Option<Vector2<f64>>) {
        if state && self.preferences_dialog.is_some() {
            self.handle_preferences_key(key, mods);
            return;
        }
        let Some(interface) = self.interface.as_ref() else {
            return;
        };
        if state
            && self
                .canvas
                .as_ref()
                .is_some_and(Canvas::state_apply_dialog_is_open)
        {
            if key == Key::Backspace
                && let Some(canvas) = self.canvas.as_mut()
            {
                canvas.province_removal_backspace();
                return;
            }
            if key == Key::Escape
                && let Some(canvas) = self.canvas.as_mut()
            {
                canvas.close_state_apply_dialog();
            }
            return;
        }
        if state
            && self
                .canvas
                .as_ref()
                .is_some_and(Canvas::inspector_picker_is_open)
        {
            let canvas = self.canvas.as_mut().unwrap();
            match key {
                Key::Escape => canvas.inspector_picker_cancel(),
                Key::Backspace => canvas.inspector_picker_backspace(),
                Key::Up => canvas.inspector_picker_move(false),
                Key::Down => canvas.inspector_picker_move(true),
                Key::PageUp => canvas.inspector_picker_page(false),
                Key::PageDown => canvas.inspector_picker_page(true),
                Key::Home => canvas.inspector_picker_home(),
                Key::End => canvas.inspector_picker_end(),
                Key::Return => canvas.inspector_picker_confirm(&mut self.alerts),
                _ => {}
            }
            return;
        }
        if state
            && self
                .canvas
                .as_ref()
                .is_some_and(Canvas::inspector_search_is_focused)
        {
            match key {
                Key::Escape => self.canvas.as_mut().unwrap().inspector_search_cancel(),
                Key::Backspace => self.canvas.as_mut().unwrap().inspector_search_backspace(),
                Key::Up => self.canvas.as_mut().unwrap().inspector_search_move(false),
                Key::Down => self.canvas.as_mut().unwrap().inspector_search_move(true),
                Key::Return => self.canvas.as_mut().unwrap().inspector_search_select(
                    interface,
                    mods.ctrl,
                    &mut self.alerts,
                ),
                _ => return,
            }
            return;
        }
        if state
            && self
                .canvas
                .as_ref()
                .is_some_and(Canvas::property_editor_is_open)
        {
            match key {
                Key::Escape => {
                    if !self
                        .canvas
                        .as_mut()
                        .is_some_and(Canvas::cancel_state_property_field_edit)
                    {
                        self.resolve_property_draft();
                    }
                    return;
                }
                Key::Tab => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.state_property_editor_next_field(mods.shift);
                    }
                    return;
                }
                Key::Backspace => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.state_property_editor_backspace();
                    }
                    return;
                }
                Key::Delete => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.state_property_editor_clear_field();
                    }
                    return;
                }
                Key::Return => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.apply_state_property_draft(&mut self.alerts);
                    }
                    return;
                }
                Key::A if mods.ctrl => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.state_property_editor_select_all();
                    }
                    return;
                }
                _ => return,
            }
        }
        if state
            && self
                .canvas
                .as_ref()
                .is_some_and(Canvas::save_blocks_editing)
        {
            if key == Key::S && mods.ctrl && !mods.shift {
                self.action_save_map();
            } else if key == Key::Escape
                && self.canvas.as_ref().is_some_and(Canvas::save_can_cancel)
            {
                if let Some(canvas) = self.canvas.as_mut() {
                    canvas.cancel_active_save(&mut self.alerts);
                }
            } else {
                self.alerts.push(Err(
                    "Editing is locked while Save, export, or recovery is active",
                ));
            }
            return;
        }
        if state && key == Key::F && mods.ctrl {
            if let Some(canvas) = self.canvas.as_mut() {
                canvas.focus_map_search();
            }
            return;
        }
        if state
            && let Some(current) = self.canvas.as_ref().map(Canvas::workspace_mode)
            && let Some(workspace) = workspace_shortcut(key, mods, current)
        {
            self.action_set_workspace(workspace);
            return;
        }
        match (&mut self.canvas, state, key) {
            (_, state, Key::Tab) => self.alerts.set_state(state),
            (_, true, Key::O) if mods.ctrl => self.action_open_map(mods.alt),
            (Some(_), true, Key::S) if mods.ctrl && mods.shift => {
                self.action_export_province_map(mods.alt)
            }
            (Some(_), true, Key::S) if mods.ctrl => self.action_save_map(),
            (Some(_), true, Key::R) if mods.ctrl && mods.alt => self.action_reveal_map(),
            (Some(canvas), true, Key::Z) if mods.ctrl => canvas.undo(&mut self.alerts),
            (Some(canvas), true, Key::Y) if mods.ctrl => canvas.redo(&mut self.alerts),
            (Some(canvas), true, Key::D) if mods.ctrl && mods.shift => {
                if (!canvas.has_unsaved_state_edits() && !canvas.property_draft_is_modified())
                    || msg_dialog_discard_state_edits()
                {
                    canvas.discard_state_edit_session(&mut self.alerts);
                }
            }
            (Some(canvas), true, Key::M) if !mods.shift => {
                if canvas
                    .move_confirmation_message()
                    .as_deref()
                    .is_none_or(msg_dialog_confirm_state_batch)
                {
                    canvas.move_selected_provinces_to_target(&mut self.alerts);
                }
            }
            (Some(canvas), true, Key::Delete) => {
                if canvas
                    .unassign_confirmation_message()
                    .as_deref()
                    .is_none_or(msg_dialog_confirm_state_batch)
                {
                    canvas.unassign_selected_provinces(&mut self.alerts);
                }
            }
            (Some(canvas), true, Key::Space) => {
                canvas.cycle_tool_brush(interface, cursor_pos, mods.shift, &mut self.alerts)
            }
            (Some(canvas), true, Key::Escape) => {
                if !canvas.map_tag_picker_cancel()
                    && !canvas.cancel_state_brush()
                    && !canvas.cancel_state_lasso()
                    && !canvas.cancel_state_fill()
                    && !canvas.clear_state_selection()
                {
                    canvas.cancel_tool();
                }
            }
            (Some(canvas), true, Key::Return) => {
                if !canvas.confirm_state_fill(&mut self.alerts)
                    && !canvas.advance_state_lasso(&mut self.alerts)
                {
                    canvas.finish_tool(&mut self.alerts);
                }
            }
            (Some(canvas), true, Key::F) => {
                canvas.activate_state_fill(StateFillMode::HoveredProvince, &mut self.alerts)
            }
            (Some(canvas), true, Key::C) if mods.shift => canvas.calculate_coastal_provinces(),
            (Some(canvas), true, Key::R) if mods.shift => canvas.calculate_recolor_map(),
            (Some(canvas), true, Key::P) if mods.shift => canvas.display_problems(&mut self.alerts),
            (Some(canvas), true, Key::M) if mods.shift => canvas.tool.cycle_brush_mask(),
            (Some(canvas), true, Key::H) => canvas.camera.reset(),
            (Some(canvas), true, Key::A) => canvas.set_tool_mode(ToolMode::PaintArea),
            (Some(canvas), true, Key::B) => {
                if canvas.is_state_workspace() {
                    canvas.activate_state_brush(StateBrushMode::AssignToTarget, &mut self.alerts);
                } else {
                    canvas.set_tool_mode(ToolMode::PaintBucket);
                }
            }
            (Some(canvas), true, Key::L) => {
                if canvas.is_state_workspace() {
                    canvas.activate_state_lasso(lasso_mode_from_mods(mods), &mut self.alerts);
                } else {
                    canvas.set_tool_mode(ToolMode::new_lasso());
                }
            }
            (Some(_), true, Key::D1) => {
                self.action_change_map_view_mode(MapViewMode::ProvinceColors)
            }
            (Some(_), true, Key::D2) => {
                self.action_change_map_view_mode(MapViewMode::ProvinceTypes)
            }
            (Some(_), true, Key::D3) => self.action_change_map_view_mode(MapViewMode::Terrain),
            (Some(_), true, Key::D4) => self.action_change_map_view_mode(MapViewMode::Continents),
            (Some(_), true, Key::D5) => self.action_change_map_view_mode(MapViewMode::Coastal),
            (Some(_), true, Key::D6) => self.action_change_map_view_mode(MapViewMode::States),
            (Some(_), true, Key::D7) => self.action_change_map_view_mode(MapViewMode::Political),
            (Some(_), true, Key::D8) => self.action_change_map_view_mode(MapViewMode::States),
            (Some(canvas), true, Key::D9) => {
                canvas.cycle_province_label_mode(&mut self.alerts);
            }
            #[cfg(any(debug_assertions, feature = "debug-mode"))]
            (Some(canvas), true, Key::F3) => {
                canvas.cycle_developer_diagnostics(&mut self.alerts);
            }
            _ => (),
        };
    }

    fn on_text(&mut self, text: String) {
        if let Some(canvas) = self.canvas.as_mut() {
            canvas.input_province_removal_text(&text);
            canvas.input_state_property_text(&text);
        }
    }

    fn on_mouse(&mut self, button: MouseButton, state: bool, mods: KeyMods, pos: Vector2<f64>) {
        if self.preferences_dialog.is_some() {
            if button == MouseButton::Left && state {
                self.handle_preferences_click(pos);
            }
            return;
        }
        if button == MouseButton::Left && !state && self.left_press_consumed {
            self.left_press_consumed = false;
            return;
        }
        if state && let Some(interface) = self.interface.as_mut() {
            interface.clear_tooltip();
        }
        if state
            && button == MouseButton::Left
            && self
                .canvas
                .as_ref()
                .is_some_and(Canvas::state_apply_dialog_is_open)
        {
            self.left_press_consumed = true;
            let action = match (self.interface.as_ref(), self.canvas.as_mut()) {
                (Some(interface), Some(canvas)) => {
                    canvas.state_apply_dialog_click(interface, pos, &mut self.alerts)
                }
                _ => StateApplyDialogAction::None,
            };
            match action {
                StateApplyDialogAction::ConfirmSave => self.action_save_map(),
                StateApplyDialogAction::ConfirmProjectSave => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.start_project_save(true, &mut self.alerts);
                    }
                }
                StateApplyDialogAction::OpenSource(path) => {
                    let result = open_source_with(&path, |path| open_file_default(path));
                    self.handle_result_none(result);
                }
                StateApplyDialogAction::CopyDetails(text) => {
                    let result = copy_text_to_clipboard(&text);
                    self.handle_result_none(result);
                }
                StateApplyDialogAction::ChooseImageOverlay => {
                    if let Some(path) = file_dialog_image_overlay()
                        && let Some(canvas) = self.canvas.as_mut()
                    {
                        canvas.load_custom_image_overlay(path, &mut self.alerts);
                    }
                }
                StateApplyDialogAction::UseProjectHeightmap => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.use_project_heightmap(&mut self.alerts);
                    }
                }
                StateApplyDialogAction::DecreaseImageOverlayOpacity => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.adjust_image_overlay_opacity(-0.1, &mut self.alerts);
                    }
                }
                StateApplyDialogAction::IncreaseImageOverlayOpacity => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.adjust_image_overlay_opacity(0.1, &mut self.alerts);
                    }
                }
                StateApplyDialogAction::ClearImageOverlay => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.clear_image_overlay(&mut self.alerts);
                    }
                }
                StateApplyDialogAction::ConfirmProvinceTransfer => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.confirm_province_removal(true, &mut self.alerts);
                    }
                }
                StateApplyDialogAction::ConfirmProvinceReferenceRemoval => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        canvas.confirm_province_removal(false, &mut self.alerts);
                    }
                }
                StateApplyDialogAction::None => {}
            }
            return;
        }
        if self
            .canvas
            .as_ref()
            .is_some_and(Canvas::state_apply_dialog_is_open)
        {
            return;
        }
        if state
            && button == MouseButton::Left
            && self
                .canvas
                .as_ref()
                .is_some_and(Canvas::inspector_picker_is_open)
        {
            self.left_press_consumed = true;
            self.action_activate_tool(pos, mods);
            return;
        }
        let ictx = self.get_interface_draw_context();
        let Some(interface) = self.interface.as_mut() else {
            return;
        };
        match (&mut self.canvas, state, button) {
            (_, true, MouseButton::Left) => match interface.on_mouse_click(pos, ictx) {
                Ok(id) => {
                    self.left_press_consumed = true;
                    self.action_interface_button(id);
                }
                Err(true) => self.action_activate_tool(pos, mods),
                Err(false) => self.left_press_consumed = true,
            },
            (Some(_), false, MouseButton::Left) => self.action_deactivate_tool(),
            (Some(canvas), true, MouseButton::Right) if interface.map_contains(pos) => {
                canvas.camera.set_panning(true)
            }
            (Some(canvas), false, MouseButton::Right) => canvas.camera.set_panning(false),
            (Some(canvas), true, MouseButton::Middle) if interface.map_contains(pos) => {
                canvas.pick_tool_brush(interface, pos, &mut self.alerts)
            }
            _ => (),
        };
    }

    fn on_mouse_position(&mut self, pos: Vector2<f64>, mods: KeyMods) {
        if self
            .canvas
            .as_ref()
            .is_some_and(Canvas::state_apply_dialog_is_open)
        {
            if let Some(interface) = self.interface.as_mut() {
                interface.clear_tooltip();
            }
            return;
        }
        let ictx = self.get_interface_draw_context();
        let Some(interface) = self.interface.as_mut() else {
            return;
        };
        interface.on_mouse_position(pos, ictx);
        if let Some(canvas) = &mut self.canvas {
            if self.painting && canvas.state_brush_is_stroking() {
                canvas.update_state_brush(interface, pos);
            } else if self.painting
                && !canvas.is_state_workspace()
                && canvas.tool.mode == ToolMode::PaintArea
                && canvas.view_mode() != ViewMode::Adjacencies
            {
                // Mouse movement should not activate the tool for the paint bucket and lasso tools
                canvas.activate_tool(interface, pos, mods.shift, &mut self.alerts);
            };
        };
    }

    fn on_mouse_relative(&mut self, rel: Vector2<f64>) {
        if let Some(canvas) = &mut self.canvas {
            canvas.camera.on_mouse_relative(rel);
        };
    }

    fn on_mouse_scroll(&mut self, [_, y]: Vector2<f64>, mods: KeyMods, cursor_pos: Vector2<f64>) {
        let Some(interface) = self.interface.as_ref() else {
            return;
        };
        let Some(canvas) = &mut self.canvas else {
            return;
        };

        if canvas.validation_results_scroll(y) {
            return;
        }
        if !canvas.inspector_scroll(interface, cursor_pos, y)
            && mods.shift
            && !canvas.state_lasso_is_active()
        {
            canvas.change_tool_radius(y);
        } else if interface.map_contains(cursor_pos) {
            canvas.camera.on_mouse_zoom(interface, y, cursor_pos);
        };
    }

    fn on_file_drop(&mut self, path: PathBuf) {
        if self
            .canvas
            .as_ref()
            .is_some_and(Canvas::save_blocks_editing)
        {
            self.alerts
                .push(Err("Finish the active save before opening another project"));
            return;
        }
        if !self.resolve_property_draft() {
            return;
        }
        if self
            .canvas
            .as_ref()
            .is_some_and(Canvas::has_unsaved_state_edits)
            && !msg_dialog_discard_state_edits()
        {
            return;
        }
        self.raw_open_map_at(path);
    }

    fn on_resize(&mut self, viewport: Viewport) {
        self.interface = Some(Interface::new(viewport));
    }

    fn on_unfocus(&mut self) {
        self.painting = false;
        self.left_press_consumed = false;
        if let Some(canvas) = self.canvas.as_mut() {
            canvas.cancel_state_brush();
            canvas.camera.set_panning(false);
        }
        self.alerts.set_state(false);
    }

    fn on_window_state(&mut self, position: Option<[i32; 2]>, size: [u32; 2], maximized: bool) {
        self.global_config.window.width = size[0].max(384);
        self.global_config.window.height = size[1].max(256);
        self.global_config.window.maximized = maximized;
        if let Some([x, y]) = position {
            self.global_config.window.x = Some(x);
            self.global_config.window.y = Some(y);
        }
    }

    fn on_close(&mut self) -> bool {
        if self.canvas.as_ref().is_some_and(Canvas::save_blocks_close) {
            self.alerts.push(Err(
                "Cannot close while a save commit, rollback, or recovery is pending",
            ));
            return false;
        }
        if self.canvas.as_ref().is_some_and(Canvas::save_is_running) {
            if let Some(canvas) = self.canvas.as_mut() {
                canvas.cancel_active_save(&mut self.alerts);
            }
            return false;
        }
        self.save_existing_global_preferences_on_close();
        if self
            .canvas
            .as_ref()
            .is_some_and(Canvas::property_draft_is_modified)
        {
            let province = self
                .canvas
                .as_ref()
                .is_some_and(Canvas::province_data_editor_is_open);
            if !msg_dialog_discard_property_draft_exit(province) {
                return false;
            }
            if let Some(canvas) = self.canvas.as_mut() {
                canvas.discard_state_property_draft(&mut self.alerts);
            }
        } else if let Some(canvas) = self.canvas.as_mut() {
            canvas.discard_unmodified_property_draft();
        }
        if self
            .canvas
            .as_ref()
            .is_some_and(Canvas::has_unsaved_state_edits)
        {
            let eligible = self
                .canvas
                .as_ref()
                .and_then(|canvas| canvas.state_save_confirmation_message().ok());
            if let Some(message) = eligible {
                return match msg_dialog_state_edits_exit(&message) {
                    StateExitResolution::Save => {
                        if let Some(canvas) = self.canvas.as_mut() {
                            canvas.start_state_save(&mut self.alerts);
                        }
                        false
                    }
                    StateExitResolution::Discard => true,
                    StateExitResolution::KeepEditing => false,
                };
            }
            return msg_dialog_discard_state_edits_exit();
        }
        if self.is_canvas_modified() {
            if msg_dialog_unsaved_changes_exit() {
                self.action_save_map();
            };
        };
        true
    }

    fn get_cursor(&self) -> CursorIcon {
        CursorIcon::Crosshair
    }
}

impl App {
    fn save_existing_global_preferences_on_close(&mut self) {
        if self.global_config_fingerprint.is_none() {
            return;
        }
        if let Some(viewport) = self.viewport {
            self.global_config.window.width = viewport.window_size[0].round().max(384.0) as u32;
            self.global_config.window.height = viewport.window_size[1].round().max(256.0) as u32;
        }
        match self
            .global_config
            .save(self.global_config_fingerprint.as_ref(), false, false)
        {
            Ok(fingerprint) => self.global_config_fingerprint = Some(fingerprint),
            Err(error) => eprintln!("Could not persist global preferences on close: {error}"),
        }
    }

    fn open_global_settings(&mut self) {
        self.preferences_dialog = Some(PreferencesDialog::Global {
            original: self.global_config.clone(),
            draft: self.global_config.clone(),
            fingerprint: self.global_config_fingerprint.clone(),
            selected: 0,
        });
    }

    fn open_project_settings(&mut self) {
        let Some(root) = self
            .canvas
            .as_ref()
            .and_then(Canvas::project)
            .map(|project| project.paths.root.clone())
        else {
            self.alerts
                .push(Err("Project Settings require a loaded HOI4 mod."));
            return;
        };
        match ProjectConfig::load(&root) {
            Ok(loaded) => {
                if let Some(issue) = &loaded.issue {
                    self.alerts.push(Err(format!(
                        "{}\n{issue}",
                        crate::localization::tr("config.invalid_project")
                    )));
                }
                self.preferences_dialog = Some(PreferencesDialog::Project {
                    root,
                    draft: loaded.value,
                    fingerprint: loaded.fingerprint,
                    selected: 0,
                });
            }
            Err(error) => self
                .alerts
                .push(Err(format!("Cannot open Project Settings: {error}"))),
        }
    }

    fn handle_preferences_key(&mut self, key: Key, mods: KeyMods) {
        let rows = preference_rows(&self.preferences_dialog);
        match key {
            Key::Escape => self.cancel_preferences_dialog(),
            Key::Tab | Key::Down => {
                if let Some(selected) = selected_preference_mut(&mut self.preferences_dialog) {
                    *selected = if mods.shift {
                        selected.checked_sub(1).unwrap_or(rows - 1)
                    } else {
                        (*selected + 1) % rows
                    };
                }
            }
            Key::Up => {
                if let Some(selected) = selected_preference_mut(&mut self.preferences_dialog) {
                    *selected = selected.checked_sub(1).unwrap_or(rows - 1);
                }
            }
            Key::Left => self.adjust_preference(false),
            Key::Right => self.adjust_preference(true),
            Key::Return | Key::Space => self.activate_preference(),
            _ => {}
        }
    }

    fn handle_preferences_click(&mut self, position: Vector2<f64>) {
        let Some(viewport) = self.viewport else {
            return;
        };
        let [x, y, width, _] = preferences_rect(viewport.window_size);
        if position[0] < x
            || position[0] > x + width
            || position[1] < y + 46.0
            || position[1] >= y + 46.0 + preference_rows(&self.preferences_dialog) as f64 * 30.0
        {
            return;
        }
        let row = ((position[1] - y - 46.0) / 30.0) as usize;
        let rows = preference_rows(&self.preferences_dialog);
        if let Some(selected) = selected_preference_mut(&mut self.preferences_dialog) {
            *selected = row.min(rows.saturating_sub(1));
        }
        if matches!(row, 5 | 4) {
            self.adjust_preference(position[0] >= x + width / 2.0);
        } else {
            self.activate_preference();
        }
    }

    fn adjust_preference(&mut self, increase: bool) {
        match self.preferences_dialog.as_mut() {
            Some(PreferencesDialog::Global {
                draft, selected, ..
            }) => match *selected {
                4 => {
                    let values = [0, 400, 800];
                    let current = values
                        .iter()
                        .position(|value| *value == draft.tooltip_delay_ms)
                        .unwrap_or(1);
                    let next = if increase {
                        (current + 1).min(values.len() - 1)
                    } else {
                        current.saturating_sub(1)
                    };
                    draft.tooltip_delay_ms = values[next];
                }
                5 => {
                    draft.max_undo_states = if increase {
                        (draft.max_undo_states + 1).min(500)
                    } else {
                        draft.max_undo_states.saturating_sub(1).max(1)
                    };
                }
                _ => self.activate_preference(),
            },
            Some(PreferencesDialog::Project {
                draft, selected, ..
            }) if *selected == 4 => {
                draft.extra_warnings.few_shared_borders_threshold = if increase {
                    draft
                        .extra_warnings
                        .few_shared_borders_threshold
                        .saturating_add(1)
                } else {
                    draft
                        .extra_warnings
                        .few_shared_borders_threshold
                        .saturating_sub(1)
                        .max(1)
                };
            }
            _ => self.activate_preference(),
        }
    }

    fn activate_preference(&mut self) {
        let selected = selected_preference_mut(&mut self.preferences_dialog)
            .map(|selected| *selected)
            .unwrap_or_default();
        match self.preferences_dialog.as_mut() {
            Some(PreferencesDialog::Global { draft, .. }) => match selected {
                0 => {
                    draft.language = crate::localization::next_language(&draft.language).to_owned();
                    crate::localization::set_language(&draft.language);
                    self.interface = None;
                }
                1 => draft.open_last_project = !draft.open_last_project,
                2 => {
                    draft.remember_workspace = !draft.remember_workspace;
                    draft.remember_map_views = draft.remember_workspace;
                }
                3 => draft.remember_overlays = !draft.remember_overlays,
                4 | 5 => self.adjust_preference(true),
                6 => draft.change_view_mode_on_undo = !draft.change_view_mode_on_undo,
                7 => {
                    draft.window = Default::default();
                    draft.workspace.state_inspector_visible = true;
                    self.interface = None;
                }
                8 => {
                    *draft = GlobalConfig::default();
                    crate::localization::set_language(&draft.language);
                    self.interface = None;
                }
                9 => self.cancel_preferences_dialog(),
                10 => self.save_global_preferences(),
                _ => {}
            },
            Some(PreferencesDialog::Project { draft, root, .. }) => match selected {
                0 => {
                    if draft.preserve_ids
                        && !confirm_dialog(
                            "Disable Preserve IDs?",
                            "Disabling Preserve IDs can break references in states, strategic regions, and other files. Changing this setting does not alter IDs until a Province Save.",
                        )
                    {
                        return;
                    }
                    draft.preserve_ids = !draft.preserve_ids;
                }
                1 => draft.generate_coastal_on_save = !draft.generate_coastal_on_save,
                2 => {
                    draft.extra_warnings.lone_pixels = !draft.extra_warnings.lone_pixels;
                    update_extra_warnings_enabled(draft);
                }
                3 => {
                    draft.extra_warnings.few_shared_borders =
                        !draft.extra_warnings.few_shared_borders;
                    update_extra_warnings_enabled(draft);
                }
                4 => self.adjust_preference(true),
                5 => *draft = ProjectConfig::default(),
                6 => {
                    let path = ProjectConfig::path(root);
                    if path.exists() {
                        self.handle_result_none(open_file_default(&path));
                    } else {
                        self.alerts
                            .push(Err("No project.toml exists yet. Choose Save to create it."));
                    }
                }
                7 => match draft.validate() {
                    Ok(()) => self.alerts.push(Ok(format!(
                        "Project configuration is valid: {} effective terrains.",
                        draft.terrains.len().saturating_sub(1)
                    ))),
                    Err(error) => self.alerts.push(Err(error.to_string())),
                },
                8 => self.cancel_preferences_dialog(),
                9 => self.save_project_preferences(),
                _ => {}
            },
            None => {}
        }
    }

    fn cancel_preferences_dialog(&mut self) {
        if let Some(PreferencesDialog::Global { original, .. }) = self.preferences_dialog.take() {
            crate::localization::set_language(&original.language);
            self.interface = None;
        } else {
            self.preferences_dialog = None;
        }
    }

    fn save_global_preferences(&mut self) {
        let Some(PreferencesDialog::Global {
            draft, fingerprint, ..
        }) = self.preferences_dialog.clone()
        else {
            return;
        };
        let saved = match draft.save(fingerprint.as_ref(), false, false) {
            Err(SaveConfigError::ChangedExternally)
                if confirm_dialog(
                    "Configuration changed outside the editor",
                    "Reload is safest. Choose Yes to Save Anyway using the current Settings draft, or No to cancel.",
                ) =>
            {
                draft.save(None, true, false)
            }
            Err(SaveConfigError::FutureSchema(_))
                if confirm_dialog(
                    "Newer configuration schema",
                    "This file was created by a newer editor. Choose Yes only to explicitly replace its known settings while preserving unknown keys.",
                ) =>
            {
                draft.save(fingerprint.as_ref(), true, true)
            }
            Err(SaveConfigError::Invalid(_))
                if confirm_dialog(
                    "Invalid configuration file",
                    "The existing file cannot be safely edited. Choose Yes to back it up as config.toml.bak and replace it with this validated Settings draft.",
                ) =>
            {
                draft.replace_invalid_file()
            }
            result => result,
        };
        match saved {
            Ok(fingerprint) => {
                self.global_config = draft.clone();
                self.global_config_fingerprint = Some(fingerprint);
                crate::localization::set_language(&draft.language);
                let project = self
                    .canvas
                    .as_ref()
                    .and_then(Canvas::project)
                    .and_then(|project| ProjectConfig::load(&project.paths.root).ok())
                    .map(|loaded| loaded.value)
                    .unwrap_or_default();
                if let Some(canvas) = self.canvas.as_mut() {
                    canvas.apply_config(crate::config::Config::from_parts(&draft, &project));
                }
                self.preferences_dialog = None;
                self.interface = None;
                self.alerts
                    .push(Ok(crate::localization::tr("config.saved")));
            }
            Err(error) => self.alerts.push(Err(error.to_string())),
        }
    }

    fn save_project_preferences(&mut self) {
        let Some(PreferencesDialog::Project {
            root,
            draft,
            fingerprint,
            ..
        }) = self.preferences_dialog.clone()
        else {
            return;
        };
        let saved = match draft.save(&root, fingerprint.as_ref(), false, false) {
            Err(SaveConfigError::ChangedExternally)
                if confirm_dialog(
                    "Project configuration changed outside the editor",
                    "Choose Yes to Save Anyway using this draft, or No to cancel and reopen Project Settings.",
                ) =>
            {
                draft.save(&root, None, true, false)
            }
            Err(SaveConfigError::FutureSchema(_))
                if confirm_dialog(
                    "Newer project configuration schema",
                    "Choose Yes only to explicitly replace its known project settings.",
                ) =>
            {
                draft.save(&root, fingerprint.as_ref(), true, true)
            }
            Err(SaveConfigError::Invalid(_))
                if confirm_dialog(
                    "Invalid project configuration",
                    "Choose Yes to back it up as project.toml.bak and replace it with this validated Project Settings draft.",
                ) =>
            {
                draft.replace_invalid_file(&root)
            }
            result => result,
        };
        match saved {
            Ok(_) => {
                if let Some(canvas) = self.canvas.as_mut() {
                    canvas.apply_config(crate::config::Config::from_parts(
                        &self.global_config,
                        &draft,
                    ));
                }
                self.preferences_dialog = None;
                self.alerts
                    .push(Ok(crate::localization::tr("config.saved")));
            }
            Err(error) => self.alerts.push(Err(error.to_string())),
        }
    }

    fn resolve_property_draft(&mut self) -> bool {
        let province = self
            .canvas
            .as_ref()
            .is_some_and(Canvas::province_data_editor_is_open);
        let Some(canvas) = self.canvas.as_mut() else {
            return true;
        };
        if !canvas.property_editor_is_open() {
            return true;
        }
        if canvas.discard_unmodified_property_draft() {
            return true;
        }
        match msg_dialog_resolve_property_draft(province) {
            DraftResolution::Apply => canvas.apply_state_property_draft(&mut self.alerts),
            DraftResolution::Discard => {
                canvas.discard_state_property_draft(&mut self.alerts);
                true
            }
            DraftResolution::KeepEditing => false,
        }
    }

    fn get_interface_draw_context(&self) -> InterfaceDrawContext {
        match &self.canvas {
            Some(canvas) => {
                let (province_modified, pending_states) = canvas.workspace_dirty_summary();
                InterfaceDrawContext {
                    map_view_mode: Some(canvas.map_view_mode()),
                    view_mode: (!canvas.is_state_workspace()).then_some(canvas.view_mode()),
                    selected_tool: (!canvas.is_state_workspace()).then_some(
                        match &canvas.tool.mode {
                            ToolMode::PaintArea => 0,
                            ToolMode::PaintBucket => 1,
                            ToolMode::Lasso(_) => 2,
                        },
                    ),
                    state_tool: canvas
                        .is_state_workspace()
                        .then_some(canvas.state_toolbar_tool()),
                    enabled_options: canvas.enabled_options(),
                    available_options: canvas.available_options(),
                    states_available: canvas.has_state_workspace(),
                    state_actions: canvas.state_action_availability(),
                    blocks_tooltips: canvas.blocks_interface_tooltips(),
                    province_modified,
                    pending_states,
                }
            }
            None => InterfaceDrawContext {
                map_view_mode: None,
                view_mode: None,
                selected_tool: None,
                state_tool: None,
                enabled_options: [false; 6],
                available_options: [false; 6],
                states_available: false,
                state_actions: StateActionAvailability::default(),
                blocks_tooltips: false,
                province_modified: false,
                pending_states: 0,
            },
        }
    }

    fn is_canvas_modified(&self) -> bool {
        if let Some(canvas) = &self.canvas {
            canvas.has_unsaved_province_edits() || canvas.has_unsaved_state_edits()
        } else {
            false
        }
    }

    pub fn action_interface_button(&mut self, id: ButtonId) {
        use self::interface::ButtonId::*;
        if id == ToolbarEditSettings {
            self.open_global_settings();
            return;
        }
        if id == ToolbarFileProjectSettings {
            self.open_project_settings();
            return;
        }
        match id {
            WorkspaceProvinces => {
                self.action_set_workspace(WorkspaceMode::Provinces);
                return;
            }
            WorkspaceStates => {
                self.action_set_workspace(WorkspaceMode::States);
                return;
            }
            WorkspaceReviewChanges => {
                if let Some(canvas) = self.canvas.as_mut() {
                    canvas.validate_project_for_ui(&mut self.alerts);
                }
                return;
            }
            WorkspaceApplyToMod => {
                self.action_save_map();
                return;
            }
            _ => {}
        }
        if self
            .canvas
            .as_ref()
            .is_some_and(Canvas::save_blocks_editing)
            && !matches!(
                id,
                ToolbarFileSave
                    | ToolbarPatchCancelSave
                    | ToolbarPatchViewSaveReport
                    | ToolbarPatchRecoverSave
            )
        {
            self.alerts.push(Err(
                "Editing is locked while Save, export, or recovery is active",
            ));
            return;
        }
        if id == ToolbarEditActivateStateLasso {
            if self.resolve_property_draft()
                && let Some(canvas) = self.canvas.as_mut()
            {
                canvas.activate_state_lasso(None, &mut self.alerts);
            }
            return;
        }
        if matches!(
            id,
            ToolbarEditActivateStateBrushAssign | ToolbarEditActivateStateBrushUnassign
        ) {
            if self.resolve_property_draft()
                && let Some(canvas) = self.canvas.as_mut()
            {
                let mode = if id == ToolbarEditActivateStateBrushAssign {
                    StateBrushMode::AssignToTarget
                } else {
                    StateBrushMode::Unassign
                };
                canvas.activate_state_brush(mode, &mut self.alerts);
            }
            return;
        }
        if matches!(
            id,
            ToolbarEditActivateStateFillHovered
                | ToolbarEditActivateStateFillConnectedState
                | ToolbarEditActivateStateFillConnectedUnassigned
                | ToolbarEditActivateStateFillWholeState
        ) {
            if self.resolve_property_draft()
                && let Some(canvas) = self.canvas.as_mut()
            {
                let mode = match id {
                    ToolbarEditActivateStateFillHovered => StateFillMode::HoveredProvince,
                    ToolbarEditActivateStateFillConnectedState => StateFillMode::ConnectedSameState,
                    ToolbarEditActivateStateFillConnectedUnassigned => {
                        StateFillMode::ConnectedUnassigned
                    }
                    ToolbarEditActivateStateFillWholeState => StateFillMode::WholeSourceState,
                    _ => unreachable!(),
                };
                canvas.activate_state_fill(mode, &mut self.alerts);
            }
            return;
        }
        if id == ToolbarEditClearStateSelection {
            if self.resolve_property_draft()
                && let Some(canvas) = self.canvas.as_mut()
            {
                if canvas.is_state_workspace() {
                    canvas.clear_state_selection();
                } else {
                    canvas.cancel_tool();
                }
            }
            return;
        }
        if id == ToolbarViewChooseBaseGameDefinitions {
            if let Some(root) = file_dialog_base_game_definitions()
                && let Some(canvas) = self.canvas.as_mut()
            {
                canvas.set_base_game_definition_root(Some(root), &mut self.alerts);
            }
            return;
        }
        if matches!(
            id,
            ToolbarEditMoveSelectedToTarget | ToolbarEditUnassignSelected
        ) && !self.resolve_property_draft()
        {
            return;
        }
        match (&mut self.canvas, id) {
            (
                _,
                WorkspaceProvinces | WorkspaceStates | WorkspaceReviewChanges | WorkspaceApplyToMod,
            ) => unreachable!(),
            (_, ToolbarFileOpenFileArchive) => self.action_open_map(true),
            (_, ToolbarFileOpenFolder) => self.action_open_map(false),
            (_, ToolbarFileProjectSettings | ToolbarEditSettings) => unreachable!(),
            (Some(_), ToolbarFileSave | ToolbarPatchSaveStateFiles) => self.action_save_map(),
            (Some(_), ToolbarFileSaveAsArchive) => self.action_export_province_map(true),
            (Some(_), ToolbarFileSaveAsFolder) => self.action_export_province_map(false),
            (Some(_), ToolbarFileReveal) => self.action_reveal_map(),
            (Some(_), ToolbarFileExportLandMap) => self.action_export_land_map(),
            (Some(_), ToolbarFileExportTerrainMap) => self.action_export_terrain_map(),
            (Some(canvas), ToolbarEditUndo) => canvas.undo(&mut self.alerts),
            (Some(canvas), ToolbarEditRedo) => canvas.redo(&mut self.alerts),
            (Some(canvas), ToolbarEditFindMap) => canvas.focus_map_search(),
            (Some(canvas), ToolbarEditNewState) => {
                canvas.open_new_state_editor(&mut self.alerts);
            }
            (Some(canvas), ToolbarEditRemoveState) => {
                canvas.open_remove_state_editor(&mut self.alerts);
            }
            (Some(canvas), ToolbarEditStateProperties) => {
                canvas.open_state_property_editor(&mut self.alerts);
            }
            (Some(canvas), ToolbarEditProvinceData) => {
                canvas.open_province_data_editor(&mut self.alerts);
            }
            (Some(canvas), ToolbarEditRemoveProvince) => {
                canvas.open_province_removal_dialog(&mut self.alerts);
            }
            (Some(canvas), ToolbarEditCoastal) => canvas.calculate_coastal_provinces(),
            (Some(canvas), ToolbarEditRecolor) => canvas.calculate_recolor_map(),
            (Some(canvas), ToolbarEditProblems) => canvas.display_problems(&mut self.alerts),
            (Some(canvas), ToolbarEditToggleLassoSnap) => canvas.toggle_lasso_snap(),
            (Some(canvas), ToolbarEditNextMaskMode) => canvas.tool.cycle_brush_mask(),
            (Some(canvas), ToolbarEditSelectTargetStateProvinces) => {
                canvas.select_target_state_provinces(&mut self.alerts);
            }
            (Some(_), ToolbarEditActivateStateLasso) => unreachable!(),
            (Some(canvas), ToolbarEditStateLassoReplace) => {
                canvas.set_state_lasso_mode(LassoSelectionMode::Replace, &mut self.alerts);
            }
            (Some(canvas), ToolbarEditStateLassoAdd) => {
                canvas.set_state_lasso_mode(LassoSelectionMode::Add, &mut self.alerts);
            }
            (Some(canvas), ToolbarEditStateLassoRemove) => {
                canvas.set_state_lasso_mode(LassoSelectionMode::Remove, &mut self.alerts);
            }
            (Some(canvas), ToolbarEditStateLassoCentroid) => {
                canvas.set_state_lasso_inclusion(
                    ProvinceInclusionMode::CentroidInside,
                    &mut self.alerts,
                );
            }
            (Some(canvas), ToolbarEditStateLassoAnyIntersection) => {
                canvas.set_state_lasso_inclusion(
                    ProvinceInclusionMode::AnyIntersection,
                    &mut self.alerts,
                );
            }
            (Some(canvas), ToolbarEditStateLassoMajority) => {
                canvas.set_state_lasso_inclusion(
                    ProvinceInclusionMode::MajorityInside,
                    &mut self.alerts,
                );
            }
            (Some(canvas), ToolbarEditConfirmStateLasso) => {
                canvas.confirm_state_lasso(&mut self.alerts)
            }
            (Some(canvas), ToolbarEditCancelStateLasso) => {
                if !canvas.cancel_state_lasso() {
                    self.alerts.push(Err("No active state lasso"));
                }
            }
            (
                Some(_),
                ToolbarEditActivateStateBrushAssign | ToolbarEditActivateStateBrushUnassign,
            ) => unreachable!(),
            (Some(canvas), ToolbarEditCancelStateBrush) => {
                if !canvas.cancel_state_brush() {
                    self.alerts.push(Err("No active State Brush"));
                }
            }
            (Some(canvas), ToolbarEditConfirmStateFill) => {
                canvas.confirm_state_fill(&mut self.alerts);
            }
            (Some(canvas), ToolbarEditCancelStateFill) => {
                if !canvas.cancel_state_fill() {
                    self.alerts.push(Err("No active State Fill"));
                }
            }
            (
                Some(_),
                ToolbarEditActivateStateFillHovered
                | ToolbarEditActivateStateFillConnectedState
                | ToolbarEditActivateStateFillConnectedUnassigned
                | ToolbarEditActivateStateFillWholeState,
            ) => unreachable!(),
            (Some(canvas), ToolbarEditMoveSelectedToTarget) => {
                if canvas
                    .move_confirmation_message()
                    .as_deref()
                    .is_none_or(msg_dialog_confirm_state_batch)
                {
                    canvas.move_selected_provinces_to_target(&mut self.alerts);
                }
            }
            (Some(canvas), ToolbarEditUnassignSelected) => {
                if canvas
                    .unassign_confirmation_message()
                    .as_deref()
                    .is_none_or(msg_dialog_confirm_state_batch)
                {
                    canvas.unassign_selected_provinces(&mut self.alerts);
                }
            }
            (Some(_), ToolbarEditClearStateSelection) => unreachable!(),
            (Some(canvas), ToolbarEditDiscardStateSession) => {
                if (!canvas.has_unsaved_state_edits() && !canvas.property_draft_is_modified())
                    || msg_dialog_discard_state_edits()
                {
                    canvas.discard_state_edit_session(&mut self.alerts);
                }
            }
            (Some(canvas), ToolbarPatchGenerate | ToolbarPatchRegenerate) => {
                canvas.generate_patch_preview(&mut self.alerts);
            }
            (Some(canvas), ToolbarPatchPreviousFile) => {
                canvas.select_patch_preview_file(-1, &mut self.alerts);
            }
            (Some(canvas), ToolbarPatchNextFile) => {
                canvas.select_patch_preview_file(1, &mut self.alerts);
            }
            (Some(canvas), ToolbarPatchValidate) => {
                canvas.validate_project_for_ui(&mut self.alerts);
            }
            (Some(canvas), ToolbarPatchValidateReview) => {
                canvas.start_round_trip_validation(true, &mut self.alerts);
            }
            (Some(canvas), ToolbarPatchCancelValidation) => {
                canvas.cancel_round_trip_validation(&mut self.alerts);
            }
            (Some(canvas), ToolbarPatchViewValidationReport) => {
                canvas.view_round_trip_report(&mut self.alerts);
            }
            (Some(canvas), ToolbarPatchClearValidation) => {
                canvas.clear_round_trip_report(&mut self.alerts);
            }
            (Some(canvas), ToolbarPatchCancelSave) => {
                canvas.cancel_state_save(&mut self.alerts);
            }
            (Some(canvas), ToolbarPatchViewSaveReport) => {
                canvas.view_state_save_report(&mut self.alerts);
            }
            (Some(canvas), ToolbarPatchRecoverSave) => {
                canvas.recover_state_save(&mut self.alerts);
            }
            (Some(canvas), ToolbarPatchClear) => {
                canvas.clear_patch_preview(&mut self.alerts);
            }
            (Some(_), ToolbarViewMode1) => {
                self.action_change_map_view_mode(MapViewMode::ProvinceColors)
            }
            (Some(_), ToolbarViewMode2) => {
                self.action_change_map_view_mode(MapViewMode::ProvinceTypes)
            }
            (Some(_), ToolbarViewMode3) => self.action_change_map_view_mode(MapViewMode::Terrain),
            (Some(_), ToolbarViewMode4) => {
                self.action_change_map_view_mode(MapViewMode::Continents)
            }
            (Some(_), ToolbarViewMode5) => self.action_change_map_view_mode(MapViewMode::Coastal),
            (Some(_), ToolbarViewMode6) => self.action_change_map_view_mode(MapViewMode::States),
            (Some(_), ToolbarViewProvinceMap) => {
                self.action_change_map_view_mode(MapViewMode::ProvinceColors)
            }
            (Some(_), ToolbarViewStateMap) => self.action_change_map_view_mode(MapViewMode::States),
            (Some(_), ToolbarViewPoliticalMap) => {
                self.action_change_map_view_mode(MapViewMode::Political)
            }
            (Some(canvas), ToolbarViewToggleAdjacencies | SidebarOptionAdjacencies) => {
                canvas.toggle_adjacencies_overlay()
            }
            (Some(canvas), ToolbarViewToggleImageOverlay | SidebarOptionImageOverlay) => {
                canvas.toggle_image_overlay(&mut self.alerts)
            }
            (Some(canvas), ToolbarViewImageOverlayPanel) => canvas.open_image_overlay_panel(),
            (Some(canvas), ToolbarImageChoose) => {
                if let Some(path) = file_dialog_image_overlay() {
                    canvas.load_custom_image_overlay(path, &mut self.alerts);
                }
            }
            (Some(canvas), ToolbarImageUseProjectHeightmap) => {
                canvas.use_project_heightmap(&mut self.alerts)
            }
            (Some(canvas), ToolbarImageToggleVisible) => {
                canvas.toggle_image_overlay(&mut self.alerts)
            }
            (Some(canvas), ToolbarImageOpacityDown) => {
                canvas.adjust_image_overlay_opacity(-0.1, &mut self.alerts)
            }
            (Some(canvas), ToolbarImageOpacityUp) => {
                canvas.adjust_image_overlay_opacity(0.1, &mut self.alerts)
            }
            (Some(canvas), ToolbarImageClear) => canvas.clear_image_overlay(&mut self.alerts),
            (Some(canvas), ToolbarViewToggleStateBoundaries | SidebarOptionStateBoundaries) => {
                canvas.toggle_state_boundaries(&mut self.alerts)
            }
            (Some(canvas), ToolbarViewToggleProvinceIds | SidebarOptionProvinceIds) => {
                canvas.toggle_province_ids()
            }
            (
                Some(canvas),
                ToolbarViewToggleProvinceBoundaries | SidebarOptionProvinceBoundaries,
            ) => canvas.toggle_province_boundaries(),
            (Some(canvas), ToolbarViewToggleRiverOverlay | SidebarOptionRiverOverlay) => {
                if canvas.toggle_river_overlay() {
                    self.alerts
                        .push(Err("You must have a map with rivers.bmp to use this"));
                }
            }
            (Some(canvas), ToolbarEditAdjacencies) => {
                canvas.set_view_mode(&mut self.alerts, ViewMode::Adjacencies)
            }
            (Some(canvas), ToolbarViewToggleStateInspector) => {
                canvas.cycle_state_inspector_visibility(&mut self.alerts);
            }
            (Some(canvas), ToolbarViewCycleProvinceLabels) => {
                canvas.cycle_province_label_mode(&mut self.alerts);
            }
            (Some(canvas), ToolbarViewCycleDeveloperDiagnostics) => {
                canvas.cycle_developer_diagnostics(&mut self.alerts);
            }
            (Some(canvas), ToolbarViewClearBaseGameDefinitions) => {
                canvas.set_base_game_definition_root(None, &mut self.alerts);
            }
            (Some(_), ToolbarViewChooseBaseGameDefinitions) => unreachable!(),
            (Some(canvas), ToolbarViewResetZoom) => canvas.camera.reset(),
            (_, ToolbarHelpAbout) => {
                show_about_dialog();
            }
            (_, ToolbarHelpCopyVersion) => {
                let summary = format!(
                    "{}\n{}\n\nBased on ScottyThePilot's HOI4 Province Editor.\nDeveloped and extended by Adrian Costa.\n\nUnofficial community tool. Not affiliated with or endorsed by Paradox Interactive.\nRepository: https://github.com/AdriianCOE/hoi4_state_editor",
                    crate::diagnostic_summary().trim_end(),
                    crate::PRODUCT_SUBTITLE,
                );
                self.handle_result_none(copy_text_to_clipboard(&summary));
            }
            (_, ToolbarHelpOpenLogs) => {
                let logs = crate::log_directory();
                let result = std::fs::create_dir_all(&logs)
                    .map_err(|error| Error::from(format!("Unable to create logs folder: {error}")))
                    .and_then(|_| open_file_default(&logs));
                self.handle_result_none(result);
            }
            (_, ToolbarViewFontLicense) => self.handle_result_none(font::view_font_license()),
            (Some(canvas), SidebarToolPaintArea) => canvas.set_tool_mode(ToolMode::PaintArea),
            (Some(canvas), SidebarToolPaintBucket) => canvas.set_tool_mode(ToolMode::PaintBucket),
            (Some(canvas), SidebarToolLasso) => canvas.set_tool_mode(ToolMode::new_lasso()),
            (Some(canvas), SidebarStateSelect) => canvas.activate_state_select(),
            (Some(canvas), SidebarStatePan) => canvas.activate_state_pan(),
            (Some(canvas), SidebarStateLasso) => {
                canvas.activate_state_lasso(None, &mut self.alerts)
            }
            (Some(canvas), SidebarStateBrush) => {
                canvas.activate_state_brush(StateBrushMode::AssignToTarget, &mut self.alerts)
            }
            (Some(canvas), SidebarStateFill) => {
                canvas.activate_state_fill(StateFillMode::ConnectedUnassigned, &mut self.alerts)
            }
            #[cfg(any(debug_assertions, feature = "debug-mode"))]
            (Some(canvas), ToolbarDebugValidatePixelCounts) => {
                canvas.validate_pixel_counts(&mut self.alerts)
            }
            #[cfg(any(debug_assertions, feature = "debug-mode"))]
            (_, ToolbarDebugTriggerCrash) => panic!("debug crash"),
            (None, _) => self
                .alerts
                .push(Err("You must have a map loaded to use this")),
        };
        if self.global_config.remember_overlays
            && let Some(canvas) = self.canvas.as_ref()
        {
            let enabled = canvas.enabled_options();
            self.global_config.overlays.rivers = enabled[0];
            self.global_config.overlays.adjacencies = enabled[1];
            self.global_config.overlays.province_ids = enabled[2];
            self.global_config.overlays.province_boundaries = enabled[3];
            self.global_config.overlays.state_boundaries = enabled[4];
            self.global_config.overlays.image = enabled[5];
        }
    }

    fn action_set_workspace(&mut self, workspace: WorkspaceMode) {
        let workspace_will_change = self
            .canvas
            .as_ref()
            .is_some_and(|canvas| canvas.workspace_mode() != workspace);
        if workspace_will_change && !self.resolve_property_draft() {
            return;
        }
        let changed = self
            .canvas
            .as_mut()
            .is_some_and(|canvas| canvas.set_workspace_mode(workspace, &mut self.alerts));
        if changed {
            self.painting = false;
            if self.global_config.remember_workspace {
                self.global_config.workspace.last_workspace = match workspace {
                    WorkspaceMode::Provinces => "provinces",
                    WorkspaceMode::States => "states",
                }
                .to_owned();
            }
            if let Some(interface) = self.interface.as_mut() {
                interface.clear_tooltip();
            }
        }
    }

    fn action_activate_tool(&mut self, pos: Vector2<f64>, mods: KeyMods) {
        if self
            .canvas
            .as_ref()
            .is_some_and(Canvas::save_blocks_editing)
        {
            self.alerts.push(Err(
                "Editing is locked while Save, export, or recovery is active",
            ));
            return;
        }
        if let (Some(interface), Some(canvas)) = (self.interface.as_ref(), self.canvas.as_mut())
            && canvas.pick_tag_from_map(interface, pos, &mut self.alerts)
        {
            return;
        }
        let (inspector_consumed, inspector_request) =
            match (self.interface.as_ref(), self.canvas.as_mut()) {
                (Some(interface), Some(canvas)) => {
                    canvas.state_inspector_click(interface, pos, &mut self.alerts)
                }
                _ => (false, None),
            };
        if let Some(request) = inspector_request {
            self.handle_inspector_external_request(request);
        }
        if inspector_consumed {
            return;
        }
        let editor_consumed = self.interface.as_ref().is_some_and(|interface| {
            self.canvas.as_mut().is_some_and(|canvas| {
                canvas.state_property_editor_click(interface, pos, &mut self.alerts)
            })
        });
        if editor_consumed {
            return;
        }
        let resolve_draft = self.interface.as_ref().is_some_and(|interface| {
            self.canvas.as_ref().is_some_and(|canvas| {
                canvas.is_state_workspace()
                    && !canvas.state_lasso_is_active()
                    && canvas.state_click_would_change_property_draft(interface, pos)
            })
        });
        if resolve_draft && !self.resolve_property_draft() {
            return;
        }
        let Some(interface) = self.interface.as_ref() else {
            return;
        };
        let Some(canvas) = &mut self.canvas else {
            return;
        };
        if canvas.state_pan_is_active() && canvas.is_state_workspace() {
            canvas.camera.set_panning(true);
            self.painting = true;
            return;
        }
        let mut inspector_request = None;
        if canvas.is_state_workspace() {
            if canvas.state_fill_is_active() {
                canvas.preview_state_fill(interface, pos, &mut self.alerts);
            } else if canvas.state_brush_is_active() {
                self.painting = canvas.begin_state_brush(interface, pos, &mut self.alerts);
            } else if canvas.state_lasso_is_active() {
                canvas.state_lasso_add_point(
                    interface,
                    pos,
                    lasso_mode_from_mods(mods),
                    &mut self.alerts,
                );
            } else {
                inspector_request =
                    canvas.select_state_at(interface, pos, mods.ctrl, &mut self.alerts);
            }
        } else if canvas.view_mode() == ViewMode::Adjacencies
            && canvas.tool.adjacency_brush.is_none()
        {
            self.alerts.push(Err("No Adjacency brush selected"));
        } else {
            self.painting = true;
            canvas.activate_tool(interface, pos, mods.shift, &mut self.alerts);
        };
        if let Some(request) = inspector_request {
            self.handle_inspector_external_request(request);
        }
    }

    fn handle_inspector_external_request(&mut self, request: InspectorExternalRequest) {
        let result = match request {
            InspectorExternalRequest::OpenSource(path) => {
                open_source_with(&path, |path| open_file_default(path))
                    .map(|_| format!("Opened {}", path.display()))
            }
            InspectorExternalRequest::CopyPath(path) => {
                copy_text_to_clipboard(&path).map(|_| "Copied state source path".to_owned())
            }
        };
        self.alerts
            .push(result.map_err(|error| format!("Error: {error}")));
    }

    fn action_deactivate_tool(&mut self) {
        self.painting = false;
        if let Some(canvas) = &mut self.canvas {
            if canvas.state_brush_is_stroking() {
                canvas.finish_state_brush(&mut self.alerts);
            } else if canvas.state_pan_is_active() {
                canvas.camera.set_panning(false);
            } else {
                canvas.deactivate_tool();
            }
        };
    }

    fn action_change_map_view_mode(&mut self, map_view_mode: MapViewMode) {
        if let Some(canvas) = &mut self.canvas {
            if !canvas.is_state_workspace() {
                self.painting = false;
            }
            canvas.set_map_view_mode(&mut self.alerts, map_view_mode);
            if self.global_config.remember_map_views {
                let value = map_view_preference(map_view_mode).to_owned();
                if canvas.is_state_workspace() {
                    self.global_config.workspace.state_map_view = value;
                } else {
                    self.global_config.workspace.province_map_view = value;
                }
            }
        };
    }

    fn action_open_map(&mut self, archive: bool) {
        if self
            .canvas
            .as_ref()
            .is_some_and(Canvas::save_blocks_editing)
        {
            self.alerts.push(Err(
                "Finish or recover the active save before opening another project",
            ));
            return;
        }
        if !self.resolve_property_draft() {
            return;
        }
        if let Some(canvas) = &mut self.canvas {
            if canvas.has_unsaved_province_edits() {
                if msg_dialog_unsaved_changes() {
                    self.action_save_map();
                };
            } else if canvas.has_unsaved_state_edits() && !msg_dialog_discard_state_edits() {
                return;
            };
        };

        if let Some(location) = file_dialog_open(archive) {
            self.raw_open_map_at(location);
        };
    }

    fn action_save_map(&mut self) {
        if self
            .canvas
            .as_ref()
            .is_some_and(|canvas| canvas.project().is_some())
        {
            let confirmation = self
                .canvas
                .as_mut()
                .expect("loaded project has a canvas")
                .prepare_project_save();
            match confirmation {
                Ok(message) => println!("{message}"),
                Err(message) if message == "No changes to save." => {
                    self.alerts.push(Ok(message));
                }
                Err(message) => self.alerts.push(Err(message)),
            }
            return;
        }
        if let Some(canvas) = &self.canvas {
            let location = canvas.location().clone();
            if msg_dialog_confirm_province_save() {
                if let Some(canvas) = self.canvas.as_mut() {
                    canvas.start_province_save(location, ProvinceSaveMode::Save, &mut self.alerts);
                }
            } else {
                self.alerts
                    .push(Ok("Province map Save cancelled before it started"));
            }
        }
    }

    fn action_export_province_map(&mut self, archive: bool) {
        if self.canvas.as_ref().is_some_and(|canvas| {
            saves_state_files(canvas.workspace_mode(), canvas.project().is_some())
        }) {
            self.alerts.push(Err(
        "Province export is available in the Provinces workspace. Use Apply State Changes for state files."
      ));
            return;
        }
        let Some(location) = file_dialog_save(archive) else {
            return;
        };
        if let Some(canvas) = self.canvas.as_mut() {
            canvas.start_province_save(location, ProvinceSaveMode::Export, &mut self.alerts);
        };
    }

    fn action_reveal_map(&mut self) {
        if let Some(canvas) = &self.canvas {
            let path = canvas.location().as_path();
            let result = reveal_in_file_browser(path);
            self.handle_result_none(result);
        };
    }

    fn action_export_land_map(&mut self) {
        if let Some(canvas) = &self.canvas {
            if let Some(path) = file_dialog_save_bmp("land") {
                canvas.export_land_map(path, &mut self.alerts);
            };
        };
    }

    fn action_export_terrain_map(&mut self) {
        if let Some(canvas) = &self.canvas {
            if let Some(path) = file_dialog_save_bmp("terrain") {
                canvas.export_terrain_map(path, &mut self.alerts);
            };
        };
    }

    fn raw_open_map_at(&mut self, location: impl IntoLocation) {
        let result: Result<String, Error> = crate::try_block! {
          let location = location.into_location()?;
          let (canvas, success_message) = match location {
            Location::Directory(root) => match ProjectPaths::discover(&root) {
              Ok(paths) => {
                let root = paths.root.clone();
                let project_config_issue = ProjectConfig::load(&root)
                    .ok()
                    .and_then(|loaded| loaded.issue);
                let project = Hoi4Project::new(paths);
                let canvas = Canvas::load_project(project)?;
                let mut success_message = format!(
                  "Loaded HOI4 mod from {}\n{}",
                  root.display(),
                  canvas.detected_capabilities_message()
                );
                if let Some(issue) = project_config_issue {
                    success_message.push_str(&format!(
                        "\n{}\n{issue}",
                        crate::localization::tr("config.invalid_project")
                    ));
                }
                (canvas, success_message)
              },
              Err(ProjectPathError::MissingHistoryDirectory(_)
                | ProjectPathError::MissingStatesDirectory(_))
                if root.join("map/provinces.bmp").is_file()
                  && root.join("map/definition.csv").is_file() => {
                  let location = Location::Directory(root);
                  let success_message = format!(
                    "Loaded Province-only project from {}. State editing is unavailable because history/states is missing.",
                    location
                  );
                  (Canvas::load(location)?, success_message)
                },
              Err(err) if ProjectPaths::is_project_root_candidate(&root) => return Err(err.into()),
              Err(_) => {
                let location = Location::Directory(root);
                let success_message = format!("Loaded legacy editable map from {}", location);
                (Canvas::load(location)?, success_message)
              }
            },
            location => {
              let success_message = format!("Loaded legacy editable map from {}", location);
              (Canvas::load(location)?, success_message)
            }
          };
          self.canvas = Some(canvas);
          self.apply_remembered_ui_preferences();
          if let Some(project) = self.canvas.as_ref().and_then(Canvas::project) {
              self.global_config.last_project = Some(project.paths.root.clone());
          }
          Ok(success_message)
        };

        self.handle_result(result);
    }

    fn handle_result_none(&mut self, result: Result<(), Error>) {
        if let Err(err) = result {
            self.alerts.push(Err(format!("Error: {}", err)));
        };
    }

    fn apply_remembered_ui_preferences(&mut self) {
        let Some(canvas) = self.canvas.as_mut() else {
            return;
        };
        if self.global_config.remember_workspace {
            let workspace = if self.global_config.workspace.last_workspace == "states"
                && canvas.has_state_workspace()
            {
                WorkspaceMode::States
            } else {
                WorkspaceMode::Provinces
            };
            canvas.set_workspace_mode(workspace, &mut self.alerts);
        }
        if self.global_config.remember_map_views {
            let preference = if canvas.is_state_workspace() {
                &self.global_config.workspace.state_map_view
            } else {
                &self.global_config.workspace.province_map_view
            };
            if let Some(mode) = map_view_from_preference(preference) {
                canvas.set_map_view_mode(&mut self.alerts, mode);
            }
        }
        if self.global_config.remember_overlays {
            let current = canvas.enabled_options();
            let desired = &self.global_config.overlays;
            if current[0] != desired.rivers {
                canvas.toggle_river_overlay();
            }
            if current[1] != desired.adjacencies {
                canvas.toggle_adjacencies_overlay();
            }
            if current[2] != desired.province_ids {
                canvas.toggle_province_ids();
            }
            if current[3] != desired.province_boundaries {
                canvas.toggle_province_boundaries();
            }
            if current[4] != desired.state_boundaries && canvas.has_state_workspace() {
                canvas.toggle_state_boundaries(&mut self.alerts);
            }
            if current[5] != desired.image {
                canvas.toggle_image_overlay(&mut self.alerts);
            }
        }
    }

    fn handle_result<T: fmt::Display>(&mut self, result: Result<T, Error>) {
        self.alerts.push(match result {
            Ok(text) => Ok(text.to_string()),
            Err(err) => Err(format!("Error: {}", err)),
        });
    }
}

pub fn open_source_with<F>(path: &Path, opener: F) -> Result<(), Error>
where
    F: FnOnce(&Path) -> Result<(), Error>,
{
    opener(path)
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("App")
            .field("canvas", &self.canvas)
            .field("alerts", &self.alerts)
            .field("glyph_cache", &format_args!("..."))
            .field("interface", &self.interface)
            .field("painting", &self.painting)
            .finish()
    }
}

fn selected_preference_mut(dialog: &mut Option<PreferencesDialog>) -> Option<&mut usize> {
    match dialog.as_mut()? {
        PreferencesDialog::Global { selected, .. }
        | PreferencesDialog::Project { selected, .. } => Some(selected),
    }
}

fn preference_rows(dialog: &Option<PreferencesDialog>) -> usize {
    match dialog {
        Some(PreferencesDialog::Global { .. }) => 11,
        Some(PreferencesDialog::Project { .. }) => 10,
        None => 0,
    }
}

fn update_extra_warnings_enabled(config: &mut ProjectConfig) {
    config.extra_warnings.enabled =
        config.extra_warnings.lone_pixels || config.extra_warnings.few_shared_borders;
}

fn map_view_preference(mode: MapViewMode) -> &'static str {
    match mode {
        MapViewMode::ProvinceColors => "province-colors",
        MapViewMode::ProvinceTypes => "province-types",
        MapViewMode::Terrain => "terrain",
        MapViewMode::Continents => "continents",
        MapViewMode::Coastal => "coastal",
        MapViewMode::States => "states",
        MapViewMode::Political => "political",
    }
}

fn map_view_from_preference(value: &str) -> Option<MapViewMode> {
    match value {
        "province-colors" => Some(MapViewMode::ProvinceColors),
        "province-types" => Some(MapViewMode::ProvinceTypes),
        "terrain" => Some(MapViewMode::Terrain),
        "continents" => Some(MapViewMode::Continents),
        "coastal" => Some(MapViewMode::Coastal),
        "states" => Some(MapViewMode::States),
        "political" => Some(MapViewMode::Political),
        _ => None,
    }
}

fn confirm_dialog(title: &str, description: &str) -> bool {
    MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(title)
        .set_description(description)
        .set_buttons(MessageButtons::YesNo)
        .show()
        == MessageDialogResult::Yes
}

fn preferences_rect(window_size: [f64; 2]) -> [f64; 4] {
    let width = window_size[0].clamp(520.0, 720.0);
    let height = 400.0;
    [
        ((window_size[0] - width) / 2.0).max(0.0),
        ((window_size[1] - height) / 2.0).max(0.0),
        width,
        height,
    ]
}

fn draw_preferences_dialog(
    ctx: Context,
    dialog: &PreferencesDialog,
    glyph_cache: &mut FontGlyphCache,
    gl: &mut GlGraphics,
) {
    let Some(viewport) = ctx.viewport else {
        return;
    };
    let [x, y, width, height] = preferences_rect(viewport.window_size);
    graphics::rectangle(
        colors::OVERLAY_T,
        [0.0, 0.0, viewport.window_size[0], viewport.window_size[1]],
        ctx.transform,
        gl,
    );
    graphics::rectangle(
        colors::BUTTON_TOOLBAR,
        [x, y, width, height],
        ctx.transform,
        gl,
    );
    let (title, selected, rows) = match dialog {
        PreferencesDialog::Global {
            draft, selected, ..
        } => (
            crate::localization::tr("settings.title"),
            *selected,
            vec![
                format!(
                    "{}: {}",
                    crate::localization::tr("settings.language"),
                    crate::localization::native_name(&draft.language)
                ),
                setting_row(
                    crate::localization::tr("settings.open_last"),
                    draft.open_last_project,
                ),
                setting_row(
                    crate::localization::tr("settings.remember_workspace"),
                    draft.remember_workspace,
                ),
                setting_row(
                    crate::localization::tr("settings.remember_overlays"),
                    draft.remember_overlays,
                ),
                format!(
                    "{}: {} ms",
                    crate::localization::tr("settings.tooltip_delay"),
                    draft.tooltip_delay_ms
                ),
                format!(
                    "{}: {}",
                    crate::localization::tr("settings.max_undo"),
                    draft.max_undo_states
                ),
                setting_row(
                    crate::localization::tr("settings.change_view_undo"),
                    draft.change_view_mode_on_undo,
                ),
                crate::localization::tr("settings.reset_layout").to_owned(),
                crate::localization::tr("settings.restore").to_owned(),
                crate::localization::tr("settings.cancel").to_owned(),
                crate::localization::tr("settings.save").to_owned(),
            ],
        ),
        PreferencesDialog::Project {
            draft, selected, ..
        } => (
            crate::localization::tr("project_settings.title"),
            *selected,
            vec![
                setting_row(
                    crate::localization::tr("project_settings.preserve_ids"),
                    draft.preserve_ids,
                ),
                setting_row(
                    crate::localization::tr("project_settings.generate_coastal"),
                    draft.generate_coastal_on_save,
                ),
                setting_row(
                    crate::localization::tr("project_settings.lone_pixels"),
                    draft.extra_warnings.lone_pixels,
                ),
                setting_row(
                    crate::localization::tr("project_settings.few_borders"),
                    draft.extra_warnings.few_shared_borders,
                ),
                format!(
                    "{}: {}",
                    crate::localization::tr("project_settings.threshold"),
                    draft.extra_warnings.few_shared_borders_threshold
                ),
                crate::localization::tr("settings.restore").to_owned(),
                crate::localization::tr("project_settings.open").to_owned(),
                format!(
                    "{} ({} effective)",
                    crate::localization::tr("project_settings.validate"),
                    draft.terrains.len().saturating_sub(1)
                ),
                crate::localization::tr("settings.cancel").to_owned(),
                crate::localization::tr("settings.save").to_owned(),
            ],
        ),
    };
    draw_dialog_text(ctx, glyph_cache, gl, [x + 16.0, y + 26.0], title);
    for (index, row) in rows.iter().enumerate() {
        let row_y = y + 46.0 + index as f64 * 30.0;
        if index == selected {
            graphics::rectangle(
                colors::BUTTON_HOVER_ACTIVE,
                [x + 8.0, row_y, width - 16.0, 26.0],
                ctx.transform,
                gl,
            );
        }
        draw_dialog_text(ctx, glyph_cache, gl, [x + 16.0, row_y + 18.0], row);
    }
}

fn setting_row(label: &str, enabled: bool) -> String {
    format!("[{}] {label}", if enabled { "x" } else { " " })
}

fn draw_dialog_text(
    ctx: Context,
    glyph_cache: &mut FontGlyphCache,
    gl: &mut GlGraphics,
    position: Vector2<f64>,
    text: &str,
) {
    graphics::text(
        colors::WHITE,
        FONT_SIZE,
        text,
        glyph_cache,
        ctx.transform.trans_pos(position),
        gl,
    )
    .expect("unable to draw preferences dialog text");
}

#[derive(Debug, Clone, Copy)]
pub struct InterfaceDrawContext {
    pub map_view_mode: Option<MapViewMode>,
    pub view_mode: Option<ViewMode>,
    pub selected_tool: Option<usize>,
    pub state_tool: Option<usize>,
    pub enabled_options: [bool; 6],
    pub available_options: [bool; 6],
    pub states_available: bool,
    pub state_actions: StateActionAvailability,
    pub blocks_tooltips: bool,
    pub province_modified: bool,
    pub pending_states: usize,
}

use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

fn show_about_dialog() {
    MessageDialog::new()
        .set_level(MessageLevel::Info)
        .set_title(crate::localization::tr("about.title"))
        .set_description(format!(
            "HOI4 Map Editor · Version {}\n{}\n\n{}\n\n{}\n\nMIT License\nhttps://github.com/AdriianCOE/hoi4_state_editor",
            crate::APP_VERSION,
            crate::localization::tr("about.subtitle"),
            crate::localization::tr("about.credits"),
            crate::localization::tr("about.disclaimer"),
        ))
        .set_buttons(MessageButtons::Ok)
        .show();
}

fn file_dialog_image_overlay() -> Option<PathBuf> {
    let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("./"));
    FileDialog::new()
        .set_title("Choose Image Overlay")
        .set_directory(root)
        .add_filter("Supported images", &["bmp", "png", "jpg", "jpeg"])
        .pick_file()
}

fn file_dialog_save_bmp(filename: &str) -> Option<PathBuf> {
    let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("./"));
    FileDialog::new()
        .set_directory(&root)
        .set_file_name(format!("{}.bmp", filename))
        .add_filter("24-bit Bitmap", &["bmp"])
        .save_file()
}

fn file_dialog_save(archive: bool) -> Option<Location> {
    let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("./"));
    if archive {
        FileDialog::new()
            .set_title("Export Province Map Archive")
            .set_directory(&root)
            .set_file_name("map.zip")
            .add_filter("ZIP Archive", &["zip"])
            .save_file()
            .map(Location::ZipArchive)
    } else {
        FileDialog::new()
            .set_title("Export Province Map As")
            .set_directory(&root)
            .pick_folder()
            .map(Location::Directory)
    }
}

fn file_dialog_open(archive: bool) -> Option<Location> {
    let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("./"));
    if archive {
        FileDialog::new()
            .set_directory(&root)
            .set_file_name("map.zip")
            .add_filter("ZIP Archive", &["zip"])
            .pick_file()
            .map(Location::ZipArchive)
    } else {
        FileDialog::new()
            .set_directory(&root)
            .set_title("Open HOI4 Mod")
            .pick_folder()
            .map(Location::Directory)
    }
}

fn file_dialog_base_game_definitions() -> Option<PathBuf> {
    let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("./"));
    FileDialog::new()
        .set_directory(&root)
        .set_title("Choose Hearts of Iron IV base game folder")
        .pick_folder()
}

fn msg_dialog_unsaved_changes_exit() -> bool {
    let result = MessageDialog::new()
        .set_title(crate::APPNAME)
        .set_description("You have unsaved changes, would you like to save them before exiting?")
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::YesNo)
        .show();

    match result {
        MessageDialogResult::Yes => true,
        MessageDialogResult::No => false,
        _ => unreachable!(),
    }
}

fn lasso_mode_from_mods(mods: KeyMods) -> Option<LassoSelectionMode> {
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

fn saves_state_files(workspace: WorkspaceMode, has_project: bool) -> bool {
    has_project && workspace == WorkspaceMode::States
}

#[cfg(test)]
mod workspace_shortcut_tests {
    use super::*;

    #[test]
    fn workspace_shortcuts_do_not_replace_plain_map_view_shortcuts() {
        let ctrl = KeyMods {
            ctrl: true,
            ..KeyMods::default()
        };
        assert_eq!(
            workspace_shortcut(Key::D1, ctrl, WorkspaceMode::States),
            Some(WorkspaceMode::Provinces)
        );
        assert_eq!(
            workspace_shortcut(Key::D2, ctrl, WorkspaceMode::Provinces),
            Some(WorkspaceMode::States)
        );
        assert_eq!(
            workspace_shortcut(Key::Tab, ctrl, WorkspaceMode::States),
            Some(WorkspaceMode::Provinces)
        );
        assert_eq!(
            workspace_shortcut(Key::D1, KeyMods::default(), WorkspaceMode::States),
            None
        );
    }

    #[test]
    fn province_export_is_blocked_only_in_the_states_workspace() {
        assert!(saves_state_files(WorkspaceMode::States, true));
        assert!(!saves_state_files(WorkspaceMode::Provinces, true));
        assert!(!saves_state_files(WorkspaceMode::Provinces, false));
    }

    #[test]
    fn province_save_confirmation_names_only_the_legacy_map_files() {
        let message = province_save_confirmation_text();
        assert!(message.contains("map/provinces.bmp"));
        assert!(message.contains("map/definition.csv"));
        assert!(!message.contains("history/states"));
    }
}

fn msg_dialog_confirm_state_batch(description: &str) -> bool {
    matches!(
        MessageDialog::new()
            .set_title(crate::APPNAME)
            .set_description(description)
            .set_level(MessageLevel::Warning)
            .set_buttons(MessageButtons::YesNo)
            .show(),
        MessageDialogResult::Yes
    )
}

fn msg_dialog_confirm_province_save() -> bool {
    matches!(
        MessageDialog::new()
            .set_title("SAVE PROVINCE MAP")
            .set_description(province_save_confirmation_text())
            .set_level(MessageLevel::Warning)
            .set_buttons(MessageButtons::YesNo)
            .show(),
        MessageDialogResult::Yes
    )
}

fn province_save_confirmation_text() -> &'static str {
    "Files to update:\n\
     • map/provinces.bmp\n\
     • map/definition.csv\n\
     • map/adjacencies.csv and id_changes.txt when generated by the legacy Province tools\n\n\
     Validation runs before writing:\n\
     • Image dimensions\n\
     • Province colors\n\
     • Definition catalog\n\
     • Province IDs"
}

fn msg_dialog_unsaved_changes() -> bool {
    let result = MessageDialog::new()
        .set_title(crate::APPNAME)
        .set_description("You have unsaved changes, would you like to save them?")
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::YesNo)
        .show();

    match result {
        MessageDialogResult::Yes => true,
        MessageDialogResult::No => false,
        _ => unreachable!(),
    }
}

fn msg_dialog_discard_state_edits_exit() -> bool {
    let result = MessageDialog::new()
        .set_title(crate::APPNAME)
        .set_description(
            "This editing session contains unsaved in-memory changes.\n\n\
       Created, removed, reassigned, or edited states have not been saved.\n\
       The current session is not eligible for automatic Save.\n\
       Discard the changes and close?\n\n\
       Yes = Discard and close\n\
       No = Keep editing",
        )
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::YesNo)
        .show();

    match result {
        MessageDialogResult::Yes => true,
        MessageDialogResult::No => false,
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateExitResolution {
    Save,
    Discard,
    KeepEditing,
}

fn msg_dialog_state_edits_exit(save_summary: &str) -> StateExitResolution {
    let result = MessageDialog::new()
        .set_title(crate::APPNAME)
        .set_description(format!(
            "This editing session contains unsaved state changes.\n\n\
       Yes = Apply State Changes\nNo = Discard and close\nCancel = Keep editing\n\n\
       {save_summary}"
        ))
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::YesNoCancel)
        .show();
    match result {
        MessageDialogResult::Yes => StateExitResolution::Save,
        MessageDialogResult::No => StateExitResolution::Discard,
        _ => StateExitResolution::KeepEditing,
    }
}

fn msg_dialog_discard_state_edits() -> bool {
    let result = MessageDialog::new()
        .set_title(crate::APPNAME)
        .set_description(
            "Discard all in-memory state edits?\n\n\
       Any current patch preview and validation result will also be discarded.\n\
       Yes = Discard changes\n\
       No = Keep editing",
        )
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::YesNo)
        .show();

    match result {
        MessageDialogResult::Yes => true,
        MessageDialogResult::No => false,
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftResolution {
    Apply,
    Discard,
    KeepEditing,
}

fn msg_dialog_resolve_property_draft(province: bool) -> DraftResolution {
    let subject = if province { "province" } else { "state" };
    let result = MessageDialog::new()
        .set_title(crate::APPNAME)
        .set_description(format!(
            "This {subject} has unapplied form changes.\n\n\
       Yes = Apply to session\n\
       No = Discard draft\n\
       Cancel = Keep editing"
        ))
        .set_level(MessageLevel::Warning)
        .set_buttons(MessageButtons::YesNoCancel)
        .show();

    match result {
        MessageDialogResult::Yes => DraftResolution::Apply,
        MessageDialogResult::No => DraftResolution::Discard,
        MessageDialogResult::Cancel => DraftResolution::KeepEditing,
        _ => DraftResolution::KeepEditing,
    }
}

fn msg_dialog_discard_property_draft_exit(province: bool) -> bool {
    let description = if province {
        "There are unapplied province changes.\n\n\
     Yes = Discard draft and continue closing\n\
     No = Keep editing"
    } else {
        "There are unapplied state changes.\n\n\
     Yes = Discard draft and continue closing\n\
     No = Keep editing"
    };
    matches!(
        MessageDialog::new()
            .set_title(crate::APPNAME)
            .set_description(description)
            .set_level(MessageLevel::Warning)
            .set_buttons(MessageButtons::YesNo)
            .show(),
        MessageDialogResult::Yes
    )
}

pub fn reveal_in_file_browser(path: impl AsRef<Path>) -> Result<(), Error> {
    use std::process::Command;

    let path = crate::util::files::canonicalize(path)?;
    if cfg!(target_os = "windows") {
        Command::new("explorer")
            .arg(&path)
            .status()
            .context("failed to execute command 'explorer'")?;
        Ok(())
    } else if cfg!(target_os = "macos") {
        Command::new("open")
            .arg(&path)
            .status()
            .context("failed to execute command 'open'")?;
        Ok(())
    } else if cfg!(target_os = "linux") {
        Command::new("xdg-open")
            .arg(&path)
            .status()
            .context("failed to execute command 'xdg-open'")?;
        Ok(())
    } else {
        Err("unable to reveal in file browser".into())
    }
}

pub fn open_file_default(path: impl AsRef<Path>) -> Result<(), Error> {
    use std::process::Command;

    let path = crate::util::files::canonicalize(path)?;
    let mut command = if cfg!(target_os = "windows") {
        let mut command = Command::new("explorer");
        command.arg(&path);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(&path);
        command
    } else if cfg!(target_os = "linux") {
        let mut command = Command::new("xdg-open");
        command.arg(&path);
        command
    } else {
        return Err("unable to open source file on this platform".into());
    };
    command
        .status()
        .context("failed to execute the platform file opener")?;
    Ok(())
}

pub fn copy_text_to_clipboard(text: &str) -> Result<(), Error> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let program = if cfg!(target_os = "windows") {
        "clip"
    } else if cfg!(target_os = "macos") {
        "pbcopy"
    } else {
        "xclip"
    };
    let mut command = Command::new(program);
    if cfg!(target_os = "linux") {
        command.args(["-selection", "clipboard"]);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to start the platform clipboard command")?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| Error::from("clipboard command stdin is unavailable"))?
        .write_all(text.as_bytes())
        .context("failed to write the source path to the clipboard")?;
    let status = child
        .wait()
        .context("failed to wait for the clipboard command")?;
    if !status.success() {
        return Err("the platform clipboard command failed".into());
    }
    Ok(())
}

#[cfg(test)]
mod source_open_tests {
    use super::{PreferencesDialog, map_view_from_preference, open_source_with, preference_rows};
    use crate::app::project::MapViewMode;
    use crate::config::{GlobalConfig, ProjectConfig};
    use std::cell::Cell;
    use std::path::Path;

    #[test]
    fn source_opener_is_injectable_without_starting_an_external_program() {
        let called = Cell::new(false);
        open_source_with(Path::new("state.txt"), |path| {
            called.set(path == Path::new("state.txt"));
            Ok(())
        })
        .unwrap();
        assert!(called.get());
    }

    #[test]
    fn settings_dialog_models_keep_global_and_project_drafts_separate() {
        let global = Some(PreferencesDialog::Global {
            original: GlobalConfig::default(),
            draft: GlobalConfig::default(),
            fingerprint: None,
            selected: 0,
        });
        let project = Some(PreferencesDialog::Project {
            root: "mod".into(),
            draft: ProjectConfig::default(),
            fingerprint: None,
            selected: 0,
        });
        assert_eq!(preference_rows(&global), 11);
        assert_eq!(preference_rows(&project), 10);
        assert_eq!(
            map_view_from_preference("political"),
            Some(MapViewMode::Political)
        );
        assert_eq!(map_view_from_preference("unknown"), None);
    }
}
