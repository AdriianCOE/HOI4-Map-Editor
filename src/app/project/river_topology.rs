//! Read-only topology facts derived from an indexed `rivers.bmp`.
//!
//! The HOI4 river bitmap is an 8-bit indexed BMP. Indices 0, 1, and 2 are
//! source/flow markers; indices 3 through 11 are river body pixels; larger
//! values are background. This module deliberately keeps decoding and graph
//! facts bounded to validation: it does not affect the river overlay or save
//! pipeline.

use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedRiverBitmap {
    pub width: u32,
    pub height: u32,
    indices: Vec<u8>,
}

impl IndexedRiverBitmap {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.get(0..2) != Some(b"BM") {
            return Err("not a BMP file".to_owned());
        }
        let pixel_offset = read_u32(bytes, 10)? as usize;
        let dib_offset = 14;
        let dib_size = read_u32(bytes, dib_offset)? as usize;
        if dib_size < 40 || bytes.len() < dib_offset + dib_size {
            return Err("requires a BITMAPINFOHEADER-compatible BMP".to_owned());
        }
        let width = read_i32(bytes, dib_offset + 4)?;
        let signed_height = read_i32(bytes, dib_offset + 8)?;
        let planes = read_u16(bytes, dib_offset + 12)?;
        let bits_per_pixel = read_u16(bytes, dib_offset + 14)?;
        let compression = read_u32(bytes, dib_offset + 16)?;
        if width <= 0 || signed_height <= 0 {
            return Err("has invalid dimensions".to_owned());
        }
        if planes != 1 || bits_per_pixel != 8 || compression != 0 {
            return Err(format!(
                "requires uncompressed 8-bit indexed pixels (planes {planes}, bpp {bits_per_pixel}, compression {compression})"
            ));
        }
        let width = width as u32;
        let height = signed_height.unsigned_abs();
        let row_stride = (width as usize).div_ceil(4) * 4;
        let pixel_bytes = row_stride
            .checked_mul(height as usize)
            .ok_or_else(|| "pixel data is too large".to_owned())?;
        let pixel_end = pixel_offset
            .checked_add(pixel_bytes)
            .ok_or_else(|| "pixel data offset overflows".to_owned())?;
        if pixel_offset < dib_offset + dib_size || pixel_end > bytes.len() {
            return Err("pixel data is truncated".to_owned());
        }

