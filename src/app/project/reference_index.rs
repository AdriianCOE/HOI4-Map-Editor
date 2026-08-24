//! Derived, read-only province reference discovery.
//!
//! This index is generation-scoped, sparse-ID-safe, and deliberately has no
//! save or editing authority. It consumes already loaded project, map, and
//! logistics data; callers rebuild it after a relevant project/session change.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::map::Map;
use crate::app::state::StateData;

use super::{Hoi4Project, ResolvedSource, SourceGeneration, StateEditSession, WorkingStateOrigin};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProvinceReferenceDomain {
    StateMembership,
    VictoryPoint,
    Building,
    Adjacency,
    Railway,
    SupplyNode,
    StrategicRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdjacencyReferenceRole {
    From,
    To,
    Through,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReferenceDomain {
    States,
    Adjacencies,
    Railways,
    SupplyNodes,
    StrategicRegions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceCoverageStatus {
    Complete,
    NotPresent,
    Incomplete { unavailable_inputs: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReferenceCoverage {
    statuses: BTreeMap<ReferenceDomain, ReferenceCoverageStatus>,
}

impl ReferenceCoverage {
    pub fn status(&self, domain: ReferenceDomain) -> &ReferenceCoverageStatus {
        self.statuses
            .get(&domain)
            .expect("every reference domain has coverage")
    }

    pub fn is_complete(&self) -> bool {
        self.statuses.values().all(|status| {
            matches!(
                status,
                ReferenceCoverageStatus::Complete | ReferenceCoverageStatus::NotPresent
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvinceReferenceSource {
    StateFile {
        path: PathBuf,
        generation: SourceGeneration,
    },
    StateSession {
        state_id: u32,
        document_path: Option<PathBuf>,
        generation: SourceGeneration,
    },
    Resolved(ResolvedSource),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvinceReference {
    StateMembership {
        state_id: u32,
        source: Arc<ProvinceReferenceSource>,
    },
    VictoryPoint {
        state_id: u32,
        value: i64,
        source: Arc<ProvinceReferenceSource>,
    },
    Building {
        state_id: u32,
        building_type: String,
        value: i64,
        source: Arc<ProvinceReferenceSource>,
    },
    Adjacency {
        row: usize,
        role: AdjacencyReferenceRole,
        source: Arc<ProvinceReferenceSource>,
    },
    Railway {
        railway_index: usize,
        sequence_ordinal: usize,
        source: Arc<ProvinceReferenceSource>,
    },
    SupplyNode {
        node_index: usize,
        source: Arc<ProvinceReferenceSource>,
    },
    StrategicRegion {
        region_id: u32,
        source: Arc<ProvinceReferenceSource>,
    },
}

impl ProvinceReference {
    pub fn domain(&self) -> ProvinceReferenceDomain {
        match self {
            Self::StateMembership { .. } => ProvinceReferenceDomain::StateMembership,
            Self::VictoryPoint { .. } => ProvinceReferenceDomain::VictoryPoint,
            Self::Building { .. } => ProvinceReferenceDomain::Building,
            Self::Adjacency { .. } => ProvinceReferenceDomain::Adjacency,
            Self::Railway { .. } => ProvinceReferenceDomain::Railway,
            Self::SupplyNode { .. } => ProvinceReferenceDomain::SupplyNode,
            Self::StrategicRegion { .. } => ProvinceReferenceDomain::StrategicRegion,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProvinceReferenceSummary {
    pub states: usize,
    pub victory_points: usize,
    pub buildings: usize,
    pub adjacencies: usize,
    pub railways: usize,
    pub supply_nodes: usize,
    pub strategic_regions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvinceReferenceIndex {
    generation: SourceGeneration,
    references_by_province: BTreeMap<u32, Vec<ProvinceReference>>,
}

impl ProvinceReferenceIndex {
    pub fn generation(&self) -> SourceGeneration {
        self.generation
    }

    pub fn references_to_province(&self, province_id: u32) -> &[ProvinceReference] {
        self.references_by_province
            .get(&province_id)
            .map_or(&[], Vec::as_slice)
    }

    pub fn references_to_province_for_generation(
        &self,
        generation: SourceGeneration,
        province_id: u32,
    ) -> Option<&[ProvinceReference]> {
        (self.generation == generation).then(|| self.references_to_province(province_id))
    }

    pub fn has_references(&self, province_id: u32) -> bool {
        !self.references_to_province(province_id).is_empty()
    }

    pub fn references_by_domain(
        &self,
        province_id: u32,
        domain: ProvinceReferenceDomain,
    ) -> impl Iterator<Item = &ProvinceReference> {
        self.references_to_province(province_id)
            .iter()
            .filter(move |reference| reference.domain() == domain)
    }

    pub fn referenced_province_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.references_by_province.keys().copied()
    }

    pub fn reference_count(&self) -> usize {
        self.references_by_province.values().map(Vec::len).sum()
    }

    pub fn summary_for_province(&self, province_id: u32) -> ProvinceReferenceSummary {
        self.references_to_province(province_id).iter().fold(
            ProvinceReferenceSummary::default(),
            |mut summary, reference| {
                match reference.domain() {
                    ProvinceReferenceDomain::StateMembership => summary.states += 1,
                    ProvinceReferenceDomain::VictoryPoint => summary.victory_points += 1,
                    ProvinceReferenceDomain::Building => summary.buildings += 1,
                    ProvinceReferenceDomain::Adjacency => summary.adjacencies += 1,
                    ProvinceReferenceDomain::Railway => summary.railways += 1,
                    ProvinceReferenceDomain::SupplyNode => summary.supply_nodes += 1,
                    ProvinceReferenceDomain::StrategicRegion => summary.strategic_regions += 1,
                }
                summary
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvinceReferenceIndexBuild {
    pub index: ProvinceReferenceIndex,
    pub coverage: ReferenceCoverage,
    pub built_in: Duration,
}

pub fn build_province_reference_index(
    project: &Hoi4Project,
    map: &Map,
    state_edit_session: Option<&StateEditSession>,
) -> ProvinceReferenceIndexBuild {
    let started = Instant::now();
    let generation = project.paths.sources.manifest().project_generation;
    let mut references_by_province = BTreeMap::<u32, Vec<ProvinceReference>>::new();
    let mut statuses = BTreeMap::new();

    statuses.insert(ReferenceDomain::States, state_coverage(project));
    statuses.insert(
        ReferenceDomain::StrategicRegions,
        strategic_region_coverage(project),
    );
    for region in &project.strategic_regions.regions {
        let source = Arc::new(ProvinceReferenceSource::Resolved((*region.source).clone()));
        for &province_id in &region.provinces {
            references_by_province.entry(province_id).or_default().push(
                ProvinceReference::StrategicRegion {
                    region_id: region.id,
                    source: source.clone(),
                },
            );
        }
    }
    if let Some(session) = state_edit_session {
        for &state_id in session.valid_state_ids() {
            let Some(data) = session.state_data(state_id) else {
                continue;
            };
            let source = state_source_from_session(project, session, state_id, generation);
            add_state_references(&mut references_by_province, state_id, &data, source);
        }
    } else {
        for (&state_id, &document_index) in &project.states_by_id {
            let Some(document) = project.states.get(document_index) else {
                continue;
            };
            let Some(data) = document.data.as_ref() else {
                continue;
            };
            let source = Arc::new(ProvinceReferenceSource::StateFile {
                path: project.paths.states_directory.join(&document.path),
                generation,
            });
            add_state_references(&mut references_by_province, state_id, data, source);
        }
    }

    let adjacency_source = project
        .paths
        .sources
        .source_files()
        .adjacencies_csv
        .clone()
        .map(ProvinceReferenceSource::Resolved)
        .map(Arc::new);
    statuses.insert(
        ReferenceDomain::Adjacencies,
        if adjacency_source.is_some() {
            ReferenceCoverageStatus::Complete
        } else {
            ReferenceCoverageStatus::NotPresent
        },
    );
    if let Some(source) = adjacency_source {
        for (row, adjacency) in map.adjacencies().iter().enumerate() {
            push(
                &mut references_by_province,
                adjacency.from_id,
                ProvinceReference::Adjacency {
                    row,
                    role: AdjacencyReferenceRole::From,
                    source: source.clone(),
                },
            );
            push(
                &mut references_by_province,
                adjacency.to_id,
                ProvinceReference::Adjacency {
                    row,
                    role: AdjacencyReferenceRole::To,
                    source: source.clone(),
                },
            );
            if let Some(province_id) = adjacency.through {
                push(
                    &mut references_by_province,
                    province_id,
                    ProvinceReference::Adjacency {
                        row,
                        role: AdjacencyReferenceRole::Through,
                        source: source.clone(),
                    },
                );
            }
        }
    }

    statuses.insert(
        ReferenceDomain::Railways,
        logistics_coverage(
            project.logistics.railways.source.is_some(),
            project.logistics.railways.issues.len(),
        ),
    );
    for (railway_index, railway) in project.logistics.railways.railways.iter().enumerate() {
        let source = Arc::new(ProvinceReferenceSource::Resolved(railway.source.clone()));
        for (sequence_ordinal, &province_id) in railway.provinces.iter().enumerate() {
            push(
                &mut references_by_province,
                province_id,
                ProvinceReference::Railway {
                    railway_index,
                    sequence_ordinal,
                    source: source.clone(),
                },
            );
        }
    }

    statuses.insert(
        ReferenceDomain::SupplyNodes,
        logistics_coverage(
            project.logistics.supply_nodes.source.is_some(),
            project.logistics.supply_nodes.issues.len(),
        ),
    );
    for (node_index, node) in project
        .logistics
        .supply_nodes
        .supply_nodes
        .iter()
        .enumerate()
    {
        push(
            &mut references_by_province,
            node.province_id,
            ProvinceReference::SupplyNode {
                node_index,
                source: Arc::new(ProvinceReferenceSource::Resolved(node.source.clone())),
            },
        );
    }

    for references in references_by_province.values_mut() {
        references.sort_by(reference_order);
    }
    ProvinceReferenceIndexBuild {
        index: ProvinceReferenceIndex {
            generation,
            references_by_province,
        },
        coverage: ReferenceCoverage { statuses },
        built_in: started.elapsed(),
    }
}

fn state_coverage(project: &Hoi4Project) -> ReferenceCoverageStatus {
    let unavailable_inputs = project.load_summary.report.files_failed.len()
        + project
            .states
            .iter()
            .filter(|document| {
                document.data.is_none()
                    || document
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.severity == super::DiagnosticSeverity::Error)
            })
            .count();
    if unavailable_inputs == 0 {
        ReferenceCoverageStatus::Complete
    } else {
        ReferenceCoverageStatus::Incomplete { unavailable_inputs }
    }
}

fn strategic_region_coverage(project: &Hoi4Project) -> ReferenceCoverageStatus {
    match project.strategic_regions.coverage {
        super::StrategicRegionCoverage::Complete => ReferenceCoverageStatus::Complete,
        super::StrategicRegionCoverage::NotPresent => ReferenceCoverageStatus::NotPresent,
        super::StrategicRegionCoverage::Incomplete { files_failed, .. } => {
            ReferenceCoverageStatus::Incomplete {
                unavailable_inputs: files_failed,
            }
        }
    }
}

fn logistics_coverage(present: bool, issues: usize) -> ReferenceCoverageStatus {
    if issues != 0 {
        ReferenceCoverageStatus::Incomplete {
            unavailable_inputs: issues,
        }
    } else if present {
        ReferenceCoverageStatus::Complete
    } else {
        ReferenceCoverageStatus::NotPresent
    }
}

fn state_source_from_session(
    project: &Hoi4Project,
    session: &StateEditSession,
    state_id: u32,
    generation: SourceGeneration,
) -> Arc<ProvinceReferenceSource> {
    Arc::new(match session.state_origin(state_id) {
        Some(WorkingStateOrigin::Loaded { document_path }) => {
            ProvinceReferenceSource::StateSession {
                state_id,
                document_path: Some(project.paths.states_directory.join(document_path)),
                generation,
            }
        }
        Some(WorkingStateOrigin::CreatedInSession) | None => {
            ProvinceReferenceSource::StateSession {
                state_id,
                document_path: None,
                generation,
            }
        }
    })
}

fn add_state_references(
    references_by_province: &mut BTreeMap<u32, Vec<ProvinceReference>>,
    state_id: u32,
    data: &StateData,
    source: Arc<ProvinceReferenceSource>,
) {
    for &province_id in &data.provinces {
        push(
            references_by_province,
            province_id,
            ProvinceReference::StateMembership {
                state_id,
                source: source.clone(),
            },
        );
    }
    for victory_point in &data.history.victory_points {
        push(
            references_by_province,
            victory_point.province_id,
            ProvinceReference::VictoryPoint {
                state_id,
                value: victory_point.value,
                source: source.clone(),
            },
        );
    }
    for (&province_id, buildings) in &data.history.province_buildings {
        for (building_type, &value) in buildings {
            push(
                references_by_province,
                province_id,
                ProvinceReference::Building {
                    state_id,
                    building_type: building_type.clone(),
                    value,
                    source: source.clone(),
                },
            );
        }
    }
}

fn push(
    references_by_province: &mut BTreeMap<u32, Vec<ProvinceReference>>,
    province_id: u32,
    reference: ProvinceReference,
) {
    references_by_province
        .entry(province_id)
        .or_default()
        .push(reference);
}

fn reference_order(left: &ProvinceReference, right: &ProvinceReference) -> std::cmp::Ordering {
    use ProvinceReference::*;
    let key = |reference: &ProvinceReference| match reference {
        StateMembership { state_id, .. } => (0, *state_id as u64, 0, String::new()),
        VictoryPoint {
            state_id, value, ..
        } => (1, *state_id as u64, *value as u64, String::new()),
        Building {
            state_id,
            building_type,
            value,
            ..
        } => (2, *state_id as u64, *value as u64, building_type.clone()),
        Adjacency { row, role, .. } => (3, *row as u64, *role as u64, String::new()),
        Railway {
            railway_index,
            sequence_ordinal,
            ..
        } => (
            4,
            *railway_index as u64,
            *sequence_ordinal as u64,
            String::new(),
        ),
        SupplyNode { node_index, .. } => (5, *node_index as u64, 0, String::new()),
        StrategicRegion { region_id, .. } => (6, *region_id as u64, 0, String::new()),
    };
    key(left).cmp(&key(right))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::app::format::{Adjacency, AdjacencyKind, Definition, DefinitionKind};
    use crate::app::map::{ProvinceKind, construct_map_data_for_sparse_tests};
    use crate::app::project::{
        LogisticsInput, LogisticsLoadIssue, LogisticsLoadResult, ProjectPaths, Railway,
        RailwayLoadResult, ResolvedLocation, ResolvedSource, SourceGeneration, SourceKind,
        StateLoadFailure, StateLoadFailureStage, StrategicRegion, StrategicRegionCoverage,
        StrategicRegionLoadResult, SupplyNode, SupplyNodeLoadResult,
    };
    use crate::app::state::{StateData, StateDocument, StateHistory, VictoryPoint, parse_text};
    use crate::config::Config;

    use super::*;

    static NEXT_TEMP_PROJECT: AtomicUsize = AtomicUsize::new(0);

    struct TempProject(PathBuf);

    impl TempProject {
        fn new() -> Self {
            let serial = NEXT_TEMP_PROJECT.fetch_add(1, Ordering::Relaxed);
            let root = env::temp_dir().join(format!(
                "hoi4-state-editor-reference-index-{}-{serial}",
                std::process::id()
            ));
            fs::create_dir_all(root.join("map")).unwrap();
            fs::create_dir_all(root.join("history/states")).unwrap();
            fs::write(root.join("map/provinces.bmp"), []).unwrap();
            fs::write(root.join("map/definition.csv"), []).unwrap();
            fs::write(root.join("map/adjacencies.csv"), []).unwrap();
            Self(root)
        }

        fn project(&self) -> Hoi4Project {
            Hoi4Project::new(ProjectPaths::discover(&self.0).unwrap())
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn source(path: &str, generation: SourceGeneration) -> ResolvedSource {
        ResolvedSource {
            logical_path: PathBuf::from(path),
            location: ResolvedLocation::Filesystem(PathBuf::from(path)),
            source_kind: SourceKind::CurrentProject,
            project_generation: generation,
        }
    }

    fn document(path: &str, data: StateData) -> StateDocument {
        StateDocument {
            path: PathBuf::from(path),
            original_bytes: Arc::from([]),
            exact_utf8: true,
            syntax: parse_text(path, ""),
            data: Some(data),
            diagnostics: Vec::new(),
            modified: false,
        }
    }

    fn state(id: u32, provinces: &[u32]) -> StateData {
        StateData {
            id: Some(id),
            provinces: BTreeSet::from_iter(provinces.iter().copied()),
            ..Default::default()
        }
    }

    fn sparse_map(adjacencies: Vec<Adjacency>) -> Map {
        let colors = [[10, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]];
        let ids = [1, 7, 42, 500];
        construct_map_data_for_sparse_tests(
            image::RgbImage::from_fn(4, 1, |x, _| image::Rgb(colors[x as usize])),
            colors
                .into_iter()
                .zip(ids)
                .map(|(rgb, id)| Definition {
                    id,
                    rgb,
                    kind: DefinitionKind::Land,
                    coastal: false,
                    terrain: "plains".to_owned(),
                    continent: 1,
                })
                .collect(),
            adjacencies,
            None,
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .unwrap()
        .map
    }

    fn adjacency(from_id: u32, to_id: u32, through: Option<u32>) -> Adjacency {
        Adjacency {
            from_id,
            to_id,
            kind: AdjacencyKind::Sea,
            through,
            start: None,
            stop: None,
            rule_name: String::new(),
            comment: String::new(),
        }
    }

    fn populated_project(temp: &TempProject) -> Hoi4Project {
        let mut first = state(10, &[1]);
        first.history = StateHistory {
            victory_points: vec![VictoryPoint {
                province_id: 1,
                value: 5,
            }],
            province_buildings: BTreeMap::from([(1, BTreeMap::from([("bunker".to_owned(), 2)]))]),
            ..Default::default()
        };
        let second = state(20, &[7]);
        let mut project = temp.project();
        project.states = vec![
            document("10-test.txt", first),
            document("20-test.txt", second),
        ];
        project.states_by_id = BTreeMap::from([(10, 0), (20, 1)]);
        project.state_by_province = [(1, 10), (7, 20)].into_iter().collect();
        let generation = project.paths.sources.manifest().project_generation;
        let railway_source = source("map/railways.txt", generation);
        let supply_source = source("map/supply_nodes.txt", generation);
        project.logistics = LogisticsLoadResult {
            railways: RailwayLoadResult {
                source: Some(railway_source.clone()),
                railways: vec![Railway {
                    source: railway_source,
                    line_number: 1,
                    level: 1,
                    declared_province_count: 3,
                    provinces: vec![1, 7, 42],
                }],
                issues: Vec::new(),
            },
            supply_nodes: SupplyNodeLoadResult {
                source: Some(supply_source.clone()),
                supply_nodes: vec![SupplyNode {
                    source: supply_source,
                    line_number: 1,
                    level: 1,
                    province_id: 500,
                }],
                issues: Vec::new(),
            },
        };
        project
    }

    #[test]
    fn indexes_sparse_cross_domain_references_in_deterministic_order() {
        let temp = TempProject::new();
        let project = populated_project(&temp);
        let map = sparse_map(vec![adjacency(1, 7, Some(42))]);

        let built = build_province_reference_index(&project, &map, None);
        let domains = built
            .index
            .references_to_province(1)
            .iter()
            .map(ProvinceReference::domain)
            .collect::<Vec<_>>();
        assert_eq!(
            domains,
            vec![
                ProvinceReferenceDomain::StateMembership,
                ProvinceReferenceDomain::VictoryPoint,
                ProvinceReferenceDomain::Building,
                ProvinceReferenceDomain::Adjacency,
                ProvinceReferenceDomain::Railway,
            ]
        );
        assert!(matches!(
            built.index.references_to_province(42),
            [
                ProvinceReference::Adjacency {
                    role: AdjacencyReferenceRole::Through,
                    ..
                },
                ProvinceReference::Railway {
                    sequence_ordinal: 2,
                    ..
                }
            ]
        ));
        assert!(matches!(
            built.index.references_to_province(500),
            [ProvinceReference::SupplyNode { node_index: 0, .. }]
        ));
        assert_eq!(built.index.reference_count(), 11);
        assert_eq!(
            built.index.referenced_province_ids().collect::<Vec<_>>(),
            vec![1, 7, 42, 500]
        );
        assert_eq!(
            built.index.summary_for_province(1),
            ProvinceReferenceSummary {
                states: 1,
                victory_points: 1,
                buildings: 1,
                adjacencies: 1,
                railways: 1,
                supply_nodes: 0,
                strategic_regions: 0,
            }
        );
        assert!(built.coverage.is_complete());
    }

    #[test]
    fn active_state_session_replaces_membership_and_includes_unsaved_province() {
        let temp = TempProject::new();
        let project = populated_project(&temp);
        let map = sparse_map(vec![adjacency(1, 7, Some(42))]);
        let mut session = StateEditSession::new(&project, &map);
        session.reassign_provinces(&[1], Some(20)).unwrap();
        session.register_created_province(999, ProvinceKind::Land, Some(10));

        let built = build_province_reference_index(&project, &map, Some(&session));
        assert!(matches!(
            built.index.references_to_province(1).first(),
            Some(ProvinceReference::StateMembership { state_id: 20, source }) if matches!(source.as_ref(), ProvinceReferenceSource::StateSession { document_path: Some(_), .. })
        ));
        assert!(matches!(
            built.index.references_to_province(999),
            [ProvinceReference::StateMembership { state_id: 10, source }]
                if matches!(source.as_ref(), ProvinceReferenceSource::StateSession { document_path: Some(_), .. })
        ));
    }

    #[test]
    fn indexes_strategic_regions_from_the_existing_loaded_result() {
        let temp = TempProject::new();
        let mut project = temp.project();
        let source = Arc::new(source(
            "map/strategicregions/500.txt",
            SourceGeneration::default(),
        ));
        project.strategic_regions = StrategicRegionLoadResult {
            regions: vec![StrategicRegion {
                id: 500,
                name_key: Some("STRATEGICREGION_500".to_owned()),
                display_name: None,
                has_provinces_field: true,
                provinces: vec![42],
                naval_terrain: None,
                source,
                span: Default::default(),
            }],
            issues: Vec::new(),
            coverage: StrategicRegionCoverage::Complete,
            files_seen: 1,
            province_references: 1,
            loading_ms: 0,
        };
        let built = build_province_reference_index(&project, &sparse_map(Vec::new()), None);
        assert!(matches!(
            built.index.references_to_province(42),
            [ProvinceReference::StrategicRegion { region_id: 500, .. }]
        ));
        assert_eq!(
            built.coverage.status(ReferenceDomain::StrategicRegions),
            &ReferenceCoverageStatus::Complete
        );
    }

    #[test]
    fn exposes_partial_coverage_and_rejects_stale_generation_access() {
        let temp = TempProject::new();
        let absent_domains =
            build_province_reference_index(&temp.project(), &sparse_map(Vec::new()), None);
        assert_eq!(
            absent_domains.coverage.status(ReferenceDomain::Railways),
            &ReferenceCoverageStatus::NotPresent
        );
        assert_eq!(
            absent_domains.coverage.status(ReferenceDomain::SupplyNodes),
            &ReferenceCoverageStatus::NotPresent
        );
        let mut project = populated_project(&temp);
        project
            .load_summary
            .report
            .files_failed
            .push(StateLoadFailure {
                path: Path::new("bad-state.txt").to_owned(),
                stage: StateLoadFailureStage::Parse,
                reason: "fixture".to_owned(),
            });
        project.logistics.railways.issues.push(LogisticsLoadIssue {
            input: LogisticsInput::Railway,
            source: None,
            line_number: None,
            span: None,
            message: "fixture".to_owned(),
        });
        let map = sparse_map(Vec::new());
        let built = build_province_reference_index(&project, &map, None);
        assert_eq!(
            built.coverage.status(ReferenceDomain::States),
            &ReferenceCoverageStatus::Incomplete {
                unavailable_inputs: 1
            }
        );
        assert_eq!(
            built.coverage.status(ReferenceDomain::Railways),
            &ReferenceCoverageStatus::Incomplete {
                unavailable_inputs: 1
            }
        );
        assert!(!built.coverage.is_complete());
        assert!(
            built
                .index
                .references_to_province_for_generation(SourceGeneration::new(99), 1)
                .is_none()
        );
    }

    #[test]
    fn project_switch_keeps_reference_indexes_generation_isolated() {
        let first_temp = TempProject::new();
        let mut first = populated_project(&first_temp);
        first.bind_project_generation(41);
        let map = sparse_map(Vec::new());
        let first_index = build_province_reference_index(&first, &map, None).index;

        let second_temp = TempProject::new();
        let mut second = second_temp.project();
        second.states = vec![document("30-test.txt", state(30, &[500]))];
        second.states_by_id = BTreeMap::from([(30, 0)]);
        second.state_by_province = [(500, 30)].into_iter().collect();
        second.bind_project_generation(42);
        let second_index = build_province_reference_index(&second, &map, None).index;

        assert_eq!(first_index.generation(), SourceGeneration::new(41));
        assert_eq!(second_index.generation(), SourceGeneration::new(42));
        assert!(first_index.has_references(1));
        assert!(!second_index.has_references(1));
        assert!(
            second_index
                .references_to_province_for_generation(SourceGeneration::new(41), 500)
                .is_none()
        );
    }
}
