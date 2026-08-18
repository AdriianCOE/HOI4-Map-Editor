use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::app::project::{DiagnosticSeverity, ProjectDiagnostic, ProjectDiagnosticKind};
use crate::app::state::{StateData, StateDocument};

#[derive(Debug, Clone, Default)]
pub struct StateIndexes {
    pub states_by_id: BTreeMap<u32, usize>,
    pub state_by_province: HashMap<u32, u32>,
    pub ambiguous_provinces: BTreeMap<u32, Vec<u32>>,
    pub unassigned_land_provinces: BTreeSet<u32>,
    pub diagnostics: Vec<ProjectDiagnostic>,
    pub document_count: usize,
    pub parsed_state_count: usize,
    pub indexed_state_count: usize,
    pub indexed_province_count: usize,
    pub ambiguous_province_count: usize,
    pub land_without_state_count: usize,
}

pub fn index_state_documents(
    documents: &[StateDocument],
    valid_province_ids: &BTreeSet<u32>,
    land_province_ids: &BTreeSet<u32>,
) -> StateIndexes {
    let mut indexes = StateIndexes {
        document_count: documents.len(),
        ..Default::default()
    };
    let mut province_states = BTreeMap::<u32, BTreeSet<u32>>::new();

    for (document_index, document) in documents.iter().enumerate() {
        indexes
            .diagnostics
            .extend(document.diagnostics.iter().cloned());

        let Some(data) = &document.data else {
            continue;
        };
        indexes.parsed_state_count += 1;

        let Some(state_id) = data.id else {
            indexes.diagnostics.push(diagnostic(
                ProjectDiagnosticKind::MissingStateId,
                DiagnosticSeverity::Error,
                document,
                "state is missing an id",
            ));
            continue;
        };

        if state_id == 0 {
            indexes.diagnostics.push(diagnostic(
                ProjectDiagnosticKind::ZeroStateId,
                DiagnosticSeverity::Error,
                document,
                "state id must not be 0",
            ));
            continue;
        }

        if let Some(first_document_index) = indexes.states_by_id.get(&state_id).copied() {
            let mut duplicate = diagnostic(
                ProjectDiagnosticKind::DuplicateStateId,
                DiagnosticSeverity::Error,
                document,
                format!("state id {state_id} is already used"),
            );
            duplicate.related_path = documents
                .get(first_document_index)
                .map(|first| first.path.clone());
            indexes.diagnostics.push(duplicate);
            continue;
        }

        indexes.states_by_id.insert(state_id, document_index);
        indexes.indexed_state_count += 1;
        index_provinces(
            &mut indexes,
            &mut province_states,
            document,
            data,
            state_id,
            valid_province_ids,
        );
    }

    indexes.ambiguous_provinces = province_states
        .into_iter()
        .filter_map(|(province_id, states)| {
            if states.len() > 1 {
                Some((province_id, states.into_iter().collect()))
            } else {
                None
            }
        })
        .collect();
    indexes.ambiguous_province_count = indexes.ambiguous_provinces.len();
    indexes.indexed_province_count = indexes.state_by_province.len();
    warn_land_without_state(&mut indexes, valid_province_ids, land_province_ids);
    indexes
}

fn index_provinces(
    indexes: &mut StateIndexes,
    province_states: &mut BTreeMap<u32, BTreeSet<u32>>,
    document: &StateDocument,
    data: &StateData,
    state_id: u32,
    valid_province_ids: &BTreeSet<u32>,
) {
    for &province_id in &data.provinces {
        if !valid_state_province(indexes, document, state_id, province_id, valid_province_ids) {
            continue;
        }

        province_states
            .entry(province_id)
            .or_default()
            .insert(state_id);
        indexes
            .state_by_province
            .entry(province_id)
            .or_insert(state_id);
        if indexes.state_by_province.get(&province_id) != Some(&state_id) {
            indexes.diagnostics.push(diagnostic(
                ProjectDiagnosticKind::ProvinceInMultipleStates,
                DiagnosticSeverity::Error,
                document,
                format!("province {province_id} is assigned to multiple states"),
            ));
        }
    }
}

fn valid_state_province(
    indexes: &mut StateIndexes,
    document: &StateDocument,
    state_id: u32,
    province_id: u32,
    valid_province_ids: &BTreeSet<u32>,
) -> bool {
    if province_id == 0 {
        indexes.diagnostics.push(diagnostic(
            ProjectDiagnosticKind::UnknownProvince,
            DiagnosticSeverity::Error,
            document,
            format!("state {state_id} references province 0"),
        ));
        return false;
    }

    if !valid_province_ids.contains(&province_id) {
        indexes.diagnostics.push(diagnostic(
            ProjectDiagnosticKind::UnknownProvince,
            DiagnosticSeverity::Error,
            document,
            format!("state {state_id} references unknown province {province_id}"),
        ));
        return false;
    }

    true
}

