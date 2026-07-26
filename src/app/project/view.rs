use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use image::{Rgba, RgbaImage};
use uord::UOrd2 as UOrd;
use vecmath::Vector2;

use crate::app::map::{Color, Extents, Map, ProvinceKind};
use crate::app::project::Hoi4Project;
use crate::util::hsl::hsl_to_rgb;

pub const AMBIGUOUS_PROVINCE_COLOR: Color = [0xff, 0x00, 0xff];
pub const UNASSIGNED_LAND_COLOR: Color = [0xff, 0x00, 0x00];
pub const UNKNOWN_PROVINCE_COLOR: Color = [0xff, 0x88, 0x00];
pub const STATE_BOUNDARY_COLOR: Color = [0x12, 0x12, 0x12];
pub const SELECTED_STATE_COLOR: Color = [0xff, 0xff, 0xff];

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MapViewMode {
  Provinces,
  States,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateSelection {
  State { state_id: u32, province_id: u32 },
  AmbiguousProvince { province_id: u32, state_ids: Vec<u32> },
  UnassignedProvince { province_id: u32 },
}

#[derive(Debug)]
pub struct StateMapViewData {
  pub image: RgbaImage,
  pub state_boundaries: Vec<UOrd<Vector2<u32>>>,
  pub generated_in: Duration,
  pub boundary_scan_in: Duration,
}

#[derive(Debug)]
pub struct StateMapRegionData {
  pub image: RgbaImage,
  pub state_boundaries: Vec<UOrd<Vector2<u32>>>,
  pub generated_in: Duration,
  pub boundary_scan_in: Duration,
}

pub fn state_color(state_id: u32) -> Color {
  let mut mixed = state_id.wrapping_mul(0x9e37_79b9);
  mixed ^= mixed >> 16;
  mixed = mixed.wrapping_mul(0x85eb_ca6b);
  mixed ^= mixed >> 13;

  let hue = (mixed % 360) as f32;
  let saturation = 0.62 + ((mixed >> 9) % 16) as f32 / 100.0;
  let lightness = 0.48 + ((mixed >> 17) % 13) as f32 / 100.0;
  hsl_to_rgb([hue, saturation, lightness])
}

pub fn generate_state_view(map: &Map, project: &Hoi4Project) -> StateMapViewData {
  generate_state_view_for(
    map,
    &project.state_by_province,
    &project.ambiguous_provinces.keys().copied().collect(),
    &project.unassigned_land_provinces
  )
}

pub fn generate_state_view_for(
  map: &Map,
  state_by_province: &HashMap<u32, u32>,
  ambiguous_provinces: &BTreeSet<u32>,
  unassigned_land_provinces: &BTreeSet<u32>,
) -> StateMapViewData {
  let started = Instant::now();
  let mut image = map.gen_texture_buffer(|province_color| {
    let province = map.get_province(province_color);
    classify_province_color_for(
      province.preserved_id,
      province.kind,
      state_by_province,
      ambiguous_provinces,
      unassigned_land_provinces
    )
  });
  let generated_in = started.elapsed();

  let boundary_started = Instant::now();
  let state_boundaries = collect_state_boundaries_for(map, state_by_province);
  for boundary in &state_boundaries {
    for [x, y] in boundary.into_array() {
      image.put_pixel(x, y, Rgba([
        STATE_BOUNDARY_COLOR[0],
        STATE_BOUNDARY_COLOR[1],
        STATE_BOUNDARY_COLOR[2],
        0xff,
      ]));
    }
  }
  let boundary_scan_in = boundary_started.elapsed();

  StateMapViewData {
    image,
    state_boundaries,
    generated_in,
    boundary_scan_in,
  }
}

pub fn generate_state_view_region_for(
  map: &Map,
  state_by_province: &HashMap<u32, u32>,
  ambiguous_provinces: &BTreeSet<u32>,
  unassigned_land_provinces: &BTreeSet<u32>,
  extents: Extents,
) -> StateMapRegionData {
  let started = Instant::now();
  let mut image = map.gen_texture_buffer_selective(extents, |province_color| {
    let province = map.get_province(province_color);
    classify_province_color_for(
      province.preserved_id,
      province.kind,
      state_by_province,
      ambiguous_provinces,
      unassigned_land_provinces
    )
  });
  let generated_in = started.elapsed();

  let boundary_started = Instant::now();
  let state_boundaries = map.iter_boundaries()
    .filter_map(|(boundary, _)| {
      let [a, b] = boundary.into_array();
      ((extents.contains(a) || extents.contains(b))
        && boundary_crosses_state(map, state_by_province, boundary))
        .then_some(boundary)
    })
    .collect::<Vec<_>>();
  for boundary in &state_boundaries {
    for [x, y] in boundary.into_array() {
      if extents.contains([x, y]) {
        image.put_pixel(
          x - extents.lower[0],
          y - extents.lower[1],
          Rgba([
            STATE_BOUNDARY_COLOR[0],
            STATE_BOUNDARY_COLOR[1],
            STATE_BOUNDARY_COLOR[2],
            0xff,
          ])
        );
      }
    }
  }
  let boundary_scan_in = boundary_started.elapsed();

  StateMapRegionData {
    image,
    state_boundaries,
    generated_in,
    boundary_scan_in,
  }
}

pub fn boundaries_for_state(
  map: &Map,
  state_by_province: &HashMap<u32, u32>,
  boundaries: &[UOrd<Vector2<u32>>],
  state_id: u32,
) -> Vec<UOrd<Vector2<u32>>> {
  boundaries
    .iter()
    .copied()
    .filter(|boundary| {
      let [a, b] = boundary.into_array();
      let a = state_at_pos(map, state_by_province, a);
      let b = state_at_pos(map, state_by_province, b);
      (a == Some(state_id) || b == Some(state_id)) && a != b
    })
    .collect()
}

#[cfg(test)]
fn classify_province_color(
  province_id: Option<u32>,
  kind: ProvinceKind,
  project: &Hoi4Project,
) -> Color {
  classify_province_color_for(
    province_id,
    kind,
    &project.state_by_province,
    &project.ambiguous_provinces.keys().copied().collect(),
    &project.unassigned_land_provinces
  )
}

pub fn classify_province_color_for(
  province_id: Option<u32>,
  kind: ProvinceKind,
  state_by_province: &HashMap<u32, u32>,
  ambiguous_provinces: &BTreeSet<u32>,
  unassigned_land_provinces: &BTreeSet<u32>,
) -> Color {
  if kind == ProvinceKind::Unknown {
    return UNKNOWN_PROVINCE_COLOR;
  }

  let Some(province_id) = province_id else {
    return kind.color();
  };

  if ambiguous_provinces.contains(&province_id) {
    return AMBIGUOUS_PROVINCE_COLOR;
  }

  if let Some(&state_id) = state_by_province.get(&province_id) {
    return state_color(state_id);
  }

  if kind == ProvinceKind::Land && unassigned_land_provinces.contains(&province_id) {
    return UNASSIGNED_LAND_COLOR;
  }

  kind.color()
}

pub fn select_state_by_province(
  province_id: u32,
  kind: ProvinceKind,
  project: &Hoi4Project,
) -> Option<StateSelection> {
  select_state_by_province_for(
    province_id,
    kind,
    &project.state_by_province,
    &project.ambiguous_provinces,
    &project.unassigned_land_provinces
  )
}

pub fn select_state_by_province_for(
  province_id: u32,
  kind: ProvinceKind,
  state_by_province: &HashMap<u32, u32>,
  ambiguous_provinces: &std::collections::BTreeMap<u32, Vec<u32>>,
  unassigned_land_provinces: &BTreeSet<u32>,
) -> Option<StateSelection> {
  if kind == ProvinceKind::Unknown {
    return None;
  }

  if let Some(states) = ambiguous_provinces.get(&province_id)
    && states.len() > 1
  {
    return Some(StateSelection::AmbiguousProvince {
      province_id,
      state_ids: states.clone(),
    });
  }

  if let Some(&state_id) = state_by_province.get(&province_id) {
    return Some(StateSelection::State {
      state_id,
      province_id,
    });
  }

  if kind == ProvinceKind::Land && unassigned_land_provinces.contains(&province_id) {
    return Some(StateSelection::UnassignedProvince { province_id });
  }

  None
}

pub fn select_state_at(
  map: &Map,
  project: &Hoi4Project,
  pos: Vector2<u32>,
) -> Option<StateSelection> {
  let province = map.get_province_at(pos);
  let province_id = province.preserved_id?;
  select_state_by_province(province_id, province.kind, project)
}

pub fn select_state_at_for(
  map: &Map,
  state_by_province: &HashMap<u32, u32>,
  ambiguous_provinces: &std::collections::BTreeMap<u32, Vec<u32>>,
  unassigned_land_provinces: &BTreeSet<u32>,
  pos: Vector2<u32>,
) -> Option<StateSelection> {
  let province = map.get_province_at(pos);
  let province_id = province.preserved_id?;
  select_state_by_province_for(
    province_id,
    province.kind,
    state_by_province,
    ambiguous_provinces,
    unassigned_land_provinces
  )
}

pub fn selection_overlay(
  map: &Map,
  project: &Hoi4Project,
  selection: Option<&StateSelection>,
) -> RgbaImage {
  selection_overlay_for(
    map,
    &project.state_by_province,
    &project.ambiguous_provinces.keys().copied().collect(),
    selection
  )
}

pub fn selection_overlay_for(
  map: &Map,
  state_by_province: &HashMap<u32, u32>,
  ambiguous_provinces: &BTreeSet<u32>,
  selection: Option<&StateSelection>,
) -> RgbaImage {
  let selected = selected_state_id(selection);
  if selected.is_none() {
    return empty_selection_overlay(map);
  }

  RgbaImage::from_fn(map.width(), map.height(), |x, y| {
    let province = map.get_province_at([x, y]);
    if selected.is_some_and(|state_id| {
      province
        .preserved_id
        .filter(|province_id| !ambiguous_provinces.contains(province_id))
        .and_then(|province_id| state_by_province.get(&province_id).copied())
        == Some(state_id)
    }) {
      Rgba([
        SELECTED_STATE_COLOR[0],
        SELECTED_STATE_COLOR[1],
        SELECTED_STATE_COLOR[2],
        0x80,
      ])
    } else {
      Rgba([0, 0, 0, 0])
    }
  })
}

pub fn collect_state_boundaries_for(
  map: &Map,
  state_by_province: &HashMap<u32, u32>,
) -> Vec<UOrd<Vector2<u32>>> {
  map
    .iter_boundaries()
    .filter_map(|(boundary, _)| {
      if boundary_crosses_state(map, state_by_province, boundary) {
        Some(boundary)
      } else {
        None
      }
    })
    .collect()
}

pub fn is_state_boundary(left: Option<u32>, right: Option<u32>) -> bool {
  left.is_some() && right.is_some() && left != right
}

fn selected_state_id(selection: Option<&StateSelection>) -> Option<u32> {
  match selection {
    Some(StateSelection::State { state_id, .. }) => Some(*state_id),
    _ => None,
  }
}

fn boundary_crosses_state(
  map: &Map,
  state_by_province: &HashMap<u32, u32>,
  boundary: UOrd<Vector2<u32>>,
) -> bool {
  let [a, b] = boundary.into_array();
  let a = state_at_pos(map, state_by_province, a);
  let b = state_at_pos(map, state_by_province, b);
  is_state_boundary(a, b)
}

fn state_at_pos(
  map: &Map,
  state_by_province: &HashMap<u32, u32>,
  pos: Vector2<u32>,
) -> Option<u32> {
  map
    .get_province_at(pos)
    .preserved_id
    .and_then(|province_id| state_by_province.get(&province_id).copied())
}

fn transparent_selection_buffer(width: u32, height: u32) -> RgbaImage {
  RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]))
}

