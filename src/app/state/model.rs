use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::{PdxDocument, TextSpan};
use crate::app::project::ProjectDiagnostic;

#[derive(Debug, Clone)]
pub struct StateDocument {
  pub path: PathBuf,
  pub syntax: PdxDocument,
  pub data: Option<StateData>,
  pub diagnostics: Vec<ProjectDiagnostic>,
  pub modified: bool
}

impl StateDocument {
  pub fn source(&self) -> &str {
    &self.syntax.source.text
  }
}

#[derive(Debug, Clone, Default)]
pub struct StateData {
  pub id: Option<u32>,
  pub name: Option<String>,
  pub provinces: BTreeSet<u32>,
  pub manpower: Option<u64>,
  pub buildings_max_level_factor: Option<f64>,
  pub state_category: Option<String>,
  pub local_supplies: Option<f64>,
  pub impassable: Option<bool>,
  pub resources: BTreeMap<String, i64>,
  pub history: StateHistory
}

#[derive(Debug, Clone, Default)]
pub struct StateHistory {
  pub owner: Option<String>,
  pub controller: Option<String>,
  pub cores: BTreeSet<String>,
  pub claims: BTreeSet<String>,
  pub removed_cores: BTreeSet<String>,
  pub removed_claims: BTreeSet<String>,
  pub victory_points: Vec<VictoryPoint>,
  pub state_buildings: BTreeMap<String, i64>,
  pub province_buildings: BTreeMap<u32, BTreeMap<String, i64>>,
  pub dated_blocks: Vec<DatedHistoryBlock>
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VictoryPoint {
  pub province_id: u32,
  pub value: i64
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatedHistoryBlock {
  pub date: String,
  pub span: TextSpan
}

#[cfg(test)]
mod tests {
  use super::{StateData, VictoryPoint};

  #[test]
  fn state_model_keeps_open_ended_names_and_large_values() {
    let mut state = StateData {
      id: Some(1),
      manpower: Some(142_000),
      ..Default::default()
    };
    state.provinces.extend([1405, 5144]);
    state.resources.insert("custom_resource".into(), 8);
    state.history.state_buildings.insert("custom_building".into(), 1);
    state.history.victory_points.push(VictoryPoint { province_id: 5144, value: 5 });

    assert!(state.provinces.contains(&5144));
    assert_eq!(state.resources["custom_resource"], 8);
    assert_eq!(state.history.victory_points[0], VictoryPoint { province_id: 5144, value: 5 });
  }
}
