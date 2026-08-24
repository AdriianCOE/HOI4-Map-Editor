use std::collections::BTreeMap;

use image::{Rgba, RgbaImage};

use crate::app::map::Color;
use crate::app::project::SourceGeneration;
use crate::app::state::StateData;
const CATEGORY_PALETTE: [Color; 12] = [
    [0x4e, 0x79, 0xa7],
    [0xf2, 0x8e, 0x2b],
    [0x59, 0xa1, 0x4f],
    [0xe1, 0x57, 0x59],
    [0x76, 0xb7, 0xb2],
    [0xb0, 0x7a, 0xa1],
    [0xed, 0xc9, 0x48],
    [0xff, 0x9d, 0xa7],
    [0x9c, 0x75, 0x5f],
    [0xba, 0xb0, 0xab],
    [0x86, 0xbc, 0xb6],
    [0x8c, 0x6d, 0xb0],
];
pub const MANPOWER_MISSING_COLOR: Color = [0x66, 0x66, 0x66];
pub const MANPOWER_ZERO_COLOR: Color = [0x25, 0x32, 0x4d];

/// Cached, read-only state presentation.  It deliberately consumes the
/// `StateEditSession` snapshot supplied by Canvas rather than re-reading files.
#[derive(Debug, Clone, Default)]
pub struct MapPresentationModel {
    pub generation: SourceGeneration,
    pub state_revision: u64,
    pub states: BTreeMap<u32, StatePresentation>,
    pub victory_points: Vec<VictoryPointMarker>,
    pub category_legend: Vec<CategoryLegendEntry>,
    pub manpower_legend: ManpowerLegend,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryLegendEntry {
    pub category: String,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ManpowerLegend {
    pub low: u64,
    pub medium: u64,
    pub high: u64,
}

#[derive(Debug, Clone)]
pub struct StatePresentation {
    pub state_id: u32,
    pub category: Option<String>,
    pub category_color: Color,
    pub manpower: Option<u64>,
    pub manpower_color: Color,
    pub demilitarized_zone: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VictoryPointMarker {
    pub province_id: u32,
    pub value: i64,
    pub location: [u32; 2],
}

pub fn build_map_presentation(
    generation: SourceGeneration,
    state_revision: u64,
    states: impl IntoIterator<Item = StateData>,
    mut province_anchor: impl FnMut(u32) -> Option<[u32; 2]>,
) -> MapPresentationModel {
    let states = states
        .into_iter()
        .filter_map(|state| state.id.map(|id| (id, state)))
        .collect::<BTreeMap<_, _>>();
    let manpower_values = states
        .values()
        .filter_map(|state| state.manpower)
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    let manpower_legend = manpower_legend(&manpower_values);
    let categories = category_palette(
        states
            .values()
            .filter_map(|state| state.state_category.as_deref()),
    );
    let mut presentation = MapPresentationModel {
        generation,
        state_revision,
        category_legend: categories
            .iter()
            .map(|(category, color)| CategoryLegendEntry {
                category: category.clone(),
                color: *color,
            })
            .collect(),
        manpower_legend,
        ..Default::default()
    };
    let mut markers = BTreeMap::new();
    for (state_id, state) in states {
        presentation.states.insert(
            state_id,
            StatePresentation {
                state_id,
                category_color: state
                    .state_category
                    .as_deref()
                    .and_then(|category| categories.get(category).copied())
                    .unwrap_or_else(|| category_color(None)),
                category: state.state_category,
                manpower_color: manpower_color(state.manpower, manpower_legend.high),
                manpower: state.manpower,
                demilitarized_zone: state.demilitarized_zone == Some(true),
            },
        );
        // State ids are ordered, therefore malformed duplicate declarations have
        // a deterministic, non-gameplay interpretation: retain the first marker.
        for victory_point in state.history.victory_points {
            markers
                .entry(victory_point.province_id)
                .or_insert(victory_point.value);
        }
    }
    presentation.victory_points = markers
        .into_iter()
        .filter_map(|(province_id, value)| {
            province_anchor(province_id).map(|location| VictoryPointMarker {
                province_id,
                value,
                location,
            })
        })
        .collect();
    presentation
}

pub fn category_color(category: Option<&str>) -> Color {
    let Some(category) = category.filter(|value| !value.is_empty()) else {
        return [0x68, 0x68, 0x68];
    };
    let hash = category.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    CATEGORY_PALETTE[hash as usize % CATEGORY_PALETTE.len()]
}

pub fn manpower_color(manpower: Option<u64>, maximum: u64) -> Color {
    match manpower {
        None => MANPOWER_MISSING_COLOR,
        Some(0) => MANPOWER_ZERO_COLOR,
        Some(value) => {
            let denominator = (maximum.max(1) as f64 + 1.0).ln();
            let normalized = ((value as f64 + 1.0).ln() / denominator).clamp(0.0, 1.0);
            sequential_manpower_color(normalized)
        }
    }
}

fn category_palette<'a>(categories: impl Iterator<Item = &'a str>) -> BTreeMap<String, Color> {
    categories
        .filter(|category| !category.is_empty())
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(index, category)| (category, CATEGORY_PALETTE[index % CATEGORY_PALETTE.len()]))
        .collect()
}

fn manpower_legend(values: &[u64]) -> ManpowerLegend {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    // The lower discrete quantile keeps a lone tail value from becoming the
    // whole scale in small projects too (the usual nearest-rank p90 would not).
    let high = sorted
        .get(sorted.len().saturating_sub(1) * 9 / 10)
        .copied()
        .unwrap_or(0);
    let low = sorted.first().copied().unwrap_or(0);
    let medium = (((high as f64 + 1.0).ln() / 2.0).exp() - 1.0).round() as u64;
    ManpowerLegend { low, medium, high }
}

fn sequential_manpower_color(value: f64) -> Color {
    const STOPS: [Color; 5] = [
        [0x1b, 0x38, 0x63],
        [0x1e, 0x6f, 0x9a],
        [0x2f, 0xa4, 0x8f],
        [0xa9, 0xcf, 0x5a],
        [0xf0, 0xd1, 0x4f],
    ];
    let scaled = value.clamp(0.0, 1.0) * (STOPS.len() - 1) as f64;
    let index = scaled.floor() as usize;
    if index + 1 == STOPS.len() {
        STOPS[index]
    } else {
        blend(STOPS[index], STOPS[index + 1], scaled.fract())
    }
}

pub fn paint_victory_points(image: &mut RgbaImage, markers: &[VictoryPointMarker]) {
    for marker in markers {
        for y in marker.location[1].saturating_sub(2)..=marker.location[1].saturating_add(2) {
            for x in marker.location[0].saturating_sub(2)..=marker.location[0].saturating_add(2) {
                if x < image.width()
                    && y < image.height()
                    && (x == marker.location[0] || y == marker.location[1])
                {
                    image.put_pixel(x, y, Rgba([0xff, 0xff, 0xff, 0xff]));
                }
            }
        }
    }
}

fn blend(low: Color, high: Color, amount: f64) -> Color {
    std::array::from_fn(|index| {
        (f64::from(low[index]) + (f64::from(high[index]) - f64::from(low[index])) * amount).round()
            as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::VictoryPoint;

    fn state(id: u32, category: Option<&str>, manpower: Option<u64>) -> StateData {
        StateData {
            id: Some(id),
            state_category: category.map(str::to_owned),
            manpower,
            ..Default::default()
        }
    }

    #[test]
    fn presentation_keeps_sparse_vps_ordered_and_first_duplicate() {
        let mut first = state(1, Some("city"), Some(10));
        first.history.victory_points = vec![
            VictoryPoint {
                province_id: 99,
                value: 5,
            },
            VictoryPoint {
                province_id: 3,
                value: 10,
            },
        ];
        let mut second = state(2, Some("custom"), Some(20));
        second.history.victory_points = vec![VictoryPoint {
            province_id: 99,
            value: 50,
        }];
        let model = build_map_presentation(SourceGeneration::new(2), 4, [first, second], |id| {
            Some([id, 1])
        });
        assert_eq!(
            model
                .victory_points
                .iter()
                .map(|marker| (marker.province_id, marker.value))
                .collect::<Vec<_>>(),
            vec![(3, 10), (99, 5)]
        );
        assert_eq!(model.generation, SourceGeneration::new(2));
    }

    #[test]
    fn category_and_manpower_colors_are_safe_and_deterministic() {
        assert_eq!(
            category_color(Some("modded_category")),
            category_color(Some("modded_category"))
        );
        assert_ne!(
            category_color(Some("modded_category")),
            category_color(None)
        );
        assert_eq!(manpower_color(Some(0), 1_000), MANPOWER_ZERO_COLOR);
        assert_eq!(manpower_color(None, 1_000), MANPOWER_MISSING_COLOR);
        let small = manpower_color(Some(10), 1_000_000);
        let large = manpower_color(Some(1_000_000), 1_000_000);
        assert_ne!(small, large);
    }

    #[test]
    fn legends_are_deterministic_and_outliers_do_not_flatten_the_scale() {
        let model = build_map_presentation(
            SourceGeneration::new(1),
            1,
            [
                state(1, Some("city"), Some(10)),
                state(2, Some("rural"), Some(20)),
                state(3, Some("city"), Some(30)),
                state(4, Some("custom"), Some(1_000_000)),
            ],
            |_| None,
        );
        assert_eq!(
            model
                .category_legend
                .iter()
                .map(|entry| entry.category.as_str())
                .collect::<Vec<_>>(),
            ["city", "custom", "rural"]
        );
        assert!(model.manpower_legend.high < 1_000_000);
        assert_ne!(
            model.states[&1].category_color,
            model.states[&2].category_color
        );
        assert_ne!(
            model.states[&1].manpower_color,
            model.states[&2].manpower_color
        );
    }

    #[test]
    fn dmz_and_project_switch_data_stay_in_the_model() {
        let mut dmz = state(10, Some("rural"), Some(1));
        dmz.demilitarized_zone = Some(true);
        let first = build_map_presentation(SourceGeneration::new(1), 0, [dmz], |_| None);
        let second =
            build_map_presentation(SourceGeneration::new(2), 0, [state(20, None, None)], |_| {
                None
            });
        assert!(first.states[&10].demilitarized_zone);
        assert!(!second.states.contains_key(&10));
    }
}