pub fn empty_selection_overlay(map: &Map) -> RgbaImage {
  transparent_selection_buffer(map.width(), map.height())
}

#[cfg(test)]
mod tests {
  use std::collections::{BTreeMap, BTreeSet, HashMap};
  use std::path::PathBuf;

  use super::*;
  use crate::app::project::{ProjectPaths, StateLoadSummary};

  fn project() -> Hoi4Project {
    Hoi4Project {
      paths: ProjectPaths {
        root: PathBuf::new(),
        map_directory: PathBuf::new(),
        provinces_bmp: PathBuf::new(),
        definition_csv: PathBuf::new(),
        adjacencies_csv: None,
        rivers_bmp: None,
        history_directory: PathBuf::new(),
        states_directory: PathBuf::new(),
      },
      states: Vec::new(),
      states_by_id: BTreeMap::new(),
      state_by_province: HashMap::new(),
      ambiguous_provinces: BTreeMap::new(),
      unassigned_land_provinces: BTreeSet::new(),
      diagnostics: Vec::new(),
      load_summary: StateLoadSummary::default(),
    }
  }

  #[test]
  fn state_colors_are_deterministic_and_not_error_colors() {
    assert_eq!(state_color(42), state_color(42));
    assert_ne!(state_color(42), state_color(43));
    assert_ne!(state_color(42), AMBIGUOUS_PROVINCE_COLOR);
    assert_ne!(state_color(42), UNASSIGNED_LAND_COLOR);
    assert!((1..=1000).all(|id| {
      let color = state_color(id);
      color != [0, 0, 0] && color != [255, 255, 255]
    }));
    let distinct = (1..=1000)
      .map(state_color)
      .collect::<BTreeSet<_>>();
    assert!(distinct.len() > 990);
  }

