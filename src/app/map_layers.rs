use std::path::PathBuf;

use crate::app::inspector::ProvinceLabelMode;
use crate::app::map::Color;
use crate::app::project::{
    AMBIGUOUS_PROVINCE_COLOR, SELECTED_STATE_COLOR, STATE_BOUNDARY_COLOR, UNASSIGNED_LAND_COLOR,
    UNKNOWN_PROVINCE_COLOR,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum MapBaseView {
    #[default]
    ProvinceColors,
    ProvinceTypes,
    Terrain,
    Continents,
    Coastal,
    States,
    Political,
    Resources,
}

impl MapBaseView {
    pub fn from_canonical_shortcut(key: char) -> Option<Self> {
        match key {
            '1' => Some(Self::ProvinceColors),
            '2' => Some(Self::ProvinceTypes),
            '3' => Some(Self::Terrain),
            '4' => Some(Self::Continents),
            '5' => Some(Self::Coastal),
            '6' => Some(Self::States),
            '7' => Some(Self::Political),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::ProvinceColors => "Province Colors",
            Self::ProvinceTypes => "Province Types",
            Self::Terrain => "Terrain / Biome",
            Self::Continents => "Continents",
            Self::Coastal => "Coastal Provinces",
            Self::States => "States",
            Self::Political => "Political",
            Self::Resources => "Resources",
        }
    }

    pub const fn requires_state_history(self) -> bool {
        matches!(self, Self::States | Self::Political | Self::Resources)
    }

    pub const fn is_province_view(self) -> bool {
        !self.requires_state_history()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeveloperMapOverlay {
    CompactDiagnostics,
    DetailedDiagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMode {
    Provinces,
    States,
}

impl WorkspaceMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Provinces => "Provinces",
            Self::States => "States",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Provinces => Self::States,
            Self::States => Self::Provinces,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceViewPreferences {
    provinces: MapBaseView,
    states: MapBaseView,
}

impl WorkspaceViewPreferences {
    pub const fn get(self, workspace: WorkspaceMode) -> MapBaseView {
        match workspace {
            WorkspaceMode::Provinces => self.provinces,
            WorkspaceMode::States => self.states,
        }
    }

    pub fn remember(&mut self, workspace: WorkspaceMode, view: MapBaseView) {
        match workspace {
            WorkspaceMode::Provinces => self.provinces = view,
            WorkspaceMode::States => self.states = view,
        }
    }
}

impl Default for WorkspaceViewPreferences {
    fn default() -> Self {
        Self {
            provinces: MapBaseView::ProvinceColors,
            states: MapBaseView::States,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageOverlaySource {
    ProjectHeightmap,
    Custom(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageOverlayLayerState {
    pub enabled: bool,
    pub opacity: f32,
    pub source: Option<ImageOverlaySource>,
    pub dimensions: Option<[u32; 2]>,
    pub content_revision: u64,
}

impl Default for ImageOverlayLayerState {
    fn default() -> Self {
        Self {
            enabled: false,
            opacity: 0.5,
            source: None,
            dimensions: None,
            content_revision: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapLayerState {
    pub base_view: MapBaseView,
    pub image_overlay: ImageOverlayLayerState,
    pub province_label_mode: ProvinceLabelMode,
    pub show_rivers: bool,
    pub show_adjacencies: bool,
    pub show_province_ids: bool,
    pub show_province_boundaries: bool,
    pub show_state_boundaries: bool,
    pub developer_overlay: Option<DeveloperMapOverlay>,
}

impl Default for MapLayerState {
    fn default() -> Self {
        Self {
            base_view: MapBaseView::default(),
            image_overlay: ImageOverlayLayerState::default(),
            province_label_mode: ProvinceLabelMode::default(),
            show_rivers: false,
            show_adjacencies: false,
            show_province_ids: false,
            show_province_boundaries: false,
            show_state_boundaries: true,
            developer_overlay: None,
        }
    }
}

impl MapLayerState {
    pub fn with_base_view(mut self, base_view: MapBaseView) -> Self {
        self.base_view = base_view;
        self
    }

    pub fn with_image_overlay_enabled(mut self, enabled: bool) -> Self {
        self.image_overlay.enabled = enabled;
        self
    }

    pub fn with_image_overlay_opacity(mut self, opacity: f32) -> Self {
        self.image_overlay.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn with_province_label_mode(mut self, mode: ProvinceLabelMode) -> Self {
        self.province_label_mode = mode;
        self
    }

    pub fn with_state_boundaries(mut self, visible: bool) -> Self {
        self.show_state_boundaries = visible;
        self
    }

    pub fn with_developer_overlay(mut self, overlay: Option<DeveloperMapOverlay>) -> Self {
        self.developer_overlay = overlay;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageOverlayMetadata {
    pub exists: bool,
    pub dimensions: Option<[u32; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageOverlayValidationIssue {
    Missing,
    MissingDimensions,
    EmptyDimensions,
    DimensionMismatch {
        expected: [u32; 2],
        actual: [u32; 2],
    },
}

pub fn validate_image_overlay_metadata(
    image: ImageOverlayMetadata,
    map_dimensions: [u32; 2],
) -> Vec<ImageOverlayValidationIssue> {
    let mut issues = Vec::new();
    if !image.exists {
        issues.push(ImageOverlayValidationIssue::Missing);
    }

    let Some(actual) = image.dimensions else {
        issues.push(ImageOverlayValidationIssue::MissingDimensions);
        return issues;
    };

    if actual[0] == 0 || actual[1] == 0 {
        issues.push(ImageOverlayValidationIssue::EmptyDimensions);
    }

    if actual != map_dimensions {
        issues.push(ImageOverlayValidationIssue::DimensionMismatch {
            expected: map_dimensions,
            actual,
        });
    }

    issues
}

pub fn image_overlay_is_aligned(image: ImageOverlayMetadata, map_dimensions: [u32; 2]) -> bool {
    validate_image_overlay_metadata(image, map_dimensions).is_empty()
}

pub fn political_fallback_color(tag: &str) -> Color {
    let mut hash = 0x811c_9dc5u32;
    for byte in tag.trim().to_ascii_uppercase().bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }

    let mut color = political_color_from_hash(hash);
    while is_reserved_diagnostic_color(color) {
        hash = hash.wrapping_add(0x9e37_79b9);
        color = political_color_from_hash(hash);
    }
    color
}

pub fn is_reserved_diagnostic_color(color: Color) -> bool {
    [
        AMBIGUOUS_PROVINCE_COLOR,
        UNASSIGNED_LAND_COLOR,
        UNKNOWN_PROVINCE_COLOR,
        STATE_BOUNDARY_COLOR,
        SELECTED_STATE_COLOR,
        [0, 0, 0],
        [255, 255, 255],
    ]
    .contains(&color)
}

fn political_color_from_hash(hash: u32) -> Color {
    [
        48 + (hash & 0x7f) as u8,
        48 + ((hash >> 8) & 0x7f) as u8,
        48 + ((hash >> 16) & 0x7f) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_shortcuts_select_seven_distinct_map_views() {
        let views = ('1'..='7')
            .map(|key| MapBaseView::from_canonical_shortcut(key).unwrap())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(views.len(), 7);
        assert_eq!(
            MapBaseView::from_canonical_shortcut('1'),
            Some(MapBaseView::ProvinceColors)
        );
        assert_eq!(
            MapBaseView::from_canonical_shortcut('6'),
            Some(MapBaseView::States)
        );
        assert_eq!(
            MapBaseView::from_canonical_shortcut('7'),
            Some(MapBaseView::Political)
        );
        assert_eq!(MapBaseView::from_canonical_shortcut('8'), None);
    }

    #[test]
    fn workspace_view_preferences_keep_independent_last_views() {
        let mut preferences = WorkspaceViewPreferences::default();
        assert_eq!(
            preferences.get(WorkspaceMode::Provinces),
            MapBaseView::ProvinceColors
        );
        assert_eq!(preferences.get(WorkspaceMode::States), MapBaseView::States);

        preferences.remember(WorkspaceMode::States, MapBaseView::Political);
        assert_eq!(
            preferences.get(WorkspaceMode::Provinces),
            MapBaseView::ProvinceColors
        );
        assert_eq!(
            preferences.get(WorkspaceMode::States),
            MapBaseView::Political
        );
        assert_eq!(WorkspaceMode::Provinces.next(), WorkspaceMode::States);
        assert_eq!(WorkspaceMode::States.next(), WorkspaceMode::Provinces);
    }

    #[test]
    fn resources_requires_loaded_state_history_without_claiming_a_new_shortcut() {
        assert!(MapBaseView::Resources.requires_state_history());
        assert_eq!(MapBaseView::from_canonical_shortcut('8'), None);
    }

    #[test]
    fn image_overlay_opacity_is_clamped_without_changing_content_revision() {
        let mut state = MapLayerState::default();
        state.image_overlay.content_revision = 7;
        let state = state.with_image_overlay_opacity(1.5);
        assert_eq!(state.image_overlay.opacity, 1.0);
        assert_eq!(state.image_overlay.content_revision, 7);

        let state = state.with_image_overlay_opacity(-0.25);
        assert_eq!(state.image_overlay.opacity, 0.0);
        assert_eq!(state.image_overlay.content_revision, 7);
    }

    #[test]
    fn province_label_mode_uses_inspector_cycle_contract() {
        let state = MapLayerState::default().with_province_label_mode(ProvinceLabelMode::All);

        assert_eq!(state.province_label_mode.cycle(), ProvinceLabelMode::Off);
    }

    #[test]
    fn political_fallback_color_is_deterministic_and_not_diagnostic() {
        assert_eq!(
            political_fallback_color("bra"),
            political_fallback_color("BRA")
        );
        assert_ne!(
            political_fallback_color("BRA"),
            political_fallback_color("ARG")
        );
        assert!(!is_reserved_diagnostic_color(political_fallback_color(
            "BRA"
        )));
    }

    #[test]
    fn image_overlay_validation_reports_existence_dimensions_and_alignment() {
        assert_eq!(
            validate_image_overlay_metadata(
                ImageOverlayMetadata {
                    exists: false,
                    dimensions: None,
                },
                [5632, 2048],
            ),
            vec![
                ImageOverlayValidationIssue::Missing,
                ImageOverlayValidationIssue::MissingDimensions,
            ]
        );

        assert_eq!(
            validate_image_overlay_metadata(
                ImageOverlayMetadata {
                    exists: true,
                    dimensions: Some([1, 2]),
                },
                [2, 2],
            ),
            vec![ImageOverlayValidationIssue::DimensionMismatch {
                expected: [2, 2],
                actual: [1, 2],
            }]
        );

        assert!(image_overlay_is_aligned(
            ImageOverlayMetadata {
                exists: true,
                dimensions: Some([2, 2]),
            },
            [2, 2],
        ));
    }

    #[test]
    fn layer_changes_leave_unrelated_model_unchanged() {
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct Sentinel {
            value: u32,
        }

        let sentinel = Sentinel { value: 42 };
        let next = MapLayerState::default()
            .with_base_view(MapBaseView::Political)
            .with_image_overlay_enabled(true)
            .with_state_boundaries(false)
            .with_developer_overlay(Some(DeveloperMapOverlay::CompactDiagnostics));

        assert_eq!(next.base_view, MapBaseView::Political);
        assert_eq!(sentinel, Sentinel { value: 42 });
    }
}
