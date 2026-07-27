use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{
  Arc,
  atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::app::map::{Bundle, ProvinceKind};
use crate::app::state::{StateData, VictoryPoint};
use crate::config::Config;
use crate::util::files::Location;

use super::patch::apply_operations;
use super::{
  DiagnosticSeverity, Hoi4Project, PatchSafety, ProjectDiagnosticKind, ProjectPatchPlan,
  ProjectPaths, SourceFingerprint, StateEditSession,
};

static WORKSPACE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundTripStatus {
  Passed,
  PassedWithReview,
  Failed,
  Cancelled,
}

impl RoundTripStatus {
  pub fn label(self) -> &'static str {
    match self {
      Self::Passed => "PASSED",
      Self::PassedWithReview => "PASSED WITH REVIEW",
      Self::Failed => "FAILED",
      Self::Cancelled => "CANCELLED",
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundTripStage {
  Preflight,
  SourceVerification,
  WorkspaceCreation,
  Copying,
  Applying,
  FilesystemVerification,
  Reloading,
  SemanticComparison,
  ByteComparison,
  DiagnosticComparison,
  Cleanup,
}

impl RoundTripStage {
  pub fn message(self) -> &'static str {
    match self {
      Self::Preflight => "Checking patch plan...",
      Self::SourceVerification => "Verifying source...",
      Self::WorkspaceCreation => "Creating isolated workspace...",
      Self::Copying => "Copying project...",
      Self::Applying => "Applying candidate changes...",
      Self::FilesystemVerification => "Verifying candidate files...",
      Self::Reloading => "Reloading candidate project...",
      Self::SemanticComparison => "Comparing working state...",
      Self::ByteComparison => "Comparing bytes...",
      Self::DiagnosticComparison => "Comparing diagnostics...",
      Self::Cleanup => "Cleaning temporary workspace...",
    }
  }
}

#[derive(Debug, Clone)]
pub struct RoundTripValidationPolicy {
  pub allow_review_required: bool,
  pub retain_failed_workspace: bool,
  pub validate_full_project: bool,
}

impl Default for RoundTripValidationPolicy {
  fn default() -> Self {
    Self {
      allow_review_required: false,
      retain_failed_workspace: false,
      validate_full_project: true,
    }
  }
}

#[derive(Debug, Clone, Default)]
pub struct RoundTripCancellation(Arc<AtomicBool>);

impl RoundTripCancellation {
  pub fn cancel(&self) {
    self.0.store(true, Ordering::Release);
  }

  pub fn is_cancelled(&self) -> bool {
    self.0.load(Ordering::Acquire)
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFingerprint {
  pub byte_len: u64,
  pub content_hash: u64,
}

impl FileFingerprint {
  fn from_bytes(bytes: &[u8]) -> Self {
    let fingerprint = SourceFingerprint::from_bytes(bytes);
    Self {
      byte_len: fingerprint.byte_len,
      content_hash: fingerprint.content_hash,
    }
  }
}

#[derive(Debug, Clone, Default)]
pub struct TemporaryProjectManifest {
  pub source_root: PathBuf,
  pub candidate_root: PathBuf,
  pub copied_files: BTreeMap<PathBuf, FileFingerprint>,
  pub expected_modified_files: BTreeSet<PathBuf>,
  pub expected_created_files: BTreeSet<PathBuf>,
  pub expected_removed_files: BTreeSet<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct TemporaryWorkspaceSummary {
  pub workspace_root: Option<PathBuf>,
  pub candidate_root: Option<PathBuf>,
  pub copied_files: usize,
  pub retained: bool,
  pub cleaned: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SourceVerificationResult {
  pub files_verified: usize,
  pub map_files_verified: usize,
  pub planned_sources_verified: usize,
  pub source_unchanged_after_validation: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CandidateApplicationResult {
  pub modified_files_applied: usize,
  pub created_files_applied: usize,
  pub removed_files_applied: usize,
  pub final_file_set_verified: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectReloadResult {
  pub files_found: usize,
  pub valid_states: usize,
  pub invalid_states: usize,
  pub indexed_states: usize,
  pub errors: usize,
  pub warnings: usize,
}

#[derive(Debug, Clone)]
pub enum SemanticDifference {
  MissingState(u32),
  UnexpectedState(u32),
  PropertyMismatch {
    state_id: u32,
    property: String,
    expected: String,
    actual: String,
  },
  ProvinceAssignmentMismatch {
    province_id: u32,
    expected: Option<u32>,
    actual: Option<u32>,
  },
}

impl SemanticDifference {
  fn describe(&self) -> String {
    match self {
      Self::MissingState(state_id) => format!("Missing State {state_id}"),
      Self::UnexpectedState(state_id) => format!("Unexpected State {state_id}"),
      Self::PropertyMismatch { state_id, property, expected, actual } => {
        format!("State {state_id} {property}: expected {expected}, got {actual}")
      },
      Self::ProvinceAssignmentMismatch { province_id, expected, actual } => {
        format!("Province {province_id}: expected State {expected:?}, got {actual:?}")
      },
    }
  }
}

#[derive(Debug, Clone, Default)]
pub struct ProjectSemanticComparison {
  pub states_match: bool,
  pub indexes_match: bool,
  pub province_coverage_match: bool,
  pub victory_points_match: bool,
  pub buildings_match: bool,
  pub created_states_match: bool,
  pub removed_states_match: bool,
  pub differences: Vec<SemanticDifference>,
}

#[derive(Debug, Clone)]
pub struct ByteDifference {
  pub path: PathBuf,
  pub first_different_offset: usize,
  pub source_len: usize,
  pub candidate_len: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ByteComparisonResult {
  pub unchanged_files_verified: usize,
  pub modified_files_verified: usize,
  pub created_files_verified: usize,
  pub removed_files_verified: usize,
  pub map_files_unchanged: bool,
  pub differences: Vec<ByteDifference>,
}

#[derive(Debug, Clone, Default)]
pub struct DiagnosticComparison {
  pub source_diagnostics: usize,
  pub candidate_diagnostics: usize,
  pub preserved_original_diagnostics: usize,
  pub expected_session_diagnostics: usize,
  pub unexpected_diagnostics: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RoundTripTimings {
  pub source_verification_ms: u128,
  pub workspace_creation_ms: u128,
  pub copy_ms: u128,
  pub plan_application_ms: u128,
  pub filesystem_verification_ms: u128,
  pub project_reload_ms: u128,
  pub semantic_comparison_ms: u128,
  pub byte_comparison_ms: u128,
  pub diagnostic_comparison_ms: u128,
  pub cleanup_ms: u128,
  pub total_ms: u128,
}

#[derive(Debug, Clone)]
pub struct RoundTripDiagnostic {
  pub stage: RoundTripStage,
  pub path: Option<PathBuf>,
  pub state_id: Option<u32>,
  pub message: String,
  pub action: String,
}

#[derive(Debug, Clone)]
pub struct RoundTripValidationReport {
  pub status: RoundTripStatus,
  pub plan_generation: u64,
  pub plan_fingerprint: SourceFingerprint,
  pub workspace: TemporaryWorkspaceSummary,
  pub source_verification: SourceVerificationResult,
  pub application: CandidateApplicationResult,
  pub reload: ProjectReloadResult,
  pub semantic_comparison: ProjectSemanticComparison,
  pub byte_comparison: ByteComparisonResult,
  pub diagnostics_comparison: DiagnosticComparison,
  pub timings: RoundTripTimings,
  pub diagnostics: Vec<RoundTripDiagnostic>,
  pub eligible_for_atomic_save_preparation: bool,
  pub no_candidate_changes: bool,
}

impl RoundTripValidationReport {
  fn new(plan: &ProjectPatchPlan) -> Self {
    Self {
      status: RoundTripStatus::Failed,
      plan_generation: plan.generation,
      plan_fingerprint: plan.content_fingerprint(),
      workspace: TemporaryWorkspaceSummary::default(),
      source_verification: SourceVerificationResult::default(),
      application: CandidateApplicationResult::default(),
      reload: ProjectReloadResult::default(),
      semantic_comparison: ProjectSemanticComparison::default(),
      byte_comparison: ByteComparisonResult::default(),
      diagnostics_comparison: DiagnosticComparison::default(),
      timings: RoundTripTimings::default(),
      diagnostics: Vec::new(),
      eligible_for_atomic_save_preparation: false,
      no_candidate_changes: false,
    }
  }

  pub fn is_stale(&self, revision: u64, plan: Option<&ProjectPatchPlan>) -> bool {
    self.plan_generation != revision
      || plan.is_none_or(|plan| {
        plan.generation != self.plan_generation
          || plan.content_fingerprint() != self.plan_fingerprint
      })
  }

  pub fn content_fingerprint(&self) -> SourceFingerprint {
    let mut bytes = b"HOI4_STATE_EDITOR_ROUND_TRIP_REPORT_V1".to_vec();
    bytes.extend_from_slice(&self.plan_generation.to_le_bytes());
    bytes.extend_from_slice(&self.plan_fingerprint.byte_len.to_le_bytes());
    bytes.extend_from_slice(&self.plan_fingerprint.content_hash.to_le_bytes());
    bytes.push(match self.status {
      RoundTripStatus::Passed => 0,
      RoundTripStatus::PassedWithReview => 1,
      RoundTripStatus::Failed => 2,
      RoundTripStatus::Cancelled => 3,
    });
    for value in [
      self.semantic_comparison.states_match,
      self.semantic_comparison.indexes_match,
      self.semantic_comparison.province_coverage_match,
      self.semantic_comparison.victory_points_match,
      self.semantic_comparison.buildings_match,
      self.semantic_comparison.created_states_match,
      self.semantic_comparison.removed_states_match,
      self.byte_comparison.map_files_unchanged,
      self.byte_comparison.differences.is_empty(),
      self.source_verification.source_unchanged_after_validation,
      self.eligible_for_atomic_save_preparation,
      self.no_candidate_changes,
    ] {
      bytes.push(u8::from(value));
    }
    for count in [
      self.application.modified_files_applied,
      self.application.created_files_applied,
      self.application.removed_files_applied,
      self.diagnostics_comparison.unexpected_diagnostics,
    ] {
      bytes.extend_from_slice(&(count as u64).to_le_bytes());
    }
    for difference in &self.semantic_comparison.differences {
      let description = difference.describe();
      bytes.extend_from_slice(&(description.len() as u64).to_le_bytes());
      bytes.extend_from_slice(description.as_bytes());
    }
    for diagnostic in &self.diagnostics {
      for text in [&diagnostic.message, &diagnostic.action] {
        bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
      }
    }
    SourceFingerprint::from_bytes(&bytes)
  }

  pub fn summary_text(&self) -> String {
    let mut text = format!(
      "ROUND-TRIP VALIDATION\nStatus: {}\n\
       Temporary validation only - the original mod was not changed.\n\
       Copied: {} | Modified: {} | Created: {} | Removed: {}\n\
       Reload: {} files | {} valid states | {} invalid states | {} errors | {} warnings\n\
       Semantic: {} | Indexes: {} | Bytes: {} | Unexpected diagnostics: {}\n\
       Workspace: {}\nTotal: {} ms",
      self.status.label(),
      self.workspace.copied_files,
      self.application.modified_files_applied,
      self.application.created_files_applied,
      self.application.removed_files_applied,
      self.reload.files_found,
      self.reload.valid_states,
      self.reload.invalid_states,
      self.reload.errors,
      self.reload.warnings,
      pass_fail(self.semantic_comparison.states_match),
      pass_fail(self.semantic_comparison.indexes_match),
      pass_fail(self.byte_comparison.differences.is_empty()),
      self.diagnostics_comparison.unexpected_diagnostics,
      if self.workspace.cleaned {
        "cleaned".to_owned()
      } else {
        self.workspace.workspace_root.as_ref()
          .map(|path| path.display().to_string())
          .unwrap_or_else(|| "not created".to_owned())
      },
      self.timings.total_ms,
    );
    if self.no_candidate_changes {
      text.push_str("\nNo candidate changes to validate.");
    }
    if self.eligible_for_atomic_save_preparation {
      text.push_str("\nEligible for atomic-save preparation (informational only; Save remains disabled).");
    }
    text
  }

  pub fn full_text(&self) -> String {
    let mut text = self.summary_text();
    text.push_str(&format!(
      "\n\nSource verification: {} files; unchanged after validation: {}\n\
       Semantic comparison: states {} | province assignments {} | coverage {} | VP {} | buildings {}\n\
       Byte comparison: unchanged {} | modified {} | created {} | removed {} | map {}\n\
       Timings: source {} ms | workspace {} ms | copy {} ms | apply {} ms | filesystem {} ms | \
       reload {} ms | semantic {} ms | bytes {} ms | diagnostics {} ms | cleanup {} ms",
      self.source_verification.files_verified,
      pass_fail(self.source_verification.source_unchanged_after_validation),
      pass_fail(self.semantic_comparison.states_match),
      pass_fail(self.semantic_comparison.indexes_match),
      pass_fail(self.semantic_comparison.province_coverage_match),
      pass_fail(self.semantic_comparison.victory_points_match),
      pass_fail(self.semantic_comparison.buildings_match),
      self.byte_comparison.unchanged_files_verified,
      self.byte_comparison.modified_files_verified,
      self.byte_comparison.created_files_verified,
      self.byte_comparison.removed_files_verified,
      pass_fail(self.byte_comparison.map_files_unchanged),
      self.timings.source_verification_ms,
      self.timings.workspace_creation_ms,
      self.timings.copy_ms,
      self.timings.plan_application_ms,
      self.timings.filesystem_verification_ms,
      self.timings.project_reload_ms,
      self.timings.semantic_comparison_ms,
      self.timings.byte_comparison_ms,
      self.timings.diagnostic_comparison_ms,
      self.timings.cleanup_ms,
    ));
    for difference in &self.semantic_comparison.differences {
      text.push_str(&format!("\nDIFFERENCE: {}", difference.describe()));
    }
    for diagnostic in &self.diagnostics {
      let path = diagnostic.path.as_ref()
        .map(|path| format!(" [{}]", path.display()))
        .unwrap_or_default();
      text.push_str(&format!(
        "\n{}{}: {}\nAction: {}",
        diagnostic.stage.message(),
        path,
        diagnostic.message,
        diagnostic.action,
      ));
    }
    text
  }
}

#[derive(Debug, Clone, Default)]
pub struct RoundTripValidator {
  pub policy: RoundTripValidationPolicy,
}

impl RoundTripValidator {
  pub fn validate<F>(
    &self,
    project: &Hoi4Project,
    edit: &StateEditSession,
    plan: &ProjectPatchPlan,
    cancellation: &RoundTripCancellation,
    mut progress: F,
  ) -> RoundTripValidationReport
  where
    F: FnMut(RoundTripStage),
  {
    let total_started = Instant::now();
    let mut report = RoundTripValidationReport::new(plan);
    let mut workspace_root = None;

    macro_rules! gate {
      ($result:expr) => {
        match $result {
          Ok(value) => value,
          Err(failure) => {
            report.status = if failure.cancelled {
              RoundTripStatus::Cancelled
            } else {
              RoundTripStatus::Failed
            };
            report.diagnostics.push(failure.diagnostic);
            finish_report(
              &self.policy,
              &mut report,
              workspace_root.as_deref(),
              total_started,
              &mut progress,
            );
            return report;
          },
        }
      };
    }

    progress(RoundTripStage::Preflight);
    gate!(check_cancelled(cancellation, RoundTripStage::Preflight));
    let plan_sets = gate!(validate_plan(project, edit, plan, &self.policy));
    if plan.files_len() == 0 {
      report.status = RoundTripStatus::Passed;
      report.no_candidate_changes = true;
      report.semantic_comparison = ProjectSemanticComparison {
        states_match: true,
        indexes_match: true,
        province_coverage_match: true,
        victory_points_match: true,
        buildings_match: true,
        created_states_match: true,
        removed_states_match: true,
        differences: Vec::new(),
      };
      report.byte_comparison.map_files_unchanged = true;
      report.source_verification.source_unchanged_after_validation = true;
      report.eligible_for_atomic_save_preparation = true;
      report.timings.total_ms = total_started.elapsed().as_millis();
      return report;
    }

    progress(RoundTripStage::SourceVerification);
    gate!(check_cancelled(cancellation, RoundTripStage::SourceVerification));
    let started = Instant::now();
    let snapshot = gate!(verify_source(project, plan, &plan_sets));
    report.timings.source_verification_ms = started.elapsed().as_millis();
    report.source_verification.files_verified = snapshot.files.len();
    report.source_verification.map_files_verified = 2;
    report.source_verification.planned_sources_verified =
      plan.modified_files.len() + plan.removed_files.len();

    progress(RoundTripStage::WorkspaceCreation);
    gate!(check_cancelled(cancellation, RoundTripStage::WorkspaceCreation));
    let started = Instant::now();
    let workspace = gate!(create_workspace(&project.paths.root));
    workspace_root = Some(workspace.root.clone());
    report.workspace.workspace_root = Some(workspace.root.clone());
    report.workspace.candidate_root = Some(workspace.candidate.clone());
    report.timings.workspace_creation_ms = started.elapsed().as_millis();

    progress(RoundTripStage::Copying);
    gate!(check_cancelled(cancellation, RoundTripStage::Copying));
    let started = Instant::now();
    let manifest = gate!(copy_source(
      project,
      &workspace.candidate,
      &snapshot,
      &plan_sets,
      cancellation,
    ));
    report.timings.copy_ms = started.elapsed().as_millis();
    report.workspace.copied_files = manifest.copied_files.len();

    progress(RoundTripStage::Applying);
    gate!(check_cancelled(cancellation, RoundTripStage::Applying));
    let started = Instant::now();
    gate!(apply_plan(
      &workspace.candidate,
      plan,
      &manifest,
      &mut report.application,
      cancellation,
    ));
    report.timings.plan_application_ms = started.elapsed().as_millis();

    progress(RoundTripStage::FilesystemVerification);
    gate!(check_cancelled(cancellation, RoundTripStage::FilesystemVerification));
    let started = Instant::now();
    gate!(verify_final_file_set(&workspace.candidate, &manifest));
    report.application.final_file_set_verified = true;
    report.timings.filesystem_verification_ms = started.elapsed().as_millis();

    progress(RoundTripStage::Reloading);
    gate!(check_cancelled(cancellation, RoundTripStage::Reloading));
    let started = Instant::now();
    let (candidate_project, land_province_ids) = gate!(reload_project(&workspace.candidate));
    report.timings.project_reload_ms = started.elapsed().as_millis();
    report.reload = ProjectReloadResult {
      files_found: candidate_project.load_summary.files_found,
      valid_states: candidate_project.load_summary.valid_states,
      invalid_states: candidate_project.load_summary.invalid_states,
      indexed_states: candidate_project.load_summary.indexed_states,
      errors: candidate_project.load_summary.errors,
      warnings: candidate_project.load_summary.warnings,
    };

    progress(RoundTripStage::SemanticComparison);
    gate!(check_cancelled(cancellation, RoundTripStage::SemanticComparison));
    let started = Instant::now();
    report.semantic_comparison =
      compare_semantics(project, edit, plan, &candidate_project, &land_province_ids);
    report.timings.semantic_comparison_ms = started.elapsed().as_millis();
    if !report.semantic_comparison.states_match
      || !report.semantic_comparison.indexes_match
      || !report.semantic_comparison.province_coverage_match
    {
      gate!(Err(Failure::new(
        RoundTripStage::SemanticComparison,
        None,
        None,
        "The reloaded candidate does not match the current working session.",
        "Inspect the listed semantic differences and regenerate the patch preview.",
      )));
    }

    progress(RoundTripStage::ByteComparison);
    gate!(check_cancelled(cancellation, RoundTripStage::ByteComparison));
    let started = Instant::now();
    report.byte_comparison =
      gate!(compare_bytes(project, plan, &workspace.candidate, &snapshot, &manifest));
    report.timings.byte_comparison_ms = started.elapsed().as_millis();

    progress(RoundTripStage::DiagnosticComparison);
    gate!(check_cancelled(cancellation, RoundTripStage::DiagnosticComparison));
    let started = Instant::now();
    report.diagnostics_comparison =
      compare_diagnostics(project, edit, plan, &candidate_project);
    report.timings.diagnostic_comparison_ms = started.elapsed().as_millis();
    if report.diagnostics_comparison.unexpected_diagnostics != 0 {
      gate!(Err(Failure::new(
        RoundTripStage::DiagnosticComparison,
        None,
        None,
        "The candidate introduced unexpected structural diagnostics.",
        "Inspect candidate diagnostics and regenerate or revise the patch plan.",
      )));
    }

    gate!(verify_source_unchanged(project, &snapshot));
    report.source_verification.source_unchanged_after_validation = true;
    report.status = if plan_sets.has_review_required {
      RoundTripStatus::PassedWithReview
    } else {
      RoundTripStatus::Passed
    };
    report.eligible_for_atomic_save_preparation =
      report.status == RoundTripStatus::Passed && plan.summary.review_required_files == 0;
    finish_report(
      &self.policy,
      &mut report,
      workspace_root.as_deref(),
      total_started,
      &mut progress,
    );
    report
  }
}

#[derive(Debug)]
struct Failure {
  diagnostic: RoundTripDiagnostic,
  cancelled: bool,
}

impl Failure {
  fn new(
    stage: RoundTripStage,
    path: Option<PathBuf>,
    state_id: Option<u32>,
    message: impl Into<String>,
    action: impl Into<String>,
  ) -> Self {
    Self {
      diagnostic: RoundTripDiagnostic {
        stage,
        path,
        state_id,
        message: message.into(),
        action: action.into(),
      },
      cancelled: false,
    }
  }

  fn io(stage: RoundTripStage, operation: &str, path: &Path, source: std::io::Error) -> Self {
    Self::new(
      stage,
      Some(path.to_owned()),
      None,
      format!("{operation} failed: {source}. The source project was not modified."),
      "Check free space, permissions, path length, antivirus locks, and retry.",
    )
  }

  fn cancelled(stage: RoundTripStage) -> Self {
    Self {
      diagnostic: RoundTripDiagnostic {
        stage,
        path: None,
        state_id: None,
        message: "Round-trip validation was cancelled between safe stages.".to_owned(),
        action: "Regenerate or run validation again when ready.".to_owned(),
      },
      cancelled: true,
    }
  }
}

#[derive(Debug)]
struct PlanSets {
  modified: BTreeSet<PathBuf>,
  created: BTreeSet<PathBuf>,
  removed: BTreeSet<PathBuf>,
  has_review_required: bool,
}

#[derive(Debug)]
struct SourceSnapshot {
  files: BTreeMap<PathBuf, FileFingerprint>,
}

#[derive(Debug)]
struct WorkspacePaths {
  root: PathBuf,
  candidate: PathBuf,
}

fn check_cancelled(
  cancellation: &RoundTripCancellation,
  stage: RoundTripStage,
) -> Result<(), Failure> {
  if cancellation.is_cancelled() {
    Err(Failure::cancelled(stage))
  } else {
    Ok(())
  }
}

fn validate_plan(
  project: &Hoi4Project,
  edit: &StateEditSession,
  plan: &ProjectPatchPlan,
  policy: &RoundTripValidationPolicy,
) -> Result<PlanSets, Failure> {
  if plan.is_stale(edit.revision()) {
    return Err(Failure::new(
      RoundTripStage::Preflight,
      None,
      None,
      "Patch preview is outdated.",
      "Regenerate it before round-trip validation.",
    ));
  }
  if !project.paths.root.is_dir() {
    return Err(Failure::new(
      RoundTripStage::Preflight,
      Some(project.paths.root.clone()),
      None,
      "The source project is no longer accessible.",
      "Restore access and regenerate the patch preview.",
    ));
  }

  let mut modified = BTreeSet::new();
  let mut created = BTreeSet::new();
  let mut removed = BTreeSet::new();
  let mut case_keys = BTreeMap::<String, &'static str>::new();
  let mut has_blocked = false;
  let mut has_review_required = false;

  for (kind, path, safety) in plan.modified_files.iter()
    .map(|file| ("modified", &file.path, file.safety))
    .chain(plan.created_files.iter().map(|file| ("created", &file.path, file.safety)))
    .chain(plan.removed_files.iter().map(|file| ("removed", &file.path, file.safety)))
  {
    validate_state_relative_path(path)?;
    has_blocked |= safety == PatchSafety::Blocked;
    has_review_required |= safety == PatchSafety::ReviewRequired;
    let key = path_key(path);
    if let Some(previous) = case_keys.insert(key, kind) {
      return Err(Failure::new(
        RoundTripStage::Preflight,
        Some(path.clone()),
        None,
        format!("Path collision: the same candidate path is both {previous} and {kind}."),
        "Regenerate the patch plan after resolving duplicate paths.",
      ));
    }
    let inserted = match kind {
      "modified" => modified.insert(path.clone()),
      "created" => created.insert(path.clone()),
      "removed" => removed.insert(path.clone()),
      _ => false,
    };
    if !inserted {
      return Err(Failure::new(
        RoundTripStage::Preflight,
        Some(path.clone()),
        None,
        format!("Duplicate {kind} path in patch plan."),
        "Regenerate the patch plan.",
      ));
    }
  }

  for file in &plan.modified_files {
    for operation in &file.operations {
      has_blocked |= operation.safety() == PatchSafety::Blocked;
      has_review_required |= operation.safety() == PatchSafety::ReviewRequired;
    }
  }
  has_blocked |= plan.diagnostics.iter().any(|diagnostic| diagnostic.safety == PatchSafety::Blocked);
  has_review_required |= plan.diagnostics.iter()
    .any(|diagnostic| diagnostic.safety == PatchSafety::ReviewRequired);

  if has_blocked {
    return Err(Failure::new(
      RoundTripStage::Preflight,
      None,
      None,
      "The patch plan contains blocked operations.",
      "Resolve every Blocked diagnostic and regenerate the preview.",
    ));
  }
  if has_review_required && !policy.allow_review_required {
    return Err(Failure::new(
      RoundTripStage::Preflight,
      None,
      None,
      "Validation not executed: the patch plan contains operations requiring review.",
      "Use the explicit isolated ReviewRequired validation option or revise the plan.",
    ));
  }

  Ok(PlanSets {
    modified,
    created,
    removed,
    has_review_required,
  })
}

fn validate_state_relative_path(path: &Path) -> Result<(), Failure> {
  if path.is_absolute() {
    return Err(unsafe_path(path, "absolute paths are not allowed"));
  }
  let components = path.components().collect::<Vec<_>>();
  if components.iter().any(|component| {
    matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))
  }) {
    return Err(unsafe_path(path, "path traversal or drive prefixes are not allowed"));
  }
  let normal = components.iter().filter_map(|component| match component {
    Component::Normal(value) => Some(value),
    Component::CurDir => None,
    _ => None,
  }).collect::<Vec<_>>();
  if normal.len() != 3
    || !normal[0].eq_ignore_ascii_case("history")
    || !normal[1].eq_ignore_ascii_case("states")
    || Path::new(normal[2]).extension().is_none_or(|ext| !ext.eq_ignore_ascii_case("txt"))
  {
    return Err(unsafe_path(path, "candidate state paths must be direct .txt files under history/states"));
  }
  Ok(())
}

fn unsafe_path(path: &Path, reason: &str) -> Failure {
  Failure::new(
    RoundTripStage::Preflight,
    Some(path.to_owned()),
    None,
    format!("Unsafe candidate path: {reason}."),
    "Regenerate the patch plan with a normalized history/states/*.txt path.",
  )
}

pub fn resolve_candidate_path(
  candidate_root: &Path,
  relative_path: &Path,
) -> Result<PathBuf, String> {
  validate_state_relative_path(relative_path).map_err(|failure| failure.diagnostic.message)?;
  let canonical_root = candidate_root.canonicalize()
    .map_err(|err| format!("failed to canonicalize candidate root: {err}"))?;
  let target = canonical_root.join(relative_path);
  let parent = target.parent().ok_or_else(|| "candidate path has no parent".to_owned())?;
  let canonical_parent = parent.canonicalize()
    .map_err(|err| format!("failed to canonicalize candidate parent: {err}"))?;
  if !path_is_within(&canonical_parent, &canonical_root) {
    return Err("candidate path escapes the temporary workspace".to_owned());
  }
  if target.exists() {
    let canonical_target = target.canonicalize()
      .map_err(|err| format!("failed to canonicalize candidate path: {err}"))?;
    if !path_is_within(&canonical_target, &canonical_root) {
      return Err("candidate path resolves outside the temporary workspace".to_owned());
    }
  }
  Ok(target)
}

fn verify_source(
  project: &Hoi4Project,
  plan: &ProjectPatchPlan,
  plan_sets: &PlanSets,
) -> Result<SourceSnapshot, Failure> {
  let files = enumerate_source_files(project)?;
  for file in &plan.modified_files {
    let source = project.paths.root.join(&file.path);
    let bytes = read(&source, RoundTripStage::SourceVerification, "read planned source")?;
    if bytes != file.before {
      return Err(source_changed(&source));
    }
    let Some(expected) = plan.source_fingerprints.get(&source) else {
      return Err(Failure::new(
        RoundTripStage::SourceVerification,
        Some(source),
        Some(file.state_id),
        "The patch plan has no fingerprint for a modified source file.",
        "Regenerate the patch preview.",
      ));
    };
    if SourceFingerprint::from_bytes(&bytes) != *expected {
      return Err(source_changed(&source));
    }
  }
  for file in &plan.removed_files {
    let source = project.paths.root.join(&file.path);
    let bytes = read(&source, RoundTripStage::SourceVerification, "read planned removal source")?;
    if bytes != file.before {
      return Err(source_changed(&source));
    }
    let Some(expected) = plan.source_fingerprints.get(&source) else {
      return Err(Failure::new(
        RoundTripStage::SourceVerification,
        Some(source),
        Some(file.state_id),
        "The patch plan has no fingerprint for a removed source file.",
        "Regenerate the patch preview.",
      ));
    };
    if SourceFingerprint::from_bytes(&bytes) != *expected {
      return Err(source_changed(&source));
    }
  }

  let existing_keys = files.keys().map(|path| path_key(path)).collect::<BTreeSet<_>>();
  for path in &plan_sets.created {
    if existing_keys.contains(&path_key(path)) {
      return Err(Failure::new(
        RoundTripStage::SourceVerification,
        Some(project.paths.root.join(path)),
        None,
        "A planned creation collides with an existing source file.",
        "Choose another state ID or filename and regenerate the plan.",
      ));
    }
  }
  Ok(SourceSnapshot { files })
}

fn enumerate_source_files(
  project: &Hoi4Project,
) -> Result<BTreeMap<PathBuf, FileFingerprint>, Failure> {
  let mut files = BTreeMap::new();
  for relative in [PathBuf::from("map/provinces.bmp"), PathBuf::from("map/definition.csv")] {
    let bytes = read(
      &project.paths.root.join(&relative),
      RoundTripStage::SourceVerification,
      "read map source",
    )?;
    files.insert(relative, FileFingerprint::from_bytes(&bytes));
  }
  let entries = fs::read_dir(&project.paths.states_directory)
    .map_err(|err| Failure::io(
      RoundTripStage::SourceVerification,
      "enumerate state sources",
      &project.paths.states_directory,
      err,
    ))?;
  for entry in entries {
    let entry = entry.map_err(|err| Failure::io(
      RoundTripStage::SourceVerification,
      "read state directory entry",
      &project.paths.states_directory,
      err,
    ))?;
    let path = entry.path();
    if !path.is_file() || path.extension().is_none_or(|ext| !ext.eq_ignore_ascii_case("txt")) {
      continue;
    }
    let relative = PathBuf::from("history/states").join(entry.file_name());
    let bytes = read(&path, RoundTripStage::SourceVerification, "read state source")?;
    files.insert(relative, FileFingerprint::from_bytes(&bytes));
  }
  Ok(files)
}

fn create_workspace(source_root: &Path) -> Result<WorkspacePaths, Failure> {
  let canonical_source = source_root.canonicalize()
    .map_err(|err| Failure::io(
      RoundTripStage::WorkspaceCreation,
      "canonicalize source root",
      source_root,
      err,
    ))?;
  let source_text = canonical_source.to_string_lossy();
  if (source_text.starts_with(r"\\") && !source_text.starts_with(r"\\?\"))
    || source_text.to_ascii_lowercase().starts_with(r"\\?\unc\")
  {
    return Err(Failure::new(
      RoundTripStage::WorkspaceCreation,
      Some(canonical_source),
      None,
      "UNC source roots are not supported when workspace separation cannot be proven.",
      "Use a local project copy for round-trip validation.",
    ));
  }
  let base = std::env::temp_dir().join("hoi4-state-editor").join("roundtrip");
  fs::create_dir_all(&base)
    .map_err(|err| Failure::io(RoundTripStage::WorkspaceCreation, "create workspace base", &base, err))?;

  let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)
    .map(|duration| duration.as_nanos())
    .unwrap_or_default();
  let mut root = None;
  for _ in 0..32 {
    let counter = WORKSPACE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let candidate = base.join(format!("{}-{timestamp:x}-{counter:x}", std::process::id()));
    match fs::create_dir(&candidate) {
      Ok(()) => {
        root = Some(candidate);
        break;
      },
      Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {},
      Err(err) => {
        return Err(Failure::io(
          RoundTripStage::WorkspaceCreation,
          "create unique workspace",
          &candidate,
          err,
        ));
      },
    }
  }
  let root = root.ok_or_else(|| Failure::new(
    RoundTripStage::WorkspaceCreation,
    Some(base),
    None,
    "Unable to allocate a unique temporary workspace.",
    "Clean stale temporary validation directories and retry.",
  ))?;
  let candidate = root.join("candidate");
  for directory in [candidate.join("map"), candidate.join("history/states")] {
    fs::create_dir_all(&directory)
      .map_err(|err| Failure::io(RoundTripStage::WorkspaceCreation, "create candidate directory", &directory, err))?;
  }
  let canonical_root = root.canonicalize()
    .map_err(|err| Failure::io(RoundTripStage::WorkspaceCreation, "canonicalize workspace", &root, err))?;
  let canonical_candidate = candidate.canonicalize()
    .map_err(|err| Failure::io(RoundTripStage::WorkspaceCreation, "canonicalize candidate", &candidate, err))?;
  if path_is_within(&canonical_root, &canonical_source)
    || path_is_within(&canonical_source, &canonical_root)
    || path_key(&canonical_root) == path_key(&canonical_source)
  {
    let _ = fs::remove_dir_all(&canonical_root);
    return Err(Failure::new(
      RoundTripStage::WorkspaceCreation,
      Some(canonical_root),
      None,
      "Temporary workspace overlaps the source project.",
      "Choose a system temporary directory outside the source project.",
    ));
  }
  Ok(WorkspacePaths {
    root: canonical_root,
    candidate: canonical_candidate,
  })
}

fn copy_source(
  project: &Hoi4Project,
  candidate_root: &Path,
  snapshot: &SourceSnapshot,
  plan_sets: &PlanSets,
  cancellation: &RoundTripCancellation,
) -> Result<TemporaryProjectManifest, Failure> {
  for (relative, fingerprint) in &snapshot.files {
    check_cancelled(cancellation, RoundTripStage::Copying)?;
    let source = project.paths.root.join(relative);
    let bytes = read(&source, RoundTripStage::Copying, "read source for copy")?;
    if FileFingerprint::from_bytes(&bytes) != *fingerprint {
      return Err(source_changed(&source));
    }
    let target = resolve_map_or_state_path(candidate_root, relative)?;
    write_new(&target, &bytes, RoundTripStage::Copying, "copy source bytes")?;
    let copied = read(&target, RoundTripStage::Copying, "verify copied bytes")?;
    if copied != bytes {
      return Err(Failure::new(
        RoundTripStage::Copying,
        Some(target),
        None,
        "Candidate baseline bytes differ from the source immediately after copy.",
        "Discard the workspace and retry after checking the filesystem.",
      ));
    }
  }
  Ok(TemporaryProjectManifest {
    source_root: project.paths.root.clone(),
    candidate_root: candidate_root.to_owned(),
    copied_files: snapshot.files.clone(),
    expected_modified_files: plan_sets.modified.clone(),
    expected_created_files: plan_sets.created.clone(),
    expected_removed_files: plan_sets.removed.clone(),
  })
}

fn apply_plan(
  candidate_root: &Path,
  plan: &ProjectPatchPlan,
  manifest: &TemporaryProjectManifest,
  application: &mut CandidateApplicationResult,
  cancellation: &RoundTripCancellation,
) -> Result<(), Failure> {
  for file in &plan.modified_files {
    check_cancelled(cancellation, RoundTripStage::Applying)?;
    let target = candidate_state_path(candidate_root, &file.path)?;
    let before = read(&target, RoundTripStage::Applying, "read candidate before modification")?;
    if before != file.before {
      return Err(Failure::new(
        RoundTripStage::Applying,
        Some(target),
        Some(file.state_id),
        "Candidate baseline does not match the planned source bytes.",
        "Discard the workspace and regenerate the patch preview.",
      ));
    }
    let Some(after) = file.after.as_ref() else {
      return Err(Failure::new(
        RoundTripStage::Applying,
        Some(target),
        Some(file.state_id),
        "A modified file has no validated preview bytes.",
        "Resolve the blocked Phase 4A patch and regenerate the plan.",
      ));
    };
    let applied = apply_operations(&before, &file.operations).map_err(|message| Failure::new(
      RoundTripStage::Applying,
      Some(target.clone()),
      Some(file.state_id),
      message,
      "Regenerate the patch preview from the unchanged source.",
    ))?;
    if &applied != after {
      return Err(Failure::new(
        RoundTripStage::Applying,
        Some(target),
        Some(file.state_id),
        "Patch operations do not reproduce the authoritative Phase 4A preview bytes.",
        "Regenerate the patch plan.",
      ));
    }
    write_existing(&target, after, RoundTripStage::Applying, "write modified candidate")?;
    application.modified_files_applied += 1;
  }
  for file in &plan.created_files {
    check_cancelled(cancellation, RoundTripStage::Applying)?;
    let target = candidate_state_path(candidate_root, &file.path)?;
    if manifest.copied_files.keys().any(|path| path_key(path) == path_key(&file.path)) {
      return Err(Failure::new(
        RoundTripStage::Applying,
        Some(target),
        Some(file.state_id),
        "Planned creation collides with the copied baseline.",
        "Choose another state ID or filename.",
      ));
    }
    write_new(&target, &file.content, RoundTripStage::Applying, "create candidate state")?;
    application.created_files_applied += 1;
  }
  for file in &plan.removed_files {
    check_cancelled(cancellation, RoundTripStage::Applying)?;
    let target = candidate_state_path(candidate_root, &file.path)?;
    let before = read(&target, RoundTripStage::Applying, "read candidate before removal")?;
    if before != file.before {
      return Err(Failure::new(
        RoundTripStage::Applying,
        Some(target),
        Some(file.state_id),
        "Candidate removal source does not match the planned bytes.",
        "Discard the workspace and regenerate the patch preview.",
      ));
    }
    fs::remove_file(&target)
      .map_err(|err| Failure::io(RoundTripStage::Applying, "remove candidate state", &target, err))?;
    application.removed_files_applied += 1;
  }
  Ok(())
}

fn verify_final_file_set(
  candidate_root: &Path,
  manifest: &TemporaryProjectManifest,
) -> Result<(), Failure> {
  let baseline = manifest.copied_files.keys()
    .filter(|path| is_state_path(path))
    .cloned()
    .collect::<BTreeSet<_>>();
  let expected = baseline.difference(&manifest.expected_removed_files)
    .cloned()
    .chain(manifest.expected_created_files.iter().cloned())
    .collect::<BTreeSet<_>>();
  let actual = enumerate_candidate_state_paths(candidate_root)?;
  if actual != expected {
    let missing = expected.difference(&actual)
      .map(|path| path.display().to_string())
      .collect::<Vec<_>>();
    let extra = actual.difference(&expected)
      .map(|path| path.display().to_string())
      .collect::<Vec<_>>();
    return Err(Failure::new(
      RoundTripStage::FilesystemVerification,
      Some(candidate_root.join("history/states")),
      None,
      format!("Candidate file set mismatch. Missing: {missing:?}; extra: {extra:?}."),
      "Discard the workspace and inspect candidate application diagnostics.",
    ));
  }
  Ok(())
}

fn reload_project(candidate_root: &Path) -> Result<(Hoi4Project, BTreeSet<u32>), Failure> {
  let config = Config {
    preserve_ids: true,
    ..Config::default()
  };
  let map_directory = candidate_root.join("map");
  let bundle = Bundle::load(&Location::Directory(map_directory.clone()), config)
    .map_err(|err| Failure::new(
      RoundTripStage::Reloading,
      Some(map_directory),
      None,
      format!("The real map loader rejected the candidate: {err}"),
      "Inspect copied map files and candidate filesystem diagnostics.",
    ))?;
  let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
  let land_province_ids = bundle.map.iter_province_data()
    .filter(|(_, province)| province.kind == ProvinceKind::Land)
    .filter_map(|(_, province)| province.preserved_id)
    .collect::<BTreeSet<_>>();
  let paths = ProjectPaths::discover(candidate_root)
    .map_err(|err| Failure::new(
      RoundTripStage::Reloading,
      Some(candidate_root.to_owned()),
      None,
      format!("Candidate project discovery failed: {err}"),
      "Inspect the temporary workspace structure.",
    ))?;
  let mut project = Hoi4Project::new(paths);
  project.load_states(&province_ids, &land_province_ids);
  Ok((project, land_province_ids))
}

pub(crate) fn reload_project_for_save(
  project_root: &Path,
) -> Result<(Hoi4Project, BTreeSet<u32>), String> {
  reload_project(project_root).map_err(|failure| {
    format!(
      "{} {}",
      failure.diagnostic.message,
      failure.diagnostic.action
    )
  })
}

fn compare_semantics(
  source: &Hoi4Project,
  edit: &StateEditSession,
  plan: &ProjectPatchPlan,
  candidate: &Hoi4Project,
  land_province_ids: &BTreeSet<u32>,
) -> ProjectSemanticComparison {
  let expected_ids = source.states_by_id.keys()
    .filter(|state_id| !edit.removed_state_ids().contains(state_id))
    .copied()
    .chain(edit.valid_state_ids().iter().copied())
    .collect::<BTreeSet<_>>();
  let actual_ids = candidate.states_by_id.keys().copied().collect::<BTreeSet<_>>();
  let mut differences = expected_ids.difference(&actual_ids)
    .copied()
    .map(SemanticDifference::MissingState)
    .chain(actual_ids.difference(&expected_ids).copied().map(SemanticDifference::UnexpectedState))
    .collect::<Vec<_>>();
  let mut victory_points_match = true;
  let mut buildings_match = true;
  for &state_id in expected_ids.intersection(&actual_ids) {
    let expected = edit.state_data(state_id).or_else(|| {
      source.state_document(state_id).and_then(|document| document.data.clone())
    });
    let Some(expected) = expected else { continue };
    let Some(actual) = candidate.state_document(state_id).and_then(|document| document.data.as_ref()) else {
      continue;
    };
    let state_differences = compare_state(state_id, &expected, actual);
    victory_points_match &= !state_differences.iter().any(|difference| {
      matches!(difference, SemanticDifference::PropertyMismatch { property, .. } if property == "victory_points")
    });
    buildings_match &= !state_differences.iter().any(|difference| {
      matches!(difference, SemanticDifference::PropertyMismatch { property, .. }
        if property == "state_buildings" || property == "province_buildings")
    });
    differences.extend(state_differences);
  }

  let province_ids = edit.state_by_province().keys().copied()
    .chain(candidate.state_by_province.keys().copied())
    .collect::<BTreeSet<_>>();
  for province_id in province_ids {
    let expected = edit.state_by_province().get(&province_id).copied();
    let actual = candidate.state_by_province.get(&province_id).copied();
    if expected != actual {
      differences.push(SemanticDifference::ProvinceAssignmentMismatch {
        province_id,
        expected,
        actual,
      });
    }
  }
  let indexes_match = edit.state_by_province() == &candidate.state_by_province
    && edit.unassigned_land_provinces() == &candidate.unassigned_land_provinces
    && source.ambiguous_provinces == candidate.ambiguous_provinces;
  let assigned_land = edit.state_by_province().keys()
    .filter(|province_id| land_province_ids.contains(province_id))
    .copied()
    .collect::<BTreeSet<_>>();
  let coverage = assigned_land.union(edit.unassigned_land_provinces())
    .copied()
    .collect::<BTreeSet<_>>();
  let source_non_land = source.state_by_province.keys()
    .filter(|province_id| !land_province_ids.contains(province_id))
    .copied()
    .collect::<BTreeSet<_>>();
  let working_non_land = edit.state_by_province().keys()
    .filter(|province_id| !land_province_ids.contains(province_id))
    .copied()
    .collect::<BTreeSet<_>>();
  let province_coverage_match = coverage == *land_province_ids
    && working_non_land == source_non_land;
  let removed_states_match = plan.removed_files.iter()
    .all(|file| !candidate.states_by_id.contains_key(&file.state_id));
  let created_states_match = plan.created_files.iter()
    .all(|file| candidate.states_by_id.contains_key(&file.state_id));
  let states_match = expected_ids == actual_ids
    && expected_ids.iter().all(|state_id| {
      let expected = edit.state_data(*state_id).or_else(|| {
        source.state_document(*state_id).and_then(|document| document.data.clone())
      });
      let Some(expected) = expected else { return false };
      let Some(actual) = candidate.state_document(*state_id).and_then(|document| document.data.as_ref()) else {
        return false;
      };
      compare_state(*state_id, &expected, actual).is_empty()
    })
    && created_states_match
    && removed_states_match;

  ProjectSemanticComparison {
    states_match,
    indexes_match,
    province_coverage_match,
    victory_points_match,
    buildings_match,
    created_states_match,
    removed_states_match,
    differences,
  }
}

pub(crate) fn compare_project_for_save(
  source: &Hoi4Project,
  edit: &StateEditSession,
  plan: &ProjectPatchPlan,
  candidate: &Hoi4Project,
  land_province_ids: &BTreeSet<u32>,
) -> (ProjectSemanticComparison, DiagnosticComparison) {
  (
    compare_semantics(source, edit, plan, candidate, land_province_ids),
    compare_diagnostics(source, edit, plan, candidate),
  )
}

fn compare_state(state_id: u32, expected: &StateData, actual: &StateData) -> Vec<SemanticDifference> {
  let mut differences = Vec::new();
  macro_rules! compare {
    ($field:ident) => {
      if expected.$field != actual.$field {
        differences.push(property_difference(
          state_id,
          stringify!($field),
          &expected.$field,
          &actual.$field,
        ));
      }
    };
  }
  compare!(id);
  compare!(name);
  compare!(provinces);
  compare!(manpower);
  compare!(buildings_max_level_factor);
  compare!(state_category);
  compare!(local_supplies);
  if expected.impassable.unwrap_or(false) != actual.impassable.unwrap_or(false) {
    differences.push(property_difference(
      state_id,
      "impassable",
      &expected.impassable.unwrap_or(false),
      &actual.impassable.unwrap_or(false),
    ));
  }
  compare!(resources);
  macro_rules! compare_history {
    ($field:ident) => {
      if expected.history.$field != actual.history.$field {
        differences.push(property_difference(
          state_id,
          stringify!($field),
          &expected.history.$field,
          &actual.history.$field,
        ));
      }
    };
  }
  compare_history!(owner);
  compare_history!(controller);
  compare_history!(cores);
  compare_history!(claims);
  compare_history!(state_buildings);
  compare_history!(province_buildings);
  if normalized_victory_points(&expected.history.victory_points)
    != normalized_victory_points(&actual.history.victory_points)
  {
    differences.push(property_difference(
      state_id,
      "victory_points",
      &normalized_victory_points(&expected.history.victory_points),
      &normalized_victory_points(&actual.history.victory_points),
    ));
  }
  differences
}

fn property_difference(
  state_id: u32,
  property: &str,
  expected: &impl std::fmt::Debug,
  actual: &impl std::fmt::Debug,
) -> SemanticDifference {
  SemanticDifference::PropertyMismatch {
    state_id,
    property: property.to_owned(),
    expected: format!("{expected:?}"),
    actual: format!("{actual:?}"),
  }
}

fn normalized_victory_points(values: &[VictoryPoint]) -> Vec<(u32, i64)> {
  let mut values = values.iter()
    .map(|value| (value.province_id, value.value))
    .collect::<Vec<_>>();
  values.sort_unstable();
  values
}

fn compare_bytes(
  project: &Hoi4Project,
  plan: &ProjectPatchPlan,
  candidate_root: &Path,
  snapshot: &SourceSnapshot,
  manifest: &TemporaryProjectManifest,
) -> Result<ByteComparisonResult, Failure> {
  let mut result = ByteComparisonResult::default();
  for relative in snapshot.files.keys() {
    let source_path = project.paths.root.join(relative);
    let source = read(&source_path, RoundTripStage::ByteComparison, "read source for byte comparison")?;
    if manifest.expected_removed_files.contains(relative) {
      let candidate = candidate_root.join(relative);
      if candidate.exists() {
        return Err(Failure::new(
          RoundTripStage::ByteComparison,
          Some(candidate),
          None,
          "A planned removal still exists in the candidate.",
          "Inspect candidate application.",
        ));
      }
      result.removed_files_verified += 1;
      continue;
    }
    let candidate_path = resolve_map_or_state_path(candidate_root, relative)?;
    let candidate = read(
      &candidate_path,
      RoundTripStage::ByteComparison,
      "read candidate for byte comparison",
    )?;
    if manifest.expected_modified_files.contains(relative) {
      let expected = plan.modified_files.iter()
        .find(|file| file.path == *relative)
        .and_then(|file| file.after.as_ref())
        .ok_or_else(|| Failure::new(
          RoundTripStage::ByteComparison,
          Some(candidate_path.clone()),
          None,
          "No authoritative preview bytes exist for a modified file.",
          "Regenerate the patch preview.",
        ))?;
      if &candidate != expected {
        result.differences.push(byte_difference(relative, expected, &candidate));
      }
      result.modified_files_verified += 1;
    } else {
      if candidate != source {
        result.differences.push(byte_difference(relative, &source, &candidate));
      }
      result.unchanged_files_verified += 1;
    }
  }
  for file in &plan.created_files {
    let candidate_path = candidate_state_path(candidate_root, &file.path)?;
    let candidate = read(&candidate_path, RoundTripStage::ByteComparison, "read created candidate")?;
    if candidate != file.content {
      result.differences.push(byte_difference(&file.path, &file.content, &candidate));
    }
    result.created_files_verified += 1;
  }
  result.map_files_unchanged = result.differences.iter()
    .all(|difference| !difference.path.starts_with("map"));
  if !result.differences.is_empty() {
    let first = &result.differences[0];
    return Err(Failure::new(
      RoundTripStage::ByteComparison,
      Some(first.path.clone()),
      None,
      format!(
        "Unexpected byte difference at offset {} (expected length {}, actual length {}).",
        first.first_different_offset,
        first.source_len,
        first.candidate_len,
      ),
      "Discard the workspace and inspect candidate application.",
    ));
  }
  Ok(result)
}

fn compare_diagnostics(
  source: &Hoi4Project,
  edit: &StateEditSession,
  plan: &ProjectPatchPlan,
  candidate: &Hoi4Project,
) -> DiagnosticComparison {
  let source_structural = diagnostic_counts(source, true);
  let candidate_structural = diagnostic_counts(candidate, true);
  let unexpected_structural = candidate_structural.iter()
    .map(|(key, &count)| count.saturating_sub(*source_structural.get(key).unwrap_or(&0)))
    .sum::<usize>();
  let candidate_unassigned = candidate.diagnostics.iter()
    .filter(|diagnostic| diagnostic.kind == ProjectDiagnosticKind::LandProvinceWithoutState)
    .count();
  let expected_unassigned = edit.unassigned_land_provinces().len();
  let unassigned_difference = usize::from(candidate_unassigned != expected_unassigned);
  let planned_paths = plan.modified_files.iter().map(|file| path_key(&file.path))
    .chain(plan.created_files.iter().map(|file| path_key(&file.path)))
    .collect::<BTreeSet<_>>();
  let expected_session_diagnostics = candidate.diagnostics.iter()
    .filter(|diagnostic| diagnostic.path.as_ref().is_some_and(|path| {
      planned_paths.contains(&path_key(
        path.strip_prefix(&candidate.paths.root).unwrap_or(path)
      ))
    }))
    .count();
  DiagnosticComparison {
    source_diagnostics: source.diagnostics.len(),
    candidate_diagnostics: candidate.diagnostics.len(),
    preserved_original_diagnostics: source_structural.values().sum(),
    expected_session_diagnostics,
    unexpected_diagnostics: unexpected_structural + unassigned_difference,
  }
}

fn diagnostic_counts(
  project: &Hoi4Project,
  structural_only: bool,
) -> BTreeMap<(ProjectDiagnosticKind, DiagnosticSeverity), usize> {
  let mut counts = BTreeMap::new();
  for diagnostic in &project.diagnostics {
    if structural_only && !is_structural_diagnostic(diagnostic.kind) {
      continue;
    }
    *counts.entry((diagnostic.kind, diagnostic.severity)).or_default() += 1;
  }
  counts
}

fn is_structural_diagnostic(kind: ProjectDiagnosticKind) -> bool {
  matches!(
    kind,
    ProjectDiagnosticKind::InvalidStateFile
      | ProjectDiagnosticKind::EmptyStateFile
      | ProjectDiagnosticKind::SyntaxError
      | ProjectDiagnosticKind::MissingStateBlock
      | ProjectDiagnosticKind::MultipleStateBlocks
      | ProjectDiagnosticKind::MissingStateId
      | ProjectDiagnosticKind::InvalidStateId
      | ProjectDiagnosticKind::ZeroStateId
      | ProjectDiagnosticKind::DuplicateStateId
      | ProjectDiagnosticKind::ProvinceInMultipleStates
      | ProjectDiagnosticKind::UnknownProvince
  )
}

fn verify_source_unchanged(
  project: &Hoi4Project,
  snapshot: &SourceSnapshot,
) -> Result<(), Failure> {
  for (relative, fingerprint) in &snapshot.files {
    let path = project.paths.root.join(relative);
    let bytes = read(&path, RoundTripStage::SourceVerification, "re-read source after validation")?;
    if FileFingerprint::from_bytes(&bytes) != *fingerprint {
      return Err(Failure::new(
        RoundTripStage::SourceVerification,
        Some(path),
        None,
        "A source file changed during temporary validation.",
        "Discard the result and regenerate the patch preview from the current source.",
      ));
    }
  }
  Ok(())
}

fn finish_report<F>(
  policy: &RoundTripValidationPolicy,
  report: &mut RoundTripValidationReport,
  workspace_root: Option<&Path>,
  total_started: Instant,
  progress: &mut F,
)
where
  F: FnMut(RoundTripStage),
{
  if let Some(workspace_root) = workspace_root {
    let retain = report.status == RoundTripStatus::Failed && policy.retain_failed_workspace;
    report.workspace.retained = retain;
    if !retain {
      progress(RoundTripStage::Cleanup);
      let started = Instant::now();
      match fs::remove_dir_all(workspace_root) {
        Ok(()) => report.workspace.cleaned = true,
        Err(err) => {
          report.status = RoundTripStatus::Failed;
          report.workspace.retained = true;
          report.diagnostics.push(RoundTripDiagnostic {
            stage: RoundTripStage::Cleanup,
            path: Some(workspace_root.to_owned()),
            state_id: None,
            message: format!(
              "Failed to remove temporary workspace: {err}. The source project was not modified."
            ),
            action: "Remove this temporary validation directory manually when it is no longer needed."
              .to_owned(),
          });
        },
      }
      report.timings.cleanup_ms = started.elapsed().as_millis();
    }
  }
  report.timings.total_ms = total_started.elapsed().as_millis();
}

fn source_changed(path: &Path) -> Failure {
  Failure::new(
    RoundTripStage::SourceVerification,
    Some(path.to_owned()),
    None,
    "Source changed after patch planning.",
    "Regenerate the patch preview before validation.",
  )
}

fn read(path: &Path, stage: RoundTripStage, operation: &str) -> Result<Vec<u8>, Failure> {
  fs::read(path).map_err(|err| Failure::io(stage, operation, path, err))
}

fn write_new(
  path: &Path,
  bytes: &[u8],
  stage: RoundTripStage,
  operation: &str,
) -> Result<(), Failure> {
  let mut file = OpenOptions::new()
    .write(true)
    .create_new(true)
    .open(path)
    .map_err(|err| Failure::io(stage, operation, path, err))?;
  file.write_all(bytes)
    .and_then(|_| file.sync_all())
    .map_err(|err| Failure::io(stage, operation, path, err))
}

fn write_existing(
  path: &Path,
  bytes: &[u8],
  stage: RoundTripStage,
  operation: &str,
) -> Result<(), Failure> {
  let mut file = OpenOptions::new()
    .write(true)
    .truncate(true)
    .open(path)
    .map_err(|err| Failure::io(stage, operation, path, err))?;
  file.write_all(bytes)
    .and_then(|_| file.sync_all())
    .map_err(|err| Failure::io(stage, operation, path, err))
}

fn resolve_map_or_state_path(candidate_root: &Path, relative: &Path) -> Result<PathBuf, Failure> {
  if is_state_path(relative) {
    candidate_state_path(candidate_root, relative)
  } else if relative == Path::new("map/provinces.bmp")
    || relative == Path::new("map/definition.csv")
  {
    let root = candidate_root.canonicalize()
      .map_err(|err| Failure::io(
        RoundTripStage::Copying,
        "canonicalize candidate root",
        candidate_root,
        err,
      ))?;
    let target = root.join(relative);
    let parent = target.parent().unwrap_or(&root).canonicalize()
      .map_err(|err| Failure::io(RoundTripStage::Copying, "canonicalize map parent", &target, err))?;
    if path_is_within(&parent, &root) {
      Ok(target)
    } else {
      Err(unsafe_path(relative, "map path escapes the candidate root"))
    }
  } else {
    Err(unsafe_path(relative, "only the two map files and direct state files are supported"))
  }
}

fn candidate_state_path(candidate_root: &Path, relative: &Path) -> Result<PathBuf, Failure> {
  resolve_candidate_path(candidate_root, relative)
    .map_err(|message| Failure::new(
      RoundTripStage::Preflight,
      Some(relative.to_owned()),
      None,
      message,
      "Regenerate the patch plan with a safe relative path.",
    ))
}

fn enumerate_candidate_state_paths(candidate_root: &Path) -> Result<BTreeSet<PathBuf>, Failure> {
  let states = candidate_root.join("history/states");
  let entries = fs::read_dir(&states)
    .map_err(|err| Failure::io(
      RoundTripStage::FilesystemVerification,
      "enumerate candidate states",
      &states,
      err,
    ))?;
  let mut paths = BTreeSet::new();
  for entry in entries {
    let entry = entry.map_err(|err| Failure::io(
      RoundTripStage::FilesystemVerification,
      "read candidate state entry",
      &states,
      err,
    ))?;
    let path = entry.path();
    if path.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("txt")) {
      paths.insert(PathBuf::from("history/states").join(entry.file_name()));
    }
  }
  Ok(paths)
}

fn byte_difference(path: &Path, expected: &[u8], actual: &[u8]) -> ByteDifference {
  let first_different_offset = expected.iter()
    .zip(actual)
    .position(|(left, right)| left != right)
    .unwrap_or_else(|| expected.len().min(actual.len()));
  ByteDifference {
    path: path.to_owned(),
    first_different_offset,
    source_len: expected.len(),
    candidate_len: actual.len(),
  }
}

fn path_is_within(path: &Path, root: &Path) -> bool {
  let path = path.to_string_lossy().replace('\\', "/").to_lowercase();
  let root = root.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
  path == root || path.starts_with(&format!("{root}/"))
}

fn path_key(path: &Path) -> String {
  path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn is_state_path(path: &Path) -> bool {
  let key = path_key(path);
  key.starts_with("history/states/") && key.ends_with(".txt")
}

fn pass_fail(value: bool) -> &'static str {
  if value { "passed" } else { "failed" }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::project::{
    EditableStateProperties, PatchPlanSummary, PatchPlanTimings, PlannedFileCreation,
    PlannedFileModification, plan_state_patches,
  };

  fn empty_plan() -> ProjectPatchPlan {
    ProjectPatchPlan {
      generation: 0,
      source_fingerprints: BTreeMap::new(),
      modified_files: Vec::new(),
      created_files: Vec::new(),
      removed_files: Vec::new(),
      diagnostics: Vec::new(),
      summary: PatchPlanSummary::default(),
      timings: PatchPlanTimings::default(),
    }
  }

  #[test]
  fn candidate_path_rejects_traversal_before_joining() {
    let root = std::env::temp_dir().join(format!("phase4b-path-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("history/states")).unwrap();
    assert!(resolve_candidate_path(&root, Path::new("../outside.txt")).is_err());
    assert!(resolve_candidate_path(&root, Path::new(r"C:\outside.txt")).is_err());
    assert!(resolve_candidate_path(&root, Path::new("history/states/1-Test.bin")).is_err());
    assert!(resolve_candidate_path(&root, Path::new("history/states/1-Test.txt")).is_ok());
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn blocked_and_review_required_plans_are_refused_by_default() {
    let root = std::env::temp_dir().join(format!("phase4b-preflight-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("map")).unwrap();
    fs::create_dir_all(root.join("history/states")).unwrap();
    fs::write(root.join("map/provinces.bmp"), []).unwrap();
    fs::write(root.join("map/definition.csv"), []).unwrap();
    let project = Hoi4Project::new(ProjectPaths::discover(&root).unwrap());
    let map = test_map();
    let edit = StateEditSession::new(&project, &map);

    let mut blocked = empty_plan();
    blocked.created_files.push(PlannedFileCreation {
      path: PathBuf::from("history/states/1-Test.txt"),
      state_id: 1,
      content: Vec::new(),
      unified_diff: String::new(),
      semantic_changes: Vec::new(),
      diagnostics: Vec::new(),
      safety: PatchSafety::Blocked,
    });
    blocked.summary.blocked_files = 1;
    assert!(validate_plan(&project, &edit, &blocked, &Default::default()).is_err());
    let report = RoundTripValidator::default().validate(
      &project,
      &edit,
      &blocked,
      &RoundTripCancellation::default(),
      |_| {},
    );
    assert_eq!(report.status, RoundTripStatus::Failed);
    assert!(report.workspace.workspace_root.is_none());

    blocked.created_files[0].safety = PatchSafety::ReviewRequired;
    blocked.summary.blocked_files = 0;
    blocked.summary.review_required_files = 1;
    assert!(validate_plan(&project, &edit, &blocked, &Default::default()).is_err());
    let policy = RoundTripValidationPolicy {
      allow_review_required: true,
      ..Default::default()
    };
    assert!(validate_plan(&project, &edit, &blocked, &policy).is_ok());

    blocked.created_files.push(PlannedFileCreation {
      path: PathBuf::from("history/states/1-test.TXT"),
      state_id: 2,
      content: Vec::new(),
      unified_diff: String::new(),
      semantic_changes: Vec::new(),
      diagnostics: Vec::new(),
      safety: PatchSafety::Safe,
    });
    assert!(validate_plan(&project, &edit, &blocked, &policy).is_err());
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn source_change_is_detected_before_workspace_creation() {
    let (root, project, edit) = test_project("source-change");
    let document = project.state_document(1).unwrap();
    let before = document.original_bytes().to_vec();
    let mut plan = empty_plan();
    plan.source_fingerprints.insert(
      document.path.clone(),
      SourceFingerprint::from_bytes(&before),
    );
    plan.modified_files.push(PlannedFileModification {
      path: PathBuf::from("history/states/1-Test.txt"),
      state_id: 1,
      operations: Vec::new(),
      before,
      after: None,
      unified_diff: String::new(),
      semantic_changes: Vec::new(),
      diagnostics: Vec::new(),
      safety: PatchSafety::Safe,
    });
    plan.summary.modified_files = 1;
    plan.summary.safe_files = 1;
    fs::write(root.join("history/states/1-Test.txt"), "state={id=1 provinces={1} manpower=2}").unwrap();
    let report = RoundTripValidator::default().validate(
      &project,
      &edit,
      &plan,
      &RoundTripCancellation::default(),
      |_| {},
    );
    assert_eq!(report.status, RoundTripStatus::Failed);
    assert!(report.workspace.workspace_root.is_none());
    assert!(report.full_text().contains("Source changed after patch planning"));
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn empty_current_plan_passes_without_workspace() {
    let (root, project, edit) = test_project("empty");
    let report = RoundTripValidator::default().validate(
      &project,
      &edit,
      &empty_plan(),
      &RoundTripCancellation::default(),
      |_| {},
    );
    assert_eq!(report.status, RoundTripStatus::Passed);
    assert!(report.no_candidate_changes);
    assert!(report.workspace.workspace_root.is_none());
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn modified_candidate_roundtrips_and_leaves_source_unchanged() {
    let (root, project, mut edit) = test_project("roundtrip");
    let source_path = root.join("history/states/1-Test.txt");
    let source_before = fs::read(&source_path).unwrap();
    let mut properties = EditableStateProperties::from_state(
      project.state_document(1).unwrap().data.as_ref().unwrap()
    );
    properties.manpower = Some(2);
    assert!(edit.update_state_properties(1, properties).unwrap());
    let plan = plan_state_patches(&project, &edit);
    assert_eq!(plan.summary.blocked_files, 0);

    let report = RoundTripValidator::default().validate(
      &project,
      &edit,
      &plan,
      &RoundTripCancellation::default(),
      |_| {},
    );

    assert_eq!(report.status, RoundTripStatus::Passed, "{}", report.full_text());
    assert!(report.semantic_comparison.states_match);
    assert!(report.semantic_comparison.indexes_match);
    assert!(report.byte_comparison.differences.is_empty());
    assert!(report.workspace.cleaned);
    assert_eq!(fs::read(&source_path).unwrap(), source_before);
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn stale_cancelled_and_review_results_are_distinct() {
    let (root, project, mut edit) = test_project("statuses");
    let mut properties = EditableStateProperties::from_state(
      project.state_document(1).unwrap().data.as_ref().unwrap()
    );
    properties.manpower = Some(3);
    edit.update_state_properties(1, properties).unwrap();
    let mut plan = plan_state_patches(&project, &edit);

    let mut stale = plan.clone();
    stale.generation = stale.generation.wrapping_add(1);
    let stale_report = RoundTripValidator::default().validate(
      &project,
      &edit,
      &stale,
      &RoundTripCancellation::default(),
      |_| {},
    );
    assert_eq!(stale_report.status, RoundTripStatus::Failed);
    assert!(stale_report.workspace.workspace_root.is_none());

    let cancellation = RoundTripCancellation::default();
    cancellation.cancel();
    let cancelled = RoundTripValidator::default().validate(
      &project,
      &edit,
      &plan,
      &cancellation,
      |_| {},
    );
    assert_eq!(cancelled.status, RoundTripStatus::Cancelled);
    assert!(cancelled.workspace.workspace_root.is_none());

    let cancellation = RoundTripCancellation::default();
    let cancel_after_workspace = cancellation.clone();
    let cancelled = RoundTripValidator::default().validate(
      &project,
      &edit,
      &plan,
      &cancellation,
      |stage| {
        if stage == RoundTripStage::Copying {
          cancel_after_workspace.cancel();
        }
      },
    );
    assert_eq!(cancelled.status, RoundTripStatus::Cancelled);
    assert!(cancelled.workspace.workspace_root.is_some());
    assert!(cancelled.workspace.cleaned);

    plan.modified_files[0].safety = PatchSafety::ReviewRequired;
    plan.summary.safe_files = 0;
    plan.summary.review_required_files = 1;
    let review = RoundTripValidator {
      policy: RoundTripValidationPolicy {
        allow_review_required: true,
        ..Default::default()
      },
    }.validate(
      &project,
      &edit,
      &plan,
      &RoundTripCancellation::default(),
      |_| {},
    );
    assert_eq!(review.status, RoundTripStatus::PassedWithReview, "{}", review.full_text());
    assert!(!review.eligible_for_atomic_save_preparation);
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn created_and_removed_states_exist_only_in_candidate() {
    let (create_root, create_project, mut create_edit) = test_project("create");
    create_edit.create_state(2, EditableStateProperties::default(), false).unwrap();
    let create_plan = plan_state_patches(&create_project, &create_edit);
    let create_report = RoundTripValidator::default().validate(
      &create_project,
      &create_edit,
      &create_plan,
      &RoundTripCancellation::default(),
      |_| {},
    );
    assert_eq!(create_report.status, RoundTripStatus::Passed, "{}", create_report.full_text());
    assert_eq!(create_report.application.created_files_applied, 1);
    assert!(!create_root.join("history/states/2-State_2.txt").exists());
    fs::remove_dir_all(create_root).unwrap();

    let (remove_root, remove_project, mut remove_edit) = test_project("remove");
    remove_edit.remove_state(1, super::super::StateRemovalPolicy::Unassign).unwrap();
    let remove_plan = plan_state_patches(&remove_project, &remove_edit);
    let source_path = remove_root.join("history/states/1-Test.txt");
    let source_before = fs::read(&source_path).unwrap();
    let remove_report = RoundTripValidator::default().validate(
      &remove_project,
      &remove_edit,
      &remove_plan,
      &RoundTripCancellation::default(),
      |_| {},
    );
    assert_eq!(remove_report.status, RoundTripStatus::Passed, "{}", remove_report.full_text());
    assert_eq!(remove_report.application.removed_files_applied, 1);
    assert_eq!(fs::read(&source_path).unwrap(), source_before);
    fs::remove_dir_all(remove_root).unwrap();
  }

  fn test_project(name: &str) -> (PathBuf, Hoi4Project, StateEditSession) {
    let root = std::env::temp_dir().join(format!(
      "phase4b-{name}-{}-{}",
      std::process::id(),
      WORKSPACE_COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("map")).unwrap();
    fs::create_dir_all(root.join("history/states")).unwrap();
    let image = image::RgbImage::from_pixel(1, 1, image::Rgb([1, 2, 3]));
    let mut bmp = Vec::new();
    crate::app::map::write_rgb_bmp_image(&mut bmp, &image).unwrap();
    fs::write(root.join("map/provinces.bmp"), bmp).unwrap();
    fs::write(
      root.join("map/definition.csv"),
      "0;0;0;0;land;false;unknown;0\n1;1;2;3;land;false;plains;1\n",
    ).unwrap();
    fs::write(
      root.join("history/states/1-Test.txt"),
      "state={id=1 state_category=rural provinces={1} history={owner=TAG}}",
    ).unwrap();
    let paths = ProjectPaths::discover(&root).unwrap();
    let config = Config {
      preserve_ids: true,
      ..Config::default()
    };
    let bundle = Bundle::load(&Location::Directory(paths.map_directory.clone()), config).unwrap();
    let valid = bundle.map.province_ids().collect::<BTreeSet<_>>();
    let land = BTreeSet::from([1]);
    let mut project = Hoi4Project::new(paths);
    project.load_states(&valid, &land);
    let edit = StateEditSession::new(&project, &bundle.map);
    (root, project, edit)
  }

  fn test_map() -> crate::app::map::Map {
    let (root, _, _) = test_project("map");
    let paths = ProjectPaths::discover(&root).unwrap();
    let config = Config {
      preserve_ids: true,
      ..Config::default()
    };
    let map = Bundle::load(&Location::Directory(paths.map_directory), config).unwrap().map;
    fs::remove_dir_all(root).unwrap();
    map
  }
}
