//! Read-only presentation derived from project validation diagnostics.
//!
//! Validators describe facts only. This module turns those facts into safe
//! navigation/source actions and a compact spatial overlay model.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::{DiagnosticSeverity, ProjectValidationDiagnostic, ResolvedLocation, SourceGeneration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticAction {
    GoToProvince(u32),
    GoToState(u32),
    GoToStrategicRegion(u32),
    GoToLocation([u32; 2]),
    OpenSource(PathBuf),
    RevealSource(PathBuf),
    CopySourcePath(String),
}

pub fn diagnostic_actions(
    diagnostic: &ProjectValidationDiagnostic,
    current_generation: SourceGeneration,
) -> Vec<DiagnosticAction> {
    if diagnostic
        .source
        .as_ref()
        .is_some_and(|source| source.project_generation != current_generation)
    {
        return Vec::new();
    }

    let mut actions = diagnostic
        .province_id
        .into_iter()
        .map(DiagnosticAction::GoToProvince)
        .collect::<Vec<_>>();
    actions.extend(
        diagnostic
            .related_province_ids
            .iter()
            .copied()
            .filter(|id| Some(*id) != diagnostic.province_id)
            .map(DiagnosticAction::GoToProvince),
    );
    actions.extend(diagnostic.state_id.map(DiagnosticAction::GoToState));
    actions.extend(
        diagnostic
            .strategic_region_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(DiagnosticAction::GoToStrategicRegion),
    );
    actions.extend(diagnostic.map_location.map(DiagnosticAction::GoToLocation));

    if let Some(source) = &diagnostic.source {
        match &source.location {
            ResolvedLocation::Filesystem(path) => {
                actions.push(DiagnosticAction::OpenSource(path.clone()));
                actions.push(DiagnosticAction::RevealSource(
                    path.parent().unwrap_or(path).to_path_buf(),
                ));
                actions.push(DiagnosticAction::CopySourcePath(path.display().to_string()));
            }
            ResolvedLocation::ArchiveEntry {
                archive_path,
                entry_path,
            } => {
                actions.push(DiagnosticAction::RevealSource(archive_path.clone()));
                actions.push(DiagnosticAction::CopySourcePath(format!(
                    "{} :: {}",
                    archive_path.display(),
                    entry_path.display()
                )));
            }
        }
    } else if let Some(path) = &diagnostic.path {
        actions.push(DiagnosticAction::OpenSource(path.clone()));
        actions.push(DiagnosticAction::RevealSource(
            path.parent().unwrap_or(path).to_path_buf(),
        ));
        actions.push(DiagnosticAction::CopySourcePath(path.display().to_string()));
    }
    actions
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemsOverlayMarker {
    pub location: [u32; 2],
    pub severity: DiagnosticSeverity,
    pub count: usize,
    pub province_ids: Vec<u32>,
    pub state_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProblemsOverlayModel {
    pub generation: SourceGeneration,
    pub revision: u64,
    pub markers: Vec<ProblemsOverlayMarker>,
}

pub fn build_overlay(
    generation: SourceGeneration,
    revision: u64,
    diagnostics: &[ProjectValidationDiagnostic],
    province_location: impl Fn(u32) -> Option<[u32; 2]>,
    state_location: impl Fn(u32) -> Option<[u32; 2]>,
) -> ProblemsOverlayModel {
    let mut markers = BTreeMap::<[u32; 2], ProblemsOverlayMarker>::new();
    for diagnostic in diagnostics {
        let location = diagnostic
            .map_location
            .or_else(|| diagnostic.province_id.and_then(&province_location))
            .or_else(|| diagnostic.state_id.and_then(&state_location));
        let Some(location) = location else { continue };
        let marker = markers
            .entry(location)
            .or_insert_with(|| ProblemsOverlayMarker {
                location,
                severity: diagnostic.severity,
                count: 0,
                province_ids: Vec::new(),
                state_ids: Vec::new(),
            });
        marker.count += 1;
        if severity_rank(diagnostic.severity) < severity_rank(marker.severity) {
            marker.severity = diagnostic.severity;
        }
        marker.province_ids.extend(diagnostic.province_id);
        marker
            .province_ids
            .extend(diagnostic.related_province_ids.iter().copied());
        marker.state_ids.extend(diagnostic.state_id);
    }
    for marker in markers.values_mut() {
        marker.province_ids.sort_unstable();
        marker.province_ids.dedup();
        marker.state_ids.sort_unstable();
        marker.state_ids.dedup();
    }
    ProblemsOverlayModel {
        generation,
        revision,
        markers: markers.into_values().collect(),
    }
}

fn severity_rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Info => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::project::{
        ProjectDiagnosticKind, ProjectValidationDomain, ResolvedSource, SourceKind,
    };
    use std::fs;

    fn diagnostic() -> ProjectValidationDiagnostic {
        ProjectValidationDiagnostic {
            kind: ProjectDiagnosticKind::MapXCrossing,
            severity: DiagnosticSeverity::Warning,
            domain: ProjectValidationDomain::Province,
            code: "MAP_X_CROSSING".to_owned(),
            message_key: "ignored".to_owned(),
            path: None,
            related_path: None,
            span: None,
            province_id: Some(7),
            state_id: Some(10),
            strategic_region_ids: Vec::new(),
            map_location: Some([7, 3]),
            related_province_ids: vec![1, 7, 42, 500],
            source: None,
            blocks_save: false,
            message: "ignored".to_owned(),
        }
    }

    #[test]
    fn actions_include_every_navigation_target_in_stable_order() {
        let actions = diagnostic_actions(&diagnostic(), SourceGeneration::new(1));
        assert_eq!(
            actions,
            vec![
                DiagnosticAction::GoToProvince(7),
                DiagnosticAction::GoToProvince(1),
                DiagnosticAction::GoToProvince(42),
                DiagnosticAction::GoToProvince(500),
                DiagnosticAction::GoToState(10),
                DiagnosticAction::GoToLocation([7, 3]),
            ]
        );
    }

    #[test]
    fn strategic_region_actions_are_sorted_and_preserve_state_navigation() {
        let mut value = diagnostic();
        value.strategic_region_ids = vec![500, 1, 42, 1];
        let actions = diagnostic_actions(&value, SourceGeneration::new(1));
        assert!(actions.contains(&DiagnosticAction::GoToState(10)));
        assert_eq!(
            actions
                .into_iter()
                .filter_map(|action| match action {
                    DiagnosticAction::GoToStrategicRegion(id) => Some(id),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [1, 42, 500]
        );
    }

    #[test]
    fn archive_sources_reveal_and_copy_without_fake_file_open() {
        let mut value = diagnostic();
        value.source = Some(ResolvedSource {
            logical_path: "map/rivers.bmp".into(),
            location: ResolvedLocation::ArchiveEntry {
                archive_path: PathBuf::from("dlc.zip"),
                entry_path: PathBuf::from("map/rivers.bmp"),
            },
            source_kind: SourceKind::Dlc,
            project_generation: SourceGeneration::new(1),
        });
        let actions = diagnostic_actions(&value, SourceGeneration::new(1));
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, DiagnosticAction::OpenSource(_)))
        );
        assert!(actions.iter().any(|action| matches!(action, DiagnosticAction::CopySourcePath(path) if path == "dlc.zip :: map/rivers.bmp")));
    }

    #[test]
    fn filesystem_sources_offer_safe_open_reveal_and_copy_targets() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-map-editor-problem-source-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.txt");
        fs::write(&path, "fixture").unwrap();
        let mut value = diagnostic();
        value.source = Some(ResolvedSource {
            logical_path: "history/states/state.txt".into(),
            location: ResolvedLocation::Filesystem(path.clone()),
            source_kind: SourceKind::CurrentProject,
            project_generation: SourceGeneration::new(1),
        });
        let actions = diagnostic_actions(&value, SourceGeneration::new(1));
        assert!(actions.contains(&DiagnosticAction::OpenSource(path.clone())));
        assert!(actions.contains(&DiagnosticAction::RevealSource(root.clone())));
        assert!(actions.contains(&DiagnosticAction::CopySourcePath(
            path.display().to_string()
        )));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_sources_expose_no_actions() {
        let mut value = diagnostic();
        value.source = Some(ResolvedSource {
            logical_path: "map/rivers.bmp".into(),
            location: ResolvedLocation::Filesystem(PathBuf::from("missing")),
            source_kind: SourceKind::CurrentProject,
            project_generation: SourceGeneration::new(1),
        });
        assert!(diagnostic_actions(&value, SourceGeneration::new(2)).is_empty());
    }

    #[test]
    fn overlay_aggregates_coordinates_and_keeps_dominant_severity() {
        let mut error = diagnostic();
        error.severity = DiagnosticSeverity::Error;
        let overlay = build_overlay(
            SourceGeneration::new(1),
            2,
            &[diagnostic(), error],
            |_| None,
            |_| None,
        );
        assert_eq!(overlay.revision, 2);
        assert_eq!(overlay.markers.len(), 1);
        assert_eq!(overlay.markers[0].location, [7, 3]);
        assert_eq!(overlay.markers[0].count, 2);
        assert_eq!(overlay.markers[0].severity, DiagnosticSeverity::Error);
        assert_eq!(overlay.markers[0].province_ids, [1, 7, 42, 500]);
    }

    #[test]
    fn overlay_uses_province_anchors_and_refreshes_without_old_markers() {
        let mut province_only = diagnostic();
        province_only.map_location = None;
        province_only.state_id = None;
        let first = build_overlay(
            SourceGeneration::new(1),
            1,
            &[province_only],
            |id| (id == 7).then_some([9, 4]),
            |_| None,
        );
        let refreshed = build_overlay(SourceGeneration::new(2), 2, &[], |_| None, |_| None);
        assert_eq!(first.markers[0].location, [9, 4]);
        assert!(refreshed.markers.is_empty());
        assert_ne!(first.generation, refreshed.generation);
        assert_eq!(refreshed.revision, 2);
    }
}
