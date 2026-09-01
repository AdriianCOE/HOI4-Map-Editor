use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    PatchPlanSummary, PatchPlanTimings, PatchSafety, PlannedFileCreation, PlannedFileModification,
    ProjectPatchPlan, SourceFingerprint,
};
use crate::app::map::ProvinceMapCandidate;
use crate::app::project::Hoi4Project;
use crate::app::project::validation::combined_candidate_digest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SaveDomain {
    ProvinceMap,
    States,
    StrategicRegions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectDirtyState {
    pub province_files: usize,
    pub state_files: usize,
    pub strategic_region_files: usize,
}

impl ProjectDirtyState {
    pub fn has_changes(self) -> bool {
        self.province_files != 0 || self.state_files != 0 || self.strategic_region_files != 0
    }
}

#[derive(Debug, Clone)]
pub struct ProjectSavePlan {
    patch_plan: ProjectPatchPlan,
    domains: BTreeSet<SaveDomain>,
    dirty: ProjectDirtyState,
    candidate_digest: SourceFingerprint,
    coastal_flags_recalculated: usize,
    strategic_region_revision: u64,
}

impl ProjectSavePlan {
    #[allow(dead_code)]
    pub(crate) fn new(
        project: &Hoi4Project,
        generation: u64,
        province: Option<&ProvinceMapCandidate>,
        states: Option<&ProjectPatchPlan>,
    ) -> Result<Self, String> {
        Self::new_with_strategic_regions(project, generation, province, states, None, 0)
    }

    pub(crate) fn new_with_strategic_regions(
        project: &Hoi4Project,
        generation: u64,
        province: Option<&ProvinceMapCandidate>,
        states: Option<&ProjectPatchPlan>,
        strategic_regions: Option<&ProjectPatchPlan>,
        strategic_region_revision: u64,
    ) -> Result<Self, String> {
        let empty_state_plan = ProjectPatchPlan {
            generation,
            source_fingerprints: BTreeMap::new(),
            modified_files: Vec::new(),
            created_files: Vec::new(),
            removed_files: Vec::new(),
            diagnostics: Vec::new(),
            summary: PatchPlanSummary::default(),
            timings: PatchPlanTimings::default(),
        };
        let state_plan = states.unwrap_or(&empty_state_plan);
        let state_files = state_plan.files_len();
        let mut patch_plan = state_plan.clone();
        let strategic_region_files = strategic_regions.map_or(0, ProjectPatchPlan::files_len);
        if let Some(strategic_regions) = strategic_regions {
            if strategic_regions.generation != strategic_region_revision {
                return Err("Strategic Region patch plan is stale.".to_owned());
            }
            merge_patch_plan(&mut patch_plan, strategic_regions)?;
        }
        patch_plan.generation = generation;
        let candidate_digest =
            combined_candidate_digest(&patch_plan, province.map(|candidate| &candidate.files));
        let province_files = if let Some(candidate) = province {
            append_province_candidate(project, candidate, &mut patch_plan)?
        } else {
            0
        };
        patch_plan
            .modified_files
            .sort_by(|a, b| a.path.cmp(&b.path));
        patch_plan.created_files.sort_by(|a, b| a.path.cmp(&b.path));
        patch_plan.removed_files.sort_by(|a, b| a.path.cmp(&b.path));
        patch_plan.summary = summarize(&patch_plan);

        let mut domains = BTreeSet::new();
        if province_files != 0 {
            domains.insert(SaveDomain::ProvinceMap);
        }
        if state_files != 0 {
            domains.insert(SaveDomain::States);
        }
        if strategic_region_files != 0 {
            domains.insert(SaveDomain::StrategicRegions);
        }
        Ok(Self {
            patch_plan,
            domains,
            dirty: ProjectDirtyState {
                province_files,
                state_files,
                strategic_region_files,
            },
            candidate_digest,
            coastal_flags_recalculated: 0,
            strategic_region_revision,
        })
    }

    pub fn patch_plan(&self) -> &ProjectPatchPlan {
        &self.patch_plan
    }

    pub fn domains(&self) -> &BTreeSet<SaveDomain> {
        &self.domains
    }

    pub fn dirty(&self) -> ProjectDirtyState {
        self.dirty
    }

    pub fn candidate_digest(&self) -> &SourceFingerprint {
        &self.candidate_digest
    }

    pub fn coastal_flags_recalculated(&self) -> usize {
        self.coastal_flags_recalculated
    }

    pub fn strategic_region_revision(&self) -> u64 {
        self.strategic_region_revision
    }

    pub(crate) fn set_coastal_flags_recalculated(&mut self, count: usize) {
        self.coastal_flags_recalculated = count;
    }

    pub fn into_patch_plan(self) -> ProjectPatchPlan {
        self.patch_plan
    }
}

fn merge_patch_plan(
    target: &mut ProjectPatchPlan,
    additional: &ProjectPatchPlan,
) -> Result<(), String> {
    let mut paths = target
        .modified_files
        .iter()
        .map(|file| file.path.clone())
        .chain(target.created_files.iter().map(|file| file.path.clone()))
        .chain(target.removed_files.iter().map(|file| file.path.clone()))
        .collect::<BTreeSet<_>>();
    for path in additional
        .modified_files
        .iter()
        .map(|file| &file.path)
        .chain(additional.created_files.iter().map(|file| &file.path))
        .chain(additional.removed_files.iter().map(|file| &file.path))
    {
        if !paths.insert(path.clone()) {
            return Err(format!("Save Project path collision: {}", path.display()));
        }
    }
    for (path, fingerprint) in &additional.source_fingerprints {
        if let Some(existing) = target
            .source_fingerprints
            .insert(path.clone(), fingerprint.clone())
            && existing != *fingerprint
        {
            return Err(format!(
                "Save Project source fingerprint collision: {}",
                path.display()
            ));
        }
    }
    target
        .modified_files
        .extend(additional.modified_files.clone());
    target
        .created_files
        .extend(additional.created_files.clone());
    target
        .removed_files
        .extend(additional.removed_files.clone());
    target.diagnostics.extend(additional.diagnostics.clone());
    Ok(())
}

fn append_province_candidate(
    project: &Hoi4Project,
    candidate: &ProvinceMapCandidate,
    plan: &mut ProjectPatchPlan,
) -> Result<usize, String> {
    if !project.paths.uses_conventional_editable_map_layout() {
        return Err(
            "Save Project does not yet write Province-map changes declared through map/default.map. The current map source remains read-only for this operation; state-only saves remain supported."
                .to_owned(),
        );
    }
    let root = &project.paths.root;
    let mut count = 0;
    for (map_relative, after) in &candidate.files {
        validate_province_path(map_relative)?;
        let relative = PathBuf::from("map").join(map_relative);
        let final_path = root.join(&relative);
        match fs::read(&final_path) {
            Ok(before) if before == *after => {}
            Ok(before) => {
                plan.source_fingerprints
                    .insert(final_path, SourceFingerprint::from_bytes(&before));
                plan.modified_files.push(PlannedFileModification {
                    path: relative,
                    state_id: 0,
                    operations: Vec::new(),
                    before,
                    after: Some(after.clone()),
                    unified_diff: format!("Province map file update: {}", map_relative.display()),
                    semantic_changes: vec![format!("Update map/{}", map_relative.display())],
                    diagnostics: Vec::new(),
                    safety: PatchSafety::Safe,
                });
                count += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                plan.created_files.push(PlannedFileCreation {
                    path: relative,
                    state_id: 0,
                    content: after.clone(),
                    unified_diff: format!("Province map file creation: {}", map_relative.display()),
                    semantic_changes: vec![format!("Create map/{}", map_relative.display())],
                    diagnostics: Vec::new(),
                    safety: PatchSafety::Safe,
                });
                count += 1;
            }
            Err(error) => {
                return Err(format!("Cannot read {}: {error}", final_path.display()));
            }
        }
    }
    Ok(count)
}

fn validate_province_path(path: &Path) -> Result<(), String> {
    if path.is_absolute() || path.components().count() != 1 {
        return Err(format!("Unsafe province map path: {}", path.display()));
    }
    match path.to_string_lossy().replace('\\', "/").as_str() {
        "provinces.bmp" | "definition.csv" | "adjacencies.csv" | "id_changes.txt" => Ok(()),
        _ => Err(format!("Unsupported province map path: {}", path.display())),
    }
}

fn summarize(plan: &ProjectPatchPlan) -> PatchPlanSummary {
    let modified_files = plan.modified_files.len();
    let created_files = plan.created_files.len();
    let removed_files = plan.removed_files.len();
    let mut summary = PatchPlanSummary {
        modified_files,
        created_files,
        removed_files,
        ..PatchPlanSummary::default()
    };
    for safety in plan
        .modified_files
        .iter()
        .map(|file| file.safety)
        .chain(plan.created_files.iter().map(|file| file.safety))
        .chain(plan.removed_files.iter().map(|file| file.safety))
    {
        match safety {
            PatchSafety::Safe => summary.safe_files += 1,
            PatchSafety::ReviewRequired => summary.review_required_files += 1,
            PatchSafety::Blocked => summary.blocked_files += 1,
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::project::ProjectPaths;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hoi4-map-editor-save-plan-{name}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("map")).unwrap();
        fs::create_dir_all(root.join("history/states")).unwrap();
        root
    }

    fn project(root: &Path) -> Hoi4Project {
        Hoi4Project {
            paths: ProjectPaths {
                root: root.to_owned(),
                map_directory: root.join("map"),
                provinces_bmp: root.join("map").join("provinces.bmp"),
                definition_csv: root.join("map").join("definition.csv"),
                adjacencies_csv: None,
                rivers_bmp: None,
                continent_txt: None,
                history_directory: root.join("history"),
                states_directory: root.join("history").join("states"),
                sources: crate::app::project::ProjectSources::for_test_placeholder(),
            },
            states: Vec::new(),
            states_by_id: BTreeMap::new(),
            state_by_province: Default::default(),
            ambiguous_provinces: Default::default(),
            unassigned_land_provinces: Default::default(),
            diagnostics: Vec::new(),
            load_summary: Default::default(),
            logistics: crate::app::project::LogisticsLoadResult::default(),
            strategic_regions: crate::app::project::StrategicRegionLoadResult::default(),
            effective_history_date: crate::app::project::EffectiveHistoryDate::Unavailable {
                reason: "test fixture".to_owned(),
            },
        }
    }

    fn candidate(files: &[(&str, &[u8])]) -> ProvinceMapCandidate {
        ProvinceMapCandidate {
            files: files
                .iter()
                .map(|(path, bytes)| (PathBuf::from(path), bytes.to_vec()))
                .collect(),
            definitions: Vec::new(),
            image: image::RgbImage::new(1, 1),
            had_id_changes: false,
        }
    }

    #[test]
    fn province_only_plan_tracks_created_files() {
        let root = temp_root("created");
        let project = project(&root);
        let plan = ProjectSavePlan::new(
            &project,
            7,
            Some(&candidate(&[("definition.csv", b"new")])),
            None,
        )
        .unwrap();

        assert_eq!(plan.dirty().province_files, 1);
        assert!(plan.domains().contains(&SaveDomain::ProvinceMap));
        assert_eq!(
            plan.patch_plan().created_files[0].path,
            PathBuf::from("map/definition.csv")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unchanged_province_candidate_is_not_dirty() {
        let root = temp_root("unchanged");
        fs::write(root.join("map/definition.csv"), b"same").unwrap();
        let project = project(&root);
        let plan = ProjectSavePlan::new(
            &project,
            7,
            Some(&candidate(&[("definition.csv", b"same")])),
            None,
        )
        .unwrap();

        assert!(!plan.dirty().has_changes());
        assert_eq!(plan.patch_plan().files_len(), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
