use std::collections::BTreeMap;

use image::{ImageFormat, Rgba, RgbaImage, imageops::FilterType};

use crate::app::map::Color;
use crate::app::project::SourceGeneration;
use crate::app::state::StateData;
use crate::util::hsl::hsl_to_rgb;

/// Cached, read-only state presentation.  It deliberately consumes the
/// `StateEditSession` snapshot supplied by Canvas rather than re-reading files.
#[derive(Debug, Clone, Default)]
pub struct MapPresentationModel {
    pub generation: SourceGeneration,
    pub state_revision: u64,
    pub states: BTreeMap<u32, StatePresentation>,
    pub victory_points: Vec<VictoryPointMarker>,
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
    let max_manpower = states
        .values()
        .filter_map(|state| state.manpower)
        .max()
        .unwrap_or(0);
    let mut presentation = MapPresentationModel {
        generation,
        state_revision,
        ..Default::default()
    };
    let mut markers = BTreeMap::new();
    for (state_id, state) in states {
        presentation.states.insert(
            state_id,
            StatePresentation {
                state_id,
                category_color: category_color(state.state_category.as_deref()),
                category: state.state_category,
                manpower_color: manpower_color(state.manpower, max_manpower),
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
    hsl_to_rgb([(hash % 360) as f32, 0.58, 0.50])
}

pub fn manpower_color(manpower: Option<u64>, maximum: u64) -> Color {
    match manpower {
        None => [0x66, 0x66, 0x66],
        Some(0) => [0x3c, 0x3c, 0x3c],
        Some(value) => {
            let denominator = (maximum.max(1) as f64 + 1.0).ln();
            let normalized = ((value as f64 + 1.0).ln() / denominator).clamp(0.0, 1.0);
            blend([0x32, 0x68, 0xb7], [0xe2, 0x44, 0x30], normalized)
        }
    }
}

pub fn checked_export_dimensions(width: u32, height: u32, scale: u32) -> Result<[u32; 2], String> {
    if !matches!(scale, 1 | 2 | 4) {
        return Err("Export scale must be 1x, 2x, or 4x".to_owned());
    }
    let width = width
        .checked_mul(scale)
        .ok_or_else(|| "Export width overflows supported dimensions".to_owned())?;
    let height = height
        .checked_mul(scale)
        .ok_or_else(|| "Export height overflows supported dimensions".to_owned())?;
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "Export image size overflows supported memory".to_owned())?;
    const MAX_EXPORT_BYTES: u64 = 512 * 1024 * 1024;
    if bytes > MAX_EXPORT_BYTES {
        return Err("Export image is too large (limit: 512 MiB of RGBA pixels)".to_owned());
    }
    Ok([width, height])
}

pub fn save_png(path: &std::path::Path, image: &RgbaImage, scale: u32) -> Result<(), String> {
    let [width, height] = checked_export_dimensions(image.width(), image.height(), scale)?;
    let output = if scale == 1 {
        image.clone()
    } else {
        image::imageops::resize(image, width, height, FilterType::Nearest)
    };
    output
        .save_with_format(path, ImageFormat::Png)
        .map_err(|error| format!("Could not write PNG: {error}"))
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
    use image::GenericImageView;

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
        assert_eq!(manpower_color(Some(0), 1_000), [0x3c, 0x3c, 0x3c]);
        assert_eq!(manpower_color(None, 1_000), [0x66, 0x66, 0x66]);
        let small = manpower_color(Some(10), 1_000_000);
        let large = manpower_color(Some(1_000_000), 1_000_000);
        assert_ne!(small, large);
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

    #[test]
    fn png_export_scales_and_rejects_unsafe_dimensions() {
        assert_eq!(checked_export_dimensions(3, 2, 1).unwrap(), [3, 2]);
        assert_eq!(checked_export_dimensions(3, 2, 2).unwrap(), [6, 4]);
        assert_eq!(checked_export_dimensions(3, 2, 4).unwrap(), [12, 8]);
        assert_eq!(checked_export_dimensions(130, 70, 1).unwrap(), [130, 70]);
        assert!(checked_export_dimensions(u32::MAX, u32::MAX, 4).is_err());
    }

    #[test]
    fn png_export_writes_a_decodable_scaled_composition() {
        let mut image = RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]));
        paint_victory_points(
            &mut image,
            &[VictoryPointMarker {
                province_id: 1,
                value: 5,
                location: [1, 1],
            }],
        );
        let path = std::env::temp_dir().join(format!(
            "hoi4-map-presentation-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        save_png(&path, &image, 4).unwrap();
        let decoded = image::open(&path).unwrap();
        assert_eq!(decoded.dimensions(), (12, 8));
        assert_eq!(&std::fs::read(&path).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
        std::fs::remove_file(path).unwrap();
    }
}
