//! Code regarding buttons and interactive elements on the screen
use graphics::context::Context;
use graphics::types::Color as DrawColor;
use graphics::{Transformed, Viewport};
use image::codecs::png::PngDecoder;
use image::{DynamicImage, GenericImageView, RgbaImage};
use once_cell::sync::Lazy;
use opengl_graphics::{GlGraphics, Texture, TextureSettings};
use vecmath::Vector2;

use super::canvas::ViewMode;
use super::colors;
use super::inspector::{MapViewport, inspector_drawer_width};
use super::project::MapViewMode;
use super::{FontGlyphCache, InterfaceDrawContext};
use crate::font::{self, FONT_SIZE};

use std::fmt;
use std::sync::Arc;

pub const PADDING: Vector2<f64> = [6.0, 4.0];
const TOOLTIP_DELAY_SECONDS: f32 = 0.4;
const TOOLTIP_MIN_WIDTH: f64 = 180.0;
const TOOLTIP_MAX_TEXT_WIDTH: f64 = 320.0;

#[inline]
fn snap_pos([x, y]: Vector2<f64>) -> Vector2<f64> {
    [x.round(), y.round()]
}

fn button_width(label: &str) -> u32 {
    (font::get_width_metric_str(label) + PADDING[0] * 2.0).round() as u32
}

const PALETTE_BUTTON: Palette = Palette {
    foreground: colors::WHITE,
    background: colors::BUTTON,
    background_active: colors::BUTTON_ACTIVE,
    background_hover: colors::BUTTON_HOVER,
    background_hover_active: colors::BUTTON_HOVER_ACTIVE,
};

const PALETTE_BUTTON_DISABLED: Palette = Palette {
    foreground: colors::NEUTRAL,
    background: colors::BUTTON_TOOLBAR,
    background_active: colors::BUTTON_TOOLBAR,
    background_hover: colors::BUTTON_TOOLBAR,
    background_hover_active: colors::BUTTON_TOOLBAR,
};

const PALETTE_BUTTON_TOOLBAR: Palette = Palette {
    foreground: colors::WHITE,
    background: colors::BUTTON_TOOLBAR,
    background_active: colors::BUTTON_TOOLBAR_ACTIVE,
    background_hover: colors::BUTTON_TOOLBAR_HOVER,
    background_hover_active: colors::BUTTON_TOOLBAR_HOVER_ACTIVE,
};

pub fn get_interface(
    interface_holder: &mut Option<Interface>,
    viewport: Viewport,
) -> &mut Interface {
    interface_holder.get_or_insert_with(|| Interface::new(viewport))
}

#[derive(Debug, Clone)]
pub struct Interface {
    toolbar_buttons: Vec<ToolbarButtonElement>,
    workspace_buttons: Vec<ButtonElement>,
    workspace_dropdowns: Vec<ToolbarButtonElement>,
    toolbar_plate: PlateComponentStyled,
    toolbar_height: u32,
    sidebar_tool_buttons: Vec<ButtonElement>,
    state_sidebar_tool_buttons: Vec<ButtonElement>,
    sidebar_option_buttons: Vec<ButtonElement>,
    sidebar_plate: PlateComponentStyled,
    sidebar_width: u32,
    inspector_width: f64,
    viewport: Viewport,
    tooltip: TooltipManager,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TooltipKey {
    Button(ButtonId),
    MapViewSelector,
    OverlaysSelector,
}

#[derive(Debug, Clone)]
struct TooltipCandidate {
    key: TooltipKey,
    text: String,
    source: [f64; 4],
}

#[derive(Debug, Clone, Default)]
struct TooltipManager {
    candidate: Option<TooltipCandidate>,
    elapsed: f32,
}

impl TooltipManager {
    fn update(&mut self, candidate: Option<TooltipCandidate>) {
        let same = self
            .candidate
            .as_ref()
            .zip(candidate.as_ref())
            .is_some_and(|(current, next)| current.key == next.key);
        if !same {
            self.elapsed = 0.0;
        }
        self.candidate = candidate;
    }

    fn tick(&mut self, dt: f32) {
        if self.candidate.is_some() {
            self.elapsed += dt.max(0.0);
        }
    }

    fn clear(&mut self) {
        self.candidate = None;
        self.elapsed = 0.0;
    }

    fn visible(&self) -> Option<&TooltipCandidate> {
        (self.elapsed >= TOOLTIP_DELAY_SECONDS)
            .then_some(self.candidate.as_ref())
            .flatten()
    }
}

impl Interface {
    pub fn new(viewport: Viewport) -> Self {
        let [window_width, window_height] = viewport.draw_size;

        let mut pos_x = 0;
        let mut toolbar_height = 0;
        let mut toolbar_buttons = Vec::with_capacity(TOOLBAR_PRIMITIVE.len());
        for toolbar_button_text in ["File", "Edit", "View", "Tools", "Help"] {
            let toolbar_primitive_buttons = TOOLBAR_PRIMITIVE
                .iter()
                .find_map(|(label, entries)| (*label == toolbar_button_text).then_some(*entries))
                .expect("main toolbar group must exist");
            let mut buttons = Vec::with_capacity(toolbar_primitive_buttons.len());
            let base =
                ButtonBase::new_fit_width(toolbar_button_text, [pos_x, 0], &PALETTE_BUTTON_TOOLBAR);

            let mut pos_y = base.height();
            for &(button_text_left, button_text_right, id) in toolbar_primitive_buttons {
                let text = [button_text_left, button_text_right];
                let base = ButtonBase::new_double_text(
                    text,
                    [pos_x, pos_y],
                    TOOLBAR_DROPDOWN_WIDTH,
                    &PALETTE_BUTTON,
                );
                pos_y += base.height();

                buttons.push(ButtonElement { base, id });
            }

            toolbar_height = base.height();
            pos_x += base.width();

            toolbar_buttons.push(ToolbarButtonElement {
                base,
                buttons,
                enabled: false,
                map_view_selector: false,
                overlays_selector: false,
            });
        }

        let menu_height = toolbar_height;
        let bar_y = menu_height;
        let mut bar_x = 0;
        let mut workspace_buttons = Vec::new();
        let compact = window_width < 700;
        let workspace_labels = if compact {
            [
                ("Provinces", ButtonId::WorkspaceProvinces),
                ("States", ButtonId::WorkspaceStates),
            ]
        } else {
            [
                ("Workspace: Provinces", ButtonId::WorkspaceProvinces),
                ("States", ButtonId::WorkspaceStates),
            ]
        };
        for (label, id) in workspace_labels {
            let base = ButtonBase::new_fit_width(label, [bar_x, bar_y], &PALETTE_BUTTON_TOOLBAR);
            bar_x += base.width();
            workspace_buttons.push(ButtonElement { base, id });
        }

        let apply_label = "Apply to Mod";
        let apply_width = button_width(apply_label);
        let mut action_x = window_width.saturating_sub(apply_width);
        workspace_buttons.push(ButtonElement {
            base: ButtonBase::new_fit_width(
                apply_label,
                [action_x, bar_y],
                &PALETTE_BUTTON_TOOLBAR,
            ),
            id: ButtonId::WorkspaceApplyToMod,
        });
        if window_width >= 720 {
            let review_label = if window_width < 980 {
                "Review"
            } else {
                "Review Changes"
            };
            let review_width = button_width(review_label);
            action_x = action_x.saturating_sub(review_width);
            workspace_buttons.push(ButtonElement {
                base: ButtonBase::new_fit_width(
                    review_label,
                    [action_x, bar_y],
                    &PALETTE_BUTTON_TOOLBAR,
                ),
                id: ButtonId::WorkspaceReviewChanges,
            });
        }

        let mut workspace_dropdowns = Vec::new();
        for (label, entries, map_view_selector, overlays_selector) in WORKSPACE_DROPDOWNS {
            let placeholder = if *map_view_selector {
                if compact {
                    "Map             "
                } else {
                    "Map View: Coastal Provinces   "
                }
            } else if compact {
                "Layers          "
            } else {
                "Overlays: Province Borders, State Borders   "
            };
            let base = ButtonBase::new_fit_width(
                placeholder,
                [bar_x, bar_y],
                &PALETTE_BUTTON_TOOLBAR,
            );
            if *overlays_selector && bar_x.saturating_add(base.width()) > action_x {
                continue;
            }
            let mut buttons = Vec::with_capacity(entries.len());
            let mut entry_y = bar_y + base.height();
            for &(left, right, id) in *entries {
                let entry = ButtonBase::new_double_text(
                    [left, right],
                    [bar_x, entry_y],
                    TOOLBAR_DROPDOWN_WIDTH,
                    &PALETTE_BUTTON,
                );
                entry_y += entry.height();
                buttons.push(ButtonElement { base: entry, id });
            }
            bar_x += base.width();
            workspace_dropdowns.push(ToolbarButtonElement {
                base,
                buttons,
                enabled: false,
                map_view_selector: *map_view_selector,
                overlays_selector: *overlays_selector,
            });
            debug_assert!(!label.is_empty());
        }
        toolbar_height += menu_height;

        let mut pos_y_top = toolbar_height;
        let mut state_pos_y_top = toolbar_height;
        let mut pos_y_bottom = window_height;
        let mut sidebar_width = 0;
        let mut sidebar_tool_buttons = Vec::new();
        let mut state_sidebar_tool_buttons = Vec::new();
        let mut sidebar_option_buttons = Vec::new();
        for &(sprite_coords, id, kind) in SIDEBAR_PRIMITIVE {
            match kind {
                SidebarPrimitiveKind::Tool => {
                    let base = ButtonBase::new_texture(
                        sprite_coords,
                        [0, pos_y_top],
                        ButtonOrigin::TopLeft,
                        &PALETTE_BUTTON,
                    );
                    sidebar_width = sidebar_width.max(base.width());
                    pos_y_top += base.height();

                    sidebar_tool_buttons.push(ButtonElement { base, id });
                }
                SidebarPrimitiveKind::StateTool => {
                    let base = ButtonBase::new_texture(
                        sprite_coords,
                        [0, state_pos_y_top],
                        ButtonOrigin::TopLeft,
                        &PALETTE_BUTTON,
                    );
                    sidebar_width = sidebar_width.max(base.width());
                    state_pos_y_top += base.height();

                    state_sidebar_tool_buttons.push(ButtonElement { base, id });
                }
                SidebarPrimitiveKind::Option => {
                    let base = ButtonBase::new_texture(
                        sprite_coords,
                        [0, pos_y_bottom],
                        ButtonOrigin::BottomLeft,
                        &PALETTE_BUTTON,
                    );
                    sidebar_width = sidebar_width.max(base.width());
                    pos_y_bottom -= base.height();

                    sidebar_option_buttons.push(ButtonElement { base, id });
                }
            };
        }

        let toolbar_plate_size = [window_width as f64, toolbar_height as f64];
        let toolbar_plate = PlateComponent {
            pos: [0.0, 0.0],
            size: toolbar_plate_size,
        };

        let sidebar_plate_size = [sidebar_width as f64, window_height as f64];
        let sidebar_plate = PlateComponent {
            pos: [0.0, toolbar_height as f64],
            size: sidebar_plate_size,
        };

        Interface {
            sidebar_tool_buttons,
            state_sidebar_tool_buttons,
            sidebar_option_buttons,
            toolbar_buttons,
            workspace_buttons,
            workspace_dropdowns,
            toolbar_plate: toolbar_plate.styled(&PALETTE_BUTTON_TOOLBAR),
            toolbar_height,
            sidebar_plate: sidebar_plate.styled(&PALETTE_BUTTON),
            sidebar_width,
            inspector_width: 0.0,
            viewport,
            tooltip: TooltipManager::default(),
        }
    }

