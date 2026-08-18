//! Focused, cached resource-map data and HOI4 resource-strip lookup.
//!
//! This deliberately understands only `common/resources` icon frames and the
//! `GFX_resources_strip` sprite.  It is not a general `.gfx` renderer.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use image::{GenericImageView, RgbaImage};

use super::political::{PoliticalProvince, territory_anchor};
use super::state::{PdxBlock, PdxEntry, PdxValue, parse_text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceMapState {
    pub state_id: u32,
    pub provinces: Vec<u32>,
    pub resources: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceMapRow {
    pub key: String,
    pub amount: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceMapLabel {
    pub state_id: u32,
    pub anchor: [f64; 2],
    pub territory_pixels: u64,
    pub rows: Vec<ResourceMapRow>,
}

pub fn prepare_resource_labels(
    states: &[ResourceMapState],
    provinces: &[PoliticalProvince],
    adjacency_pairs: &[(u32, u32)],
) -> Vec<ResourceMapLabel> {
    states
        .iter()
        .filter_map(|state| {
            let rows = state
                .resources
                .iter()
                .filter(|(_, amount)| **amount != 0)
                .map(|(key, amount)| ResourceMapRow {
                    key: key.clone(),
                    amount: *amount,
                })
                .collect::<Vec<_>>();
            let (anchor, territory_pixels) = territory_anchor(
                &state.provinces.iter().copied().collect(),
                provinces,
                adjacency_pairs,
            )?;
            (!rows.is_empty()).then_some(ResourceMapLabel {
                state_id: state.state_id,
                anchor,
                territory_pixels,
                rows,
            })
        })
        .collect()
}

/// Keeps world-map rendering quiet while retaining tiny State data at close zoom.
pub const fn resource_label_visible(territory_pixels: u64, zoom: f64) -> bool {
    (territory_pixels >= 100 && zoom >= 0.80)
        || (territory_pixels >= 24 && zoom >= 1.25)
        || (territory_pixels >= 4 && zoom >= 2.0)
        || zoom >= 3.0
}

#[derive(Debug, Clone)]
struct ResourceStrip {
    path: PathBuf,
    frames: u32,
}

/// Layered, lazy decoder for the seven-frame HOI4 resource strip.
#[derive(Debug, Default)]
pub struct ResourceIconResolver {
    frames: BTreeMap<String, u32>,
    strip: Option<ResourceStrip>,
    cached_icons: BTreeMap<String, Option<RgbaImage>>,
}

impl ResourceIconResolver {
    pub fn load(project_root: &Path, base_game_root: Option<&Path>) -> Self {
        let mut resolver = Self::default();
        let roots = base_game_root
            .into_iter()
            .chain(std::iter::once(project_root));
        for root in roots {
            resolver.load_layer(root);
        }
        resolver
    }

    pub fn icon(&mut self, key: &str) -> Option<&RgbaImage> {
        if !self.cached_icons.contains_key(key) {
            let decoded = self.decode_icon(key);
            self.cached_icons.insert(key.to_owned(), decoded);
        }
        self.cached_icons.get(key).and_then(Option::as_ref)
    }

    fn load_layer(&mut self, root: &Path) {
        for path in text_files_recursive(&root.join("common/resources"), "txt") {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let document = parse_text(&path, text);
            let entries = document
                .entries
                .iter()
                .find(|entry| {
                    entry
                        .key
                        .as_ref()
                        .is_some_and(|key| key.text.eq_ignore_ascii_case("resources"))
                })
                .and_then(|entry| block(&entry.value).map(|block| block.entries.as_slice()))
                .unwrap_or(document.entries.as_slice());
            for entry in entries {
                let Some(key) = entry.key.as_ref().map(|scalar| scalar.text.as_str()) else {
                    continue;
                };
                let Some(block) = block(&entry.value) else {
                    continue;
                };
                if let Some(frame) =
                    scalar(block, "icon_frame").and_then(|value| value.parse().ok())
                {
                    self.frames.insert(key.to_owned(), frame);
                }
            }
        }
        // HOI4 declares the real strip in `interface/general_stuff.gfx`.  A
        // focused sibling name supports compact mod fixtures without turning
        // this into a recursive game-interface framework.
        for path in [
            root.join("interface/general_stuff.gfx"),
            root.join("interface/resources.gfx"),
        ] {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let document = parse_text(&path, text);
            if let Some(strip) = find_resource_strip(&document.entries, root) {
                self.strip = Some(strip);
            }
        }
    }

    fn decode_icon(&self, key: &str) -> Option<RgbaImage> {
        let frame = self.frames.get(key)?.checked_sub(1)?;
        let strip = self.strip.as_ref()?;
        if frame >= strip.frames {
            return None;
        }
        let image = image::open(&strip.path)
            .ok()
            .or_else(|| decode_uncompressed_dds(&strip.path))?;
        let (width, height) = image.dimensions();
        let frame_width = width.checked_div(strip.frames)?;
        (frame_width > 0 && height > 0).then(|| {
            image
                .view(frame * frame_width, 0, frame_width, height)
                .to_image()
        })
    }
}

/// HOI4's observed `resources_strip.dds` is a 32-bit BGRA DDS. `image` does
/// not decode that legacy variant, so support only this narrow, non-compressed
/// form instead of adding a general DDS framework.
fn decode_uncompressed_dds(path: &Path) -> Option<image::DynamicImage> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 128 || &bytes[..4] != b"DDS " {
        return None;
    }
    let read_u32 = |offset| {
        bytes
            .get(offset..offset + 4)
            .and_then(|slice| slice.try_into().ok())
            .map(u32::from_le_bytes)
    };
    let height = read_u32(12)?;
    let width = read_u32(16)?;
    let bits_per_pixel = read_u32(88)?;
    let red_mask = read_u32(92)?;
    let green_mask = read_u32(96)?;
    let blue_mask = read_u32(100)?;
    let alpha_mask = read_u32(104)?;
    if bits_per_pixel != 32 || width == 0 || height == 0 {
        return None;
    }
    let pixels_len = width.checked_mul(height)?.checked_mul(4)? as usize;
    let data = bytes.get(128..128 + pixels_len)?;
    let mut image = RgbaImage::new(width, height);
    for (index, pixel) in data.chunks_exact(4).enumerate() {
        let raw = u32::from_le_bytes(pixel.try_into().ok()?);
        image.put_pixel(
            (index as u32) % width,
            (index as u32) / width,
            image::Rgba([
                dds_component(raw, red_mask)?,
                dds_component(raw, green_mask)?,
                dds_component(raw, blue_mask)?,
                if alpha_mask == 0 {
                    255
                } else {
                    dds_component(raw, alpha_mask)?
                },
            ]),
        );
    }
    Some(image::DynamicImage::ImageRgba8(image))
}

fn dds_component(value: u32, mask: u32) -> Option<u8> {
    if mask == 0 {
        return Some(0);
    }
    let shift = mask.trailing_zeros();
    let bits = mask.count_ones();
    let max = (1u32.checked_shl(bits)?).checked_sub(1)?;
    Some((((value & mask) >> shift) * 255 / max) as u8)
}

fn find_resource_strip(entries: &[PdxEntry], root: &Path) -> Option<ResourceStrip> {
    for entry in entries {
        if entry
            .key
            .as_ref()
            .is_some_and(|key| key.text.eq_ignore_ascii_case("spriteType"))
            && let Some(block) = block(&entry.value)
            && scalar(block, "name")
                .is_some_and(|name| name.trim_matches('"') == "GFX_resources_strip")
        {
            let texture = scalar(block, "texturefile")?.trim_matches('"');
            let frames = scalar(block, "noOfFrames")
                .and_then(|value| value.parse().ok())
                .unwrap_or(1);
            return Some(ResourceStrip {
                path: root.join(texture.replace('/', std::path::MAIN_SEPARATOR_STR)),
                frames,
            });
        }
        if let Some(block) = block(&entry.value)
            && let Some(found) = find_resource_strip(&block.entries, root)
        {
            return Some(found);
        }
    }
    None
}

fn block(value: &PdxValue) -> Option<&PdxBlock> {
    match value {
        PdxValue::Block(block) => Some(block),
        PdxValue::Scalar(_) => None,
    }
}

fn scalar<'a>(block: &'a PdxBlock, name: &str) -> Option<&'a str> {
    block.entries.iter().find_map(|entry| {
        entry
            .key
            .as_ref()
            .filter(|key| key.text.eq_ignore_ascii_case(name))
            .and(match &entry.value {
                PdxValue::Scalar(value) => Some(value.text.as_str()),
                PdxValue::Block(_) => None,
            })
    })
}

