//! Read-only topology facts derived from a province bitmap.
//!
//! This deliberately does not mirror a renderer or create map-edit commands.
//! It is a single bounded analysis used by validation only. Horizontal
//! neighbours wrap because the HOI4 world is cylindrical; vertical neighbours
//! do not wrap.

use std::collections::{BTreeMap, VecDeque};

use crate::app::map::{Bundle, Color, Extents};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProvinceGeometryInfo {
    pub color: Color,
    pub province_id: Option<u32>,
    pub pixel_count: u64,
    pub component_count: usize,
    pub bounding_box: Extents,
    pub representative_locations: Vec<[u32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapXCrossing {
    pub location: [u32; 2],
    pub colors: [Color; 4],
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ProvinceGeometryAnalysis {
    pub provinces: BTreeMap<Color, ProvinceGeometryInfo>,
    pub x_crossings: Vec<MapXCrossing>,
}

impl ProvinceGeometryAnalysis {
    pub fn analyze(bundle: &Bundle) -> Self {
        let [width, height] = bundle.map.dimensions();
        if width == 0 || height == 0 {
            return Self::default();
        }

        let pixel_total = (width as usize).saturating_mul(height as usize);
        let mut colors = Vec::with_capacity(pixel_total);
        let mut provinces = BTreeMap::<Color, ProvinceGeometryInfo>::new();
        for y in 0..height {
            for x in 0..width {
                let color = bundle.map.get_color_at([x, y]);
                colors.push(color);
                provinces
                    .entry(color)
                    .and_modify(|info| {
                        info.pixel_count += 1;
                        info.bounding_box = info.bounding_box.join_point([x, y]);
                    })
                    .or_insert_with(|| ProvinceGeometryInfo {
                        color,
                        province_id: bundle.map.province_id_for_color(color),
                        pixel_count: 1,
                        component_count: 0,
                        bounding_box: Extents::new_point([x, y]),
                        representative_locations: Vec::new(),
                    });
            }
        }

        let mut visited = vec![false; pixel_total];
        let mut queue = VecDeque::<usize>::new();
        for index in 0..pixel_total {
            if visited[index] {
                continue;
            }
            visited[index] = true;
            let color = colors[index];
            let x = (index % width as usize) as u32;
            let y = (index / width as usize) as u32;
            let info = provinces.get_mut(&color).expect("scanned color exists");
            info.component_count += 1;
            info.representative_locations.push([x, y]);
            queue.push_back(index);

            while let Some(current) = queue.pop_front() {
                let current_x = (current % width as usize) as u32;
                let current_y = (current / width as usize) as u32;
                let mut neighbours = [[0, 0]; 4];
                let mut count = 0;
                neighbours[count] = [(current_x + width - 1) % width, current_y];
                count += 1;
                neighbours[count] = [(current_x + 1) % width, current_y];
                count += 1;
                if current_y > 0 {
                    neighbours[count] = [current_x, current_y - 1];
                    count += 1;
                }
                if current_y + 1 < height {
                    neighbours[count] = [current_x, current_y + 1];
                    count += 1;
                }
                for [next_x, next_y] in neighbours.into_iter().take(count) {
                    let next = next_y as usize * width as usize + next_x as usize;
                    if !visited[next] && colors[next] == color {
                        visited[next] = true;
                        queue.push_back(next);
                    }
                }
            }
        }

        let mut x_crossings = Vec::new();
        if height >= 2 && width >= 2 {
            // A width of two has only one distinct horizontal pair. Checking a
            // seam block as well would report the same 2x2 topology twice.
            let x_count = if width == 2 { 1 } else { width };
            for y in 0..height - 1 {
                for x in 0..x_count {
                    let right = (x + 1) % width;
                    let block = [
                        colors[y as usize * width as usize + x as usize],
                        colors[y as usize * width as usize + right as usize],
                        colors[(y as usize + 1) * width as usize + x as usize],
                        colors[(y as usize + 1) * width as usize + right as usize],
                    ];
                    if block.iter().enumerate().all(|(left, color)| {
                        block.iter().skip(left + 1).all(|other| color != other)
                    }) {
                        x_crossings.push(MapXCrossing {
                            location: [x, y],
                            colors: block,
                        });
                    }
                }
            }
        }

        Self {
            provinces,
            x_crossings,
        }
    }
}
