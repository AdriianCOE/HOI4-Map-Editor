use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::time::{Duration, Instant};

use geo::algorithm::contains::Contains;
use geo::{LineString, Polygon};
use vecmath::Vector2;

use crate::app::map::{Extents, Map, ProvinceKind};
use crate::util::XYIter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LassoSelectionMode {
    #[default]
    Replace,
    Add,
    Remove,
}

impl LassoSelectionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Replace => "Replace",
            Self::Add => "Add",
            Self::Remove => "Remove",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProvinceInclusionMode {
    #[default]
    CentroidInside,
    AnyIntersection,
    MajorityInside,
}

impl ProvinceInclusionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::CentroidInside => "Centroid",
            Self::AnyIntersection => "Any intersection",
            Self::MajorityInside => "Majority",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LassoCandidateSet {
    pub selectable: BTreeSet<u32>,
    pub blocked: BTreeSet<u32>,
    pub already_selected: BTreeSet<u32>,
    pub ambiguous: BTreeSet<u32>,
    pub invalid_state: BTreeSet<u32>,
    pub ignored_non_land: usize,
    pub ignored_unknown_pixels: usize,
    pub scanned_pixels: usize,
    pub bounds: Option<Extents>,
    pub computed_in: Duration,
}

#[derive(Debug, Clone, Default)]
pub enum StateLassoPhase {
    #[default]
    Inactive,
    Drawing {
        points: Vec<Vector2<f64>>,
        mode: LassoSelectionMode,
        inclusion: ProvinceInclusionMode,
    },
    Preview {
        points: Vec<Vector2<f64>>,
        candidates: LassoCandidateSet,
        mode: LassoSelectionMode,
        inclusion: ProvinceInclusionMode,
    },
}

impl StateLassoPhase {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Inactive => "Inactive",
            Self::Drawing { .. } => "Drawing",
            Self::Preview { .. } => "Preview",
        }
    }

    pub fn points(&self) -> Option<&[Vector2<f64>]> {
        match self {
            Self::Drawing { points, .. } | Self::Preview { points, .. } => Some(points),
            Self::Inactive => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ProvinceHit {
    kind: ProvinceKind,
    center: Vector2<f64>,
    total_pixels: u64,
    pixels_inside: u64,
}

#[allow(deprecated)]
pub fn classify_state_lasso(
    map: &Map,
    points: &[Vector2<f64>],
    inclusion: ProvinceInclusionMode,
    selected: &BTreeSet<u32>,
    state_by_province: &HashMap<u32, u32>,
    ambiguous_provinces: &BTreeSet<u32>,
    valid_state_ids: &BTreeSet<u32>,
) -> Result<LassoCandidateSet, StateLassoError> {
    use geo::Coordinate;

    if points.len() < 3 {
        return Err(StateLassoError::NotEnoughPoints);
    }

    let started = Instant::now();
    let bounds = polygon_bounds(points, map).ok_or(StateLassoError::OutsideMap)?;
    let polygon = Polygon::new(LineString::from(points.to_vec()), Vec::new());
    let mut hits = BTreeMap::<u32, ProvinceHit>::new();
    let mut ignored_unknown_pixels = 0;
    let mut scanned_pixels = 0;

    for [x, y] in XYIter::from_extents(bounds) {
        scanned_pixels += 1;
        let province = map.get_province_at([x, y]);
        let Some(province_id) = province.preserved_id.filter(|id| *id != 0) else {
            ignored_unknown_pixels += 1;
            continue;
        };
        let hit = hits.entry(province_id).or_insert_with(|| ProvinceHit {
            kind: province.kind,
            center: province.center_of_mass(),
            total_pixels: province.pixel_count,
            pixels_inside: 0,
        });
        if inclusion != ProvinceInclusionMode::CentroidInside
            && polygon.contains(&Coordinate::from([x as f64 + 0.5, y as f64 + 0.5]))
        {
            hit.pixels_inside += 1;
        }
    }

    let mut candidates = LassoCandidateSet {
        ignored_unknown_pixels,
        scanned_pixels,
        bounds: Some(bounds),
        ..Default::default()
    };
    for (province_id, hit) in hits {
        let included = match inclusion {
            ProvinceInclusionMode::CentroidInside => {
                polygon.contains(&Coordinate::from(hit.center))
            }
            ProvinceInclusionMode::AnyIntersection => hit.pixels_inside > 0,
            ProvinceInclusionMode::MajorityInside => {
                hit.pixels_inside.saturating_mul(2) > hit.total_pixels
            }
        };
        if !included {
            continue;
        }
        if hit.kind != ProvinceKind::Land {
            candidates.ignored_non_land += 1;
            continue;
        }
        if ambiguous_provinces.contains(&province_id) {
            candidates.ambiguous.insert(province_id);
            candidates.blocked.insert(province_id);
            continue;
        }
        if state_by_province
            .get(&province_id)
            .is_some_and(|state_id| !valid_state_ids.contains(state_id))
        {
            candidates.invalid_state.insert(province_id);
            candidates.blocked.insert(province_id);
            continue;
        }
        if selected.contains(&province_id) {
            candidates.already_selected.insert(province_id);
        }
        candidates.selectable.insert(province_id);
    }
    candidates.computed_in = started.elapsed();
    Ok(candidates)
}

fn polygon_bounds(points: &[Vector2<f64>], map: &Map) -> Option<Extents> {
    let min_x = points
        .iter()
        .map(|point| point[0])
        .fold(f64::INFINITY, f64::min);
    let min_y = points
        .iter()
        .map(|point| point[1])
        .fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|point| point[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = points
        .iter()
        .map(|point| point[1])
        .fold(f64::NEG_INFINITY, f64::max);
    let [width, height] = map.dimensions();
    if max_x < 0.0 || max_y < 0.0 || min_x >= width as f64 || min_y >= height as f64 {
        return None;
    }

    Some(Extents::new(
        [
            max_x.ceil().clamp(0.0, (width - 1) as f64) as u32,
            max_y.ceil().clamp(0.0, (height - 1) as f64) as u32,
        ],
        [
            min_x.floor().clamp(0.0, (width - 1) as f64) as u32,
            min_y.floor().clamp(0.0, (height - 1) as f64) as u32,
        ],
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateLassoError {
    NotEnoughPoints,
    OutsideMap,
}

impl fmt::Display for StateLassoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnoughPoints => write!(f, "State lasso needs at least three points"),
            Self::OutsideMap => write!(f, "State lasso does not intersect the map"),
        }
    }
}

impl std::error::Error for StateLassoError {}
