//! Runtime ownership for read-only map presentation.
//!
//! `Canvas` supplies current map/session inputs and remains responsible for frame
//! orchestration. This module owns only derived presentation caches.  It never
//! mutates project or State edit data, and it has no dependency on `Canvas` or
//! `App`.

mod export;

pub(crate) use export::{ExportOverlays, compose_export_overlays, save_png};

use std::collections::{BTreeMap, HashMap};

use image::{Rgba, RgbaImage};
use opengl_graphics::{Filter, Texture, TextureSettings};

use super::map::Map;
use super::map_layers::MapBaseView;
use super::political::{PoliticalCountryCatalog, PoliticalLabel, TerritoryAnchorIndex};
use super::project::{
    MapPresentationModel, ProblemsOverlayModel, ProjectGeneration, ProjectValidationDiagnostic,
    SourceGeneration, StateEditSession, StrategicRegionLoadResult, StrategicRegionStateSplitModel,
    StrategicRegionsMapModel, build_map_presentation, build_overlay, build_state_split_model,
    build_strategic_regions_map_model, collect_strategic_region_boundaries,
    strategic_region_selection_overlay, strategic_region_state_split_overlay,
    strategic_region_texture,
};
use super::resources::{ResourceIconResolver, ResourceMapLabel};

/// Lazy assets and metadata specific to the Political base view.
#[derive(Default)]
pub(crate) struct PoliticalPresentationCache {
    pub(crate) texture: Option<Texture>,
    pub(crate) country_catalog: Option<PoliticalCountryCatalog>,
    pub(crate) cache_generation: Option<ProjectGeneration>,
    pub(crate) labels: Vec<PoliticalLabel>,
    pub(crate) flag_textures: BTreeMap<String, Texture>,
    pub(crate) adjacency_pairs: Vec<(u32, u32)>,
    pub(crate) language: String,
}

/// Lazy assets and metadata specific to Resources labels.
#[derive(Default)]
pub(crate) struct ResourcesPresentationCache {
    pub(crate) labels: Vec<ResourceMapLabel>,
    pub(crate) icon_resolver: Option<ResourceIconResolver>,
    pub(crate) icon_textures: BTreeMap<String, Option<Texture>>,
    pub(crate) cache_generation: Option<ProjectGeneration>,
}

/// Derived, read-only assets for the Strategic Regions base view. The base
/// model is map-generation scoped; the split decoration is State-revision
/// scoped; and selection is intentionally independent from both.
#[derive(Default)]
pub(crate) struct StrategicRegionsPresentationCache {
    pub(crate) model: Option<StrategicRegionsMapModel>,
    pub(crate) texture: Option<Texture>,
    pub(crate) boundaries: Vec<uord::UOrd2<vecmath::Vector2<u32>>>,
    pub(crate) split: Option<StrategicRegionStateSplitModel>,
    pub(crate) split_texture: Option<Texture>,
    pub(crate) split_revision: Option<u64>,
    pub(crate) selection_texture: Option<Texture>,
    pub(crate) selected_id: Option<u32>,
}

/// Generation- and revision-bound resources needed to present State data.
///
/// Invalidation matrix:
///
/// | Event | State model/textures | Problems overlay |
/// | --- | --- | --- |
/// | Project replacement | drop | drop |
/// | State revision | drop | retained until validation refresh |
/// | Validation refresh | retain | rebuild |
/// | View change | retain (lazy build on demand) | retain |
#[derive(Default)]
pub(crate) struct PresentationRuntime {
    generation: Option<ProjectGeneration>,
    pub(crate) political: PoliticalPresentationCache,
    pub(crate) resources: ResourcesPresentationCache,
    pub(crate) strategic_regions: StrategicRegionsPresentationCache,
    pub(crate) territory_anchor_index: Option<TerritoryAnchorIndex>,
    pub(crate) territory_anchor_generation: Option<ProjectGeneration>,
    map_presentation: Option<MapPresentationModel>,
    state_category_texture: Option<Texture>,
    manpower_texture: Option<Texture>,
    dmz_texture: Option<Texture>,
    problems_overlay: ProblemsOverlayModel,
    problems_overlay_revision: u64,
}