  #[test]
  fn state_texture_classification_prefers_diagnostics_over_state_color() {
    let mut project = project();
    project.state_by_province.insert(10, 1);
    project.state_by_province.insert(11, 2);
    project.state_by_province.insert(13, 4);
    project.ambiguous_provinces.insert(11, vec![2, 3]);
    project.unassigned_land_provinces.insert(12);

    assert_eq!(
      classify_province_color(Some(10), ProvinceKind::Land, &project),
      state_color(1)
    );
    assert_eq!(
      classify_province_color(Some(11), ProvinceKind::Land, &project),
      AMBIGUOUS_PROVINCE_COLOR
    );
    assert_eq!(
      classify_province_color(Some(12), ProvinceKind::Land, &project),
      UNASSIGNED_LAND_COLOR
    );
    assert_eq!(
      classify_province_color(Some(13), ProvinceKind::Unknown, &project),
      UNKNOWN_PROVINCE_COLOR
    );
    assert_eq!(
      classify_province_color(None, ProvinceKind::Unknown, &project),
      UNKNOWN_PROVINCE_COLOR
    );
    assert_eq!(
      classify_province_color(Some(14), ProvinceKind::Sea, &project),
      ProvinceKind::Sea.color()
    );
  }

  #[test]
  fn selection_uses_same_precedence_as_visual_classification() {
    let mut project = project();
    project.state_by_province.insert(10, 1);
    project.state_by_province.insert(11, 2);
    project.state_by_province.insert(13, 4);
    project.ambiguous_provinces.insert(11, vec![2, 3]);
    project.unassigned_land_provinces.insert(12);

    assert_eq!(
      select_state_by_province(10, ProvinceKind::Land, &project),
      Some(StateSelection::State {
        state_id: 1,
        province_id: 10
      })
    );
    assert_eq!(
      select_state_by_province(11, ProvinceKind::Land, &project),
      Some(StateSelection::AmbiguousProvince {
        province_id: 11,
        state_ids: vec![2, 3]
      })
    );
    assert_eq!(
      select_state_by_province(12, ProvinceKind::Land, &project),
      Some(StateSelection::UnassignedProvince { province_id: 12 })
    );
    assert_eq!(
      select_state_by_province(13, ProvinceKind::Sea, &project),
      Some(StateSelection::State {
        state_id: 4,
        province_id: 13
      })
    );
    assert_eq!(
      select_state_by_province(13, ProvinceKind::Unknown, &project),
      None
    );
  }