    #[inline]
    pub const fn get_window_size(&self) -> [f64; 2] {
        self.viewport.window_size
    }

    #[inline]
    pub fn get_window_center(&self) -> [f64; 2] {
        self.get_map_viewport().center()
    }

    #[inline]
    pub const fn get_toolbar_height(&self) -> u32 {
        self.toolbar_height
    }

    #[inline]
    pub const fn get_sidebar_width(&self) -> u32 {
        self.sidebar_width
    }

    pub fn set_inspector_width(&mut self, width: f64) {
        self.inspector_width = width.max(0.0);
    }

    pub fn get_map_viewport(&self) -> MapViewport {
        MapViewport::new(
            self.sidebar_width as f64,
            self.toolbar_height as f64,
            (self.viewport.window_size[0] - self.sidebar_width as f64).max(1.0),
            (self.viewport.window_size[1] - self.toolbar_height as f64).max(1.0),
        )
    }

    pub fn get_inspector_viewport(&self) -> Option<MapViewport> {
        let width = inspector_drawer_width(
            self.viewport.window_size[0],
            self.sidebar_width as f64,
            self.inspector_width,
        );
        (width > 0.0).then(|| {
            MapViewport::new(
                self.viewport.window_size[0] - width,
                self.toolbar_height as f64,
                width,
                (self.viewport.window_size[1] - self.toolbar_height as f64).max(1.0),
            )
        })
    }

    #[inline]
    pub fn inspector_contains(&self, pos: Vector2<f64>) -> bool {
        self.get_inspector_viewport()
            .is_some_and(|viewport| viewport.contains(pos))
    }

    #[inline]
    pub fn map_contains(&self, pos: Vector2<f64>) -> bool {
        self.get_map_viewport().contains(pos) && !self.inspector_contains(pos)
    }

    /// Called when the mouse is clicked to act on the interface and change its state.
    /// If a button was clicked, `Ok` is returned with the appropriate button ID.
    /// If a button was not clicked, a boolean is returned indicating whether or not
    /// the input just processed should be deferred to something below the interface.
    pub fn on_mouse_click(
        &mut self,
        pos: Vector2<f64>,
        ictx: InterfaceDrawContext,
    ) -> Result<ButtonId, bool> {
        self.tooltip.clear();
        let menu_was_open = self
            .toolbar_buttons
            .iter()
            .chain(self.workspace_dropdowns.iter())
            .any(|menu| menu.enabled);
        if menu_was_open {
            let clicked_toolbar = self
                .toolbar_buttons
                .iter()
                .position(|menu| menu.base.test(pos));
            let clicked_workspace = self
                .workspace_dropdowns
                .iter()
                .position(|menu| menu.base.test(pos));
            if clicked_toolbar.is_some() || clicked_workspace.is_some() {
                let enable = clicked_toolbar
                    .map(|index| !self.toolbar_buttons[index].enabled)
                    .or_else(|| {
                        clicked_workspace.map(|index| !self.workspace_dropdowns[index].enabled)
                    })
                    .unwrap_or(false);
                self.toolbar_buttons
                    .iter_mut()
                    .chain(self.workspace_dropdowns.iter_mut())
                    .for_each(|menu| menu.enabled = false);
                if let Some(index) = clicked_toolbar {
                    self.toolbar_buttons[index].enabled = enable;
                } else if let Some(index) = clicked_workspace {
                    self.workspace_dropdowns[index].enabled = enable;
                }
                return Err(false);
            }

            let clicked = self
                .toolbar_buttons
                .iter()
                .chain(self.workspace_dropdowns.iter())
                .filter(|menu| menu.enabled)
                .flat_map(|menu| menu.visible_buttons(ictx))
                .find(|button| button.base.test(pos))
                .map(|button| button.id);
            self.toolbar_buttons
                .iter_mut()
                .chain(self.workspace_dropdowns.iter_mut())
                .for_each(|menu| menu.enabled = false);
            return match clicked {
                Some(id) if ictx.state_actions.button_enabled(id) => Ok(id),
                _ => Err(false),
            };
        }
        for button in &self.workspace_buttons {
            if button.base.test(pos) {
                let enabled = match button.id {
                    ButtonId::WorkspaceStates => ictx.states_available,
                    ButtonId::WorkspaceReviewChanges => ictx.state_actions.state_view,
                    ButtonId::WorkspaceApplyToMod => ictx.state_actions.state_view,
                    _ => true,
                };
                return if enabled {
                    Ok(button.id)
                } else {
                    Err(false)
                };
            }
        }
        for dropdown in &mut self.workspace_dropdowns {
            if dropdown.base.test(pos) {
                dropdown.enabled = !dropdown.enabled;
                return Err(false);
            }
            if dropdown.enabled {
                for button in dropdown.visible_buttons(ictx) {
                    if button.base.test(pos) {
                        dropdown.enabled = false;
                        return if ictx.state_actions.button_enabled(button.id) {
                            Ok(button.id)
                        } else {
                            Err(false)
                        };
                    }
                }
            }
        }
        let tool_buttons = if ictx.state_actions.state_view {
            &self.state_sidebar_tool_buttons
        } else {
            &self.sidebar_tool_buttons
        };
        for sidebar_button in tool_buttons {
            if sidebar_button.base.test(pos) {
                return Ok(sidebar_button.id);
            };
        }

        for (index, sidebar_button) in self.sidebar_option_buttons.iter().enumerate() {
            if sidebar_button.base.test(pos) {
                return if ictx.available_options[index] {
                    Ok(sidebar_button.id)
                } else {
                    Err(false)
                };
            };
        }

        for toolbar_button in &mut self.toolbar_buttons {
            if toolbar_button.base.test(pos) {
                toolbar_button.enabled = !toolbar_button.enabled;
                return Err(false);
            };

            if toolbar_button.enabled {
                for button in toolbar_button.visible_buttons(ictx) {
                    if button.base.test(pos) {
                        toolbar_button.enabled = false;
                        return if ictx.state_actions.button_enabled(button.id) {
                            Ok(button.id)
                        } else {
                            Err(false)
                        };
                    };
                }
            };
        }

        let hit_deadzone = self.toolbar_plate.test(pos) || self.sidebar_plate.test(pos);

        Err(!hit_deadzone)
    }

    pub fn on_mouse_position(&mut self, pos: Vector2<f64>, ictx: InterfaceDrawContext) {
        let hovered = self
            .toolbar_buttons
            .iter()
            .chain(self.workspace_dropdowns.iter())
            .position(|menu| menu.base.test(pos));
        if self.menu_is_open()
            && let Some(hovered) = hovered
        {
            self.toolbar_buttons
                .iter_mut()
                .chain(self.workspace_dropdowns.iter_mut())
                .enumerate()
                .for_each(|(index, menu)| menu.enabled = index == hovered);
        }
        let _ = ictx;
    }

    pub fn tick(&mut self, dt: f32) {
        self.tooltip.tick(dt);
    }

    pub fn clear_tooltip(&mut self) {
        self.tooltip.clear();
    }

    pub fn draw(
        &mut self,
        ctx: Context,
        ictx: InterfaceDrawContext,
        pos: Option<Vector2<f64>>,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        self.update_tooltip(pos, ictx);
        self.sidebar_plate.draw(ctx, false, false, gl);

        let tool_buttons = if ictx.state_actions.state_view {
            &self.state_sidebar_tool_buttons
        } else {
            &self.sidebar_tool_buttons
        };
        for (i, sidebar_button) in tool_buttons.iter().enumerate() {
            if ictx.state_actions.state_view {
                let hover = sidebar_button.base.test_maybe(pos);
                sidebar_button.base.draw(
                    ctx,
                    hover,
                    ictx.state_tool == Some(i),
                    glyph_cache,
                    gl,
                );
                continue;
            }
            let selected_tool = match (ictx.view_mode, i) {
                // color map mode has the primary 3 tools (paint area, paint bucket, lasso) available
                (Some(ViewMode::Color), _) => ictx.selected_tool,
                // coastal and adjacencies are read-only and have no tools
                (Some(ViewMode::Coastal | ViewMode::Adjacencies), _) => continue,
                // kind, terrain, and continent only have the first tool (paint area) available
                (Some(ViewMode::Kind | ViewMode::Terrain | ViewMode::Continent), 0) => Some(0),
                (_, _) => continue,
            };

            let hover = sidebar_button.base.test_maybe(pos);
            let active = Some(i) == selected_tool;
            sidebar_button
                .base
                .draw(ctx, hover, active, glyph_cache, gl);
        }

        for (i, sidebar_button) in self.sidebar_option_buttons.iter().enumerate() {
            let active = ictx.enabled_options[i];
            sidebar_button.draw(
                ctx,
                pos,
                active,
                ictx.available_options[i],
                glyph_cache,
                gl,
            );
        }
        if let Some(top) = self
            .sidebar_option_buttons
            .iter()
            .map(|button| button.base.plate().pos[1])
            .reduce(f64::min)
        {
            graphics::rectangle(
                colors::WHITE_T,
                [3.0, top - 3.0, (self.sidebar_width - 6) as f64, 1.0],
                ctx.transform,
                gl,
            );
        }

        self.toolbar_plate.draw(ctx, false, false, gl);

        for button in &self.workspace_buttons {
            let active = matches!(
                (button.id, ictx.state_actions.state_view),
                (ButtonId::WorkspaceProvinces, false) | (ButtonId::WorkspaceStates, true)
            );
            let enabled = match button.id {
                ButtonId::WorkspaceStates => ictx.states_available,
                ButtonId::WorkspaceReviewChanges => ictx.state_actions.state_view,
                ButtonId::WorkspaceApplyToMod => ictx.state_actions.state_view,
                _ => true,
            };
            button.draw(ctx, pos, active, enabled, glyph_cache, gl);
        }
        for dropdown in &self.workspace_dropdowns {
            self.draw_toolbar_button(dropdown, ctx, ictx, pos, glyph_cache, gl);
        }
        for toolbar_button in &self.toolbar_buttons {
            self.draw_toolbar_button(toolbar_button, ctx, ictx, pos, glyph_cache, gl);
        }
        if let Some(tooltip) = self.tooltip.visible() {
            draw_tooltip(
                ctx,
                &tooltip.text,
                tooltip.source,
                self.viewport.window_size,
                glyph_cache,
                gl,
            );
        }
    }

