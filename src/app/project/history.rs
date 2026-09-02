//! Read-only resolution of State history at the project's initial bookmark.
//!
//! State source files remain the authoritative script representation.  This
//! module derives a session-facing model from the subset of history commands
//! that the editor already understands; it never rewrites dated blocks.

use std::collections::{BTreeMap, BTreeSet};

use crate::app::state::{
    Hoi4Date, PdxBlock, PdxEntry, PdxValue, StateData, VictoryPoint, parse_text,
};

use super::{ProjectSources, ResolvedSource};

pub const BOOKMARKS_DIRECTORY: &str = "common/bookmarks";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveHistoryDate {
    Resolved {
        date: Hoi4Date,
        source: ResolvedSource,
    },
    Unavailable {
        reason: String,
    },
}

impl EffectiveHistoryDate {
    pub fn date(&self) -> Option<Hoi4Date> {
        match self {
            Self::Resolved { date, .. } => Some(*date),
            Self::Unavailable { .. } => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Resolved { date, .. } => format!("Initial bookmark: {date}"),
            Self::Unavailable { reason } => format!("Initial bookmark unavailable: {reason}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveHistoryOrigin {
    Direct,
    Dated(Hoi4Date),
    ImplicitOwner,
    Unresolved,
}

impl EffectiveHistoryOrigin {
    pub fn label(self) -> String {
        match self {
            Self::Direct => "declared".to_owned(),
            Self::Dated(date) => format!("dated {date}"),
            Self::ImplicitOwner => "implicit from owner".to_owned(),
            Self::Unresolved => "unresolved".to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EffectiveStateHistory {
    pub data: StateData,
    pub owner_origin: EffectiveHistoryOrigin,
    pub controller_origin: EffectiveHistoryOrigin,
    /// Display-only controller. It equals `data.history.controller` when the
    /// source declares one; otherwise it is inherited from the effective
    /// owner without manufacturing a serializable `controller = ...` field.
    pub controller_value: Option<String>,
}

/// Exact editable State-history values affected by dated blocks at the active
/// initial bookmark.
///
/// Without a resolved bookmark, the impact includes every dated operation in
/// the source rather than guessing which point in the timeline an edit targets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatedHistoryImpact {
    pub owner_has_dated_effects: bool,
    pub controller_has_dated_effects: bool,
    pub cores: BTreeSet<String>,
    pub claims: BTreeSet<String>,
    pub victory_points: BTreeSet<u32>,
    pub state_buildings: BTreeSet<String>,
    pub province_buildings: BTreeMap<u32, BTreeSet<String>>,
}

pub fn dated_history_impact(
    data: &StateData,
    effective_date: Option<Hoi4Date>,
) -> DatedHistoryImpact {
    if data.history.dated_blocks.is_empty() {
        return DatedHistoryImpact::default();
    }

    let mut impact = DatedHistoryImpact::default();
    for block in data
        .history
        .dated_blocks
        .iter()
        .filter(|block| effective_date.is_none_or(|date| block.date <= date))
    {
        let changes = &block.changes;
        impact.owner_has_dated_effects |= changes.owner.is_some();
        impact.controller_has_dated_effects |= changes.controller.is_some();
        impact.cores.extend(changes.added_cores.iter().cloned());
        impact.cores.extend(changes.removed_cores.iter().cloned());
        impact.claims.extend(changes.added_claims.iter().cloned());
        impact.claims.extend(changes.removed_claims.iter().cloned());
        impact
            .victory_points
            .extend(changes.victory_points.iter().map(|point| point.province_id));
        impact
            .state_buildings
            .extend(changes.state_buildings.keys().cloned());
        for (province_id, buildings) in &changes.province_buildings {
            impact
                .province_buildings
                .entry(*province_id)
                .or_default()
                .extend(buildings.keys().cloned());
        }
    }
    impact
}

pub fn resolve_effective_history_date(sources: &ProjectSources) -> EffectiveHistoryDate {
    let listing = match sources.list_files(BOOKMARKS_DIRECTORY) {
        Ok(listing) => listing,
        Err(error) => {
            return EffectiveHistoryDate::Unavailable {
                reason: format!("could not list {BOOKMARKS_DIRECTORY}: {error}"),
            };
        }
    };

    let mut earliest = None::<(Hoi4Date, ResolvedSource)>;
    for source in listing.files {
        if !source
            .logical_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        {
            continue;
        }
        let Ok(bytes) = sources.read_resolved(&source) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let document = parse_text(source.logical_path.clone(), text);
        for date in bookmark_dates(&document.entries) {
            if earliest.as_ref().is_none_or(|(current, _)| date < *current) {
                earliest = Some((date, source.clone()));
            }
        }
    }

    earliest.map_or_else(
        || EffectiveHistoryDate::Unavailable {
            reason: format!("no valid bookmark date was found in {BOOKMARKS_DIRECTORY}"),
        },
        |(date, source)| EffectiveHistoryDate::Resolved { date, source },
    )
}

pub fn effective_state_history(
    data: &StateData,
    effective_date: Option<Hoi4Date>,
) -> EffectiveStateHistory {
    let mut data = data.clone();
    let mut owner_origin = data
        .history
        .owner
        .as_ref()
        .map_or(EffectiveHistoryOrigin::Unresolved, |_| {
            EffectiveHistoryOrigin::Direct
        });
    let mut controller_origin = data
        .history
        .controller
        .as_ref()
        .map_or(EffectiveHistoryOrigin::Unresolved, |_| {
            EffectiveHistoryOrigin::Direct
        });

    for tag in &data.history.removed_cores {
        data.history.cores.remove(tag);
    }
    for tag in &data.history.removed_claims {
        data.history.claims.remove(tag);
    }

    if let Some(effective_date) = effective_date {
        let mut blocks = data.history.dated_blocks.clone();
        blocks.sort_by_key(|block| block.date);
        for block in blocks
            .into_iter()
            .filter(|block| block.date <= effective_date)
        {
            let changes = block.changes;
            if let Some(owner) = changes.owner {
                data.history.owner = Some(owner);
                owner_origin = EffectiveHistoryOrigin::Dated(block.date);
            }
            if let Some(controller) = changes.controller {
                data.history.controller = Some(controller);
                controller_origin = EffectiveHistoryOrigin::Dated(block.date);
            }
            for tag in changes.removed_cores {
                data.history.cores.remove(&tag);
            }
            for tag in changes.added_cores {
                data.history.cores.insert(tag);
            }
            for tag in changes.removed_claims {
                data.history.claims.remove(&tag);
            }
            for tag in changes.added_claims {
                data.history.claims.insert(tag);
            }
            for victory_point in changes.victory_points {
                apply_victory_point(&mut data.history.victory_points, victory_point);
            }
            data.history.state_buildings.extend(changes.state_buildings);
            for (province_id, buildings) in changes.province_buildings {
                data.history
                    .province_buildings
                    .entry(province_id)
                    .or_default()
                    .extend(buildings);
            }
        }
    }

    let controller_value = if let Some(controller) = data.history.controller.clone() {
        Some(controller)
    } else if let Some(owner) = data.history.owner.clone() {
        controller_origin = EffectiveHistoryOrigin::ImplicitOwner;
        Some(owner)
    } else {
        None
    };

    EffectiveStateHistory {
        data,
        owner_origin,
        controller_origin,
        controller_value,
    }
}

fn bookmark_dates(entries: &[PdxEntry]) -> Vec<Hoi4Date> {
    let mut dates = Vec::new();
    for entry in entries {
        let key = entry.key.as_ref().map(|key| key.text.as_str());
        if key == Some("bookmark")
            && let PdxValue::Block(block) = &entry.value
        {
            dates.extend(date_in_bookmark(block));
        }
        if let PdxValue::Block(block) = &entry.value {
            dates.extend(bookmark_dates(&block.entries));
        }
    }
    dates
}

fn date_in_bookmark(bookmark: &PdxBlock) -> Option<Hoi4Date> {
    bookmark.entries.iter().find_map(|entry| {
        (entry.key.as_ref().map(|key| key.text.as_str()) == Some("date"))
            .then(|| match &entry.value {
                PdxValue::Scalar(value) => Hoi4Date::parse(&value.text),
                PdxValue::Block(_) => None,
            })
            .flatten()
    })
}

fn apply_victory_point(values: &mut Vec<VictoryPoint>, value: VictoryPoint) {
    if let Some(existing) = values
        .iter_mut()
        .find(|existing| existing.province_id == value.province_id)
    {
        *existing = value;
    } else {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::project::{ProjectSources, SourceGeneration};
    use crate::app::state::{DatedHistoryBlock, StateHistoryDelta, TextSpan};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::PathBuf;

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "hoi4-state-editor-history-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            fs::create_dir(path.join("map")).unwrap();
            fs::write(path.join("map/provinces.bmp"), []).unwrap();
            fs::write(path.join("map/definition.csv"), []).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn selects_the_earliest_resolved_bookmark_date() {
        let root = TempRoot::new("bookmarks");
        fs::create_dir_all(root.0.join(BOOKMARKS_DIRECTORY)).unwrap();
        fs::write(
            root.0.join("common/bookmarks/later.txt"),
            "bookmarks={ bookmark={ date=1925.6.1.12 } }",
        )
        .unwrap();
        fs::write(
            root.0.join("common/bookmarks/initial.txt"),
            "bookmarks={ bookmark={ date=1924.1.1.12 } }",
        )
        .unwrap();
        let sources = ProjectSources::discover(&root.0, None, SourceGeneration::new(5)).unwrap();
        assert!(matches!(
            resolve_effective_history_date(&sources),
            EffectiveHistoryDate::Resolved { date, .. } if date == Hoi4Date::parse("1924.1.1.12").unwrap()
        ));
    }

    #[test]
    fn honors_source_precedence_and_replace_path_for_bookmarks() {
        let project = TempRoot::new("bookmark-project-override");
        let base = TempRoot::new("bookmark-base-override");
        for root in [&project, &base] {
            fs::create_dir_all(root.0.join(BOOKMARKS_DIRECTORY)).unwrap();
        }
        fs::write(
            project.0.join("descriptor.mod"),
            "name=\"test\"\nreplace_path=\"common/bookmarks\"\n",
        )
        .unwrap();
        fs::write(
            project.0.join("common/bookmarks/start.txt"),
            "bookmarks={ bookmark={ date=1925.1.1.12 } }",
        )
        .unwrap();
        fs::write(
            base.0.join("common/bookmarks/lower-only.txt"),
            "bookmarks={ bookmark={ date=1918.1.1.12 } }",
        )
        .unwrap();
        let sources =
            ProjectSources::discover(&project.0, Some(base.0.clone()), SourceGeneration::new(7))
                .unwrap();
        assert!(matches!(
            resolve_effective_history_date(&sources),
            EffectiveHistoryDate::Resolved { date, .. } if date == Hoi4Date::parse("1925.1.1.12").unwrap()
        ));
    }

    #[test]
    fn applies_only_history_up_to_the_initial_bookmark_and_marks_implicit_control() {
        let mut data = StateData::default();
        data.history.owner = Some("TOL".to_owned());
        data.history.cores = BTreeSet::from(["TOL".to_owned()]);
        data.history.claims = BTreeSet::from(["OLD".to_owned()]);
        data.history.victory_points.push(VictoryPoint {
            province_id: 2,
            value: 3,
        });
        data.history
            .state_buildings
            .insert("infrastructure".to_owned(), 1);
        data.history.dated_blocks = vec![
            DatedHistoryBlock {
                date: Hoi4Date::parse("1924.1.1.12").unwrap(),
                span: TextSpan::default(),
                changes: StateHistoryDelta {
                    controller: Some("KOZ".to_owned()),
                    removed_cores: BTreeSet::from(["TOL".to_owned()]),
                    added_cores: BTreeSet::from(["KOZ".to_owned()]),
                    removed_claims: BTreeSet::from(["OLD".to_owned()]),
                    added_claims: BTreeSet::from(["KOZ".to_owned()]),
                    victory_points: vec![VictoryPoint {
                        province_id: 2,
                        value: 8,
                    }],
                    state_buildings: BTreeMap::from([("infrastructure".to_owned(), 4)]),
                    ..Default::default()
                },
            },
            DatedHistoryBlock {
                date: Hoi4Date::parse("1925.1.1.12").unwrap(),
                span: TextSpan::default(),
                changes: StateHistoryDelta {
                    owner: Some("LATER".to_owned()),
                    ..Default::default()
                },
            },
        ];
        let resolved = effective_state_history(&data, Hoi4Date::parse("1924.1.1.12"));
        assert_eq!(resolved.data.history.owner.as_deref(), Some("TOL"));
        assert_eq!(resolved.data.history.controller.as_deref(), Some("KOZ"));
        assert_eq!(
            resolved.data.history.cores,
            BTreeSet::from(["KOZ".to_owned()])
        );
        assert_eq!(
            resolved.data.history.claims,
            BTreeSet::from(["KOZ".to_owned()])
        );
        assert_eq!(resolved.data.history.victory_points[0].value, 8);
        assert_eq!(resolved.data.history.state_buildings["infrastructure"], 4);
        assert_eq!(
            resolved.controller_origin,
            EffectiveHistoryOrigin::Dated(Hoi4Date::parse("1924.1.1.12").unwrap())
        );

        data.history.dated_blocks.clear();
        let implicit = effective_state_history(&data, None);
        assert!(implicit.data.history.controller.is_none());
        assert_eq!(implicit.controller_value.as_deref(), Some("TOL"));
        assert_eq!(
            implicit.controller_origin,
            EffectiveHistoryOrigin::ImplicitOwner
        );
    }

    #[test]
    fn dated_history_impact_tracks_only_values_effective_at_the_bookmark() {
        let mut data = StateData::default();
        data.history.dated_blocks = vec![
            DatedHistoryBlock {
                date: Hoi4Date::parse("1924.1.1").unwrap(),
                span: TextSpan::default(),
                changes: StateHistoryDelta {
                    controller: Some("KOZ".to_owned()),
                    added_cores: BTreeSet::from(["GER".to_owned()]),
                    removed_claims: BTreeSet::from(["ITA".to_owned()]),
                    victory_points: vec![VictoryPoint {
                        province_id: 1234,
                        value: 5,
                    }],
                    state_buildings: BTreeMap::from([("arms_factory".to_owned(), 1)]),
                    province_buildings: BTreeMap::from([(
                        5678,
                        BTreeMap::from([("bunker".to_owned(), 1)]),
                    )]),
                    ..Default::default()
                },
            },
            DatedHistoryBlock {
                date: Hoi4Date::parse("1925.1.1").unwrap(),
                span: TextSpan::default(),
                changes: StateHistoryDelta {
                    owner: Some("LATER".to_owned()),
                    added_cores: BTreeSet::from(["FRA".to_owned()]),
                    ..Default::default()
                },
            },
        ];

        let initial = dated_history_impact(&data, Some(Hoi4Date::parse("1924.1.1.12").unwrap()));
        assert!(!initial.owner_has_dated_effects);
        assert!(initial.controller_has_dated_effects);
        assert_eq!(initial.cores, BTreeSet::from(["GER".to_owned()]));
        assert_eq!(initial.claims, BTreeSet::from(["ITA".to_owned()]));
        assert_eq!(initial.victory_points, BTreeSet::from([1234]));
        assert_eq!(
            initial.state_buildings,
            BTreeSet::from(["arms_factory".to_owned()])
        );
        assert_eq!(
            initial.province_buildings,
            BTreeMap::from([(5678, BTreeSet::from(["bunker".to_owned()]),)])
        );

        let unknown_date = dated_history_impact(&data, None);
        assert!(unknown_date.owner_has_dated_effects);
        assert_eq!(
            unknown_date.cores,
            BTreeSet::from(["FRA".to_owned(), "GER".to_owned()])
        );
    }

    #[test]
    fn keeps_direct_fallback_when_bookmarks_are_unavailable() {
        let root = TempRoot::new("no-bookmarks");
        let sources = ProjectSources::discover(&root.0, None, SourceGeneration::new(6)).unwrap();
        assert!(matches!(
            resolve_effective_history_date(&sources),
            EffectiveHistoryDate::Unavailable { .. }
        ));
    }
}