        let mut indices = vec![0; width as usize * height as usize];
        for file_row in 0..height as usize {
            let map_y = if signed_height > 0 {
                height as usize - 1 - file_row
            } else {
                file_row
            };
            let source_start = pixel_offset + file_row * row_stride;
            let target_start = map_y * width as usize;
            indices[target_start..target_start + width as usize]
                .copy_from_slice(&bytes[source_start..source_start + width as usize]);
        }
        Ok(Self {
            width,
            height,
            indices,
        })
    }

    fn index_at(&self, x: u32, y: u32) -> u8 {
        self.indices[y as usize * self.width as usize + x as usize]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RiverComponent {
    pub id: usize,
    pub pixel_count: usize,
    pub edge_count: usize,
    pub representative_location: [u32; 2],
    pub valid_sources: Vec<[u32; 2]>,
    pub valid_flow_markers: Vec<[u32; 2]>,
    pub invalid_flow_markers: Vec<[u32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RiverTopologyAnalysis {
    pub components: Vec<RiverComponent>,
}

impl RiverTopologyAnalysis {
    pub fn analyze(image: &IndexedRiverBitmap) -> Self {
        let total = image.width as usize * image.height as usize;
        let mut visited = vec![false; total];
        let mut queue = VecDeque::<usize>::new();
        let mut components = Vec::new();

        for start in 0..total {
            if visited[start] || !is_river(image.indices[start]) {
                continue;
            }
            visited[start] = true;
            queue.push_back(start);
            let representative_location = [
                (start % image.width as usize) as u32,
                (start / image.width as usize) as u32,
            ];
            let mut component = RiverComponent {
                id: components.len(),
                pixel_count: 0,
                edge_count: 0,
                representative_location,
                valid_sources: Vec::new(),
                valid_flow_markers: Vec::new(),
                invalid_flow_markers: Vec::new(),
            };

            while let Some(current) = queue.pop_front() {
                let x = (current % image.width as usize) as u32;
                let y = (current / image.width as usize) as u32;
                let value = image.indices[current];
                let neighbours = neighbours(image, x, y);
                let river_neighbours = neighbours
                    .into_iter()
                    .flatten()
                    .filter(|&[next_x, next_y]| is_river(image.index_at(next_x, next_y)))
                    .collect::<Vec<_>>();
                component.pixel_count += 1;
                component.edge_count += river_neighbours
                    .iter()
                    .filter(|&&[next_x, next_y]| next_x > x || next_y > y)
                    .count();

                let body_neighbours = river_neighbours
                    .iter()
                    .filter(|&&[next_x, next_y]| is_body(image.index_at(next_x, next_y)))
                    .count();
                if value == 0 && river_neighbours.len() <= 1 {
                    component.valid_sources.push([x, y]);
                }
                if matches!(value, 1 | 2) {
                    if body_neighbours != 0 {
                        component.valid_flow_markers.push([x, y]);
                    } else {
                        component.invalid_flow_markers.push([x, y]);
                    }
                }
                for [next_x, next_y] in river_neighbours {
                    let next = next_y as usize * image.width as usize + next_x as usize;
                    if !visited[next] {
                        visited[next] = true;
                        queue.push_back(next);
                    }
                }
            }
            components.push(component);
        }
        Self { components }
    }
}

fn neighbours(image: &IndexedRiverBitmap, x: u32, y: u32) -> [Option<[u32; 2]>; 4] {
    [
        x.checked_sub(1).map(|next_x| [next_x, y]),
        (x + 1 < image.width).then_some([x + 1, y]),
        y.checked_sub(1).map(|next_y| [x, next_y]),
        (y + 1 < image.height).then_some([x, y + 1]),
    ]
}

const fn is_river(value: u8) -> bool {
    value <= 11
}

const fn is_body(value: u8) -> bool {
    value >= 3 && value <= 11
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "BMP header is truncated".to_owned())?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "BMP header is truncated".to_owned())?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
    Ok(read_u32(bytes, offset)? as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed_bmp(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let stride = (width as usize).div_ceil(4) * 4;
        let offset = 14 + 40 + 256 * 4;
        let mut bytes = vec![0; offset + stride * height as usize];
        let length = bytes.len() as u32;
        bytes[0..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&length.to_le_bytes());
        bytes[10..14].copy_from_slice(&(offset as u32).to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&8u16.to_le_bytes());
        for y in 0..height as usize {
            let file_y = height as usize - 1 - y;
            let start = offset + file_y * stride;
            bytes[start..start + width as usize]
                .copy_from_slice(&pixels[y * width as usize..(y + 1) * width as usize]);
        }
        bytes
    }

    #[test]
    fn supports_small_images_without_vertical_or_horizontal_wrap() {
        for (width, height, pixels, components) in [
            (1, 1, vec![0], 1),
            (2, 2, vec![0, 3, 3, 2], 1),
            (3, 2, vec![0, 255, 2, 255, 255, 255], 2),
        ] {
            let image = IndexedRiverBitmap::parse(&indexed_bmp(width, height, &pixels)).unwrap();
            assert_eq!(
                RiverTopologyAnalysis::analyze(&image).components.len(),
                components
            );
        }
    }

    #[test]
    fn detects_cycles_without_flagging_a_branched_tree() {
        let loop_image =
            IndexedRiverBitmap::parse(&indexed_bmp(3, 2, &[0, 3, 3, 3, 3, 3])).unwrap();
        let loop_component = &RiverTopologyAnalysis::analyze(&loop_image).components[0];
        assert!(loop_component.edge_count >= loop_component.pixel_count);

        let tree =
            IndexedRiverBitmap::parse(&indexed_bmp(3, 3, &[255, 0, 255, 2, 3, 2, 255, 3, 255]))
                .unwrap();
        let tree_component = &RiverTopologyAnalysis::analyze(&tree).components[0];
        assert!(tree_component.edge_count < tree_component.pixel_count);
    }
}