  #[test]
  fn selected_state_id_ignores_non_state_selections() {
    assert_eq!(
      selected_state_id(Some(&StateSelection::State {
        state_id: 7,
        province_id: 70,
      })),
      Some(7)
    );
    assert_eq!(
      selected_state_id(Some(&StateSelection::UnassignedProvince { province_id: 70 })),
      None
    );
    assert_eq!(selected_state_id(None), None);
  }

  #[test]
  fn state_boundary_rule_only_marks_edges_between_known_different_states() {
    assert!(is_state_boundary(Some(1), Some(2)));
    assert!(!is_state_boundary(Some(1), Some(1)));
    assert!(!is_state_boundary(Some(1), None));
    assert!(!is_state_boundary(None, Some(2)));
    assert!(!is_state_boundary(None, None));
  }

  #[test]
  fn view_mode_is_independent_from_selection_state() {
    let mut view_mode = MapViewMode::States;
    let selection = StateSelection::State {
      state_id: 1,
      province_id: 10,
    };

    assert_eq!(view_mode, MapViewMode::States);
    assert_eq!(
      selection,
      StateSelection::State {
        state_id: 1,
        province_id: 10
      }
    );
    view_mode = MapViewMode::Provinces;
    assert_eq!(view_mode, MapViewMode::Provinces);
  }
}
