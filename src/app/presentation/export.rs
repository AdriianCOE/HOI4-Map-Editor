//! CPU PNG export composition independent of Canvas state.

use std::collections::HashMap;
use std::path::Path;

use image::{ImageFormat, Rgba, RgbaImage, imageops::FilterType};

use crate::app::map::Map;
use crate::app::project::{MapPresentationModel, ProblemsOverlayModel, paint_victory_points};
use crate::app::resources::ResourceMapLabel;

/// Immutable overlay selection and models for one export operation.
pub(crate) struct ExportOverlays<'a> {
    pub(crate) state_by_province: &'a HashMap<u32, u32>,
    pub(crate) presentation: Option<&'a MapPresentationModel>,
    pub(crate) resource_labels: &'a [ResourceMapLabel],
    pub(crate) problems: &'a ProblemsOverlayModel,
    pub(crate) show_dmz: bool,
    pub(crate) show_resources: bool,
    pub(crate) show_victory_points: bool,
    pub(crate) show_problems: bool,
}

/// Compose overlays in the same fixed order used by live presentation:
/// DMZ, Resources, Victory Points, then Problems.
pub(crate) fn compose_export_overlays(
    map: &Map,
    image: &mut RgbaImage,
    overlays: ExportOverlays<'_>,
) {
    if overlays.show_dmz
        && let Some(model) = overlays.presentation
    {
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let has_dmz = map
                .get_province_at([x, y])
                .preserved_id
                .and_then(|province_id| overlays.state_by_province.get(&province_id))
                .and_then(|state_id| model.states.get(state_id))
                .is_some_and(|state| state.demilitarized_zone);
            if has_dmz && (x.wrapping_add(y) % 8 < 2) {
                *pixel = Rgba([0xf5, 0xd5, 0x3a, 0xff]);
            }
        }
    }
    if overlays.show_resources {
        for label in overlays.resource_labels {
            let x = label.anchor[0].round().max(0.0) as u32;
            let y = label.anchor[1].round().max(0.0) as u32;
            if x < image.width() && y < image.height() {
                image.put_pixel(x, y, Rgba([0x62, 0xe8, 0x91, 0xff]));
            }
        }
    }
    if overlays.show_victory_points
        && let Some(model) = overlays.presentation
    {
        paint_victory_points(image, &model.victory_points);
    }
    if overlays.show_problems {
        for marker in &overlays.problems.markers {
            if marker.location[0] < image.width() && marker.location[1] < image.height() {
                image.put_pixel(
                    marker.location[0],
                    marker.location[1],
                    Rgba([0xee, 0x42, 0x31, 0xff]),
                );
            }
        }
    }
}

pub(crate) fn checked_export_dimensions(
    width: u32,
    height: u32,
    scale: u32,
) -> Result<[u32; 2], String> {
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

pub(crate) fn save_png(path: &Path, image: &RgbaImage, scale: u32) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    #[test]
    fn dimensions_support_existing_scales_and_custom_maps() {
        assert_eq!(checked_export_dimensions(3, 2, 4).unwrap(), [12, 8]);
        assert_eq!(checked_export_dimensions(130, 70, 2).unwrap(), [260, 140]);
        assert!(checked_export_dimensions(u32::MAX, u32::MAX, 4).is_err());
    }

    #[test]
    fn png_export_remains_decodable_at_all_scales() {
        let image = RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]));
        for (scale, dimensions) in [(1, (3, 2)), (2, (6, 4)), (4, (12, 8))] {
            let path = std::env::temp_dir().join(format!(
                "hoi4-presentation-export-{}-{scale}.png",
                std::process::id()
            ));
            save_png(&path, &image, scale).unwrap();
            assert_eq!(image::open(&path).unwrap().dimensions(), dimensions);
            std::fs::remove_file(path).unwrap();
        }
    }
}
