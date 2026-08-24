//! Read-only presentation model for the Strategic Regions map view.
//!
//! The model consumes the already-loaded Strategic Region result; it never
//! scans source files or mutates project data. IDs are deliberately kept in
//! sparse maps so a malformed high ID cannot allocate a giant vector.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use image::{Rgba, RgbaImage};
use uord::UOrd2 as UOrd;
use vecmath::Vector2;

use crate::app::map::{Color, Map, ProvinceKind};
use crate::util::hsl::hsl_to_rgb;

use super::{StrategicRegionCoverage, StrategicRegionLoadResult};

pub const STRATEGIC_REGION_AMBIGUOUS_COLOR: Color = [0xe8, 0x45, 0xa0];
pub const STRATEGIC_REGION_UNASSIGNED_COLOR: Color = [0x82, 0x82, 0x82];
pub const STRATEGIC_REGION_UNKNOWN_COLOR: Color = [0x9b, 0x71, 0x24];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategicRegionMembership {
    Assigned(u32),
    Ambiguous,
    Unassigned,
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub struct StrategicRegionsMapModel {
    pub membership: BTreeMap<u32, StrategicRegionMembership>,
    pub boundaries: Vec<UOrd<Vector2<u32>>>,
}

/// An intentionally separate state-revision cache. The base region texture
/// must remain valid when an unsaved State edit changes membership.
#[derive(Debug, Clone, Default)]
pub struct StrategicRegionStateSplitModel {
    pub split_region_ids: BTreeSet<u32>,
}

pub fn strategic_region_color(region_id: u32) -> Color {
    // Fixed integer mixing: stable across processes and Rust versions.
    let mut mixed = region_id.wrapping_mul(0x9e37_79b9);
    mixed ^= mixed >> 16;
    mixed = mixed.wrapping_mul(0x85eb_ca6b);
    mixed ^= mixed >> 13;
    hsl_to_rgb([
        (mixed % 360) as f32,
        0.58 + ((mixed >> 9) % 18) as f32 / 100.0,
        0.42 + ((mixed >> 17) % 17) as f32 / 100.0,
    ])
}

pub fn build_strategic_regions_map_model(
    loaded: &StrategicRegionLoadResult,
    valid_province_ids: impl IntoIterator<Item = u32>,
) -> StrategicRegionsMapModel {
    let valid = valid_province_ids.into_iter().collect::<BTreeSet<_>>();
    let mut candidates = BTreeMap::<u32, Vec<u32>>::new();
    if loaded.coverage.is_complete() {
        for region in &loaded.regions {
            for &province_id in &region.provinces {
                if valid.contains(&province_id) {
                    candidates.entry(province_id).or_default().push(region.id);
                }
            }
        }
    }
    let membership = valid
        .into_iter()
        .map(|province_id| {
            let status = match loaded.coverage {
                StrategicRegionCoverage::NotPresent
                | StrategicRegionCoverage::Incomplete { .. } => StrategicRegionMembership::Unknown,
                StrategicRegionCoverage::Complete => match candidates.get(&province_id) {
                    None => StrategicRegionMembership::Unassigned,
                    Some(ids) if ids.len() == 1 => StrategicRegionMembership::Assigned(ids[0]),
                    Some(_) => StrategicRegionMembership::Ambiguous,
                },
            };
            (province_id, status)
        })
        .collect();
    StrategicRegionsMapModel {
        membership,
        boundaries: Vec::new(),
    }
}

pub fn collect_strategic_region_boundaries(
    map: &Map,
    membership: &BTreeMap<u32, StrategicRegionMembership>,
) -> Vec<UOrd<Vector2<u32>>> {
    map.iter_boundaries()
        .filter_map(|(boundary, _)| {
            let [left, right] = boundary.into_array();
            let left = map.get_province_at(left).preserved_id.and_then(|id| membership.get(&id));
            let right = map.get_province_at(right).preserved_id.and_then(|id| membership.get(&id));
            matches!((left, right), (Some(StrategicRegionMembership::Assigned(a)), Some(StrategicRegionMembership::Assigned(b))) if a != b)
                .then_some(boundary)
        })
        .collect()
}

pub fn strategic_region_texture(map: &Map, model: &StrategicRegionsMapModel) -> RgbaImage {
    map.gen_texture_buffer(|province_color| {
        let province = map.get_province(province_color);
        color_for_membership(province.preserved_id, province.kind, &model.membership)
    })
}

pub fn strategic_region_selection_overlay(
    map: &Map,
    model: &StrategicRegionsMapModel,
    selected_id: Option<u32>,
) -> RgbaImage {
    RgbaImage::from_fn(map.width(), map.height(), |x, y| {
        let selected = map.get_province_at([x, y]).preserved_id.and_then(|province_id| {
            matches!(model.membership.get(&province_id), Some(StrategicRegionMembership::Assigned(id)) if Some(*id) == selected_id)
                .then_some(())
        });
        if selected.is_some() {
            Rgba([255, 255, 255, 80])
        } else {
            Rgba([0, 0, 0, 0])
        }
    })
}

pub fn build_state_split_model(
    model: &StrategicRegionsMapModel,
    state_by_province: &HashMap<u32, u32>,
) -> StrategicRegionStateSplitModel {
    let mut states_by_region = BTreeMap::<u32, BTreeSet<u32>>::new();
    for (&province_id, membership) in &model.membership {
        if let (StrategicRegionMembership::Assigned(region_id), Some(state_id)) =
            (membership, state_by_province.get(&province_id))
        {
            states_by_region
                .entry(*region_id)
                .or_default()
                .insert(*state_id);
        }
    }
    StrategicRegionStateSplitModel {
        split_region_ids: states_by_region
            .into_iter()
            .filter_map(|(id, states)| (states.len() > 1).then_some(id))
            .collect(),
    }
}

pub fn strategic_region_state_split_overlay(
    map: &Map,
    model: &StrategicRegionsMapModel,
    split: &StrategicRegionStateSplitModel,
) -> RgbaImage {
    RgbaImage::from_fn(map.width(), map.height(), |x, y| {
        let region = map.get_province_at([x, y]).preserved_id.and_then(|id| {
            match model.membership.get(&id) {
                Some(StrategicRegionMembership::Assigned(id)) => Some(*id),
                _ => None,
            }
        });
        if region.is_some_and(|id| split.split_region_ids.contains(&id)) && (x + y) % 11 < 2 {
            Rgba([255, 255, 255, 52])
        } else {
            Rgba([0, 0, 0, 0])
        }
    })
}

fn color_for_membership(
    province_id: Option<u32>,
    kind: ProvinceKind,
    membership: &BTreeMap<u32, StrategicRegionMembership>,
) -> Color {
    if kind == ProvinceKind::Unknown {
        return STRATEGIC_REGION_UNKNOWN_COLOR;
    }
    match province_id.and_then(|id| membership.get(&id)).copied() {
        Some(StrategicRegionMembership::Assigned(id)) => strategic_region_color(id),
        Some(StrategicRegionMembership::Ambiguous) => STRATEGIC_REGION_AMBIGUOUS_COLOR,
        Some(StrategicRegionMembership::Unassigned) => STRATEGIC_REGION_UNASSIGNED_COLOR,
        Some(StrategicRegionMembership::Unknown) | None => STRATEGIC_REGION_UNKNOWN_COLOR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::project::{StrategicRegion, StrategicRegionLoadResult};

    fn loaded(
        coverage: StrategicRegionCoverage,
        regions: Vec<(u32, Vec<u32>)>,
    ) -> StrategicRegionLoadResult {
        StrategicRegionLoadResult {
            coverage,
            regions: regions
                .into_iter()
                .map(|(id, provinces)| StrategicRegion {
                    id,
                    provinces,
                    name_key: None,
                    display_name: None,
                    has_provinces_field: true,
                    naval_terrain: None,
                    source: std::sync::Arc::new(crate::app::project::ResolvedSource {
                        logical_path: "map/strategicregions/test.txt".into(),
                        location: crate::app::project::ResolvedLocation::Filesystem(
                            Default::default(),
                        ),
                        source_kind: crate::app::project::SourceKind::CurrentProject,
                        project_generation: crate::app::project::SourceGeneration::default(),
                    }),
                    span: Default::default(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn complete_data_distinguishes_assigned_ambiguous_and_unassigned() {
        let model = build_strategic_regions_map_model(
            &loaded(
                StrategicRegionCoverage::Complete,
                vec![(4, vec![2]), (9, vec![3, 2])],
            ),
            [1, 2, 3],
        );
        assert_eq!(model.membership[&1], StrategicRegionMembership::Unassigned);
        assert_eq!(model.membership[&2], StrategicRegionMembership::Ambiguous);
        assert_eq!(model.membership[&3], StrategicRegionMembership::Assigned(9));
    }

    #[test]
    fn incomplete_and_absent_sources_are_unknown() {
        for coverage in [
            StrategicRegionCoverage::NotPresent,
            StrategicRegionCoverage::Incomplete {
                files_visible: 2,
                files_failed: 1,
            },
        ] {
            let model =
                build_strategic_regions_map_model(&loaded(coverage, vec![(4, vec![2])]), [2]);
            assert_eq!(model.membership[&2], StrategicRegionMembership::Unknown);
        }
    }

    #[test]
    fn region_colors_are_deterministic_and_opaque() {
        assert_eq!(strategic_region_color(99), strategic_region_color(99));
        assert_ne!(strategic_region_color(99), strategic_region_color(100));
    }

    #[test]
    fn split_model_uses_effective_state_membership() {
        let model = build_strategic_regions_map_model(
            &loaded(StrategicRegionCoverage::Complete, vec![(4, vec![2, 3])]),
            [2, 3],
        );
        let split = build_state_split_model(&model, &HashMap::from([(2, 1), (3, 2)]));
        assert!(split.split_region_ids.contains(&4));
    }
}
