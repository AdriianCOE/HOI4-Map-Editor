use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use super::{PdxDocument, TextSpan};
use crate::app::project::ProjectDiagnostic;

/// A calendar instant used by HOI4 history and bookmark scripts.
///
/// The engine commonly writes dates as `year.month.day.hour`; state history
/// usually omits the hour.  Keeping a parsed value avoids accidental lexical
/// ordering (`1939.10.1` before `1939.2.1`) and gives omitted hours the
/// deterministic value zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hoi4Date {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
}

impl Hoi4Date {
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.parse().ok()?;
        let hour = match parts.next() {
            Some(value) => value.parse().ok()?,
            None => 0,
        };
        (parts.next().is_none()
            && (1..=12).contains(&month)
            && (1..=31).contains(&day)
            && hour <= 23)
            .then_some(Self {
                year,
                month,
                day,
                hour,
            })
    }
}

impl std::fmt::Display for Hoi4Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.hour == 0 {
            write!(f, "{}.{}.{}", self.year, self.month, self.day)
        } else {
            write!(f, "{}.{}.{}.{}", self.year, self.month, self.day, self.hour)
        }
    }
}

#[derive(Debug, Clone)]
pub struct StateDocument {
    pub path: PathBuf,
    pub original_bytes: Arc<[u8]>,
    pub exact_utf8: bool,
    pub syntax: PdxDocument,
    pub data: Option<StateData>,
    pub diagnostics: Vec<ProjectDiagnostic>,
    pub modified: bool,
}

impl StateDocument {
    pub fn source(&self) -> &str {
        &self.syntax.source.text
    }

    pub fn original_bytes(&self) -> &[u8] {
        &self.original_bytes
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
    /// Initial-state-only presentation flag. Dated/conditional script is intentionally not evaluated.
    pub demilitarized_zone: Option<bool>,
    pub resources: BTreeMap<String, i64>,
    pub history: StateHistory,
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
    pub dated_blocks: Vec<DatedHistoryBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VictoryPoint {
    pub province_id: u32,
    pub value: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatedHistoryBlock {
    pub date: Hoi4Date,
    pub span: TextSpan,
    pub changes: StateHistoryDelta,
}

/// Supported operations declared inside one dated state-history block.
/// Unknown script remains in the parsed source document and is deliberately
/// not represented here; this is a read model, not a script rewriter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateHistoryDelta {
    pub owner: Option<String>,
    pub controller: Option<String>,
    pub added_cores: BTreeSet<String>,
    pub removed_cores: BTreeSet<String>,
    pub added_claims: BTreeSet<String>,
    pub removed_claims: BTreeSet<String>,
    pub victory_points: Vec<VictoryPoint>,
    pub state_buildings: BTreeMap<String, i64>,
    pub province_buildings: BTreeMap<u32, BTreeMap<String, i64>>,
}

#[cfg(test)]
mod tests {
    use super::{Hoi4Date, StateData, VictoryPoint};

    #[test]
    fn parses_and_orders_calendar_dates_numerically() {
        assert_eq!(Hoi4Date::parse("1924.1.1.12").unwrap().hour, 12);
        assert_eq!(Hoi4Date::parse("1939.1.1").unwrap().hour, 0);
        assert!(Hoi4Date::parse("1939.10.1").unwrap() > Hoi4Date::parse("1939.2.1").unwrap());
        assert!(Hoi4Date::parse("1939.1").is_none());
        assert!(Hoi4Date::parse("1939.1.1.0.1").is_none());
        assert!(Hoi4Date::parse("1939.13.1").is_none());
    }

    #[test]
    fn state_model_keeps_open_ended_names_and_large_values() {
        let mut state = StateData {
            id: Some(1),
            manpower: Some(142_000),
            ..Default::default()
        };
        state.provinces.extend([1405, 5144]);
        state.resources.insert("custom_resource".into(), 8);
        state
            .history
            .state_buildings
            .insert("custom_building".into(), 1);
        state.history.victory_points.push(VictoryPoint {
            province_id: 5144,
            value: 5,
        });

        assert!(state.provinces.contains(&5144));
        assert_eq!(state.resources["custom_resource"], 8);
        assert_eq!(
            state.history.victory_points[0],
            VictoryPoint {
                province_id: 5144,
                value: 5
            }
        );
    }
}