    fn update_tooltip(&mut self, pos: Option<Vector2<f64>>, ictx: InterfaceDrawContext) {
        let candidate = if ictx.blocks_tooltips || self.menu_is_open() {
            None
        } else {
            pos.and_then(|pos| self.hovered_tooltip(pos, ictx))
        };
        self.tooltip.update(candidate);
    }

    fn menu_is_open(&self) -> bool {
        self.toolbar_buttons
            .iter()
            .chain(self.workspace_dropdowns.iter())
            .any(|button| button.enabled)
    }

    fn hovered_tooltip(
        &self,
        pos: Vector2<f64>,
        ictx: InterfaceDrawContext,
    ) -> Option<TooltipCandidate> {
        for button in &self.workspace_buttons {
            if button.base.test(pos)
                && let Some(text) = button.tooltip(ictx.view_mode)
            {
                return Some(TooltipCandidate {
                    key: TooltipKey::Button(button.id),
                    text: text.to_owned(),
                    source: button.base.rect(),
                });
            }
        }
        for dropdown in &self.workspace_dropdowns {
            if !dropdown.base.test(pos) {
                continue;
            }
            if dropdown.map_view_selector {
                return Some(TooltipCandidate {
                    key: TooltipKey::MapViewSelector,
                    text: format!(
                        "Map View: {}\nChoose the data rendered as the base map.",
                        ictx.map_view_mode.map_or("None", MapViewMode::label)
                    ),
                    source: dropdown.base.rect(),
                });
            }
            if dropdown.overlays_selector {
                return Some(TooltipCandidate {
                    key: TooltipKey::OverlaysSelector,
                    text: format!(
                        "Overlays: {}\nToggle visual layers without changing map data.",
                        overlay_summary(ictx.enabled_options)
                    ),
                    source: dropdown.base.rect(),
                });
            }
        }
        let tool_buttons = if ictx.state_actions.state_view {
            &self.state_sidebar_tool_buttons
        } else {
            &self.sidebar_tool_buttons
        };
        tool_buttons
            .iter()
            .chain(self.sidebar_option_buttons.iter())
            .find_map(|button| {
                (button.base.test(pos))
                    .then(|| button.tooltip(ictx.view_mode))
                    .flatten()
                    .map(|text| TooltipCandidate {
                        key: TooltipKey::Button(button.id),
                        text: text.to_owned(),
                        source: button.base.rect(),
                    })
            })
    }

