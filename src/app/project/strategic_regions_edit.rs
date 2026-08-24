//! Working-copy editing for Strategic Region documents.
//!
//! This is intentionally independent from Canvas and the save executor.  It
//! keeps immutable loaded data separate from a sparse working map and retains
//! the parsed source documents needed for byte-preserving candidate patches.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::app::state::{PdxBlock, PdxDocument, PdxValue, TextSpan, parse_text};

use super::{
    Hoi4Project, PatchPlanSummary, PatchPlanTimings, PatchSafety, PlannedFileCreation,
    PlannedFileModification, ProjectPatchPlan, ResolvedSource, SourceFingerprint, SourceKind,
    StrategicRegion, StrategicRegionCoverage, StrategicRegionLoadResult, TextPatchOperation,
};

#[derive(Debug, Clone)]
pub struct StrategicRegionDocument {
    pub source: Arc<ResolvedSource>,
    pub original_bytes: Arc<[u8]>,
    pub syntax: PdxDocument,
    pub records: BTreeMap<u32, StrategicRegionRecordLocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrategicRegionRecordLocation {
    pub region_span: TextSpan,
    pub provinces_span: Option<TextSpan>,
    pub naval_terrain_span: Option<TextSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingStrategicRegion {
    pub id: u32,
    pub name_key: Option<String>,
    pub display_name: Option<String>,
    pub has_provinces_field: bool,
    pub provinces: Vec<u32>,
    pub naval_terrain: Option<String>,
    pub source: Arc<ResolvedSource>,
    pub span: TextSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StrategicRegionEditCommand {
    ReplaceRegions {
        before: BTreeMap<u32, WorkingStrategicRegion>,
        after: BTreeMap<u32, WorkingStrategicRegion>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategicRegionEditError {
    CoverageIncomplete,
    NotPresent,
    DuplicateRegionId(u32),
    RegionNotFound(u32),
    SourceUnavailable(PathBuf),
    SourceNotUtf8(PathBuf),
    SourceSyntax(PathBuf),
    ReadOnlyLowerSource(PathBuf),
    SourceRecordUnavailable { region_id: u32, path: PathBuf },
    UnsafeCommentAssociation { region_id: u32, path: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategicRegionProvinceMembership {
    Assigned(u32),
    Unassigned,
    Ambiguous,
    Unknown,
}

impl std::fmt::Display for StrategicRegionEditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CoverageIncomplete => write!(
                formatter,
                "Strategic Region editing is disabled while source coverage is incomplete"
            ),
            Self::NotPresent => write!(
                formatter,
                "Strategic Region editing is unavailable because the domain is not present"
            ),
            Self::DuplicateRegionId(id) => write!(
                formatter,
                "Strategic Region {id} has a duplicate identity and is read-only"
            ),
            Self::RegionNotFound(id) => write!(formatter, "Strategic Region {id} was not found"),
            Self::SourceUnavailable(path) => write!(
                formatter,
                "Strategic Region source {} is unavailable",
                path.display()
            ),
            Self::SourceNotUtf8(path) => write!(
                formatter,
                "Strategic Region source {} is not UTF-8",
                path.display()
            ),
            Self::SourceSyntax(path) => write!(
                formatter,
                "Strategic Region source {} has syntax errors",
                path.display()
            ),
            Self::ReadOnlyLowerSource(path) => write!(
                formatter,
                "Strategic Region source {} is archive-backed and cannot be stale-protected for override materialization",
                path.display()
            ),
            Self::SourceRecordUnavailable { region_id, path } => write!(
                formatter,
                "Strategic Region {region_id} cannot be patched safely in {}",
                path.display()
            ),
            Self::UnsafeCommentAssociation { region_id, path } => write!(
                formatter,
                "Strategic Region {region_id} has comments inside an edited field in {}; preserving their association requires manual editing",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StrategicRegionEditError {}

/// The session owns only working state, history and source baselines. It has
/// no filesystem write capability; a caller must produce a candidate plan.
#[derive(Debug, Clone)]
pub struct StrategicRegionEditSession {
    baseline: BTreeMap<u32, WorkingStrategicRegion>,
    working: BTreeMap<u32, WorkingStrategicRegion>,
    documents: BTreeMap<PathBuf, StrategicRegionDocument>,
    coverage: StrategicRegionCoverage,
    duplicate_region_ids: BTreeSet<u32>,
    undo_stack: Vec<StrategicRegionEditCommand>,
    redo_stack: Vec<StrategicRegionEditCommand>,
    revision: u64,
}

impl StrategicRegionEditSession {
    pub fn new(project: &Hoi4Project) -> Result<Self, StrategicRegionEditError> {
        let coverage = project.strategic_regions.coverage;
        let mut counts = BTreeMap::<u32, usize>::new();
        for region in &project.strategic_regions.regions {
            *counts.entry(region.id).or_default() += 1;
        }
        let duplicate_region_ids = counts
            .into_iter()
            .filter_map(|(id, count)| (count > 1).then_some(id))
            .collect::<BTreeSet<_>>();
        let mut documents = BTreeMap::new();
        for source in project
            .strategic_regions
            .regions
            .iter()
            .map(|region| region.source.clone())
        {
            let path = source.logical_path.clone();
            if documents.contains_key(&path) {
                continue;
            }
            let bytes = project
                .paths
                .sources
                .read_resolved(&source)
                .map_err(|_| StrategicRegionEditError::SourceUnavailable(path.clone()))?;
            let text = String::from_utf8(bytes.clone())
                .map_err(|_| StrategicRegionEditError::SourceNotUtf8(path.clone()))?;
            let syntax = parse_text(path.clone(), text);
            if !syntax.diagnostics.is_empty() {
                return Err(StrategicRegionEditError::SourceSyntax(path));
            }
            let records = locate_records(&syntax);
            documents.insert(
                path,
                StrategicRegionDocument {
                    source,
                    original_bytes: Arc::from(bytes),
                    syntax,
                    records,
                },
            );
        }
        let baseline = project
            .strategic_regions
            .regions
            .iter()
            .filter(|region| !duplicate_region_ids.contains(&region.id))
            .map(|region| (region.id, working_region(region)))
            .collect::<BTreeMap<_, _>>();
        Ok(Self {
            working: baseline.clone(),
            baseline,
            documents,
            coverage,
            duplicate_region_ids,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            revision: 0,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn coverage(&self) -> StrategicRegionCoverage {
        self.coverage
    }
    pub fn is_dirty(&self) -> bool {
        self.working != self.baseline
    }
    pub fn regions(&self) -> impl Iterator<Item = &WorkingStrategicRegion> {
        self.working.values()
    }
    pub fn region(&self, id: u32) -> Option<&WorkingStrategicRegion> {
        self.working.get(&id)
    }
    pub fn document(&self, path: &std::path::Path) -> Option<&StrategicRegionDocument> {
        self.documents.get(path)
    }
    pub fn dirty_region_ids(&self) -> BTreeSet<u32> {
        self.working
            .iter()
            .filter_map(|(&id, working)| (self.baseline.get(&id) != Some(working)).then_some(id))
            .collect()
    }

    pub fn history_len(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn membership_of(&self, province_id: u32) -> StrategicRegionProvinceMembership {
        if !self.coverage.is_complete() {
            return StrategicRegionProvinceMembership::Unknown;
        }
        let owners = self
            .working
            .values()
            .filter(|region| region.provinces.contains(&province_id))
            .map(|region| region.id)
            .collect::<Vec<_>>();
        match owners.as_slice() {
            [] => StrategicRegionProvinceMembership::Unassigned,
            [id] => StrategicRegionProvinceMembership::Assigned(*id),
            _ => StrategicRegionProvinceMembership::Ambiguous,
        }
    }

    pub fn editability_reason(&self, target_region_id: Option<u32>) -> Option<String> {
        match self.coverage {
            StrategicRegionCoverage::NotPresent => {
                return Some("Strategic Region data is not present.".to_owned());
            }
            StrategicRegionCoverage::Incomplete { .. } => {
                return Some("Strategic Region data is incomplete.".to_owned());
            }
            StrategicRegionCoverage::Complete => {}
        }
        target_region_id.and_then(|id| self.ensure_mutable(id).err().map(|error| error.to_string()))
    }

    pub fn assign_province(
        &mut self,
        province_id: u32,
        target_region_id: u32,
    ) -> Result<bool, StrategicRegionEditError> {
        self.assign_provinces_to_region([province_id], target_region_id)
    }

    /// Applies a complete gesture as one working-copy command. Every source
    /// record that could lose a membership is checked before anything changes.
    pub fn assign_provinces_to_region(
        &mut self,
        province_ids: impl IntoIterator<Item = u32>,
        target_region_id: u32,
    ) -> Result<bool, StrategicRegionEditError> {
        let province_ids = province_ids
            .into_iter()
            .filter(|id| *id != 0)
            .collect::<BTreeSet<_>>();
        self.preflight_assignment(&province_ids, target_region_id)?;
        let mut after = self.working.clone();
        for region in after.values_mut() {
            region.provinces.retain(|id| !province_ids.contains(id));
        }
        let target = after
            .get_mut(&target_region_id)
            .expect("checked target region");
        target.has_provinces_field = true;
        target.provinces.extend(province_ids);
        self.commit(after)
    }

    fn preflight_assignment(
        &self,
        province_ids: &BTreeSet<u32>,
        target_region_id: u32,
    ) -> Result<(), StrategicRegionEditError> {
        self.ensure_mutable(target_region_id)?;
        let affected = self
            .working
            .values()
            .filter(|region| region.provinces.iter().any(|id| province_ids.contains(id)))
            .map(|region| region.id)
            .chain(std::iter::once(target_region_id))
            .collect::<BTreeSet<_>>();
        for id in affected {
            self.ensure_mutable(id)?;
        }
        // Rendering a clone performs the same source-span/comment preflight as
        // Save planning, while retaining this operation as a pure session API.
        let mut candidate = self.clone();
        let mut after = candidate.working.clone();
        for region in after.values_mut() {
            region.provinces.retain(|id| !province_ids.contains(id));
        }
        let target = after
            .get_mut(&target_region_id)
            .expect("checked target region");
        target.has_provinces_field = true;
        target.provinces.extend(province_ids.iter().copied());
        candidate.working = after;
        for document in candidate.documents.values() {
            render_document(&candidate, document)?;
        }
        Ok(())
    }

    pub fn remove_province(
        &mut self,
        region_id: u32,
        province_id: u32,
    ) -> Result<bool, StrategicRegionEditError> {
        self.ensure_mutable(region_id)?;
        let mut after = self.working.clone();
        after
            .get_mut(&region_id)
            .expect("checked region")
            .provinces
            .retain(|&id| id != province_id);
        self.commit(after)
    }

    pub fn set_naval_terrain(
        &mut self,
        region_id: u32,
        naval_terrain: Option<String>,
    ) -> Result<bool, StrategicRegionEditError> {
        self.ensure_mutable(region_id)?;
        let mut after = self.working.clone();
        after
            .get_mut(&region_id)
            .expect("checked region")
            .naval_terrain = naval_terrain;
        self.commit(after)
    }

    pub fn undo(&mut self) -> bool {
        let Some(command) = self.undo_stack.pop() else {
            return false;
        };
        match &command {
            StrategicRegionEditCommand::ReplaceRegions { before, .. } => {
                self.working = before.clone()
            }
        }
        self.redo_stack.push(command);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(command) = self.redo_stack.pop() else {
            return false;
        };
        match &command {
            StrategicRegionEditCommand::ReplaceRegions { after, .. } => {
                self.working = after.clone()
            }
        }
        self.undo_stack.push(command);
        self.revision = self.revision.wrapping_add(1);
        true
    }

    fn ensure_mutable(&self, id: u32) -> Result<(), StrategicRegionEditError> {
        match self.coverage {
            StrategicRegionCoverage::NotPresent => {
                return Err(StrategicRegionEditError::NotPresent);
            }
            StrategicRegionCoverage::Incomplete { .. } => {
                return Err(StrategicRegionEditError::CoverageIncomplete);
            }
            StrategicRegionCoverage::Complete => {}
        }
        if self.duplicate_region_ids.contains(&id) {
            return Err(StrategicRegionEditError::DuplicateRegionId(id));
        }
        let Some(region) = self.working.get(&id) else {
            return Err(StrategicRegionEditError::RegionNotFound(id));
        };
        let path = &region.source.logical_path;
        let Some(document) = self.documents.get(path) else {
            return Err(StrategicRegionEditError::SourceUnavailable(path.clone()));
        };
        if document.source.source_kind != SourceKind::CurrentProject
            && document.source.filesystem_path().is_none()
        {
            return Err(StrategicRegionEditError::ReadOnlyLowerSource(path.clone()));
        }
        if !document.records.contains_key(&id) {
            return Err(StrategicRegionEditError::SourceRecordUnavailable {
                region_id: id,
                path: path.clone(),
            });
        }
        Ok(())
    }

    fn commit(
        &mut self,
        after: BTreeMap<u32, WorkingStrategicRegion>,
    ) -> Result<bool, StrategicRegionEditError> {
        if after == self.working {
            return Ok(false);
        }
        let before = std::mem::replace(&mut self.working, after.clone());
        self.undo_stack
            .push(StrategicRegionEditCommand::ReplaceRegions { before, after });
        self.redo_stack.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }
}

fn working_region(region: &StrategicRegion) -> WorkingStrategicRegion {
    WorkingStrategicRegion {
        id: region.id,
        name_key: region.name_key.clone(),
        display_name: region.display_name.clone(),
        has_provinces_field: region.has_provinces_field,
        provinces: region.provinces.clone(),
        naval_terrain: region.naval_terrain.clone(),
        source: region.source.clone(),
        span: region.span,
    }
}

fn locate_records(document: &PdxDocument) -> BTreeMap<u32, StrategicRegionRecordLocation> {
    document
        .entries
        .iter()
        .filter_map(|entry| (entry.key.as_ref()?.text == "strategic_region").then_some(entry))
        .filter_map(|entry| match &entry.value {
            PdxValue::Block(block) => Some((entry.span, block)),
            _ => None,
        })
        .filter_map(|(region_span, block)| {
            let id = scalar_field(block, "id")?.parse().ok()?;
            Some((
                id,
                StrategicRegionRecordLocation {
                    region_span,
                    provinces_span: field_span(block, "provinces"),
                    naval_terrain_span: field_span(block, "naval_terrain"),
                },
            ))
        })
        .collect()
}

fn scalar_field<'a>(block: &'a PdxBlock, key: &str) -> Option<&'a str> {
    block.entries.iter().find_map(|entry| {
        (entry.key.as_ref()?.text == key)
            .then_some(match &entry.value {
                PdxValue::Scalar(value) => Some(value.text.as_str()),
                PdxValue::Block(_) => None,
            })
            .flatten()
    })
}

fn field_span(block: &PdxBlock, key: &str) -> Option<TextSpan> {
    block
        .entries
        .iter()
        .find_map(|entry| (entry.key.as_ref()?.text == key).then_some(entry.span))
}

pub fn working_load_result(
    session: &StrategicRegionEditSession,
    loaded: &StrategicRegionLoadResult,
) -> StrategicRegionLoadResult {
    let mut result = loaded.clone();
    result.regions = loaded
        .regions
        .iter()
        .map(|region| {
            session.region(region.id).map_or_else(
                || region.clone(),
                |working| StrategicRegion {
                    id: working.id,
                    name_key: working.name_key.clone(),
                    display_name: working.display_name.clone(),
                    has_provinces_field: working.has_provinces_field,
                    provinces: working.provinces.clone(),
                    naval_terrain: working.naval_terrain.clone(),
                    source: working.source.clone(),
                    span: working.span,
                },
            )
        })
        .collect();
    result.province_references = result
        .regions
        .iter()
        .map(|region| region.provinces.len())
        .sum();
    result
}

/// Produces candidate-only project writes. Lower-source documents become a
/// complete project override at the same logical path; they are never opened
/// for direct mutation.
pub fn plan_strategic_region_patches(
    project: &Hoi4Project,
    session: &StrategicRegionEditSession,
) -> Result<ProjectPatchPlan, StrategicRegionEditError> {
    let mut modified_files = Vec::new();
    let mut created_files = Vec::new();
    let dirty = session.dirty_region_ids();
    let mut dirty_documents = BTreeSet::new();
    for id in dirty {
        let region = session.region(id).expect("dirty region remains present");
        dirty_documents.insert(region.source.logical_path.clone());
    }
    for logical_path in dirty_documents {
        let document = session
            .documents
            .get(&logical_path)
            .ok_or_else(|| StrategicRegionEditError::SourceUnavailable(logical_path.clone()))?;
        let after = render_document(session, document)?;
        if after == document.original_bytes.as_ref() {
            continue;
        }
        let semantic_changes = vec![format!(
            "Update Strategic Region document {}",
            logical_path.display()
        )];
        if document.source.source_kind == SourceKind::CurrentProject {
            let before = document.original_bytes.to_vec();
            modified_files.push(PlannedFileModification {
                path: logical_path.clone(),
                state_id: 0,
                operations: vec![TextPatchOperation::Replace {
                    range: 0..before.len(),
                    expected: before.clone(),
                    replacement: after.clone(),
                    state_id: 0,
                    field: "strategic_regions".to_owned(),
                    description: format!(
                        "Patch Strategic Region document {}",
                        logical_path.display()
                    ),
                    safety: PatchSafety::Safe,
                }],
                before,
                after: Some(after),
                unified_diff: format!("Strategic Region update: {}", logical_path.display()),
                semantic_changes,
                diagnostics: Vec::new(),
                safety: PatchSafety::Safe,
            });
        } else {
            created_files.push(PlannedFileCreation {
                path: logical_path.clone(),
                state_id: 0,
                content: after,
                unified_diff: format!(
                    "Strategic Region project override: {}",
                    logical_path.display()
                ),
                semantic_changes,
                diagnostics: Vec::new(),
                safety: PatchSafety::Safe,
            });
        }
    }
    let mut source_fingerprints = BTreeMap::new();
    for file in &modified_files {
        source_fingerprints.insert(
            project.paths.root.join(&file.path),
            SourceFingerprint::from_bytes(&file.before),
        );
    }
    // Lower-source edits become a project creation, but the original resolved
    // file remains an input to that override. Its fingerprint is retained so
    // both candidate validation and the final transaction reject a stale
    // lower source rather than silently materializing an obsolete copy.
    for logical_path in session.documents.keys() {
        let document = &session.documents[logical_path];
        if document.source.source_kind != SourceKind::CurrentProject
            && session.working.values().any(|region| {
                region.source.logical_path == *logical_path
                    && session.baseline.get(&region.id) != Some(region)
            })
        {
            let path = document.source.filesystem_path().ok_or_else(|| {
                StrategicRegionEditError::ReadOnlyLowerSource(logical_path.clone())
            })?;
            source_fingerprints.insert(
                path.to_owned(),
                SourceFingerprint::from_bytes(&document.original_bytes),
            );
        }
    }
    let summary = PatchPlanSummary {
        modified_files: modified_files.len(),
        created_files: created_files.len(),
        safe_files: modified_files.len() + created_files.len(),
        ..Default::default()
    };
    Ok(ProjectPatchPlan {
        generation: session.revision(),
        source_fingerprints,
        modified_files,
        created_files,
        removed_files: Vec::new(),
        diagnostics: Vec::new(),
        summary,
        timings: PatchPlanTimings::default(),
    })
}

fn render_document(
    session: &StrategicRegionEditSession,
    document: &StrategicRegionDocument,
) -> Result<Vec<u8>, StrategicRegionEditError> {
    let mut replacements = Vec::<(std::ops::Range<usize>, Vec<u8>)>::new();
    for (&id, location) in &document.records {
        let Some(working) = session.region(id) else {
            continue;
        };
        let Some(baseline) = session.baseline.get(&id) else {
            continue;
        };
        if working.provinces != baseline.provinces
            || working.has_provinces_field != baseline.has_provinces_field
        {
            if let Some(span) = location.provinces_span {
                reject_inline_comments(document, span, id)?;
                replacements.push((
                    span.start..span.end(),
                    render_provinces(working).into_bytes(),
                ));
            } else if working.has_provinces_field {
                let offset = location.region_span.end().saturating_sub(1);
                replacements.push((
                    offset..offset,
                    format!("\n    {}\n", render_provinces(working)).into_bytes(),
                ));
            } else {
                return Err(StrategicRegionEditError::SourceRecordUnavailable {
                    region_id: id,
                    path: document.source.logical_path.clone(),
                });
            }
        }
        if working.naval_terrain != baseline.naval_terrain {
            match (location.naval_terrain_span, &working.naval_terrain) {
                (Some(span), Some(terrain)) => {
                    reject_inline_comments(document, span, id)?;
                    replacements.push((
                        span.start..span.end(),
                        format!("naval_terrain = {terrain}").into_bytes(),
                    ))
                }
                (Some(span), None) => {
                    reject_inline_comments(document, span, id)?;
                    replacements.push((span.start..span.end(), Vec::new()))
                }
                (None, Some(terrain)) => {
                    let offset = location.region_span.end().saturating_sub(1);
                    replacements.push((
                        offset..offset,
                        format!("\n    naval_terrain = {terrain}\n").into_bytes(),
                    ));
                }
                (None, None) => {}
            }
        }
    }
    let mut bytes = document.original_bytes.to_vec();
    replacements.sort_by_key(|item| std::cmp::Reverse(item.0.start));
    for (range, replacement) in replacements {
        if range.end > bytes.len() || range.start > range.end {
            return Err(StrategicRegionEditError::SourceRecordUnavailable {
                region_id: 0,
                path: document.source.logical_path.clone(),
            });
        }
        bytes.splice(range, replacement);
    }
    Ok(bytes)
}

fn reject_inline_comments(
    document: &StrategicRegionDocument,
    span: TextSpan,
    region_id: u32,
) -> Result<(), StrategicRegionEditError> {
    let bytes = document.original_bytes.as_ref();
    if bytes
        .get(span.start..span.end())
        .is_some_and(|field| field.contains(&b'#'))
    {
        return Err(StrategicRegionEditError::UnsafeCommentAssociation {
            region_id,
            path: document.source.logical_path.clone(),
        });
    }
    Ok(())
}

fn render_provinces(region: &WorkingStrategicRegion) -> String {
    let members = region
        .provinces
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" ");
    format!("provinces = {{ {members} }}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::project::{ProjectPaths, ProjectSources, SourceGeneration};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    fn project() -> (std::path::PathBuf, Hoi4Project) {
        let root = std::env::temp_dir().join(format!(
            "hoi4-sr-edit-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("map/strategicregions")).unwrap();
        fs::create_dir_all(root.join("history/states")).unwrap();
        fs::write(root.join("map/provinces.bmp"), []).unwrap();
        fs::write(root.join("map/definition.csv"), []).unwrap();
        fs::write(root.join("map/strategicregions/a.txt"), "# before\nstrategic_region = { id = 100 provinces = { 42 } unknown = { keep = yes } }\n# middle\nstrategic_region = { id = 500 provinces = { 7 } naval_terrain = ocean }").unwrap();
        let sources = ProjectSources::discover(&root, None, SourceGeneration::new(1)).unwrap();
        let paths = ProjectPaths::discover(&root).unwrap();
        let mut project = Hoi4Project::new(paths);
        project.paths.sources = sources;
        project.load_strategic_regions();
        (root, project)
    }

    #[test]
    fn membership_move_undo_redo_keeps_sparse_working_regions() {
        let (root, project) = project();
        let mut session = StrategicRegionEditSession::new(&project).unwrap();
        assert!(session.assign_province(42, 500).unwrap());
        assert_eq!(session.region(100).unwrap().provinces, Vec::<u32>::new());
        assert_eq!(session.region(500).unwrap().provinces, vec![7, 42]);
        assert!(session.undo());
        assert_eq!(session.region(100).unwrap().provinces, vec![42]);
        assert!(session.redo());
        assert_eq!(session.region(500).unwrap().provinces, vec![7, 42]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_assignment_is_one_history_command_and_is_atomic() {
        let (root, project) = project();
        let mut session = StrategicRegionEditSession::new(&project).unwrap();
        assert!(
            session
                .assign_provinces_to_region([42, 7, 42], 500)
                .unwrap()
        );
        assert_eq!(session.history_len(), 1);
        assert_eq!(session.region(100).unwrap().provinces, Vec::<u32>::new());
        assert_eq!(session.region(500).unwrap().provinces, vec![7, 42]);
        assert!(session.undo());
        assert_eq!(session.region(100).unwrap().provinces, vec![42]);
        assert_eq!(session.region(500).unwrap().provinces, vec![7]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn membership_classifies_unassigned_unique_and_ambiguous_provinces() {
        let (root, project) = project();
        let mut session = StrategicRegionEditSession::new(&project).unwrap();
        assert_eq!(
            session.membership_of(1),
            StrategicRegionProvinceMembership::Unassigned
        );
        assert_eq!(
            session.membership_of(42),
            StrategicRegionProvinceMembership::Assigned(100)
        );
        session.working.get_mut(&500).unwrap().provinces.push(42);
        assert_eq!(
            session.membership_of(42),
            StrategicRegionProvinceMembership::Ambiguous
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_assignment_that_already_matches_is_a_noop() {
        let (root, project) = project();
        let mut session = StrategicRegionEditSession::new(&project).unwrap();
        let revision = session.revision();
        assert!(!session.assign_provinces_to_region([7, 7], 500).unwrap());
        assert_eq!(session.history_len(), 0);
        assert_eq!(session.revision(), revision);
        assert!(!session.is_dirty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_preflight_rejects_comment_protected_source_without_mutation() {
        let (root, mut project) = project();
        fs::write(
            root.join("map/strategicregions/a.txt"),
            "strategic_region = { id = 100 provinces = { 42 # retain\n } }\nstrategic_region = { id = 500 provinces = { 7 } }",
        )
        .unwrap();
        project.load_strategic_regions();
        let mut session = StrategicRegionEditSession::new(&project).unwrap();
        assert!(matches!(
            session.assign_provinces_to_region([42], 500),
            Err(StrategicRegionEditError::UnsafeCommentAssociation { region_id: 100, .. })
        ));
        assert_eq!(session.history_len(), 0);
        assert_eq!(session.region(100).unwrap().provinces, vec![42]);
        assert_eq!(session.region(500).unwrap().provinces, vec![7]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn candidate_patch_preserves_unknown_fields_comments_and_other_region() {
        let (root, project) = project();
        let mut session = StrategicRegionEditSession::new(&project).unwrap();
        session.assign_province(42, 500).unwrap();
        let plan = plan_strategic_region_patches(&project, &session).unwrap();
        let bytes = plan.modified_files[0].after.as_ref().unwrap();
        let text = std::str::from_utf8(bytes).unwrap();
        assert!(text.contains("# before"));
        assert!(text.contains("unknown = { keep = yes }"));
        assert!(text.contains("id = 100"));
        assert!(text.contains("id = 500"));
        let reparsed = parse_text("a.txt", text);
        assert!(reparsed.diagnostics.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lower_source_edit_materializes_project_override_and_tracks_lower_baseline() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-sr-lower-edit-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let base = root.with_extension("base");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(root.join("map")).unwrap();
        fs::create_dir_all(root.join("history/states")).unwrap();
        fs::write(root.join("map/provinces.bmp"), []).unwrap();
        fs::write(root.join("map/definition.csv"), []).unwrap();
        fs::create_dir_all(base.join("map/strategicregions")).unwrap();
        fs::write(
            base.join("map/strategicregions/base.txt"),
            "strategic_region = { id = 10 provinces = { 1 } }",
        )
        .unwrap();
        let paths = ProjectPaths::discover(&root).unwrap();
        let sources =
            ProjectSources::discover(&root, Some(base.clone()), SourceGeneration::new(1)).unwrap();
        let mut project = Hoi4Project::new(paths);
        project.paths.sources = sources;
        project.load_strategic_regions();
        let mut session = StrategicRegionEditSession::new(&project).unwrap();
        session.assign_province(2, 10).unwrap();
        let plan = plan_strategic_region_patches(&project, &session).unwrap();
        assert_eq!(plan.created_files.len(), 1);
        assert_eq!(
            plan.created_files[0].path,
            PathBuf::from("map/strategicregions/base.txt")
        );
        assert!(
            plan.source_fingerprints
                .contains_key(&base.join("map/strategicregions/base.txt"))
        );
        assert!(!root.join("map/strategicregions/base.txt").exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(base).unwrap();
    }
}