fn text_files_recursive(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return paths;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            paths.extend(text_files_recursive(&path, extension));
        } else if path
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case(extension))
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn province(id: u32, center: [f64; 2], pixels: u64) -> PoliticalProvince {
        PoliticalProvince {
            id,
            is_land: true,
            center,
            pixel_count: pixels,
        }
    }

    #[test]
    fn resource_rows_are_sorted_and_omit_zero() {
        let mut resources = BTreeMap::new();
        resources.insert("steel".into(), 20);
        resources.insert("aluminium".into(), 4);
        resources.insert("oil".into(), 0);
        let labels = prepare_resource_labels(
            &[ResourceMapState {
                state_id: 200,
                provinces: vec![3],
                resources,
            }],
            &[province(3, [5.0, 5.0], 300)],
            &[],
        );
        assert_eq!(labels[0].state_id, 200);
        assert_eq!(
            labels[0]
                .rows
                .iter()
                .map(|row| (row.key.as_str(), row.amount))
                .collect::<Vec<_>>(),
            vec![("aluminium", 4), ("steel", 20)]
        );
    }

    #[test]
    fn resource_visibility_defers_tiny_states_until_close_zoom() {
        assert!(!resource_label_visible(3, 2.9));
        assert!(resource_label_visible(3, 3.0));
        assert!(resource_label_visible(200, 0.8));
    }

    #[test]
    fn mod_strip_and_frame_override_base_and_cached_failures_are_non_blocking() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-resource-icons-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let base = root.join("base");
        let project = root.join("mod");
        write_resource_fixture(&base, 1, [[10, 0, 0, 255], [20, 0, 0, 255]]);
        write_resource_fixture(&project, 2, [[30, 0, 0, 255], [40, 0, 0, 255]]);

        let mut resolver = ResourceIconResolver::load(&project, Some(&base));
        assert_eq!(
            resolver.icon("steel").unwrap().get_pixel(0, 0).0,
            [40, 0, 0, 255]
        );
        assert!(resolver.icon("custom_resource").is_none());
        assert!(resolver.icon("custom_resource").is_none());
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            resolver.icon("steel").unwrap().get_pixel(0, 0).0,
            [40, 0, 0, 255]
        );
    }

    fn write_resource_fixture(root: &Path, frame: u32, pixels: [[u8; 4]; 2]) {
        fs::create_dir_all(root.join("common/resources")).unwrap();
        fs::create_dir_all(root.join("interface")).unwrap();
        fs::create_dir_all(root.join("gfx/interface")).unwrap();
        fs::write(
            root.join("common/resources/resources.txt"),
            format!("steel = {{ icon_frame = {frame} }}"),
        )
        .unwrap();
        fs::write(
            root.join("interface/resources.gfx"),
            "spriteTypes = { spriteType = { name = \"GFX_resources_strip\" texturefile = \"gfx/interface/resources_strip.png\" noOfFrames = 2 } }",
        )
        .unwrap();
        let mut image = RgbaImage::new(2, 1);
        image.put_pixel(0, 0, image::Rgba(pixels[0]));
        image.put_pixel(1, 0, image::Rgba(pixels[1]));
        image
            .save(root.join("gfx/interface/resources_strip.png"))
            .unwrap();
    }

    #[test]
    #[ignore = "requires local HOI4 and Azarya installations"]
    fn local_azarya_state_200_keeps_integral_decimal_resources_and_uses_base_icons() {
        let root = PathBuf::from(std::env::var("HOI4_STATE_EDITOR_AZARYA_ROOT").unwrap());
        let base = PathBuf::from(std::env::var("HOI4_STATE_EDITOR_BASE_ROOT").unwrap());
        let batch = crate::app::state::load_state_documents(&root.join("history/states"));
        let state_200 = batch
            .documents
            .iter()
            .filter_map(|document| document.data.as_ref())
            .find(|state| state.id == Some(200))
            .unwrap();
        assert_eq!(state_200.resources.get("steel"), Some(&20));
        let mut resolver = ResourceIconResolver::load(&root, Some(&base));
        assert_eq!(resolver.frames.get("steel"), Some(&5));
        assert!(
            resolver
                .strip
                .as_ref()
                .is_some_and(|strip| strip.path.exists())
        );
        assert!(resolver.icon("steel").is_some());
    }
}