    fn draw_toolbar_button(
        &self,
        toolbar_button: &ToolbarButtonElement,
        ctx: Context,
        ictx: InterfaceDrawContext,
        pos: Option<Vector2<f64>>,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let selector_label = if toolbar_button.map_view_selector {
            Some(format!(
                "Map View: {}",
                ictx.map_view_mode.map_or("None", MapViewMode::label)
            ))
        } else if toolbar_button.overlays_selector {
            Some(format!(
                "Overlays: {}",
                overlay_summary(ictx.enabled_options)
            ))
        } else {
            None
        };
        if toolbar_button.enabled {
            if let Some(label) = selector_label.as_deref() {
                let label = fit_toolbar_label(label, toolbar_button.base.plate().size[0] - 24.0);
                toolbar_button
                    .base
                    .draw_with_label(ctx, true, true, &label, glyph_cache, gl);
                draw_chevron(ctx, toolbar_button.base.plate(), true, gl);
            } else {
                toolbar_button.base.draw(ctx, true, true, glyph_cache, gl);
            }
            for button in toolbar_button.visible_buttons(ictx) {
                let active = map_view_button_active(button.id, ictx.map_view_mode)
                    || overlay_button_active(button.id, ictx.enabled_options);
                button.draw(
                    ctx,
                    pos,
                    active,
                    ictx.state_actions.button_enabled(button.id),
                    glyph_cache,
                    gl,
                );
            }
        } else {
            let hover = toolbar_button.base.test_maybe(pos);
            if let Some(label) = selector_label.as_deref() {
                let label = fit_toolbar_label(label, toolbar_button.base.plate().size[0] - 24.0);
                toolbar_button.base.draw_with_label(
                    ctx,
                    hover,
                    false,
                    &label,
                    glyph_cache,
                    gl,
                );
                draw_chevron(ctx, toolbar_button.base.plate(), false, gl);
            } else {
                toolbar_button.base.draw(ctx, hover, false, glyph_cache, gl);
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ButtonElement {
    base: ButtonBase,
    id: ButtonId,
}

impl ButtonElement {
    fn draw(
        &self,
        ctx: Context,
        pos: Option<Vector2<f64>>,
        active: bool,
        enabled: bool,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        if enabled {
            self.base
                .draw(ctx, self.base.test_maybe(pos), active, glyph_cache, gl);
        } else {
            self.base.draw_with_palette(
                ctx,
                false,
                false,
                &PALETTE_BUTTON_DISABLED,
                glyph_cache,
                gl,
            );
        }
    }

    fn at_y(&self, y: u32) -> Self {
        let delta = y as f64 - self.base.plate().pos[1];
        Self {
            base: self.base.translated_y(delta),
            id: self.id,
        }
    }

    fn tooltip(&self, view_mode: Option<ViewMode>) -> Option<&'static str> {
        use ButtonId::*;
        match self.id {
            SidebarStateSelect => return Some("Select: Click a province to select its state"),
            SidebarStatePan => return Some("Pan: Drag the map without changing state data"),
            SidebarStateLasso => return Some("State Lasso (L): Select provinces by polygon"),
            SidebarStateBrush => {
                return Some("State Brush (B): Assign touched provinces to the target state");
            }
            SidebarStateFill => {
                return Some("State Fill (F): Preview a connected fill into the target state");
            }
            SidebarOptionRiverOverlay => {
                return Some("Rivers Overlay: Show or hide map/rivers.bmp");
            }
            SidebarOptionAdjacencies => {
                return Some("Adjacencies Overlay: Show province connections over any Map View");
            }
            SidebarOptionProvinceIds => {
                return Some("Province IDs Overlay: Show or hide province IDs");
            }
            SidebarOptionProvinceBoundaries => {
                return Some("Province Borders Overlay: Show or hide province borders");
            }
            SidebarOptionStateBoundaries => {
                return Some("State Borders Overlay: Show or hide state boundaries");
            }
            SidebarOptionImageOverlay => {
                return Some("Image Overlay: Show or hide the selected read-only reference image");
            }
            WorkspaceProvinces => {
                return Some(
                    "Workspace: Provinces\nEdit province geometry, terrain and definition data.",
                );
            }
            WorkspaceStates => {
                return Some(
                    "Workspace: States\nEdit state ownership, properties, provinces and buildings.",
                );
            }
            WorkspaceReviewChanges => {
                return Some(
                    "Review Changes\nInspect the current lossless patch preview before applying it.",
                );
            }
            WorkspaceApplyToMod => {
                return Some(
                    "Apply to Mod\nValidate, back up and apply the current supported changes.",
                );
            }
            _ => {}
        }
        let view_mode = view_mode?;

        match (self.id, view_mode) {
            (SidebarToolPaintArea, ViewMode::Color) => {
                Some("Paint Area: Drag to paint provinces under the brush")
            }
            (SidebarToolPaintArea, ViewMode::Kind) => {
                Some("Paint Area: Drag to assign province types")
            }
            (SidebarToolPaintArea, ViewMode::Terrain) => {
                Some("Paint Area: Drag to assign province terrain types")
            }
            (SidebarToolPaintArea, ViewMode::Continent) => {
                Some("Paint Area: Drag to assign provinces to continents")
            }
            (SidebarToolPaintBucket, ViewMode::Color) => {
                Some("Paint Bucket: Fill the hovered province with the current brush")
            }
            (SidebarToolLasso, ViewMode::Color) => {
                Some("Lasso: Draw a custom selection and then apply the current brush")
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct ToolbarButtonElement {
    base: ButtonBase,
    buttons: Vec<ButtonElement>,
    enabled: bool,
    map_view_selector: bool,
    overlays_selector: bool,
}

impl ToolbarButtonElement {
    fn visible_buttons(&self, ictx: InterfaceDrawContext) -> Vec<ButtonElement> {
        let mut y = self.base.plate().pos[1] as u32 + self.base.height();
        self.buttons
            .iter()
            .filter(|button| button_visible(button.id, ictx))
            .map(|button| {
                let button = button.at_y(y);
                y += button.base.height();
                button
            })
            .collect()
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonOrigin {
    TopLeft,
    #[allow(unused)]
    TopRight,
    BottomLeft,
    #[allow(unused)]
    BottomRight,
}

impl ButtonOrigin {
    pub const fn to_vector(self) -> Vector2<f64> {
        match self {
            ButtonOrigin::TopLeft => [0.0, 0.0],
            ButtonOrigin::TopRight => [1.0, 0.0],
            ButtonOrigin::BottomLeft => [0.0, 1.0],
            ButtonOrigin::BottomRight => [1.0, 1.0],
        }
    }
}

#[derive(Debug, Clone)]
enum ButtonBase {
    BoxFitWidth {
        text: TextComponent,
        plate: PlateComponent,
        colors: &'static Palette,
    },
    BoxDoubleText {
        text_left: TextComponent,
        text_right: TextComponent,
        plate: PlateComponent,
        colors: &'static Palette,
    },
    BoxTexture {
        texture: TextureComponent,
        plate: PlateComponent,
        colors: &'static Palette,
    },
}

impl ButtonBase {
    fn new_fit_width(text: &'static str, pos: Vector2<u32>, colors: &'static Palette) -> Self {
        let v_metrics = font::get_v_metrics();
        let text_pos = [
            pos[0] as f64 + PADDING[0],
            pos[1] as f64 + PADDING[1] + v_metrics.ascent,
        ];
        let plate_pos = [pos[0] as f64, pos[1] as f64];
        let plate_width = (font::get_width_metric_str(text) + PADDING[0] * 2.0).round();
        let plate_height = (v_metrics.ascent - v_metrics.descent + PADDING[1] * 2.0).round();
        ButtonBase::BoxFitWidth {
            text: TextComponent {
                pos: text_pos,
                text,
            },
            plate: PlateComponent {
                pos: plate_pos,
                size: [plate_width, plate_height],
            },
            colors,
        }
    }

    fn new_double_text(
        text: [&'static str; 2],
        pos: Vector2<u32>,
        width: u32,
        colors: &'static Palette,
    ) -> Self {
        let v_metrics = font::get_v_metrics();
        let text_y = pos[1] as f64 + PADDING[1] + v_metrics.ascent;
        let text_pos_left = [pos[0] as f64 + PADDING[0], text_y];
        let text_width_right = font::get_width_metric_str(text[1]);
        let text_pos_right = [
            pos[0] as f64 + width as f64 - text_width_right - PADDING[0],
            text_y,
        ];
        let plate_pos = [pos[0] as f64, pos[1] as f64];
        let plate_height = (v_metrics.ascent - v_metrics.descent + PADDING[1] * 2.0).round();
        ButtonBase::BoxDoubleText {
            text_left: TextComponent {
                pos: text_pos_left,
                text: text[0],
            },
            text_right: TextComponent {
                pos: text_pos_right,
                text: text[1],
            },
            plate: PlateComponent {
                pos: plate_pos,
                size: [width as f64, plate_height],
            },
            colors,
        }
    }

    fn new_texture(
        sprite_coords: [u32; 4],
        pos: Vector2<u32>,
        origin: ButtonOrigin,
        colors: &'static Palette,
    ) -> Self {
        let pad = f64::min(PADDING[0], PADDING[1]);
        let size = [
            sprite_coords[2] as f64 + pad * 2.0,
            sprite_coords[3] as f64 + pad * 2.0,
        ];
        let offset = vecmath::vec2_mul(size, origin.to_vector());
        let texture = Arc::new(get_sprite(sprite_coords));
        let texture_pos = vecmath::vec2_sub([pos[0] as f64 + pad, pos[1] as f64 + pad], offset);
        let plate_pos = vecmath::vec2_sub([pos[0] as f64, pos[1] as f64], offset);
        ButtonBase::BoxTexture {
            texture: TextureComponent {
                pos: texture_pos,
                texture,
            },
            plate: PlateComponent {
                pos: plate_pos,
                size,
            },
            colors,
        }
    }

    fn width(&self) -> u32 {
        self.plate().size[0] as u32
    }

    fn height(&self) -> u32 {
        self.plate().size[1] as u32
    }

    fn test_maybe(&self, pos: Option<Vector2<f64>>) -> bool {
        if let Some(pos) = pos {
            self.test(pos)
        } else {
            false
        }
    }

    fn test(&self, pos: Vector2<f64>) -> bool {
        self.plate().test(pos)
    }

    fn draw(
        &self,
        ctx: Context,
        hover: bool,
        active: bool,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        self.draw_with_palette(ctx, hover, active, self.colors(), glyph_cache, gl);
    }

    fn draw_with_label(
        &self,
        ctx: Context,
        hover: bool,
        active: bool,
        label: &str,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let Self::BoxFitWidth {
            text,
            plate,
            colors,
        } = self
        else {
            self.draw(ctx, hover, active, glyph_cache, gl);
            return;
        };
        plate.draw(ctx, hover, active, colors, gl);
        graphics::text(
            colors.foreground,
            FONT_SIZE,
            label,
            glyph_cache,
            ctx.transform.trans_pos(text.pos),
            gl,
        )
        .expect("unable to draw toolbar text");
    }

    fn draw_with_palette(
        &self,
        ctx: Context,
        hover: bool,
        active: bool,
        colors: &Palette,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        match self {
            ButtonBase::BoxFitWidth { text, plate, .. } => {
                plate.draw(ctx, hover, active, colors, gl);
                text.draw(ctx, colors, glyph_cache, gl);
            }
            ButtonBase::BoxDoubleText {
                text_left,
                text_right,
                plate,
                ..
            } => {
                plate.draw(ctx, hover, active, colors, gl);
                text_left.draw(ctx, colors, glyph_cache, gl);
                text_right.draw(ctx, colors, glyph_cache, gl);
            }
            ButtonBase::BoxTexture { texture, plate, .. } => {
                plate.draw(ctx, hover, active, colors, gl);
                texture.draw(ctx, gl);
            }
        }
    }

    fn colors(&self) -> &'static Palette {
        match self {
            ButtonBase::BoxFitWidth { colors, .. }
            | ButtonBase::BoxDoubleText { colors, .. }
            | ButtonBase::BoxTexture { colors, .. } => colors,
        }
    }

    fn translated_y(&self, delta: f64) -> Self {
        let mut translated = self.clone();
        match &mut translated {
            Self::BoxFitWidth { text, plate, .. } => {
                text.pos[1] += delta;
                plate.pos[1] += delta;
            }
            Self::BoxDoubleText {
                text_left,
                text_right,
                plate,
                ..
            } => {
                text_left.pos[1] += delta;
                text_right.pos[1] += delta;
                plate.pos[1] += delta;
            }
            Self::BoxTexture { texture, plate, .. } => {
                texture.pos[1] += delta;
                plate.pos[1] += delta;
            }
        }
        translated
    }

    fn plate(&self) -> &PlateComponent {
        match self {
            ButtonBase::BoxFitWidth { plate, .. } => plate,
            ButtonBase::BoxDoubleText { plate, .. } => plate,
            ButtonBase::BoxTexture { plate, .. } => plate,
        }
    }

    fn rect(&self) -> [f64; 4] {
        let plate = self.plate();
        [
            plate.pos[0],
            plate.pos[1],
            plate.size[0],
            plate.size[1],
        ]
    }
}

#[derive(Debug, Clone, Copy)]
struct TextComponent {
    pos: Vector2<f64>,
    text: &'static str,
}

impl TextComponent {
    fn draw(
        &self,
        ctx: Context,
        colors: &Palette,
        glyph_cache: &mut FontGlyphCache,
        gl: &mut GlGraphics,
    ) {
        let transform = ctx.transform.trans_pos(self.pos);
        graphics::text(
            colors.foreground,
            FONT_SIZE,
            self.text,
            glyph_cache,
            transform,
            gl,
        )
        .expect("unable to draw text");
    }
}

#[derive(Clone)]
struct TextureComponent {
    pos: Vector2<f64>,
    texture: Arc<Texture>,
}

impl TextureComponent {
    fn draw(&self, ctx: Context, gl: &mut GlGraphics) {
        let transform = ctx.transform.trans_pos(self.pos);
        graphics::image(&*self.texture, transform, gl);
    }
}

impl fmt::Debug for TextureComponent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("TextureComponent")
            .field("pos", &self.pos)
            .field("texture", &format_args!("..."))
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
struct PlateComponentStyled {
    plate: PlateComponent,
    colors: &'static Palette,
}

impl PlateComponentStyled {
    fn draw(&self, ctx: Context, hover: bool, active: bool, gl: &mut GlGraphics) {
        self.plate.draw(ctx, hover, active, self.colors, gl);
    }

    fn test(&self, pos: Vector2<f64>) -> bool {
        self.plate.test(pos)
    }
}

#[derive(Debug, Clone, Copy)]
struct PlateComponent {
    pos: Vector2<f64>,
    size: Vector2<f64>,
}

impl PlateComponent {
    fn draw(&self, ctx: Context, hover: bool, active: bool, colors: &Palette, gl: &mut GlGraphics) {
        let color = if active {
            if hover {
                colors.background_hover_active
            } else {
                colors.background_active
            }
        } else {
            if hover {
                colors.background_hover
            } else {
                colors.background
            }
        };

        graphics::rectangle(
            color,
            [self.pos[0], self.pos[1], self.size[0], self.size[1]],
            ctx.transform,
            gl,
        );
    }

    fn test(&self, pos: Vector2<f64>) -> bool {
        let upper = vecmath::vec2_add(self.pos, self.size);
        pos[0] >= self.pos[0] && pos[1] >= self.pos[1] && pos[0] < upper[0] && pos[1] < upper[1]
    }

    const fn styled(self, colors: &'static Palette) -> PlateComponentStyled {
        PlateComponentStyled {
            plate: self,
            colors,
        }
    }
}

#[derive(Debug, Clone)]
struct Palette {
    foreground: DrawColor,
    background: DrawColor,
    background_active: DrawColor,
    background_hover: DrawColor,
    background_hover_active: DrawColor,
}

fn draw_tooltip(
    ctx: Context,
    text: &str,
    source: [f64; 4],
    window_size: [f64; 2],
    glyph_cache: &mut FontGlyphCache,
    gl: &mut GlGraphics,
) {
    let v_metrics = font::get_v_metrics();
    let lines = wrap_tooltip(text, TOOLTIP_MAX_TEXT_WIDTH);
    let line_height = (v_metrics.ascent - v_metrics.descent + 2.0).round();
    let text_width = lines
        .iter()
        .map(|line| font::get_width_metric_str(line))
        .fold(0.0, f64::max)
        .min(TOOLTIP_MAX_TEXT_WIDTH);
    let plate_width = (text_width.max(TOOLTIP_MIN_WIDTH) + PADDING[0] * 2.0)
        .min(window_size[0].max(1.0));
    let plate_height = (line_height * lines.len() as f64 + PADDING[1] * 2.0)
        .min(window_size[1].max(1.0));
    let below = source[1] + source[3] + 8.0;
    let above = source[1] - plate_height - 8.0;
    let y = if below + plate_height <= window_size[1] {
        below
    } else {
        above.max(0.0)
    };
    let x = source[0]
        .min((window_size[0] - plate_width).max(0.0))
        .max(0.0);
    let plate_pos = snap_pos([x, y]);

    graphics::rectangle(
        colors::BUTTON_HOVER,
        [plate_pos[0], plate_pos[1], plate_width, plate_height],
        ctx.transform,
        gl,
    );
    graphics::rectangle(
        colors::WHITE_T,
        [plate_pos[0], plate_pos[1], plate_width, 1.0],
        ctx.transform,
        gl,
    );
    for (index, line) in lines.iter().enumerate() {
        let text_pos = snap_pos([
            plate_pos[0] + PADDING[0],
            plate_pos[1] + PADDING[1] + v_metrics.ascent + line_height * index as f64,
        ]);
        graphics::text(
            colors::WHITE,
            FONT_SIZE,
            line,
            glyph_cache,
            ctx.transform.trans_pos(text_pos),
            gl,
        )
        .expect("unable to draw tooltip text");
    }
}

fn wrap_tooltip(text: &str, max_width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && font::get_width_metric_str(&candidate) > max_width {
                lines.push(std::mem::take(&mut line));
                line.push_str(word);
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn fit_toolbar_label(label: &str, max_width: f64) -> String {
    if font::get_width_metric_str(label) <= max_width {
        return label.to_owned();
    }
    let mut text = label.chars().collect::<Vec<_>>();
    while !text.is_empty() {
        text.pop();
        let candidate = format!("{}…", text.iter().collect::<String>());
        if font::get_width_metric_str(&candidate) <= max_width {
            return candidate;
        }
    }
    "…".to_owned()
}

fn draw_chevron(ctx: Context, plate: &PlateComponent, open: bool, gl: &mut GlGraphics) {
    let center = [
        plate.pos[0] + plate.size[0] - 11.0,
        plate.pos[1] + plate.size[1] / 2.0,
    ];
    let direction = if open { -1.0 } else { 1.0 };
    graphics::line_from_to(
        colors::WHITE,
        1.4,
        [center[0] - 4.0, center[1] - 2.0 * direction],
        [center[0], center[1] + 2.0 * direction],
        ctx.transform,
        gl,
    );
    graphics::line_from_to(
        colors::WHITE,
        1.4,
        [center[0], center[1] + 2.0 * direction],
        [center[0] + 4.0, center[1] - 2.0 * direction],
        ctx.transform,
        gl,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ButtonId {
    ToolbarFileOpenFileArchive,
    ToolbarFileOpenFolder,
    ToolbarFileSave,
    ToolbarFileSaveAsArchive,
    ToolbarFileSaveAsFolder,
    ToolbarFileReveal,
    ToolbarFileExportLandMap,
    ToolbarFileExportTerrainMap,
    WorkspaceProvinces,
    WorkspaceStates,
    WorkspaceReviewChanges,
    WorkspaceApplyToMod,
    ToolbarEditUndo,
    ToolbarEditRedo,
    ToolbarEditFindMap,
    ToolbarEditNewState,
    ToolbarEditRemoveState,
    ToolbarEditStateProperties,
    ToolbarEditProvinceData,
    ToolbarEditSelectTargetStateProvinces,
    ToolbarEditActivateStateLasso,
    ToolbarEditStateLassoReplace,
    ToolbarEditStateLassoAdd,
    ToolbarEditStateLassoRemove,
    ToolbarEditStateLassoCentroid,
    ToolbarEditStateLassoAnyIntersection,
    ToolbarEditStateLassoMajority,
    ToolbarEditConfirmStateLasso,
    ToolbarEditCancelStateLasso,
    ToolbarEditActivateStateBrushAssign,
    ToolbarEditActivateStateBrushUnassign,
    ToolbarEditCancelStateBrush,
    ToolbarEditActivateStateFillHovered,
    ToolbarEditActivateStateFillConnectedState,
    ToolbarEditActivateStateFillConnectedUnassigned,
    ToolbarEditActivateStateFillWholeState,
    ToolbarEditConfirmStateFill,
    ToolbarEditCancelStateFill,
    ToolbarEditMoveSelectedToTarget,
    ToolbarEditUnassignSelected,
    ToolbarEditClearStateSelection,
    ToolbarEditDiscardStateSession,
    ToolbarEditCoastal,
    ToolbarEditRecolor,
    ToolbarEditProblems,
    ToolbarEditToggleLassoSnap,
    ToolbarEditNextMaskMode,
    ToolbarEditAdjacencies,
    ToolbarPatchGenerate,
    ToolbarPatchRegenerate,
    ToolbarPatchPreviousFile,
    ToolbarPatchNextFile,
    ToolbarPatchValidate,
    ToolbarPatchValidateReview,
    ToolbarPatchCancelValidation,
    ToolbarPatchViewValidationReport,
    ToolbarPatchClearValidation,
    ToolbarPatchSaveStateFiles,
    ToolbarPatchCancelSave,
    ToolbarPatchViewSaveReport,
    ToolbarPatchRecoverSave,
    ToolbarPatchClear,
    ToolbarViewMode1,
    ToolbarViewMode2,
    ToolbarViewMode3,
    ToolbarViewMode4,
    ToolbarViewMode5,
    ToolbarViewMode6,
    ToolbarViewProvinceMap,
    ToolbarViewStateMap,
    ToolbarViewPoliticalMap,
    ToolbarViewToggleAdjacencies,
    ToolbarViewToggleImageOverlay,
    ToolbarViewToggleStateBoundaries,
    ToolbarViewToggleProvinceIds,
    ToolbarViewToggleProvinceBoundaries,
    ToolbarViewToggleRiverOverlay,
    ToolbarViewToggleStateInspector,
    ToolbarViewCycleProvinceLabels,
    ToolbarViewCycleDeveloperDiagnostics,
    ToolbarViewChooseBaseGameDefinitions,
    ToolbarViewClearBaseGameDefinitions,
    ToolbarViewResetZoom,
    ToolbarViewFontLicense,
    ToolbarHelpAbout,
    ToolbarHelpCopyVersion,
    ToolbarHelpOpenLogs,
    ToolbarImageChoose,
    ToolbarImageUseProjectHeightmap,
    ToolbarImageToggleVisible,
    ToolbarImageOpacityDown,
    ToolbarImageOpacityUp,
    ToolbarImageClear,
    #[cfg(any(debug_assertions, feature = "debug-mode"))]
    ToolbarDebugValidatePixelCounts,
    #[cfg(any(debug_assertions, feature = "debug-mode"))]
    ToolbarDebugTriggerCrash,
    SidebarToolPaintArea,
    SidebarToolPaintBucket,
    SidebarToolLasso,
    SidebarStateSelect,
    SidebarStatePan,
    SidebarStateLasso,
    SidebarStateBrush,
    SidebarStateFill,
    SidebarOptionProvinceIds,
    SidebarOptionProvinceBoundaries,
    SidebarOptionRiverOverlay,
    SidebarOptionAdjacencies,
    SidebarOptionStateBoundaries,
    SidebarOptionImageOverlay,
}

fn map_view_button_active(id: ButtonId, view: Option<MapViewMode>) -> bool {
    matches!(
        (id, view),
        (ButtonId::ToolbarViewMode1, Some(MapViewMode::ProvinceColors))
            | (ButtonId::ToolbarViewMode2, Some(MapViewMode::ProvinceTypes))
            | (ButtonId::ToolbarViewMode3, Some(MapViewMode::Terrain))
            | (ButtonId::ToolbarViewMode4, Some(MapViewMode::Continents))
            | (ButtonId::ToolbarViewMode5, Some(MapViewMode::Coastal))
            | (ButtonId::ToolbarViewStateMap, Some(MapViewMode::States))
            | (ButtonId::ToolbarViewPoliticalMap, Some(MapViewMode::Political))
    )
}

fn overlay_button_active(id: ButtonId, enabled: [bool; 6]) -> bool {
    let index = match id {
        ButtonId::ToolbarViewToggleRiverOverlay => 0,
        ButtonId::ToolbarViewToggleAdjacencies => 1,
        ButtonId::ToolbarViewToggleProvinceIds => 2,
        ButtonId::ToolbarViewToggleProvinceBoundaries => 3,
        ButtonId::ToolbarViewToggleStateBoundaries => 4,
        ButtonId::ToolbarViewToggleImageOverlay | ButtonId::ToolbarImageToggleVisible => 5,
        _ => return false,
    };
    enabled[index]
}

fn overlay_summary(enabled: [bool; 6]) -> String {
    let labels = [
        "Rivers",
        "Adjacencies",
        "Province IDs",
        "Province Borders",
        "State Borders",
        "Image",
    ];
    let active = labels
        .into_iter()
        .zip(enabled)
        .filter_map(|(label, active)| active.then_some(label))
        .collect::<Vec<_>>();
    if active.is_empty() {
        "None".to_owned()
    } else if active.len() > 2 {
        format!("{} active", active.len())
    } else {
        active.join(", ")
    }
}

fn button_visible(id: ButtonId, ictx: InterfaceDrawContext) -> bool {
    use ButtonId::*;
    let state_only = matches!(
        id,
        ToolbarEditNewState
            | ToolbarEditRemoveState
            | ToolbarEditStateProperties
            | ToolbarEditProvinceData
            | ToolbarEditSelectTargetStateProvinces
            | ToolbarEditMoveSelectedToTarget
            | ToolbarEditUnassignSelected
            | ToolbarEditDiscardStateSession
            | ToolbarPatchGenerate
            | ToolbarPatchRegenerate
            | ToolbarPatchPreviousFile
            | ToolbarPatchNextFile
            | ToolbarPatchValidate
            | ToolbarPatchValidateReview
            | ToolbarPatchCancelValidation
            | ToolbarPatchViewValidationReport
            | ToolbarPatchClearValidation
            | ToolbarPatchSaveStateFiles
            | ToolbarPatchCancelSave
            | ToolbarPatchViewSaveReport
            | ToolbarPatchRecoverSave
            | ToolbarPatchClear
    );
    let province_only = matches!(
        id,
        ToolbarEditCoastal
            | ToolbarEditRecolor
            | ToolbarEditProblems
            | ToolbarEditAdjacencies
    );
    if state_only && !ictx.state_actions.state_view {
        return false;
    }
    if province_only && ictx.state_actions.state_view {
        return false;
    }
    if matches!(
        id,
        ToolbarEditToggleLassoSnap
            | ToolbarEditStateLassoReplace
            | ToolbarEditStateLassoAdd
            | ToolbarEditStateLassoRemove
            | ToolbarEditStateLassoCentroid
            | ToolbarEditStateLassoAnyIntersection
            | ToolbarEditStateLassoMajority
            | ToolbarEditConfirmStateLasso
            | ToolbarEditCancelStateLasso
    ) {
        return ictx.state_actions.state_view && ictx.state_actions.lasso_active;
    }
    if matches!(
        id,
        ToolbarEditNextMaskMode
            | ToolbarEditActivateStateBrushAssign
            | ToolbarEditActivateStateBrushUnassign
            | ToolbarEditCancelStateBrush
    ) {
        return ictx.state_actions.state_view && ictx.state_actions.brush_active;
    }
    if matches!(
        id,
        ToolbarEditActivateStateFillHovered
            | ToolbarEditActivateStateFillConnectedState
            | ToolbarEditActivateStateFillConnectedUnassigned
            | ToolbarEditActivateStateFillWholeState
            | ToolbarEditConfirmStateFill
            | ToolbarEditCancelStateFill
    ) {
        return ictx.state_actions.state_view && ictx.state_actions.fill_active;
    }
    if matches!(id, ToolbarViewStateMap | ToolbarViewPoliticalMap)
        && !ictx.states_available
    {
        return false;
    }
    true
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StateActionAvailability {
    pub state_view: bool,
    pub lasso_active: bool,
    pub lasso_preview: bool,
    pub brush_active: bool,
    pub fill_active: bool,
    pub fill_preview: bool,
    pub has_selection: bool,
    pub has_target: bool,
    pub can_move: bool,
    pub can_unassign: bool,
    pub can_edit_properties: bool,
    pub can_edit_province_data: bool,
    pub can_create_state: bool,
    pub can_remove_state: bool,
    pub property_editor_open: bool,
    pub property_draft_modified: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub has_edits: bool,
    pub has_patch_preview: bool,
    pub patch_preview_files: usize,
    pub patch_preview_stale: bool,
    pub patch_preview_blocked: bool,
    pub patch_preview_review_required: bool,
    pub validation_running: bool,
    pub has_validation_report: bool,
    pub save_eligible: bool,
    pub save_running: bool,
    pub save_cancellable: bool,
    pub recovery_required: bool,
    pub has_save_report: bool,
}

impl StateActionAvailability {
    fn button_enabled(self, id: ButtonId) -> bool {
        use ButtonId::*;
        match id {
            WorkspaceReviewChanges | WorkspaceApplyToMod => self.state_view,
            ToolbarEditUndo => self.can_undo,
            ToolbarEditRedo => self.can_redo,
            ToolbarEditNewState => {
                self.state_view && self.can_create_state && !self.property_editor_open
            }
            ToolbarEditRemoveState => {
                self.state_view && self.can_remove_state && !self.property_editor_open
            }
            ToolbarEditStateProperties => {
                self.state_view && self.can_edit_properties && !self.property_editor_open
            }
            ToolbarEditProvinceData => {
                self.state_view && self.can_edit_province_data && !self.property_editor_open
            }
            ToolbarEditSelectTargetStateProvinces => self.state_view && self.has_target,
            ToolbarEditActivateStateLasso => self.state_view && !self.lasso_active,
            ToolbarEditStateLassoReplace
            | ToolbarEditStateLassoAdd
            | ToolbarEditStateLassoRemove
            | ToolbarEditStateLassoCentroid
            | ToolbarEditStateLassoAnyIntersection
            | ToolbarEditStateLassoMajority => self.state_view,
            ToolbarEditConfirmStateLasso => self.lasso_preview,
            ToolbarEditCancelStateLasso => self.lasso_active,
            ToolbarEditActivateStateBrushAssign => {
                self.state_view && self.has_target && !self.property_editor_open
            }
            ToolbarEditActivateStateBrushUnassign => self.state_view && !self.property_editor_open,
            ToolbarEditCancelStateBrush => self.brush_active,
            ToolbarEditActivateStateFillHovered
            | ToolbarEditActivateStateFillConnectedState
            | ToolbarEditActivateStateFillConnectedUnassigned
            | ToolbarEditActivateStateFillWholeState => {
                self.state_view && self.has_target && !self.property_editor_open
            }
            ToolbarEditConfirmStateFill => self.fill_preview,
            ToolbarEditCancelStateFill => self.fill_active,
            ToolbarEditMoveSelectedToTarget => self.state_view && self.can_move,
            ToolbarEditUnassignSelected => self.state_view && self.can_unassign,
            ToolbarEditClearStateSelection => !self.state_view || self.has_selection,
            ToolbarEditDiscardStateSession => self.state_view && self.has_edits,
            ToolbarPatchGenerate => self.state_view,
            ToolbarPatchRegenerate => self.state_view && self.has_patch_preview,
            ToolbarPatchPreviousFile | ToolbarPatchNextFile => {
                self.state_view && self.patch_preview_files > 1
            }
            ToolbarPatchValidate => {
                self.state_view
                    && self.has_patch_preview
                    && !self.patch_preview_stale
                    && !self.patch_preview_blocked
                    && !self.patch_preview_review_required
                    && !self.validation_running
            }
            ToolbarPatchValidateReview => {
                self.state_view
                    && self.has_patch_preview
                    && !self.patch_preview_stale
                    && !self.patch_preview_blocked
                    && self.patch_preview_review_required
                    && !self.validation_running
            }
            ToolbarPatchCancelValidation => self.validation_running,
            ToolbarPatchViewValidationReport | ToolbarPatchClearValidation => {
                self.has_validation_report && !self.validation_running
            }
            ToolbarPatchSaveStateFiles => self.save_eligible && !self.save_running,
            ToolbarPatchCancelSave => self.save_running && self.save_cancellable,
            ToolbarPatchViewSaveReport => self.has_save_report && !self.save_running,
            ToolbarPatchRecoverSave => self.recovery_required && !self.save_running,
            ToolbarPatchClear => self.has_patch_preview,
            ToolbarViewStateMap | ToolbarViewPoliticalMap => self.state_view,
            _ => true,
        }
    }
}

type ToolbarButtonPrimitive<'a> = (&'a str, &'a [(&'a str, &'a str, ButtonId)]);
type ToolbarPrimitive<'a> = &'a [ToolbarButtonPrimitive<'a>];
type SidebarPrimitive<'a> = &'a [([u32; 4], ButtonId, SidebarPrimitiveKind)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarPrimitiveKind {
    Tool,
    StateTool,
    Option,
}

const TOOLBAR_DROPDOWN_WIDTH: u32 = 320;
const WORKSPACE_DROPDOWNS: &[(
    &str,
    &[(&str, &str, ButtonId)],
    bool,
    bool,
)] = &[
    (
        "Map View",
        &[
            ("Province Colors", "1", ButtonId::ToolbarViewMode1),
            ("Province Types", "2", ButtonId::ToolbarViewMode2),
            ("Terrain / Biome", "3", ButtonId::ToolbarViewMode3),
            ("Continents", "4", ButtonId::ToolbarViewMode4),
            ("Coastal Provinces", "5", ButtonId::ToolbarViewMode5),
            ("States", "6", ButtonId::ToolbarViewStateMap),
            ("Political", "7", ButtonId::ToolbarViewPoliticalMap),
        ],
        true,
        false,
    ),
    (
        "Overlays",
        &[
            ("Rivers", "", ButtonId::ToolbarViewToggleRiverOverlay),
            (
                "Adjacencies",
                "",
                ButtonId::ToolbarViewToggleAdjacencies,
            ),
            ("Province IDs", "9", ButtonId::ToolbarViewToggleProvinceIds),
            (
                "Province Borders",
                "",
                ButtonId::ToolbarViewToggleProvinceBoundaries,
            ),
            (
                "State Borders",
                "",
                ButtonId::ToolbarViewToggleStateBoundaries,
            ),
            (
                "Image Overlay",
                "",
                ButtonId::ToolbarViewToggleImageOverlay,
            ),
        ],
        false,
        true,
    ),
];
const TOOLBAR_PRIMITIVE: ToolbarPrimitive<'static> = &[
    (
        "File",
        &[
            (
                "Open File or Archive...",
                "Ctrl+Alt+O",
                ButtonId::ToolbarFileOpenFileArchive,
            ),
            ("Open HOI4 Mod...", "Ctrl+O", ButtonId::ToolbarFileOpenFolder),
            (
                "Review Changes",
                "",
                ButtonId::WorkspaceReviewChanges,
            ),
            ("Save", "Ctrl+S", ButtonId::ToolbarFileSave),
            (
                "Save As Archive...",
                "Ctrl+Shift+Alt+S",
                ButtonId::ToolbarFileSaveAsArchive,
            ),
            (
                "Save As...",
                "Ctrl+Shift+S",
                ButtonId::ToolbarFileSaveAsFolder,
            ),
            (
                "Reveal in File Browser",
                "Ctrl+Alt+R",
                ButtonId::ToolbarFileReveal,
            ),
            ("Export Land Map...", "", ButtonId::ToolbarFileExportLandMap),
            (
                "Export Terrain Map...",
                "",
                ButtonId::ToolbarFileExportTerrainMap,
            ),
        ],
    ),
    (
        "Edit",
        &[
            ("Undo", "Ctrl+Z", ButtonId::ToolbarEditUndo),
            ("Redo", "Ctrl+Y", ButtonId::ToolbarEditRedo),
            ("Find on Map", "Ctrl+F", ButtonId::ToolbarEditFindMap),
            ("New State", "", ButtonId::ToolbarEditNewState),
            (
                "Remove state from session",
                "",
                ButtonId::ToolbarEditRemoveState,
            ),
            (
                "Edit state properties",
                "",
                ButtonId::ToolbarEditStateProperties,
            ),
            ("Edit province data", "", ButtonId::ToolbarEditProvinceData),
            (
                "Select All Provinces in Target State",
                "",
                ButtonId::ToolbarEditSelectTargetStateProvinces,
            ),
            (
                "Move Selected Provinces to Target State",
                "M",
                ButtonId::ToolbarEditMoveSelectedToTarget,
            ),
            (
                "Unassign Selected Provinces",
                "Delete",
                ButtonId::ToolbarEditUnassignSelected,
            ),
            (
                "Clear Selection",
                "Esc",
                ButtonId::ToolbarEditClearStateSelection,
            ),
            (
                "Discard all in-memory state edits",
                "Ctrl+Shift+D",
                ButtonId::ToolbarEditDiscardStateSession,
            ),
            (
                "Re-calculate Coastal Provinces",
                "Shift+C",
                ButtonId::ToolbarEditCoastal,
            ),
            (
                "Re-color Provinces",
                "Shift+R",
                ButtonId::ToolbarEditRecolor,
            ),
            (
                "Calculate Map Errors/Warnings",
                "Shift+P",
                ButtonId::ToolbarEditProblems,
            ),
            (
                "Edit Adjacencies",
                "",
                ButtonId::ToolbarEditAdjacencies,
            ),
        ],
    ),
    (
        "Tools",
        &[
            (
                "Lasso Options: Pixel Snap",
                "",
                ButtonId::ToolbarEditToggleLassoSnap,
            ),
            (
                "Lasso Options: Replace Selection",
                "",
                ButtonId::ToolbarEditStateLassoReplace,
            ),
            (
                "Lasso Options: Add to Selection",
                "",
                ButtonId::ToolbarEditStateLassoAdd,
            ),
            (
                "Lasso Options: Remove from Selection",
                "",
                ButtonId::ToolbarEditStateLassoRemove,
            ),
            (
                "Lasso Options: Include Centroid",
                "",
                ButtonId::ToolbarEditStateLassoCentroid,
            ),
            (
                "Lasso Options: Include Any Intersection",
                "",
                ButtonId::ToolbarEditStateLassoAnyIntersection,
            ),
            (
                "Lasso Options: Include Majority",
                "",
                ButtonId::ToolbarEditStateLassoMajority,
            ),
            (
                "Lasso: Confirm Selection",
                "Enter",
                ButtonId::ToolbarEditConfirmStateLasso,
            ),
            (
                "Lasso: Cancel",
                "Esc",
                ButtonId::ToolbarEditCancelStateLasso,
            ),
            (
                "Brush Options: Next Mask Mode",
                "Shift+M",
                ButtonId::ToolbarEditNextMaskMode,
            ),
            (
                "Brush Mode: Assign to Target",
                "",
                ButtonId::ToolbarEditActivateStateBrushAssign,
            ),
            (
                "Brush Mode: Unassign",
                "",
                ButtonId::ToolbarEditActivateStateBrushUnassign,
            ),
            (
                "Brush: Cancel",
                "Esc",
                ButtonId::ToolbarEditCancelStateBrush,
            ),
            (
                "Fill Mode: Hovered Province",
                "",
                ButtonId::ToolbarEditActivateStateFillHovered,
            ),
            (
                "Fill Mode: Connected Same State",
                "",
                ButtonId::ToolbarEditActivateStateFillConnectedState,
            ),
            (
                "Fill Mode: Connected Unassigned",
                "",
                ButtonId::ToolbarEditActivateStateFillConnectedUnassigned,
            ),
            (
                "Fill Mode: Whole Source State",
                "",
                ButtonId::ToolbarEditActivateStateFillWholeState,
            ),
            (
                "Fill: Apply Preview",
                "Enter",
                ButtonId::ToolbarEditConfirmStateFill,
            ),
            (
                "Fill: Cancel",
                "Esc",
                ButtonId::ToolbarEditCancelStateFill,
            ),
            (
                "Preview / Generate",
                "",
                ButtonId::ToolbarPatchGenerate,
            ),
            (
                "Preview / Regenerate",
                "",
                ButtonId::ToolbarPatchRegenerate,
            ),
            (
                "Preview / Previous File",
                "",
                ButtonId::ToolbarPatchPreviousFile,
            ),
            (
                "Preview / Next File",
                "",
                ButtonId::ToolbarPatchNextFile,
            ),
            (
                "Validation / Temporary Copy",
                "",
                ButtonId::ToolbarPatchValidate,
            ),
            (
                "Validation / Review-Required",
                "",
                ButtonId::ToolbarPatchValidateReview,
            ),
            (
                "Validation / Cancel",
                "",
                ButtonId::ToolbarPatchCancelValidation,
            ),
            (
                "Validation / View Report",
                "",
                ButtonId::ToolbarPatchViewValidationReport,
            ),
            (
                "Validation / Clear Result",
                "",
                ButtonId::ToolbarPatchClearValidation,
            ),
            (
                "Save / Cancel",
                "",
                ButtonId::ToolbarPatchCancelSave,
            ),
            (
                "Save / View Report",
                "",
                ButtonId::ToolbarPatchViewSaveReport,
            ),
            (
                "Save / Recover Interrupted",
                "",
                ButtonId::ToolbarPatchRecoverSave,
            ),
            (
                "Preview / Clear",
                "",
                ButtonId::ToolbarPatchClear,
            ),
        ],
    ),
    (
        "View",
        &[
            ("Map View: Province Colors", "1", ButtonId::ToolbarViewMode1),
            ("Map View: Province Types", "2", ButtonId::ToolbarViewMode2),
            ("Map View: Terrain / Biome", "3", ButtonId::ToolbarViewMode3),
            ("Map View: Continents", "4", ButtonId::ToolbarViewMode4),
            ("Map View: Coastal Provinces", "5", ButtonId::ToolbarViewMode5),
            ("Map View: States", "6", ButtonId::ToolbarViewStateMap),
            ("Map View: Political", "7", ButtonId::ToolbarViewPoliticalMap),
            (
                "Overlays: Rivers",
                "",
                ButtonId::ToolbarViewToggleRiverOverlay,
            ),
            (
                "Overlays: Adjacencies",
                "",
                ButtonId::ToolbarViewToggleAdjacencies,
            ),
            (
                "Overlays: Province IDs",
                "9",
                ButtonId::ToolbarViewToggleProvinceIds,
            ),
            (
                "Overlays: Province Borders",
                "",
                ButtonId::ToolbarViewToggleProvinceBoundaries,
            ),
            (
                "Overlays: State Borders",
                "",
                ButtonId::ToolbarViewToggleStateBoundaries,
            ),
            (
                "Overlays: Image Overlay...",
                "",
                ButtonId::ToolbarViewToggleImageOverlay,
            ),
            (
                "Overlays: Configure Image...",
                "",
                ButtonId::ToolbarImageChoose,
            ),
            (
                "Overlays: Use Project Heightmap",
                "",
                ButtonId::ToolbarImageUseProjectHeightmap,
            ),
            (
                "Overlays: Opacity -10%",
                "",
                ButtonId::ToolbarImageOpacityDown,
            ),
            (
                "Overlays: Opacity +10%",
                "",
                ButtonId::ToolbarImageOpacityUp,
            ),
            (
                "Overlays: Clear Image",
                "",
                ButtonId::ToolbarImageClear,
            ),
            (
                "Panels: State Inspector",
                "",
                ButtonId::ToolbarViewToggleStateInspector,
            ),
            (
                "Panels: Developer Diagnostics",
                "F3",
                ButtonId::ToolbarViewCycleDeveloperDiagnostics,
            ),
            (
                "Definitions: Choose Base Game...",
                "",
                ButtonId::ToolbarViewChooseBaseGameDefinitions,
            ),
            (
                "Definitions: Clear Base Game",
                "",
                ButtonId::ToolbarViewClearBaseGameDefinitions,
            ),
            ("Reset Zoom", "H", ButtonId::ToolbarViewResetZoom),
        ],
    ),
    (
        "Help",
        &[
            ("About HOI4 Map Editor", "", ButtonId::ToolbarHelpAbout),
            (
                "Copy Version Information",
                "",
                ButtonId::ToolbarHelpCopyVersion,
            ),
            ("Open Logs Folder", "", ButtonId::ToolbarHelpOpenLogs),
            ("Font Licenses", "", ButtonId::ToolbarViewFontLicense),
        ],
    ),
];

const SIDEBAR_PRIMITIVE: SidebarPrimitive<'static> = &[
    (
        [00, 00, 24, 24],
        ButtonId::SidebarToolPaintArea,
        SidebarPrimitiveKind::Tool,
    ),
    (
        [24, 00, 24, 24],
        ButtonId::SidebarToolPaintBucket,
        SidebarPrimitiveKind::Tool,
    ),
    (
        [48, 00, 24, 24],
        ButtonId::SidebarToolLasso,
        SidebarPrimitiveKind::Tool,
    ),
    (
        [00, 00, 24, 24],
        ButtonId::SidebarStateSelect,
        SidebarPrimitiveKind::StateTool,
    ),
    (
        [24, 00, 24, 24],
        ButtonId::SidebarStatePan,
        SidebarPrimitiveKind::StateTool,
    ),
    (
        [48, 00, 24, 24],
        ButtonId::SidebarStateLasso,
        SidebarPrimitiveKind::StateTool,
    ),
    (
        [00, 00, 24, 24],
        ButtonId::SidebarStateBrush,
        SidebarPrimitiveKind::StateTool,
    ),
    (
        [24, 00, 24, 24],
        ButtonId::SidebarStateFill,
        SidebarPrimitiveKind::StateTool,
    ),
    (
        [48, 24, 24, 24],
        ButtonId::SidebarOptionRiverOverlay,
        SidebarPrimitiveKind::Option,
    ),
    (
        [24, 24, 24, 24],
        ButtonId::SidebarOptionAdjacencies,
        SidebarPrimitiveKind::Option,
    ),
    (
        [00, 24, 24, 24],
        ButtonId::SidebarOptionProvinceIds,
        SidebarPrimitiveKind::Option,
    ),
    (
        [24, 24, 24, 24],
        ButtonId::SidebarOptionProvinceBoundaries,
        SidebarPrimitiveKind::Option,
    ),
    (
        [48, 24, 24, 24],
        ButtonId::SidebarOptionStateBoundaries,
        SidebarPrimitiveKind::Option,
    ),
    (
        [00, 24, 24, 24],
        ButtonId::SidebarOptionImageOverlay,
        SidebarPrimitiveKind::Option,
    ),
];

fn get_sprite(sprite_coords: [u32; 4]) -> Texture {
    const SPRITESHEET_DATA: &[u8] = include_bytes!("../../assets/spritesheet.png");
    static SPRITESHEET: Lazy<RgbaImage> = Lazy::new(|| {
        let decoder = PngDecoder::new(SPRITESHEET_DATA).expect("unable to decode spritesheet");
        let img = DynamicImage::from_decoder(decoder).expect("unable to decode spritesheet");
        img.to_rgba8()
    });

    let [x, y, width, height] = sprite_coords;
    let view = SPRITESHEET.view(x, y, width, height);
    Texture::from_image(&view.to_image(), &TextureSettings::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface() -> Interface {
        let viewport = Viewport {
            rect: [0, 0, 1200, 800],
            draw_size: [1200, 800],
            window_size: [1200.0, 800.0],
        };
        let toolbar_buttons = ["File", "Edit", "View", "Tools"]
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                let x = index as u32 * 70;
                ToolbarButtonElement {
                    base: ButtonBase::new_fit_width(
                        label,
                        [x, 0],
                        &PALETTE_BUTTON_TOOLBAR,
                    ),
                    buttons: if label == "Tools" {
                        vec![ButtonElement {
                            base: ButtonBase::new_double_text(
                                ["Open Mod Folder", ""],
                                [x, 24],
                                TOOLBAR_DROPDOWN_WIDTH,
                                &PALETTE_BUTTON,
                            ),
                            id: ButtonId::ToolbarFileOpenFolder,
                        }]
                    } else {
                        Vec::new()
                    },
                    enabled: false,
                    map_view_selector: false,
                    overlays_selector: false,
                }
            })
            .collect();
        Interface {
            sidebar_tool_buttons: Vec::new(),
            state_sidebar_tool_buttons: Vec::new(),
            sidebar_option_buttons: Vec::new(),
            toolbar_buttons,
            workspace_buttons: Vec::new(),
            workspace_dropdowns: Vec::new(),
            toolbar_plate: PlateComponent {
                pos: [0.0, 0.0],
                size: [1200.0, 48.0],
            }
            .styled(&PALETTE_BUTTON_TOOLBAR),
            toolbar_height: 48,
            sidebar_plate: PlateComponent {
                pos: [0.0, 48.0],
                size: [0.0, 752.0],
            }
            .styled(&PALETTE_BUTTON),
            sidebar_width: 0,
            inspector_width: 0.0,
            viewport,
            tooltip: TooltipManager::default(),
        }
    }

    fn context(state_view: bool) -> InterfaceDrawContext {
        InterfaceDrawContext {
            map_view_mode: Some(MapViewMode::ProvinceColors),
            view_mode: Some(ViewMode::Color),
            selected_tool: Some(0),
            state_tool: None,
            enabled_options: [false; 6],
            available_options: [true; 6],
            states_available: true,
            state_actions: StateActionAvailability {
                state_view,
                ..Default::default()
            },
            blocks_tooltips: false,
        }
    }

    #[test]
    fn map_view_menu_marks_only_the_current_canonical_view() {
        assert!(map_view_button_active(
            ButtonId::ToolbarViewMode1,
            Some(MapViewMode::ProvinceColors)
        ));
        assert!(map_view_button_active(
            ButtonId::ToolbarViewStateMap,
            Some(MapViewMode::States)
        ));
        assert!(map_view_button_active(
            ButtonId::ToolbarViewPoliticalMap,
            Some(MapViewMode::Political)
        ));
        assert!(!map_view_button_active(
            ButtonId::ToolbarViewMode1,
            Some(MapViewMode::States)
        ));
        assert!(!map_view_button_active(
            ButtonId::ToolbarViewMode2,
            Some(MapViewMode::ProvinceColors)
        ));
    }

    #[test]
    fn map_view_menu_contains_only_the_seven_distinct_views() {
        let (_, entries, _, _) = WORKSPACE_DROPDOWNS
            .iter()
            .find(|(label, _, _, _)| *label == "Map View")
            .unwrap();
        let ids = entries.iter().map(|entry| entry.2).collect::<Vec<_>>();

        assert_eq!(ids.len(), 7);
        assert!(!ids.contains(&ButtonId::ToolbarViewProvinceMap));
        assert!(!ids.contains(&ButtonId::ToolbarViewMode6));
        assert_eq!(
            ids,
            vec![
                ButtonId::ToolbarViewMode1,
                ButtonId::ToolbarViewMode2,
                ButtonId::ToolbarViewMode3,
                ButtonId::ToolbarViewMode4,
                ButtonId::ToolbarViewMode5,
                ButtonId::ToolbarViewStateMap,
                ButtonId::ToolbarViewPoliticalMap,
            ]
        );
    }

    #[test]
    fn main_menu_is_limited_to_the_five_product_groups() {
        let labels = ["File", "Edit", "View", "Tools", "Help"];
        assert_eq!(TOOLBAR_PRIMITIVE.len(), labels.len());
        for label in labels {
            assert!(TOOLBAR_PRIMITIVE.iter().any(|(actual, _)| *actual == label));
        }
        assert!(!TOOLBAR_PRIMITIVE.iter().any(|(label, _)| matches!(
            *label,
            "State Lasso" | "State Brush" | "State Fill" | "Patch Preview" | "Map View"
        )));
    }

    #[test]
    fn open_menu_owns_overlapping_clicks_and_outside_close() {
        let mut interface = interface();
        let ictx = context(true);
        let tools = interface.toolbar_buttons[3].base.plate();
        let tools_pos = [
            tools.pos[0] + tools.size[0] / 2.0,
            tools.pos[1] + tools.size[1] / 2.0,
        ];
        assert_eq!(interface.on_mouse_click(tools_pos, ictx), Err(false));
        assert!(interface.toolbar_buttons[3].enabled);
        interface.on_mouse_position([600.0, 500.0], ictx);
        assert!(interface.toolbar_buttons[3].enabled);

        let first = *interface.toolbar_buttons[3].visible_buttons(ictx)[0]
            .base
            .plate();
        let item_pos = [
            first.pos[0] + first.size[0] / 2.0,
            first.pos[1] + first.size[1] / 2.0,
        ];
        assert_eq!(
            interface.on_mouse_click(item_pos, ictx),
            Ok(ButtonId::ToolbarFileOpenFolder)
        );
        assert!(!interface.toolbar_buttons[3].enabled);

        assert_eq!(interface.on_mouse_click(tools_pos, ictx), Err(false));
        assert_eq!(interface.on_mouse_click([600.0, 500.0], ictx), Err(false));
        assert!(!interface.toolbar_buttons[3].enabled);
    }

    #[test]
    fn view_menu_reflects_independent_overlay_states() {
        let enabled = [true, false, true, false, true, true];

        assert!(overlay_button_active(
            ButtonId::ToolbarViewToggleRiverOverlay,
            enabled
        ));
        assert!(!overlay_button_active(
            ButtonId::ToolbarViewToggleAdjacencies,
            enabled
        ));
        assert!(overlay_button_active(
            ButtonId::ToolbarViewToggleImageOverlay,
            enabled
        ));
        assert!(overlay_button_active(
            ButtonId::ToolbarImageToggleVisible,
            enabled
        ));
    }

    #[test]
    fn edit_and_tool_entries_follow_the_active_workspace_and_tool() {
        let provinces = context(false);
        assert!(!button_visible(ButtonId::ToolbarEditNewState, provinces));
        assert!(button_visible(ButtonId::ToolbarEditCoastal, provinces));

        let mut states = context(true);
        assert!(button_visible(ButtonId::ToolbarEditNewState, states));
        assert!(!button_visible(ButtonId::ToolbarEditCoastal, states));
        assert!(!button_visible(
            ButtonId::ToolbarEditToggleLassoSnap,
            states
        ));
        states.state_actions.lasso_active = true;
        assert!(button_visible(
            ButtonId::ToolbarEditToggleLassoSnap,
            states
        ));
    }

    #[test]
    fn compact_overlay_summary_lists_only_enabled_layers() {
        assert_eq!(
            overlay_summary([true, false, false, false, true, false]),
            "Rivers, State Borders"
        );
        assert_eq!(overlay_summary([false; 6]), "None");
    }

    #[test]
    fn tooltip_manager_delays_and_clears_the_single_candidate() {
        let candidate = TooltipCandidate {
            key: TooltipKey::Button(ButtonId::WorkspaceStates),
            text: "Workspace: States".to_owned(),
            source: [10.0, 10.0, 100.0, 24.0],
        };
        let mut manager = TooltipManager::default();
        manager.update(Some(candidate));
        manager.tick(TOOLTIP_DELAY_SECONDS - 0.01);
        assert!(manager.visible().is_none());
        manager.tick(0.02);
        assert_eq!(
            manager.visible().map(|tooltip| tooltip.text.as_str()),
            Some("Workspace: States")
        );
        manager.clear();
        assert!(manager.visible().is_none());
    }

    #[test]
    fn selector_labels_use_visual_chevrons_not_textual_arrows() {
        let label = fit_toolbar_label("Map View: Province Colors", 300.0);
        assert_eq!(label, "Map View: Province Colors");
        assert!(!label.ends_with('v'));
        assert!(!label.contains('▾'));
        assert!(wrap_tooltip(
            "Review Changes\nInspect the current lossless patch preview before applying it.",
            180.0
        )
        .len()
            >= 2);
    }
}
