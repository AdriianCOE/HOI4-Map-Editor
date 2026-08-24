//! Read-only Strategic Regions inspector presentation and requests.
//!
//! This controller only transforms the project-owned load result into stable
//! rows and explicit requests. It intentionally has no Canvas, App, camera,
//! selection, filesystem, or save dependency.

use std::path::PathBuf;

use crate::app::project::{
    ResolvedLocation, SourceGeneration, SourceKind, StrategicRegion, StrategicRegionCoverage,
    StrategicRegionLoadResult,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StrategicRegionsController {
    pub(crate) visible: bool,
    pub(crate) search: String,
    generation: Option<SourceGeneration>,
    pub(crate) list_offset: usize,
    cached_rows: Vec<StrategicRegionRowModel>,
    cached_rows_key: Option<(SourceGeneration, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StrategicRegionRowModel {
    pub(crate) id: u32,
    pub(crate) name: String,
    pub(crate) name_key: Option<String>,
    pub(crate) province_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StrategicRegionMemberModel {
    pub(crate) id: u32,
    pub(crate) navigable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StrategicRegionDetailModel {
    pub(crate) id: u32,
    pub(crate) display_name: String,
    pub(crate) name_key: Option<String>,
    pub(crate) provinces: Vec<StrategicRegionMemberModel>,
    pub(crate) naval_terrain: Option<String>,
    pub(crate) logical_path: String,
    pub(crate) provenance: String,
    pub(crate) source_display_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StrategicRegionsCoveragePresentation {
    NotPresent,
    Complete {
        regions: usize,
    },
    Incomplete {
        regions: usize,
        files_visible: usize,
        files_loaded: usize,
        files_failed: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StrategicRegionsPresentation {
    pub(crate) coverage: StrategicRegionsCoveragePresentation,
    pub(crate) rows: Vec<StrategicRegionRowModel>,
    pub(crate) detail: Option<StrategicRegionDetailModel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StrategicRegionsRequest {
    SelectStrategicRegion(u32),
    NavigateProvince(u32),
    OpenSource(PathBuf),
    RevealSource(PathBuf),
    CopySource(String),
}

impl StrategicRegionsController {
    pub(crate) fn open(&mut self, generation: SourceGeneration) {
        self.ensure_generation(generation);
        self.visible = true;
    }

    pub(crate) fn close(&mut self) {
        self.visible = false;
    }

    pub(crate) fn focus(&mut self, generation: SourceGeneration) -> bool {
        self.ensure_generation(generation);
        self.visible = true;
        self.list_offset = 0;
        true
    }

    pub(crate) fn reset_for_generation(&mut self, generation: SourceGeneration) {
        if self.generation != Some(generation) {
            self.search.clear();
            self.list_offset = 0;
            self.generation = Some(generation);
            self.cached_rows.clear();
            self.cached_rows_key = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn set_search(&mut self, search: String) {
        self.search = search;
        self.list_offset = 0;
    }

    pub(crate) fn append_search(&mut self, text: &str) {
        self.search
            .extend(text.chars().filter(|character| !character.is_control()));
        self.list_offset = 0;
    }

    pub(crate) fn backspace_search(&mut self) {
        self.search.pop();
        self.list_offset = 0;
    }

    pub(crate) fn presentation(
        &mut self,
        generation: SourceGeneration,
        loaded: &StrategicRegionLoadResult,
        selected_id: Option<u32>,
        province_exists: impl Fn(u32) -> bool,
    ) -> StrategicRegionsPresentation {
        self.reset_for_generation(generation);
        let key = (generation, self.search.clone());
        if self.cached_rows_key.as_ref() != Some(&key) {
            self.cached_rows = filtered_rows(loaded, &self.search);
            self.cached_rows_key = Some(key);
        }
        let detail = selected_id.and_then(|id| {
            loaded
                .regions
                .iter()
                .find(|region| region.id == id)
                .map(|region| detail_model(region, &province_exists))
        });
        StrategicRegionsPresentation {
            coverage: coverage_presentation(loaded),
            rows: self.cached_rows.clone(),
            detail,
        }
    }

    pub(crate) fn member_request(
        &self,
        _detail: &StrategicRegionDetailModel,
        member: &StrategicRegionMemberModel,
    ) -> Option<StrategicRegionsRequest> {
        (member.navigable).then_some(StrategicRegionsRequest::NavigateProvince(member.id))
    }

    pub(crate) const fn selection_request(id: u32) -> StrategicRegionsRequest {
        StrategicRegionsRequest::SelectStrategicRegion(id)
    }

    pub(crate) fn source_requests(&self, region: &StrategicRegion) -> Vec<StrategicRegionsRequest> {
        match &region.source.location {
            ResolvedLocation::Filesystem(path) => vec![
                StrategicRegionsRequest::OpenSource(path.clone()),
                StrategicRegionsRequest::RevealSource(path.parent().unwrap_or(path).to_path_buf()),
                StrategicRegionsRequest::CopySource(path.display().to_string()),
            ],
            ResolvedLocation::ArchiveEntry {
                archive_path,
                entry_path,
            } => vec![
                StrategicRegionsRequest::RevealSource(archive_path.clone()),
                StrategicRegionsRequest::CopySource(format!(
                    "{} :: {}",
                    archive_path.display(),
                    entry_path.display()
                )),
            ],
        }
    }

    fn ensure_generation(&mut self, generation: SourceGeneration) {
        self.reset_for_generation(generation);
    }
}

fn filtered_rows(loaded: &StrategicRegionLoadResult, query: &str) -> Vec<StrategicRegionRowModel> {
    let needle = query.trim().to_lowercase();
    let mut rows = loaded
        .regions
        .iter()
        .filter(|region| region_matches(region, &needle))
        .map(|region| StrategicRegionRowModel {
            id: region.id,
            name: region_name(region),
            name_key: region.name_key.clone(),
            province_count: region.provinces.len(),
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.id);
    rows
}

fn region_matches(region: &StrategicRegion, needle: &str) -> bool {
    needle.is_empty()
        || region.id.to_string().contains(needle)
        || region_name(region).to_lowercase().contains(needle)
        || region
            .name_key
            .as_deref()
            .is_some_and(|key| key.to_lowercase().contains(needle))
        || region
            .provinces
            .iter()
            .any(|province| province.to_string().contains(needle))
}

fn region_name(region: &StrategicRegion) -> String {
    region
        .display_name
        .clone()
        .or_else(|| region.name_key.clone())
        .unwrap_or_else(|| "<unnamed>".to_owned())
}

fn detail_model(
    region: &StrategicRegion,
    province_exists: &impl Fn(u32) -> bool,
) -> StrategicRegionDetailModel {
    let source_display_path = match &region.source.location {
        ResolvedLocation::Filesystem(path) => path.display().to_string(),
        ResolvedLocation::ArchiveEntry {
            archive_path,
            entry_path,
        } => format!("{} :: {}", archive_path.display(), entry_path.display()),
    };
    StrategicRegionDetailModel {
        id: region.id,
        display_name: region_name(region),
        name_key: region.name_key.clone(),
        provinces: region
            .provinces
            .iter()
            .copied()
            .map(|id| StrategicRegionMemberModel {
                id,
                navigable: province_exists(id),
            })
            .collect(),
        naval_terrain: region.naval_terrain.clone(),
        logical_path: region.source.logical_path.display().to_string(),
        provenance: source_kind_label(region.source.source_kind).to_owned(),
        source_display_path,
    }
}

fn source_kind_label(source_kind: SourceKind) -> &'static str {
    match source_kind {
        SourceKind::CurrentProject => "Current Project",
        SourceKind::Dlc => "DLC",
        SourceKind::IntegratedDlc => "Integrated DLC",
        SourceKind::BaseGame => "Base Game",
    }
}

fn coverage_presentation(
    loaded: &StrategicRegionLoadResult,
) -> StrategicRegionsCoveragePresentation {
    match loaded.coverage {
        StrategicRegionCoverage::NotPresent => StrategicRegionsCoveragePresentation::NotPresent,
        StrategicRegionCoverage::Complete => StrategicRegionsCoveragePresentation::Complete {
            regions: loaded.regions.len(),
        },
        StrategicRegionCoverage::Incomplete {
            files_visible,
            files_failed,
        } => StrategicRegionsCoveragePresentation::Incomplete {
            regions: loaded.regions.len(),
            files_visible,
            files_loaded: files_visible.saturating_sub(files_failed),
            files_failed,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use super::*;
    use crate::app::project::{
        ResolvedSource, SourceKind, StrategicRegionCoverage, StrategicRegionLoadResult,
    };
    use crate::app::state::TextSpan;

    fn region(
        id: u32,
        name_key: &str,
        display_name: Option<&str>,
        provinces: Vec<u32>,
    ) -> StrategicRegion {
        StrategicRegion {
            id,
            name_key: Some(name_key.to_owned()),
            display_name: display_name.map(str::to_owned),
            has_provinces_field: true,
            provinces,
            naval_terrain: Some("ocean".to_owned()),
            source: Arc::new(ResolvedSource {
                logical_path: format!("map/strategicregions/{id}.txt").into(),
                location: ResolvedLocation::Filesystem(PathBuf::from(format!(
                    "C:/fixture/{id}.txt"
                ))),
                source_kind: SourceKind::CurrentProject,
                project_generation: SourceGeneration::new(1),
            }),
            span: TextSpan { start: 0, len: 1 },
        }
    }

    fn loaded(regions: Vec<StrategicRegion>) -> StrategicRegionLoadResult {
        StrategicRegionLoadResult {
            regions,
            coverage: StrategicRegionCoverage::Complete,
            ..StrategicRegionLoadResult::default()
        }
    }

    #[test]
    fn list_is_id_sorted_and_searches_id_localized_raw_and_province() {
        let data = loaded(vec![
            region(500, "STRATEGICREGION_500", Some("Mediterranean"), vec![42]),
            region(1, "RAW_ONE", Some("Alps"), vec![3]),
            region(42, "RAW_FORTY_TWO", None, vec![9]),
            region(7, "RAW_SEVEN", Some("Baltic"), vec![1]),
        ]);
        let mut controller = StrategicRegionsController::default();
        let valid = BTreeSet::from([1, 3, 9, 42]);
        assert_eq!(
            controller
                .presentation(SourceGeneration::new(1), &data, None, |id| valid
                    .contains(&id))
                .rows
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>(),
            [1, 7, 42, 500]
        );
        controller.set_search("500".to_owned());
        assert_eq!(
            controller
                .presentation(SourceGeneration::new(1), &data, None, |id| valid
                    .contains(&id))
                .rows[0]
                .id,
            500
        );
        controller.set_search("mediterranean".to_owned());
        assert_eq!(
            controller
                .presentation(SourceGeneration::new(1), &data, None, |id| valid
                    .contains(&id))
                .rows[0]
                .id,
            500
        );
        controller.set_search("strategicregion_500".to_owned());
        assert_eq!(
            controller
                .presentation(SourceGeneration::new(1), &data, None, |id| valid
                    .contains(&id))
                .rows[0]
                .id,
            500
        );
        controller.set_search("42".to_owned());
        assert_eq!(
            controller
                .presentation(SourceGeneration::new(1), &data, None, |id| valid
                    .contains(&id))
                .rows
                .len(),
            2
        );
    }

    #[test]
    fn detail_keeps_raw_key_missing_members_and_navigation_is_typed() {
        let data = loaded(vec![region(7, "RAW_SEVEN", Some("Baltic"), vec![42, 9999])]);
        let mut controller = StrategicRegionsController::default();
        controller.focus(SourceGeneration::new(1));
        let view = controller.presentation(SourceGeneration::new(1), &data, Some(7), |id| id == 42);
        let detail = view.detail.unwrap();
        assert_eq!(detail.display_name, "Baltic");
        assert_eq!(detail.name_key.as_deref(), Some("RAW_SEVEN"));
        assert_eq!(detail.provinces[1].id, 9999);
        assert!(!detail.provinces[1].navigable);
        assert_eq!(
            controller.member_request(&detail, &detail.provinces[0]),
            Some(StrategicRegionsRequest::NavigateProvince(42))
        );
        assert_eq!(
            controller.member_request(&detail, &detail.provinces[1]),
            None
        );
    }

    #[test]
    fn missing_localization_falls_back_and_generation_reset_drops_selection() {
        let data = loaded(vec![region(50000, "MY_CUSTOM_REGION", None, vec![])]);
        let mut controller = StrategicRegionsController::default();
        controller.focus(SourceGeneration::new(1));
        let view = controller.presentation(SourceGeneration::new(1), &data, Some(50000), |_| false);
        assert_eq!(view.rows[0].name, "MY_CUSTOM_REGION");
        controller.set_search("custom".to_owned());
        assert_eq!(
            controller
                .presentation(SourceGeneration::new(1), &data, Some(50000), |_| false)
                .rows
                .len(),
            1
        );
        let empty = StrategicRegionLoadResult::default();
        let switched = controller.presentation(SourceGeneration::new(2), &empty, None, |_| false);
        assert!(switched.detail.is_none());
        assert!(controller.search.is_empty());
        assert_eq!(
            switched.coverage,
            StrategicRegionsCoveragePresentation::NotPresent
        );
    }

    #[test]
    fn source_actions_keep_archives_non_openable_and_coverage_is_honest() {
        let mut archive_region = region(1, "ONE", None, vec![]);
        Arc::make_mut(&mut archive_region.source).location = ResolvedLocation::ArchiveEntry {
            archive_path: PathBuf::from("dlc.zip"),
            entry_path: PathBuf::from("map/strategicregions/1.txt"),
        };
        let controller = StrategicRegionsController::default();
        let requests = controller.source_requests(&archive_region);
        assert!(
            !requests
                .iter()
                .any(|request| matches!(request, StrategicRegionsRequest::OpenSource(_)))
        );
        assert!(requests.iter().any(|request| matches!(request, StrategicRegionsRequest::RevealSource(path) if path == &PathBuf::from("dlc.zip"))));
        let partial = StrategicRegionLoadResult {
            regions: vec![archive_region],
            coverage: StrategicRegionCoverage::Incomplete {
                files_visible: 2,
                files_failed: 1,
            },
            ..StrategicRegionLoadResult::default()
        };
        assert!(matches!(
            coverage_presentation(&partial),
            StrategicRegionsCoveragePresentation::Incomplete { regions: 1, .. }
        ));
    }

    #[test]
    fn source_models_use_human_provenance_and_filesystem_actions() {
        let filesystem = region(7, "SEVEN", None, vec![]);
        let controller = StrategicRegionsController::default();
        let detail = detail_model(&filesystem, &|_| false);
        assert_eq!(detail.provenance, "Current Project");
        assert_eq!(
            controller.source_requests(&filesystem),
            vec![
                StrategicRegionsRequest::OpenSource(PathBuf::from("C:/fixture/7.txt")),
                StrategicRegionsRequest::RevealSource(PathBuf::from("C:/fixture")),
                StrategicRegionsRequest::CopySource("C:/fixture/7.txt".to_owned()),
            ]
        );
        assert!(matches!(
            coverage_presentation(&loaded(vec![filesystem])),
            StrategicRegionsCoveragePresentation::Complete { regions: 1 }
        ));
    }
}
