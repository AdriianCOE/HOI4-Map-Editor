use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFillMode {
    HoveredProvince,
    ConnectedSameState,
    ConnectedUnassigned,
    WholeSourceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFillProvinceKind {
    Land,
    Sea,
    Lake,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateFillProvince {
    pub province_id: u32,
    pub kind: StateFillProvinceKind,
    pub state_id: Option<u32>,
    pub ambiguous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFillBlockedReason {
    ProvinceZero,
    UnknownProvince,
    NonLand,
    AmbiguousProvince,
    InvalidState,
    AssignedToOtherState,
    NotUnassigned,
    MissingSourceState,
    AlreadyAtDestination,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateFillBlockedProvince {
    pub province_id: u32,
    pub reason: StateFillBlockedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateFillPreview {
    pub found: Vec<u32>,
    pub applicable: Vec<u32>,
    pub blocked: Vec<StateFillBlockedProvince>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProvinceAdjacency {
    neighbors_by_province: BTreeMap<u32, BTreeSet<u32>>,
}

impl ProvinceAdjacency {
    pub fn from_pairs(pairs: impl IntoIterator<Item = (u32, u32)>) -> Self {
        let mut adjacency = Self::default();
        for (a, b) in pairs {
            if a == 0 || b == 0 || a == b {
                continue;
            }
            adjacency
                .neighbors_by_province
                .entry(a)
                .or_default()
                .insert(b);
            adjacency
                .neighbors_by_province
                .entry(b)
                .or_default()
                .insert(a);
        }
        adjacency
    }

    fn neighbors(&self, province_id: u32) -> impl Iterator<Item = u32> + '_ {
        self.neighbors_by_province
            .get(&province_id)
            .into_iter()
            .flatten()
            .copied()
    }
}

pub fn plan_state_fill(
    provinces: impl IntoIterator<Item = StateFillProvince>,
    adjacency: &ProvinceAdjacency,
    valid_state_ids: &BTreeSet<u32>,
    mode: StateFillMode,
    hovered_province_id: u32,
    destination_state_id: Option<u32>,
) -> StateFillPreview {
    let provinces = provinces
        .into_iter()
        .map(|province| (province.province_id, province))
        .collect::<BTreeMap<_, _>>();
    let mut blocked = BTreeMap::<u32, StateFillBlockedReason>::new();
    let Some(source) = candidate(
        &provinces,
        valid_state_ids,
        hovered_province_id,
        destination_state_id,
        &mut blocked,
    ) else {
        return preview(BTreeSet::new(), BTreeSet::new(), blocked);
    };

    let found = match mode {
        StateFillMode::HoveredProvince => BTreeSet::from([source.province_id]),
        StateFillMode::ConnectedSameState => {
            let Some(source_state_id) = source.state_id else {
                blocked.insert(
                    source.province_id,
                    StateFillBlockedReason::MissingSourceState,
                );
                return preview(BTreeSet::new(), BTreeSet::new(), blocked);
            };
            connected(
                &provinces,
                adjacency,
                valid_state_ids,
                source.province_id,
                destination_state_id,
                &mut blocked,
                |province| {
                    if province.state_id == Some(source_state_id) {
                        Ok(())
                    } else {
                        Err(StateFillBlockedReason::AssignedToOtherState)
                    }
                },
            )
        }
        StateFillMode::ConnectedUnassigned => {
            if source.state_id.is_some() {
                blocked.insert(source.province_id, StateFillBlockedReason::NotUnassigned);
                return preview(BTreeSet::new(), BTreeSet::new(), blocked);
            }
            connected(
                &provinces,
                adjacency,
                valid_state_ids,
                source.province_id,
                destination_state_id,
                &mut blocked,
                |province| {
                    if province.state_id.is_none() {
                        Ok(())
                    } else {
                        Err(StateFillBlockedReason::NotUnassigned)
                    }
                },
            )
        }
        StateFillMode::WholeSourceState => {
            let Some(source_state_id) = source.state_id else {
                blocked.insert(
                    source.province_id,
                    StateFillBlockedReason::MissingSourceState,
                );
                return preview(BTreeSet::new(), BTreeSet::new(), blocked);
            };
            provinces
                .values()
                .filter(|province| province.state_id == Some(source_state_id))
                .filter_map(|province| {
                    candidate(
                        &provinces,
                        valid_state_ids,
                        province.province_id,
                        destination_state_id,
                        &mut blocked,
                    )
                    .map(|province| province.province_id)
                })
                .collect()
        }
    };

    let applicable = found
        .iter()
        .copied()
        .filter(|province_id| {
            provinces
                .get(province_id)
                .is_some_and(|province| province.state_id != destination_state_id)
        })
        .collect();
    preview(found, applicable, blocked)
}

fn connected(
    provinces: &BTreeMap<u32, StateFillProvince>,
    adjacency: &ProvinceAdjacency,
    valid_state_ids: &BTreeSet<u32>,
    start: u32,
    destination_state_id: Option<u32>,
    blocked: &mut BTreeMap<u32, StateFillBlockedReason>,
    accepts: impl Fn(StateFillProvince) -> Result<(), StateFillBlockedReason>,
) -> BTreeSet<u32> {
    let mut found = BTreeSet::new();
    let mut queued = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);

    while let Some(province_id) = queue.pop_front() {
        let Some(province) = candidate(
            provinces,
            valid_state_ids,
            province_id,
            destination_state_id,
            blocked,
        ) else {
            continue;
        };
        if let Err(reason) = accepts(province) {
            blocked.insert(province_id, reason);
            continue;
        }
        found.insert(province_id);

        for neighbor in adjacency.neighbors(province_id) {
            if queued.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }

    found
}

fn candidate(
    provinces: &BTreeMap<u32, StateFillProvince>,
    valid_state_ids: &BTreeSet<u32>,
    province_id: u32,
    destination_state_id: Option<u32>,
    blocked: &mut BTreeMap<u32, StateFillBlockedReason>,
) -> Option<StateFillProvince> {
    if province_id == 0 {
        blocked.insert(province_id, StateFillBlockedReason::ProvinceZero);
        return None;
    }

    let Some(province) = provinces.get(&province_id).copied() else {
        blocked.insert(province_id, StateFillBlockedReason::UnknownProvince);
        return None;
    };

    if province.kind != StateFillProvinceKind::Land {
        blocked.insert(province_id, StateFillBlockedReason::NonLand);
        return None;
    }
    if province.ambiguous {
        blocked.insert(province_id, StateFillBlockedReason::AmbiguousProvince);
        return None;
    }
    if province
        .state_id
        .is_some_and(|state_id| !valid_state_ids.contains(&state_id))
    {
        blocked.insert(province_id, StateFillBlockedReason::InvalidState);
        return None;
    }
    if province.state_id == destination_state_id {
        blocked
            .entry(province_id)
            .or_insert(StateFillBlockedReason::AlreadyAtDestination);
    }

    Some(province)
}

fn preview(
    found: BTreeSet<u32>,
    applicable: BTreeSet<u32>,
    blocked: BTreeMap<u32, StateFillBlockedReason>,
) -> StateFillPreview {
    StateFillPreview {
        found: found.into_iter().collect(),
        applicable: applicable.into_iter().collect(),
        blocked: blocked
            .into_iter()
            .map(|(province_id, reason)| StateFillBlockedProvince {
                province_id,
                reason,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::map::{Bundle, ProvinceKind};
    use crate::app::project::{Hoi4Project, ProjectPaths, StateEditSession};
    use crate::config::Config;
    use crate::util::files::Location;
    use std::path::PathBuf;

    fn province(
        province_id: u32,
        kind: StateFillProvinceKind,
        state_id: Option<u32>,
    ) -> StateFillProvince {
        StateFillProvince {
            province_id,
            kind,
            state_id,
            ambiguous: false,
        }
    }

    fn fixture() -> (Vec<StateFillProvince>, ProvinceAdjacency, BTreeSet<u32>) {
        let mut provinces = vec![
            province(1, StateFillProvinceKind::Land, Some(10)),
            province(2, StateFillProvinceKind::Land, Some(10)),
            province(3, StateFillProvinceKind::Sea, None),
            province(4, StateFillProvinceKind::Land, Some(10)),
            province(5, StateFillProvinceKind::Land, Some(20)),
            province(6, StateFillProvinceKind::Land, None),
            province(7, StateFillProvinceKind::Land, None),
            province(8, StateFillProvinceKind::Land, Some(99)),
            province(9, StateFillProvinceKind::Unknown, None),
            province(11, StateFillProvinceKind::Lake, None),
        ];
        provinces.push(StateFillProvince {
            province_id: 12,
            kind: StateFillProvinceKind::Land,
            state_id: Some(10),
            ambiguous: true,
        });
        let adjacency = ProvinceAdjacency::from_pairs([
            (2, 1),
            (1, 2),
            (2, 3),
            (3, 4),
            (2, 5),
            (6, 7),
            (7, 5),
            (1, 8),
            (1, 9),
            (1, 11),
            (1, 12),
            (0, 1),
        ]);
        (provinces, adjacency, BTreeSet::from([10, 20]))
    }

    #[test]
    fn hovered_province_returns_one_applicable_id_for_reassign() {
        let (provinces, adjacency, valid_states) = fixture();
        let preview = plan_state_fill(
            provinces,
            &adjacency,
            &valid_states,
            StateFillMode::HoveredProvince,
            2,
            Some(20),
        );

        assert_eq!(preview.found, vec![2]);
        assert_eq!(preview.applicable, vec![2]);
        assert!(preview.blocked.is_empty());
    }

    #[test]
    fn connected_same_state_does_not_cross_sea_or_other_state() {
        let (provinces, adjacency, valid_states) = fixture();
        let preview = plan_state_fill(
            provinces,
            &adjacency,
            &valid_states,
            StateFillMode::ConnectedSameState,
            1,
            Some(20),
        );

        assert_eq!(preview.found, vec![1, 2]);
        assert_eq!(preview.applicable, vec![1, 2]);
        assert_eq!(
            preview.blocked,
            vec![
                StateFillBlockedProvince {
                    province_id: 3,
                    reason: StateFillBlockedReason::NonLand
                },
                StateFillBlockedProvince {
                    province_id: 5,
                    reason: StateFillBlockedReason::AssignedToOtherState
                },
                StateFillBlockedProvince {
                    province_id: 8,
                    reason: StateFillBlockedReason::InvalidState
                },
                StateFillBlockedProvince {
                    province_id: 9,
                    reason: StateFillBlockedReason::NonLand
                },
                StateFillBlockedProvince {
                    province_id: 11,
                    reason: StateFillBlockedReason::NonLand
                },
                StateFillBlockedProvince {
                    province_id: 12,
                    reason: StateFillBlockedReason::AmbiguousProvince
                },
            ]
        );
    }

    #[test]
    fn connected_unassigned_stays_inside_unassigned_land_component() {
        let (provinces, adjacency, valid_states) = fixture();
        let preview = plan_state_fill(
            provinces,
            &adjacency,
            &valid_states,
            StateFillMode::ConnectedUnassigned,
            6,
            Some(10),
        );

        assert_eq!(preview.found, vec![6, 7]);
        assert_eq!(preview.applicable, vec![6, 7]);
        assert_eq!(
            preview.blocked,
            vec![StateFillBlockedProvince {
                province_id: 5,
                reason: StateFillBlockedReason::NotUnassigned
            }]
        );
    }

    #[test]
    fn whole_source_state_uses_all_valid_non_ambiguous_source_provinces() {
        let (provinces, adjacency, valid_states) = fixture();
        let preview = plan_state_fill(
            provinces,
            &adjacency,
            &valid_states,
            StateFillMode::WholeSourceState,
            1,
            Some(20),
        );

        assert_eq!(preview.found, vec![1, 2, 4]);
        assert_eq!(preview.applicable, vec![1, 2, 4]);
        assert_eq!(
            preview.blocked,
            vec![StateFillBlockedProvince {
                province_id: 12,
                reason: StateFillBlockedReason::AmbiguousProvince,
            }]
        );
    }

    #[test]
    fn deterministic_dedup_and_noop_reporting_are_stable() {
        let (provinces, adjacency, valid_states) = fixture();
        let preview = plan_state_fill(
            provinces,
            &adjacency,
            &valid_states,
            StateFillMode::ConnectedSameState,
            2,
            Some(10),
        );

        assert_eq!(preview.found, vec![1, 2]);
        assert!(preview.applicable.is_empty());
        assert_eq!(
            preview.blocked[0],
            StateFillBlockedProvince {
                province_id: 1,
                reason: StateFillBlockedReason::AlreadyAtDestination,
            }
        );
        assert_eq!(
            preview.blocked[1],
            StateFillBlockedProvince {
                province_id: 2,
                reason: StateFillBlockedReason::AlreadyAtDestination,
            }
        );
    }

    #[test]
    #[ignore = "requires HOI4_STATE_EDITOR_REAL_MOD_ROOT"]
    fn real_mod_state_fill_is_one_in_memory_transaction() {
        let root = std::env::var_os("HOI4_STATE_EDITOR_REAL_MOD_ROOT")
            .map(PathBuf::from)
            .expect("set HOI4_STATE_EDITOR_REAL_MOD_ROOT");
        let paths = ProjectPaths::discover(&root).unwrap();
        let bundle = Bundle::load(
            &Location::Directory(paths.map_directory.clone()),
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .unwrap();
        let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
        let land_ids = bundle
            .map
            .iter_province_data()
            .filter(|(_, province)| province.kind == ProvinceKind::Land)
            .filter_map(|(_, province)| province.preserved_id)
            .collect::<BTreeSet<_>>();
        let mut project = Hoi4Project::new(paths);
        project.load_states(&province_ids, &land_ids);
        let mut edit = StateEditSession::new(&project, &bundle.map);
        let source = 5144;
        let source_state = edit
            .province_state_id(source)
            .expect("State for province 5144");
        let target = edit
            .valid_state_ids()
            .iter()
            .copied()
            .find(|state_id| *state_id != source_state)
            .expect("another valid state");
        let provinces = bundle.map.iter_province_data().filter_map(|(_, province)| {
            let province_id = province.preserved_id?;
            Some(StateFillProvince {
                province_id,
                kind: match province.kind {
                    ProvinceKind::Land => StateFillProvinceKind::Land,
                    ProvinceKind::Sea => StateFillProvinceKind::Sea,
                    ProvinceKind::Lake => StateFillProvinceKind::Lake,
                    ProvinceKind::Unknown => StateFillProvinceKind::Unknown,
                },
                state_id: edit.province_state_id(province_id),
                ambiguous: project.ambiguous_provinces.contains_key(&province_id),
            })
        });
        let preview = plan_state_fill(
            provinces,
            &ProvinceAdjacency::default(),
            edit.valid_state_ids(),
            StateFillMode::HoveredProvince,
            source,
            Some(target),
        );
        assert_eq!(preview.applicable, vec![source]);
        let commands = edit.summary().commands;
        let province_data = edit.province_data(source);
        edit.reassign_provinces(&preview.applicable, Some(target))
            .unwrap();
        assert_eq!(edit.summary().commands, commands + 1);
        assert_eq!(edit.province_state_id(source), Some(target));
        assert_eq!(edit.province_data(source), province_data);
        assert!(edit.undo());
        assert_eq!(edit.province_state_id(source), Some(source_state));
        edit.discard();
        assert!(!edit.is_dirty());
    }
}