impl PresentationRuntime {
    pub(crate) fn new(adjacency_pairs: Vec<(u32, u32)>, language: String) -> Self {
        Self {
            political: PoliticalPresentationCache {
                adjacency_pairs,
                language,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub(crate) fn on_project_replaced(&mut self, generation: ProjectGeneration) {
        self.generation = Some(generation);
        let adjacency_pairs = std::mem::take(&mut self.political.adjacency_pairs);
        let language = std::mem::take(&mut self.political.language);
        self.political = PoliticalPresentationCache {
            adjacency_pairs,
            language,
            ..Default::default()
        };
        self.resources = ResourcesPresentationCache::default();
        self.strategic_regions = StrategicRegionsPresentationCache::default();
        self.territory_anchor_index = None;
        self.territory_anchor_generation = None;
        self.invalidate_state_revision();
        self.problems_overlay = ProblemsOverlayModel::default();
        self.problems_overlay_revision = 0;
    }

    /// State edits invalidate only State-derived presentation. Political and
    /// Resources retain their established view-specific refresh paths.
    pub(crate) fn on_state_revision_changed(&mut self) {
        self.invalidate_state_revision();
    }

    pub(crate) fn invalidate_state_revision(&mut self) {
        self.map_presentation = None;
        self.state_category_texture = None;
        self.manpower_texture = None;
        self.dmz_texture = None;
        self.strategic_regions.split = None;
        self.strategic_regions.split_texture = None;
        self.strategic_regions.split_revision = None;
    }

    pub(crate) fn ensure_state_presentation(
        &mut self,
        generation: ProjectGeneration,
        map: &Map,
        edit: &StateEditSession,
    ) {
        if self.generation != Some(generation) {
            self.on_project_replaced(generation);
        }
        let source_generation = SourceGeneration::new(generation.0);
        let revision = edit.revision();
        if self.map_presentation.as_ref().is_some_and(|model| {
            model.generation == source_generation && model.state_revision == revision
        }) {
            return;
        }
        let anchors = map
            .iter_province_data()
            .filter_map(|(_, province)| {
                let id = province.preserved_id?;
                let center = province.center_of_mass();
                Some((id, [center[0].floor() as u32, center[1].floor() as u32]))
            })
            .collect::<BTreeMap<_, _>>();
        let states = edit
            .valid_state_ids()
            .iter()
            .filter_map(|state_id| edit.state_data(*state_id))
            .collect::<Vec<_>>();
        self.map_presentation = Some(build_map_presentation(
            source_generation,
            revision,
            states,
            |id| anchors.get(&id).copied(),
        ));
        self.refresh_state_textures(map, edit.state_by_province());
    }

    fn refresh_state_textures(&mut self, map: &Map, state_by_province: &HashMap<u32, u32>) {
        let Some(presentation) = self.map_presentation.as_ref() else {
            return;
        };
        let settings = TextureSettings::new().mag(Filter::Nearest);
        let category = map.gen_texture_buffer(|province_color| {
            let province = map.get_province(province_color);
            province
                .preserved_id
                .and_then(|province_id| state_by_province.get(&province_id))
                .and_then(|state_id| presentation.states.get(state_id))
                .map_or_else(|| province.kind.color(), |state| state.category_color)
        });
        let manpower = map.gen_texture_buffer(|province_color| {
            let province = map.get_province(province_color);
            province
                .preserved_id
                .and_then(|province_id| state_by_province.get(&province_id))
                .and_then(|state_id| presentation.states.get(state_id))
                .map_or_else(|| province.kind.color(), |state| state.manpower_color)
        });
        let dmz = RgbaImage::from_fn(map.width(), map.height(), |x, y| {
            let province = map.get_province_at([x, y]);
            let has_dmz = province
                .preserved_id
                .and_then(|province_id| state_by_province.get(&province_id))
                .and_then(|state_id| presentation.states.get(state_id))
                .is_some_and(|state| state.demilitarized_zone);
            if has_dmz && (x.wrapping_add(y) % 8 < 2) {
                Rgba([0xf5, 0xd5, 0x3a, 0xb0])
            } else {
                Rgba([0, 0, 0, 0])
            }
        });
        self.state_category_texture = Some(Texture::from_image(&category, &settings));
        self.manpower_texture = Some(Texture::from_image(&manpower, &settings));
        self.dmz_texture = Some(Texture::from_image(&dmz, &settings));
    }

    pub(crate) fn texture_for_view(&self, view: MapBaseView) -> Option<&Texture> {
        match view {
            MapBaseView::StateCategory => self.state_category_texture.as_ref(),
            MapBaseView::Manpower => self.manpower_texture.as_ref(),
            MapBaseView::StrategicRegions => self.strategic_regions.texture.as_ref(),
            _ => None,
        }
    }

    pub(crate) fn ensure_strategic_regions_presentation(
        &mut self,
        generation: ProjectGeneration,
        map: &Map,
        loaded: &StrategicRegionLoadResult,
        state_by_province: Option<&HashMap<u32, u32>>,
        state_revision: Option<u64>,
    ) {
        if self.generation != Some(generation) {
            self.on_project_replaced(generation);
        }
        if self.strategic_regions.model.is_none() {
            let mut model = build_strategic_regions_map_model(loaded, map.province_ids());
            model.boundaries = collect_strategic_region_boundaries(map, &model.membership);
            let settings = TextureSettings::new().mag(Filter::Nearest);
            self.strategic_regions.texture = Some(Texture::from_image(
                &strategic_region_texture(map, &model),
                &settings,
            ));
            self.strategic_regions.boundaries = model.boundaries.clone();
            self.strategic_regions.model = Some(model);
        }
        let Some(model) = self.strategic_regions.model.as_ref() else {
            return;
        };
        if self.strategic_regions.split_revision != state_revision {
            self.strategic_regions.split =
                state_by_province.map(|states| build_state_split_model(model, states));
            self.strategic_regions.split_texture =
                self.strategic_regions.split.as_ref().map(|split| {
                    Texture::from_image(
                        &strategic_region_state_split_overlay(map, model, split),
                        &TextureSettings::new().mag(Filter::Nearest),
                    )
                });
            self.strategic_regions.split_revision = state_revision;
        }
    }

    pub(crate) fn set_strategic_region_selection(&mut self, map: &Map, selected_id: Option<u32>) {
        if self.strategic_regions.selected_id == selected_id {
            return;
        }
        self.strategic_regions.selection_texture =
            self.strategic_regions.model.as_ref().map(|model| {
                Texture::from_image(
                    &strategic_region_selection_overlay(map, model, selected_id),
                    &TextureSettings::new().mag(Filter::Nearest),
                )
            });
        self.strategic_regions.selected_id = selected_id;
    }

    pub(crate) fn dmz_texture(&self) -> Option<&Texture> {
        self.dmz_texture.as_ref()
    }

    pub(crate) fn map_presentation(&self) -> Option<&MapPresentationModel> {
        self.map_presentation.as_ref()
    }

    pub(crate) fn rebuild_problems_overlay(
        &mut self,
        generation: ProjectGeneration,
        diagnostics: &[ProjectValidationDiagnostic],
        map: &Map,
        edit: Option<&StateEditSession>,
    ) {
        if self.generation != Some(generation) {
            self.on_project_replaced(generation);
        }
        self.problems_overlay_revision = self.problems_overlay_revision.wrapping_add(1);
        let province_locations = map
            .iter_province_data()
            .filter_map(|(_, province)| {
                let id = province.preserved_id?;
                let center = province.center_of_mass();
                Some((id, [center[0].floor() as u32, center[1].floor() as u32]))
            })
            .collect::<BTreeMap<_, _>>();
        let state_locations = edit
            .map(|edit| {
                edit.valid_state_ids()
                    .iter()
                    .copied()
                    .filter_map(|state_id| {
                        let province_id = edit.state_data(state_id)?.provinces.first().copied()?;
                        Some((state_id, *province_locations.get(&province_id)?))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        self.problems_overlay = build_overlay(
            SourceGeneration::new(generation.0),
            self.problems_overlay_revision,
            diagnostics,
            |province_id| province_locations.get(&province_id).copied(),
            |state_id| state_locations.get(&state_id).copied(),
        );
    }

    pub(crate) fn problems_overlay(&self) -> &ProblemsOverlayModel {
        &self.problems_overlay
    }

    pub(crate) fn debug_assert_generation(&self, generation: ProjectGeneration) {
        debug_assert!(self.generation.is_none_or(|value| value == generation));
        debug_assert!(
            self.map_presentation
                .as_ref()
                .is_none_or(|model| model.generation == SourceGeneration::new(generation.0))
        );
        debug_assert!(
            self.problems_overlay.generation == SourceGeneration::default()
                || self.problems_overlay.generation == SourceGeneration::new(generation.0)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_replacement_drops_every_runtime_cache() {
        let mut runtime = PresentationRuntime {
            generation: Some(ProjectGeneration(1)),
            political: PoliticalPresentationCache {
                cache_generation: Some(ProjectGeneration(1)),
                adjacency_pairs: vec![(7, 9)],
                language: "en-US".to_owned(),
                ..Default::default()
            },
            resources: ResourcesPresentationCache {
                cache_generation: Some(ProjectGeneration(1)),
                ..Default::default()
            },
            territory_anchor_generation: Some(ProjectGeneration(1)),
            map_presentation: Some(MapPresentationModel {
                generation: SourceGeneration::new(1),
                state_revision: 4,
                ..Default::default()
            }),
            problems_overlay: ProblemsOverlayModel {
                generation: SourceGeneration::new(1),
                revision: 3,
                ..Default::default()
            },
            ..Default::default()
        };

        runtime.on_project_replaced(ProjectGeneration(2));

        assert_eq!(runtime.generation, Some(ProjectGeneration(2)));
        assert!(runtime.map_presentation.is_none());
        assert_eq!(runtime.problems_overlay, ProblemsOverlayModel::default());
        assert!(runtime.political.cache_generation.is_none());
        assert!(runtime.resources.cache_generation.is_none());
        assert!(runtime.territory_anchor_generation.is_none());
        assert_eq!(runtime.political.adjacency_pairs, vec![(7, 9)]);
    }

    #[test]
    fn state_revision_only_drops_state_derived_presentation() {
        let problems = ProblemsOverlayModel {
            generation: SourceGeneration::new(1),
            revision: 3,
            ..Default::default()
        };
        let mut runtime = PresentationRuntime {
            map_presentation: Some(MapPresentationModel {
                generation: SourceGeneration::new(1),
                state_revision: 4,
                ..Default::default()
            }),
            problems_overlay: problems.clone(),
            ..Default::default()
        };

        runtime.on_state_revision_changed();

        assert!(runtime.map_presentation.is_none());
        assert_eq!(runtime.problems_overlay, problems);
    }
}