fn warn_land_without_state(
    indexes: &mut StateIndexes,
    valid_province_ids: &BTreeSet<u32>,
    land_province_ids: &BTreeSet<u32>,
) {
    for &province_id in land_province_ids {
        if valid_province_ids.contains(&province_id)
            && !indexes.state_by_province.contains_key(&province_id)
        {
            indexes.unassigned_land_provinces.insert(province_id);
            indexes.land_without_state_count += 1;
            indexes.diagnostics.push(ProjectDiagnostic::new(
                ProjectDiagnosticKind::LandProvinceWithoutState,
                DiagnosticSeverity::Warning,
                None,
                None,
                format!("land province {province_id} is not assigned to any state"),
            ));
        }
    }
}

fn diagnostic(
    kind: ProjectDiagnosticKind,
    severity: DiagnosticSeverity,
    document: &StateDocument,
    message: impl Into<String>,
) -> ProjectDiagnostic {
    ProjectDiagnostic::new(kind, severity, Some(document.path.clone()), None, message)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::{StateIndexes, index_state_documents};
    use crate::app::project::{DiagnosticSeverity, ProjectDiagnosticKind};
    use crate::app::state::{StateData, StateDocument, parse_text};

    fn document(path: &str, data: StateData) -> StateDocument {
        StateDocument {
            path: PathBuf::from(path),
            original_bytes: Vec::new().into(),
            exact_utf8: true,
            syntax: parse_text(path, ""),
            data: Some(data),
            diagnostics: Vec::new(),
            modified: false,
        }
    }

    fn state(id: u32, provinces: &[u32]) -> StateData {
        let mut data = StateData {
            id: Some(id),
            ..Default::default()
        };
        data.provinces.extend(provinces.iter().copied());
        data
    }

    fn ids(values: &[u32]) -> BTreeSet<u32> {
        values.iter().copied().collect()
    }

    fn kinds(indexes: &StateIndexes) -> Vec<ProjectDiagnosticKind> {
        indexes
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind)
            .collect()
    }

    #[test]
    fn first_valid_state_id_wins_and_duplicate_is_not_indexed() {
        let documents = vec![
            document("first.txt", state(1, &[10])),
            document("duplicate.txt", state(1, &[11])),
            document("second.txt", state(2, &[12])),
        ];

        let indexes = index_state_documents(&documents, &ids(&[10, 11, 12]), &BTreeSet::new());

        assert_eq!(indexes.states_by_id.get(&1), Some(&0));
        assert_eq!(indexes.states_by_id.get(&2), Some(&2));
        assert_eq!(indexes.state_by_province.get(&10), Some(&1));
        assert_eq!(indexes.state_by_province.get(&11), None);
        assert_eq!(indexes.state_by_province.get(&12), Some(&2));
        assert!(kinds(&indexes).contains(&ProjectDiagnosticKind::DuplicateStateId));
    }

    #[test]
    fn province_conflicts_keep_first_state_and_record_all_ambiguous_states() {
        let documents = vec![
            document("a.txt", state(1, &[10, 11])),
            document("b.txt", state(2, &[10])),
            document("c.txt", state(3, &[10])),
        ];

        let indexes = index_state_documents(&documents, &ids(&[10, 11]), &BTreeSet::new());

        assert_eq!(indexes.state_by_province.get(&10), Some(&1));
        assert_eq!(indexes.ambiguous_provinces.get(&10), Some(&vec![1, 2, 3]));
        assert_eq!(
            indexes
                .diagnostics
                .iter()
                .filter(
                    |diagnostic| diagnostic.kind == ProjectDiagnosticKind::ProvinceInMultipleStates
                )
                .count(),
            2
        );
    }

    #[test]
    fn sparse_province_values_are_valid_state_keys_without_dense_indexes() {
        let documents = vec![document("sparse.txt", state(1, &[1, 7, 42, 500]))];
        let valid = ids(&[1, 7, 42, 500]);
        let indexes = index_state_documents(&documents, &valid, &valid);

        for province_id in [1, 7, 42, 500] {
            assert_eq!(indexes.state_by_province.get(&province_id), Some(&1));
        }
        assert!(indexes.unassigned_land_provinces.is_empty());
        assert!(!kinds(&indexes).contains(&ProjectDiagnosticKind::UnknownProvince));
    }

    #[test]
    fn unknown_and_zero_provinces_are_diagnosed_and_excluded() {
        let documents = vec![document("state.txt", state(1, &[0, 10, 99]))];

        let indexes = index_state_documents(&documents, &ids(&[10]), &BTreeSet::new());

        assert_eq!(indexes.state_by_province.get(&10), Some(&1));
        assert_eq!(indexes.state_by_province.get(&0), None);
        assert_eq!(indexes.state_by_province.get(&99), None);
        assert_eq!(
            indexes
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.kind == ProjectDiagnosticKind::UnknownProvince)
                .count(),
            2
        );
    }

    #[test]
    fn land_without_state_is_warned_only_for_known_land() {
        let documents = vec![document("state.txt", state(1, &[10]))];

        let indexes = index_state_documents(&documents, &ids(&[10, 11]), &ids(&[10, 11, 99]));

        assert_eq!(indexes.land_without_state_count, 1);
        assert_eq!(indexes.unassigned_land_provinces, BTreeSet::from([11]));
        assert!(indexes.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == ProjectDiagnosticKind::LandProvinceWithoutState
                && diagnostic.severity == DiagnosticSeverity::Warning
                && diagnostic.message.contains("11")
        }));
    }
}
