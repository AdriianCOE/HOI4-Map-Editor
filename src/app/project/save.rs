use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::validation::{compare_project_for_save, reload_project_for_save};
use super::{
    Hoi4Project, PatchSafety, ProjectPatchPlan, RoundTripStatus, RoundTripValidationReport,
    SourceFingerprint, StateEditSession,
};

static TRANSACTION_COUNTER: AtomicU64 = AtomicU64::new(0);
const INTERNAL_DIRECTORY: &str = ".hoi4-state-editor";
const LOCK_FILE: &str = "save.lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveTransactionState {
    Preparing,
    VerifyingSource,
    BackingUp,
    BackupVerified,
    Staging,
    Staged,
    Committing,
    ValidatingSavedProject,
    Completed,
    RollingBack,
    RolledBack,
    RollbackFailed,
    Cancelled,
    FailedBeforeCommit,
    RecoveryRequired,
}

impl SaveTransactionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing",
            Self::VerifyingSource => "Verifying source",
            Self::BackingUp => "Backing up",
            Self::BackupVerified => "Backup verified",
            Self::Staging => "Staging",
            Self::Staged => "Staged",
            Self::Committing => "Committing",
            Self::ValidatingSavedProject => "Validating",
            Self::Completed => "Completed",
            Self::RollingBack => "Rolling back",
            Self::RolledBack => "Rolled back",
            Self::RollbackFailed => "Rollback failed",
            Self::Cancelled => "Cancelled",
            Self::FailedBeforeCommit => "Failed before commit",
            Self::RecoveryRequired => "Recovery required",
        }
    }

    pub fn cancellable(self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::VerifyingSource
                | Self::BackingUp
                | Self::BackupVerified
                | Self::Staging
                | Self::Staged
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSaveBlockReason {
    NoPatchPlan,
    PatchPlanStale,
    NoChanges,
    ReviewRequired,
    BlockedPatch,
    NoRoundTripReport,
    RoundTripReportStale,
    RoundTripNotPassed,
    SourceChanged,
    DraftPending,
    ToolInteractionActive,
    RecoveryRequired,
    SaveAlreadyRunning,
}

impl StateSaveBlockReason {
    pub fn message(self) -> &'static str {
        match self {
            Self::NoPatchPlan => "Generate and validate a Safe patch preview before saving.",
            Self::PatchPlanStale => {
                "The patch preview is outdated. Regenerate and validate it again."
            }
            Self::NoChanges => "There are no state changes to save.",
            Self::ReviewRequired => {
                "Review-required patches need explicit isolated validation and approval."
            }
            Self::BlockedPatch => "Blocked patches cannot be saved.",
            Self::NoRoundTripReport => "Run the isolated round-trip validation before saving.",
            Self::RoundTripReportStale => "The round-trip validation is outdated.",
            Self::RoundTripNotPassed => "Only a current eligible round-trip report can authorize Save.",
            Self::SourceChanged => {
                "The source project changed after validation. Regenerate and validate again."
            }
            Self::DraftPending => "Apply or discard the pending editor draft before saving.",
            Self::ToolInteractionActive => {
                "Finish or cancel the active lasso/brush interaction before saving."
            }
            Self::RecoveryRequired => {
                "An interrupted state save must be recovered before another Save."
            }
            Self::SaveAlreadyRunning => "A state save transaction is already running.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedFingerprint {
    #[serde(with = "hex_u64")]
    pub byte_len: u64,
    #[serde(with = "hex_u64")]
    pub content_hash: u64,
}

mod hex_u64 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{value:016X}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let value = String::deserialize(deserializer)?;
        u64::from_str_radix(&value, 16).map_err(serde::de::Error::custom)
    }
}

impl From<&SourceFingerprint> for PersistedFingerprint {
    fn from(value: &SourceFingerprint) -> Self {
        Self {
            byte_len: value.byte_len,
            content_hash: value.content_hash,
        }
    }
}

impl PersistedFingerprint {
    fn matches(&self, bytes: &[u8]) -> bool {
        let actual = SourceFingerprint::from_bytes(bytes);
        self.byte_len == actual.byte_len && self.content_hash == actual.content_hash
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSaveAuthorization {
    pub session_revision: u64,
    pub patch_plan_generation: u64,
    pub patch_plan_digest: String,
    pub validation_report_digest: String,
    #[serde(default)]
    pub allow_review_required: bool,
    pub source_fingerprints: BTreeMap<PathBuf, PersistedFingerprint>,
}

#[derive(Debug, Clone)]
pub struct StateSaveEligibility {
    pub eligible: bool,
    pub reasons: Vec<StateSaveBlockReason>,
    pub authorization: Option<StateSaveAuthorization>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StateSaveConditions {
    pub draft_pending: bool,
    pub tool_interaction_active: bool,
    pub recovery_required: bool,
    pub save_running: bool,
    pub allow_review_required: bool,
}

pub fn state_save_eligibility(
    project: &Hoi4Project,
    edit: &StateEditSession,
    plan: Option<&ProjectPatchPlan>,
    report: Option<&RoundTripValidationReport>,
    conditions: StateSaveConditions,
) -> StateSaveEligibility {
    let mut reasons = Vec::new();
    if conditions.draft_pending {
        reasons.push(StateSaveBlockReason::DraftPending);
    }
    if conditions.tool_interaction_active {
        reasons.push(StateSaveBlockReason::ToolInteractionActive);
    }
    if conditions.recovery_required {
        reasons.push(StateSaveBlockReason::RecoveryRequired);
    }
    if conditions.save_running {
        reasons.push(StateSaveBlockReason::SaveAlreadyRunning);
    }
    let Some(plan) = plan else {
        reasons.push(StateSaveBlockReason::NoPatchPlan);
        return ineligible(reasons);
    };
    if plan.is_stale(edit.revision()) {
        reasons.push(StateSaveBlockReason::PatchPlanStale);
    }
    if plan.files_len() == 0 {
        reasons.push(StateSaveBlockReason::NoChanges);
    }
    if plan.summary.blocked_files != 0
        || plan
            .modified_files
            .iter()
            .any(|file| file.safety == PatchSafety::Blocked)
        || plan
            .created_files
            .iter()
            .any(|file| file.safety == PatchSafety::Blocked)
        || plan
            .removed_files
            .iter()
            .any(|file| file.safety == PatchSafety::Blocked)
    {
        reasons.push(StateSaveBlockReason::BlockedPatch);
    }
    let review_required = plan.summary.review_required_files != 0
        || plan
            .modified_files
            .iter()
            .any(|file| file.safety == PatchSafety::ReviewRequired)
        || plan
            .created_files
            .iter()
            .any(|file| file.safety == PatchSafety::ReviewRequired)
        || plan
            .removed_files
            .iter()
            .any(|file| file.safety == PatchSafety::ReviewRequired);
    if review_required && !conditions.allow_review_required {
        reasons.push(StateSaveBlockReason::ReviewRequired);
    }
    let Some(report) = report else {
        reasons.push(StateSaveBlockReason::NoRoundTripReport);
        return ineligible(reasons);
    };
    if report.is_stale(edit.revision(), Some(plan)) {
        reasons.push(StateSaveBlockReason::RoundTripReportStale);
    }
    let validation_passed = report.status == RoundTripStatus::Passed
        || (review_required
            && conditions.allow_review_required
            && report.status == RoundTripStatus::PassedWithReview);
    if !validation_passed || !report.eligible_for_atomic_save_preparation {
        reasons.push(StateSaveBlockReason::RoundTripNotPassed);
    }
    if verify_source(project, plan).is_err() {
        reasons.push(StateSaveBlockReason::SourceChanged);
    }
    deduplicate_reasons(&mut reasons);
    if !reasons.is_empty() {
        return ineligible(reasons);
    }
    let authorization = StateSaveAuthorization {
        session_revision: edit.revision(),
        patch_plan_generation: plan.generation,
        patch_plan_digest: plan.content_fingerprint().digest_hex(),
        validation_report_digest: report.content_fingerprint().digest_hex(),
        allow_review_required: conditions.allow_review_required,
        source_fingerprints: plan
            .source_fingerprints
            .iter()
            .map(|(path, fingerprint)| (path.clone(), fingerprint.into()))
            .collect(),
    };
    StateSaveEligibility {
        eligible: true,
        reasons,
        authorization: Some(authorization),
    }
}

fn ineligible(reasons: Vec<StateSaveBlockReason>) -> StateSaveEligibility {
    StateSaveEligibility {
        eligible: false,
        reasons,
        authorization: None,
    }
}

fn deduplicate_reasons(reasons: &mut Vec<StateSaveBlockReason>) {
    let mut seen = BTreeSet::new();
    reasons.retain(|reason| seen.insert(*reason as u8));
}

#[derive(Debug, Clone, Default)]
pub struct StateSaveCancellation(Arc<AtomicBool>);

impl StateSaveCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateSaveFault {
    #[default]
    None,
    FailStagingAt(usize),
    FailAfterCommit(usize),
    FailPostValidation,
    LeaveInterruptedAfterCommit(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveFileKind {
    Modify,
    Create,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileOperationProgress {
    Pending,
    Staged,
    DestinationMoved,
    CandidateInstalled,
    Removed,
    RolledBack,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveFileOperationJournal {
    pub kind: SaveFileKind,
    pub relative_path: PathBuf,
    pub stage_path: Option<PathBuf>,
    pub rollback_path: Option<PathBuf>,
    pub progress: FileOperationProgress,
    pub original_fingerprint: Option<PersistedFingerprint>,
    pub candidate_fingerprint: Option<PersistedFingerprint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveFailure {
    pub operation: String,
    pub path: Option<PathBuf>,
    pub message: String,
    pub commit_started: bool,
    pub rollback_attempted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveTransactionJournal {
    pub transaction_id: String,
    pub project_root: PathBuf,
    pub started_at: String,
    pub application_version: String,
    pub state: SaveTransactionState,
    pub backup_manifest_path: PathBuf,
    pub completed_steps: Vec<String>,
    pub authorization: StateSaveAuthorization,
    pub failure: Option<SaveFailure>,
    pub operations: Vec<SaveFileOperationJournal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifestEntry {
    pub kind: SaveFileKind,
    pub relative_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub original_fingerprint: Option<PersistedFingerprint>,
    pub candidate_fingerprint: Option<PersistedFingerprint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub transaction_id: String,
    pub project_root: PathBuf,
    pub created_at: String,
    pub plan_digest: String,
    pub entries: Vec<BackupManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SaveLock {
    transaction_id: String,
    timestamp: String,
    pid: u32,
    project_root: PathBuf,
    journal_path: PathBuf,
    application_version: String,
}

#[derive(Debug, Clone)]
pub struct RecoveryInfo {
    pub lock_path: PathBuf,
    pub journal_path: PathBuf,
    pub transaction_id: String,
    pub state: Option<SaveTransactionState>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSaveOutcome {
    Completed,
    RolledBack,
    RollbackFailed,
    Cancelled,
    FailedBeforeCommit,
    RecoveryRequired,
}

#[derive(Debug, Clone)]
pub struct StateSaveReport {
    pub outcome: StateSaveOutcome,
    pub transaction_id: Option<String>,
    pub state: SaveTransactionState,
    pub backup_path: Option<PathBuf>,
    pub journal_path: Option<PathBuf>,
    pub modified_files: usize,
    pub created_files: usize,
    pub removed_files: usize,
    pub error: Option<String>,
    pub rollback_errors: Vec<String>,
    pub reloaded_project: Option<Hoi4Project>,
}

impl StateSaveReport {
    pub fn summary_text(&self) -> String {
        let outcome = match self.outcome {
            StateSaveOutcome::Completed => "COMPLETED",
            StateSaveOutcome::RolledBack => "FAILED; ROLLBACK COMPLETED",
            StateSaveOutcome::RollbackFailed => "CRITICAL: ROLLBACK INCOMPLETE",
            StateSaveOutcome::Cancelled => "CANCELLED BEFORE COMMIT",
            StateSaveOutcome::FailedBeforeCommit => "FAILED BEFORE COMMIT",
            StateSaveOutcome::RecoveryRequired => "RECOVERY REQUIRED",
        };
        let mut text = format!(
            "STATE SAVE\nStatus: {outcome}\nState: {}\nModified: {} | Created: {} | Removed: {}",
            self.state.label(),
            self.modified_files,
            self.created_files,
            self.removed_files,
        );
        if let Some(path) = &self.backup_path {
            text.push_str(&format!("\nBackup: {}", path.display()));
        }
        if let Some(path) = &self.journal_path {
            text.push_str(&format!("\nJournal: {}", path.display()));
        }
        if let Some(error) = &self.error {
            text.push_str(&format!("\nError: {error}"));
        }
        for error in &self.rollback_errors {
            text.push_str(&format!("\nRollback error: {error}"));
        }
        text
    }
}

pub fn save_confirmation_text(project: &Hoi4Project, plan: &ProjectPatchPlan) -> String {
    format!(
        "SAVE STATE FILES\n\nProject root: {}\nModified files: {}\nCreated files: {}\n\
     Removed files: {}\n\nRound-trip validation: Passed\nBlocked patches: 0\n\
     Review-required patches: {}\n\nA verified backup will be created under:\n{}\n\n\
     This will modify the real mod project. Continue?",
        project.paths.root.display(),
        plan.summary.modified_files,
        plan.summary.created_files,
        plan.summary.removed_files,
        plan.summary.review_required_files,
        project
            .paths
            .root
            .join(INTERNAL_DIRECTORY)
            .join("backups")
            .display(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn execute_state_save(
    project: &Hoi4Project,
    edit: &StateEditSession,
    plan: &ProjectPatchPlan,
    report: &RoundTripValidationReport,
    authorization: &StateSaveAuthorization,
    cancellation: &StateSaveCancellation,
    fault: StateSaveFault,
    mut progress: impl FnMut(SaveTransactionState, usize, usize),
) -> StateSaveReport {
    let counts = (
        plan.modified_files.len(),
        plan.created_files.len(),
        plan.removed_files.len(),
    );
    if let Err(error) = validate_authorization(edit, plan, report, authorization) {
        return bare_report(StateSaveOutcome::FailedBeforeCommit, counts, error);
    }
    if let Err(error) = verify_source(project, plan) {
        return bare_report(StateSaveOutcome::FailedBeforeCommit, counts, error);
    }

    let root = match canonical_root(&project.paths.root) {
        Ok(root) => root,
        Err(error) => return bare_report(StateSaveOutcome::FailedBeforeCommit, counts, error),
    };
    let transaction_id = transaction_id();
    let internal = root.join(INTERNAL_DIRECTORY);
    let transaction_directory = internal.join("transactions").join(&transaction_id);
    let backup_directory = internal.join("backups").join(&transaction_id);
    let journal_path = transaction_directory.join("journal.toml");
    let backup_manifest_path = backup_directory.join("manifest.toml");
    let lock_path = internal.join(LOCK_FILE);
    if let Err(error) = fs::create_dir_all(&transaction_directory)
        .and_then(|_| fs::create_dir_all(backup_directory.join("files")))
    {
        return bare_report(
            StateSaveOutcome::FailedBeforeCommit,
            counts,
            format!("Failed to create save metadata directories: {error}"),
        );
    }

    let lock = SaveLock {
        transaction_id: transaction_id.clone(),
        timestamp: timestamp(),
        pid: std::process::id(),
        project_root: root.clone(),
        journal_path: journal_path.clone(),
        application_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    if let Err(error) = create_lock(&lock_path, &lock) {
        return bare_report(StateSaveOutcome::RecoveryRequired, counts, error);
    }

    let resolved = match resolve_operations(&root, plan, &transaction_id) {
        Ok(resolved) => resolved,
        Err(error) => {
            let _ = fs::remove_file(&lock_path);
            return bare_report(StateSaveOutcome::FailedBeforeCommit, counts, error);
        }
    };
    let mut journal = SaveTransactionJournal {
        transaction_id: transaction_id.clone(),
        project_root: root.clone(),
        started_at: timestamp(),
        application_version: env!("CARGO_PKG_VERSION").to_owned(),
        state: SaveTransactionState::Preparing,
        authorization: authorization.clone(),
        operations: resolved
            .iter()
            .map(ResolvedOperation::journal_entry)
            .collect(),
        backup_manifest_path: backup_manifest_path.clone(),
        completed_steps: Vec::new(),
        failure: None,
    };
    if let Err(error) = write_journal(&journal_path, &journal) {
        let _ = fs::remove_file(&lock_path);
        return contextual_report(
            StateSaveOutcome::FailedBeforeCommit,
            counts,
            &transaction_id,
            SaveTransactionState::FailedBeforeCommit,
            Some(&backup_directory),
            Some(&journal_path),
            error,
        );
    }

    let map_snapshot = match snapshot_map_files(project) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return fail_before_commit(
                &mut journal,
                &journal_path,
                &lock_path,
                &resolved,
                &backup_directory,
                counts,
                error,
            );
        }
    };

    progress(SaveTransactionState::VerifyingSource, 0, resolved.len());
    if cancellation.is_cancelled() {
        return cancel_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
        );
    }
    if let Err(error) = transition(
        &mut journal,
        &journal_path,
        SaveTransactionState::VerifyingSource,
        "source verification started",
    )
    .and_then(|_| verify_source(project, plan))
    {
        return fail_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
            error,
        );
    }

    progress(SaveTransactionState::BackingUp, 0, resolved.len());
    if let Err(error) = transition(
        &mut journal,
        &journal_path,
        SaveTransactionState::BackingUp,
        "backup started",
    )
    .and_then(|_| {
        create_and_verify_backup(
            &backup_directory,
            &backup_manifest_path,
            &transaction_id,
            &root,
            authorization,
            &resolved,
        )
    }) {
        return fail_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
            error,
        );
    }
    if let Err(error) = transition(
        &mut journal,
        &journal_path,
        SaveTransactionState::BackupVerified,
        "backup verified",
    ) {
        return fail_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
            error,
        );
    }

    progress(SaveTransactionState::Staging, 0, resolved.len());
    if cancellation.is_cancelled() {
        return cancel_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
        );
    }
    if let Err(error) = transition(
        &mut journal,
        &journal_path,
        SaveTransactionState::Staging,
        "staging started",
    )
    .and_then(|_| stage_candidates(&resolved, &mut journal, &journal_path, fault, &mut progress))
    {
        return fail_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
            error,
        );
    }
    if let Err(error) = transition(
        &mut journal,
        &journal_path,
        SaveTransactionState::Staged,
        "all candidates staged and verified",
    ) {
        return fail_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
            error,
        );
    }
    if cancellation.is_cancelled() {
        return cancel_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
        );
    }
    if let Err(error) = verify_source(project, plan) {
        return fail_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
            error,
        );
    }

    progress(SaveTransactionState::Committing, 0, resolved.len());
    if let Err(error) = transition(
        &mut journal,
        &journal_path,
        SaveTransactionState::Committing,
        "commit started",
    ) {
        return fail_before_commit(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
            error,
        );
    }
    match commit_operations(&resolved, &mut journal, &journal_path, fault, &mut progress) {
        Ok(CommitResult::Completed) => (),
        Ok(CommitResult::Interrupted) => {
            let _ = transition(
                &mut journal,
                &journal_path,
                SaveTransactionState::RecoveryRequired,
                "fault injection left transaction interrupted",
            );
            return contextual_report(
                StateSaveOutcome::RecoveryRequired,
                counts,
                &transaction_id,
                SaveTransactionState::RecoveryRequired,
                Some(&backup_directory),
                Some(&journal_path),
                "Fault injection left the committed prefix for recovery.".to_owned(),
            );
        }
        Err(error) => {
            return rollback_after_failure(
                &mut journal,
                &journal_path,
                &lock_path,
                &resolved,
                &backup_directory,
                counts,
                error,
                project,
                &map_snapshot,
            );
        }
    }

    progress(
        SaveTransactionState::ValidatingSavedProject,
        resolved.len(),
        resolved.len(),
    );
    if let Err(error) = transition(
        &mut journal,
        &journal_path,
        SaveTransactionState::ValidatingSavedProject,
        "post-save validation started",
    ) {
        return rollback_after_failure(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
            error,
            project,
            &map_snapshot,
        );
    }
    if fault == StateSaveFault::FailPostValidation {
        return rollback_after_failure(
            &mut journal,
            &journal_path,
            &lock_path,
            &resolved,
            &backup_directory,
            counts,
            "Injected post-save validation failure.".to_owned(),
            project,
            &map_snapshot,
        );
    }
    let reloaded_project =
        match validate_saved_project(&root, project, edit, plan, &resolved, &map_snapshot) {
            Ok(project) => project,
            Err(error) => {
                return rollback_after_failure(
                    &mut journal,
                    &journal_path,
                    &lock_path,
                    &resolved,
                    &backup_directory,
                    counts,
                    error,
                    project,
                    &map_snapshot,
                );
            }
        };

    if let Err(error) = transition(
        &mut journal,
        &journal_path,
        SaveTransactionState::Completed,
        "post-save reload and comparisons passed",
    ) {
        return contextual_report(
            StateSaveOutcome::RecoveryRequired,
            counts,
            &transaction_id,
            SaveTransactionState::RecoveryRequired,
            Some(&backup_directory),
            Some(&journal_path),
            format!("Save succeeded but the completed journal could not be persisted: {error}"),
        );
    }
    let mut cleanup_errors = Vec::new();
    for operation in &resolved {
        if let Err(error) = remove_if_exists(operation.stage_path()) {
            cleanup_errors.push(error);
        }
        if let Err(error) = remove_if_exists(operation.rollback_path()) {
            cleanup_errors.push(error);
        }
    }
    if !cleanup_errors.is_empty() {
        return contextual_report(
            StateSaveOutcome::RecoveryRequired,
            counts,
            &transaction_id,
            SaveTransactionState::Completed,
            Some(&backup_directory),
            Some(&journal_path),
            format!(
                "Save completed, but transaction cleanup requires recovery: {}",
                cleanup_errors.join("; ")
            ),
        );
    }
    let _ = fs::remove_file(&lock_path);
    progress(
        SaveTransactionState::Completed,
        resolved.len(),
        resolved.len(),
    );
    StateSaveReport {
        outcome: StateSaveOutcome::Completed,
        transaction_id: Some(transaction_id),
        state: SaveTransactionState::Completed,
        backup_path: Some(backup_directory),
        journal_path: Some(journal_path),
        modified_files: counts.0,
        created_files: counts.1,
        removed_files: counts.2,
        error: None,
        rollback_errors: Vec::new(),
        reloaded_project: Some(reloaded_project),
    }
}

#[derive(Debug, Clone)]
enum ResolvedOperation {
    Modify {
        relative: PathBuf,
        final_path: PathBuf,
        stage_path: PathBuf,
        rollback_path: PathBuf,
        before: Vec<u8>,
        after: Vec<u8>,
    },
    Create {
        relative: PathBuf,
        final_path: PathBuf,
        stage_path: PathBuf,
        content: Vec<u8>,
    },
    Remove {
        relative: PathBuf,
        final_path: PathBuf,
        rollback_path: PathBuf,
        before: Vec<u8>,
    },
}

impl ResolvedOperation {
    fn relative(&self) -> &Path {
        match self {
            Self::Modify { relative, .. }
            | Self::Create { relative, .. }
            | Self::Remove { relative, .. } => relative,
        }
    }

    fn stage_path(&self) -> Option<&Path> {
        match self {
            Self::Modify { stage_path, .. } | Self::Create { stage_path, .. } => Some(stage_path),
            Self::Remove { .. } => None,
        }
    }

    fn rollback_path(&self) -> Option<&Path> {
        match self {
            Self::Modify { rollback_path, .. } | Self::Remove { rollback_path, .. } => {
                Some(rollback_path)
            }
            Self::Create { .. } => None,
        }
    }

    fn before(&self) -> Option<&[u8]> {
        match self {
            Self::Modify { before, .. } | Self::Remove { before, .. } => Some(before),
            Self::Create { .. } => None,
        }
    }

    fn candidate(&self) -> Option<&[u8]> {
        match self {
            Self::Modify { after, .. } => Some(after),
            Self::Create { content, .. } => Some(content),
            Self::Remove { .. } => None,
        }
    }

    fn kind(&self) -> SaveFileKind {
        match self {
            Self::Modify { .. } => SaveFileKind::Modify,
            Self::Create { .. } => SaveFileKind::Create,
            Self::Remove { .. } => SaveFileKind::Remove,
        }
    }

    fn journal_entry(&self) -> SaveFileOperationJournal {
        SaveFileOperationJournal {
            kind: self.kind(),
            relative_path: self.relative().to_owned(),
            original_fingerprint: self
                .before()
                .map(SourceFingerprint::from_bytes)
                .as_ref()
                .map(Into::into),
            candidate_fingerprint: self
                .candidate()
                .map(SourceFingerprint::from_bytes)
                .as_ref()
                .map(Into::into),
            stage_path: self.stage_path().map(Path::to_owned),
            rollback_path: self.rollback_path().map(Path::to_owned),
            progress: FileOperationProgress::Pending,
        }
    }
}

fn resolve_operations(
    root: &Path,
    plan: &ProjectPatchPlan,
    transaction_id: &str,
) -> Result<Vec<ResolvedOperation>, String> {
    let mut paths = BTreeSet::new();
    let mut operations = Vec::with_capacity(plan.files_len());
    for file in &plan.modified_files {
        validate_plan_path(&file.path)?;
        if file.safety != PatchSafety::Safe {
            return Err(format!("{} is not Safe", file.path.display()));
        }
        let after = file
            .after
            .clone()
            .ok_or_else(|| format!("{} has no final candidate bytes", file.path.display()))?;
        let final_path = resolve_final_path(root, &file.path)?;
        ensure_unique_path(&mut paths, &file.path)?;
        ensure_writable_existing(&final_path)?;
        operations.push(ResolvedOperation::Modify {
            relative: file.path.clone(),
            stage_path: internal_sibling(&final_path, "stage", transaction_id)?,
            rollback_path: internal_sibling(&final_path, "rollback", transaction_id)?,
            final_path,
            before: file.before.clone(),
            after,
        });
    }
    for file in &plan.created_files {
        validate_plan_path(&file.path)?;
        if file.safety != PatchSafety::Safe {
            return Err(format!("{} is not Safe", file.path.display()));
        }
        let final_path = resolve_final_path(root, &file.path)?;
        ensure_unique_path(&mut paths, &file.path)?;
        operations.push(ResolvedOperation::Create {
            relative: file.path.clone(),
            stage_path: internal_sibling(&final_path, "stage", transaction_id)?,
            final_path,
            content: file.content.clone(),
        });
    }
    for file in &plan.removed_files {
        validate_plan_path(&file.path)?;
        if file.safety != PatchSafety::Safe {
            return Err(format!("{} is not Safe", file.path.display()));
        }
        let final_path = resolve_final_path(root, &file.path)?;
        ensure_unique_path(&mut paths, &file.path)?;
        ensure_writable_existing(&final_path)?;
        operations.push(ResolvedOperation::Remove {
            relative: file.path.clone(),
            rollback_path: internal_sibling(&final_path, "rollback", transaction_id)?,
            final_path,
            before: file.before.clone(),
        });
    }
    operations.sort_by_key(|operation| {
        (
            match operation.kind() {
                SaveFileKind::Modify => 0,
                SaveFileKind::Create => 1,
                SaveFileKind::Remove => 2,
            },
            normalized_path(operation.relative()),
        )
    });
    Ok(operations)
}

fn validate_plan_path(path: &Path) -> Result<(), String> {
    if path.is_absolute() {
        return Err(format!("Absolute state path is unsafe: {}", path.display()));
    }
    let components = path.components().collect::<Vec<_>>();
    if components.len() != 3
        || !matches!(components[0], Component::Normal(value) if eq_ascii(value, "history"))
        || !matches!(components[1], Component::Normal(value) if eq_ascii(value, "states"))
    {
        return Err(format!(
            "State paths must be direct history/states/*.txt paths: {}",
            path.display()
        ));
    }
    let Component::Normal(filename) = components[2] else {
        return Err(format!("Unsafe state filename: {}", path.display()));
    };
    let filename = filename.to_string_lossy();
    if !filename.to_ascii_lowercase().ends_with(".txt")
        || filename.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*'])
        || filename.ends_with(['.', ' '])
    {
        return Err(format!("Unsafe state filename: {}", path.display()));
    }
    let stem = filename
        .trim_end_matches([' ', '.'])
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return Err(format!("Reserved Windows filename: {}", path.display()));
    }
    Ok(())
}

fn eq_ascii(value: &std::ffi::OsStr, expected: &str) -> bool {
    value.to_string_lossy().eq_ignore_ascii_case(expected)
}

fn resolve_final_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let states = root.join("history").join("states");
    let canonical_states = dunce::canonicalize(&states)
        .map_err(|error| format!("Cannot canonicalize {}: {error}", states.display()))?;
    let filename = relative
        .file_name()
        .ok_or_else(|| format!("State path has no filename: {}", relative.display()))?;
    let final_path = canonical_states.join(filename);
    if final_path.parent() != Some(canonical_states.as_path()) {
        return Err(format!(
            "State path escaped the states directory: {}",
            relative.display()
        ));
    }
    Ok(final_path)
}

fn ensure_unique_path(paths: &mut BTreeSet<String>, path: &Path) -> Result<(), String> {
    let normalized = normalized_path(path).to_ascii_lowercase();
    if paths.insert(normalized) {
        Ok(())
    } else {
        Err(format!(
            "Case-insensitive state path collision: {}",
            path.display()
        ))
    }
}

fn ensure_writable_existing(path: &Path) -> Result<(), String> {
    let link_metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
    if link_metadata.file_type().is_symlink() {
        return Err(format!(
            "Symbolic links are not accepted for state files: {}",
            path.display()
        ));
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
    if metadata.permissions().readonly() {
        Err(format!("State file is read-only: {}", path.display()))
    } else {
        Ok(())
    }
}

fn internal_sibling(
    final_path: &Path,
    purpose: &str,
    transaction_id: &str,
) -> Result<PathBuf, String> {
    let filename = final_path
        .file_name()
        .ok_or_else(|| format!("Final path has no filename: {}", final_path.display()))?
        .to_string_lossy();
    Ok(final_path.with_file_name(format!("{filename}.hse-{purpose}-{transaction_id}")))
}

fn validate_authorization(
    edit: &StateEditSession,
    plan: &ProjectPatchPlan,
    report: &RoundTripValidationReport,
    authorization: &StateSaveAuthorization,
) -> Result<(), String> {
    if plan.files_len() == 0 {
        return Err("There are no state changes to save.".to_owned());
    }
    if authorization.session_revision != edit.revision()
        || authorization.patch_plan_generation != plan.generation
        || authorization.patch_plan_digest != plan.content_fingerprint().digest_hex()
    {
        return Err("The current patch plan is not the plan authorized for Save.".to_owned());
    }
    let review_required = plan.summary.review_required_files != 0
        || plan
            .modified_files
            .iter()
            .any(|file| file.safety == PatchSafety::ReviewRequired)
        || plan
            .created_files
            .iter()
            .any(|file| file.safety == PatchSafety::ReviewRequired)
        || plan
            .removed_files
            .iter()
            .any(|file| file.safety == PatchSafety::ReviewRequired);
    let validation_passed = report.status == RoundTripStatus::Passed
        || (review_required
            && authorization.allow_review_required
            && report.status == RoundTripStatus::PassedWithReview);
    if !validation_passed
        || !report.eligible_for_atomic_save_preparation
        || report.is_stale(edit.revision(), Some(plan))
        || authorization.validation_report_digest != report.content_fingerprint().digest_hex()
    {
        return Err(
            "The current patch plan is not the plan that passed round-trip validation. \
       Regenerate and validate it again."
                .to_owned(),
        );
    }
    if authorization.source_fingerprints.len() != plan.source_fingerprints.len()
        || plan.source_fingerprints.iter().any(|(path, fingerprint)| {
            authorization.source_fingerprints.get(path)
                != Some(&PersistedFingerprint::from(fingerprint))
        })
    {
        return Err("The authorized source fingerprint set changed.".to_owned());
    }
    Ok(())
}

fn verify_source(project: &Hoi4Project, plan: &ProjectPatchPlan) -> Result<(), String> {
    let root = canonical_root(&project.paths.root)?;
    let mut planned_paths = BTreeSet::new();
    for file in &plan.modified_files {
        validate_plan_path(&file.path)?;
        planned_paths.insert(normalized_path(&file.path).to_ascii_lowercase());
        let final_path = resolve_final_path(&root, &file.path)?;
        verify_exact_file(&final_path, &file.before)?;
    }
    for file in &plan.removed_files {
        validate_plan_path(&file.path)?;
        planned_paths.insert(normalized_path(&file.path).to_ascii_lowercase());
        let final_path = resolve_final_path(&root, &file.path)?;
        verify_exact_file(&final_path, &file.before)?;
    }
    for file in &plan.created_files {
        validate_plan_path(&file.path)?;
        let normalized = normalized_path(&file.path).to_ascii_lowercase();
        if !planned_paths.insert(normalized) {
            return Err(format!(
                "Case-insensitive path collision: {}",
                file.path.display()
            ));
        }
        let final_path = resolve_final_path(&root, &file.path)?;
        if final_path.exists() {
            return Err(format!(
                "Created state destination now exists: {}",
                final_path.display()
            ));
        }
    }
    for (source, fingerprint) in &plan.source_fingerprints {
        let canonical_source = dunce::canonicalize(source)
            .map_err(|error| format!("Cannot canonicalize {}: {error}", source.display()))?;
        let canonical_states = dunce::canonicalize(root.join("history").join("states"))
            .map_err(|error| format!("Cannot canonicalize states directory: {error}"))?;
        if canonical_source.parent() != Some(canonical_states.as_path()) {
            return Err(format!(
                "Planned source is outside history/states: {}",
                source.display()
            ));
        }
        let bytes = fs::read(source)
            .map_err(|error| format!("Cannot re-read {}: {error}", source.display()))?;
        if SourceFingerprint::from_bytes(&bytes) != *fingerprint {
            return Err(format!(
                "Source changed after validation: {}",
                source.display()
            ));
        }
    }
    Ok(())
}

fn verify_exact_file(path: &Path, expected: &[u8]) -> Result<(), String> {
    let actual =
        fs::read(path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "Source changed after validation: {}",
            path.display()
        ))
    }
}

fn create_and_verify_backup(
    backup_directory: &Path,
    manifest_path: &Path,
    transaction_id: &str,
    root: &Path,
    authorization: &StateSaveAuthorization,
    operations: &[ResolvedOperation],
) -> Result<(), String> {
    let files_root = backup_directory.join("files");
    let mut entries = Vec::with_capacity(operations.len());
    for operation in operations {
        let backup_path = operation.before().map(|bytes| {
            let path = files_root.join(operation.relative());
            (path, bytes)
        });
        if let Some((path, bytes)) = &backup_path {
            let parent = path
                .parent()
                .ok_or_else(|| format!("Backup path has no parent: {}", path.display()))?;
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
            write_new_synced(path, bytes)?;
        }
        entries.push(BackupManifestEntry {
            kind: operation.kind(),
            relative_path: operation.relative().to_owned(),
            original_fingerprint: operation
                .before()
                .map(SourceFingerprint::from_bytes)
                .as_ref()
                .map(Into::into),
            candidate_fingerprint: operation
                .candidate()
                .map(SourceFingerprint::from_bytes)
                .as_ref()
                .map(Into::into),
            backup_path: backup_path.map(|(path, _)| path),
        });
    }
    let manifest = BackupManifest {
        transaction_id: transaction_id.to_owned(),
        project_root: root.to_owned(),
        created_at: timestamp(),
        plan_digest: authorization.patch_plan_digest.clone(),
        entries,
    };
    write_toml_atomic(manifest_path, &manifest)?;
    verify_backup(manifest_path)
}

fn verify_backup(manifest_path: &Path) -> Result<(), String> {
    let manifest: BackupManifest = read_toml(manifest_path)?;
    let backup_root = manifest_path
        .parent()
        .ok_or_else(|| format!("Manifest has no parent: {}", manifest_path.display()))?;
    let canonical_backup_root = dunce::canonicalize(backup_root)
        .map_err(|error| format!("Cannot canonicalize {}: {error}", backup_root.display()))?;
    for entry in &manifest.entries {
        match (&entry.original_fingerprint, &entry.backup_path) {
            (Some(expected), Some(path)) => {
                let canonical = dunce::canonicalize(path).map_err(|error| {
                    format!("Cannot canonicalize backup {}: {error}", path.display())
                })?;
                if !canonical.starts_with(&canonical_backup_root) {
                    return Err(format!(
                        "Backup escaped its directory: {}",
                        canonical.display()
                    ));
                }
                let bytes = fs::read(&canonical).map_err(|error| {
                    format!("Cannot verify backup {}: {error}", canonical.display())
                })?;
                if !expected.matches(&bytes) {
                    return Err(format!(
                        "Backup fingerprint mismatch for {}",
                        entry.relative_path.display()
                    ));
                }
            }
            (None, None) if entry.kind == SaveFileKind::Create => (),
            _ => {
                return Err(format!(
                    "Incomplete backup manifest entry for {}",
                    entry.relative_path.display()
                ));
            }
        }
    }
    Ok(())
}

fn stage_candidates(
    operations: &[ResolvedOperation],
    journal: &mut SaveTransactionJournal,
    journal_path: &Path,
    fault: StateSaveFault,
    progress: &mut impl FnMut(SaveTransactionState, usize, usize),
) -> Result<(), String> {
    let mut staged = 0;
    for (index, operation) in operations.iter().enumerate() {
        let Some(stage_path) = operation.stage_path() else {
            continue;
        };
        if fault == StateSaveFault::FailStagingAt(index) {
            return Err(format!(
                "Injected staging failure at {}",
                operation.relative().display()
            ));
        }
        let bytes = operation.candidate().ok_or_else(|| {
            format!(
                "Missing candidate bytes for {}",
                operation.relative().display()
            )
        })?;
        write_new_synced(stage_path, bytes)?;
        let verified = fs::read(stage_path)
            .map_err(|error| format!("Cannot verify stage {}: {error}", stage_path.display()))?;
        if verified != bytes {
            return Err(format!(
                "Staged bytes differ for {}",
                operation.relative().display()
            ));
        }
        journal.operations[index].progress = FileOperationProgress::Staged;
        write_journal(journal_path, journal)?;
        staged += 1;
        progress(SaveTransactionState::Staging, staged, operations.len());
    }
    Ok(())
}

enum CommitResult {
    Completed,
    Interrupted,
}

fn commit_operations(
    operations: &[ResolvedOperation],
    journal: &mut SaveTransactionJournal,
    journal_path: &Path,
    fault: StateSaveFault,
    progress: &mut impl FnMut(SaveTransactionState, usize, usize),
) -> Result<CommitResult, String> {
    for (index, operation) in operations.iter().enumerate() {
        verify_operation_source(operation)?;
        match operation {
            ResolvedOperation::Modify {
                final_path,
                stage_path,
                rollback_path,
                ..
            } => {
                ensure_absent(rollback_path)?;
                fs::rename(final_path, rollback_path).map_err(|error| {
                    rename_error("move destination to rollback", final_path, error)
                })?;
                journal.operations[index].progress = FileOperationProgress::DestinationMoved;
                write_journal(journal_path, journal)?;
                fs::rename(stage_path, final_path)
                    .map_err(|error| rename_error("install staged candidate", stage_path, error))?;
                journal.operations[index].progress = FileOperationProgress::CandidateInstalled;
            }
            ResolvedOperation::Create {
                final_path,
                stage_path,
                ..
            } => {
                if final_path.exists() {
                    return Err(format!(
                        "Created destination appeared during commit: {}",
                        final_path.display()
                    ));
                }
                fs::rename(stage_path, final_path)
                    .map_err(|error| rename_error("install created state", stage_path, error))?;
                journal.operations[index].progress = FileOperationProgress::CandidateInstalled;
            }
            ResolvedOperation::Remove {
                final_path,
                rollback_path,
                ..
            } => {
                ensure_absent(rollback_path)?;
                fs::rename(final_path, rollback_path).map_err(|error| {
                    rename_error("move removed state to rollback", final_path, error)
                })?;
                journal.operations[index].progress = FileOperationProgress::Removed;
            }
        }
        write_journal(journal_path, journal)?;
        progress(
            SaveTransactionState::Committing,
            index + 1,
            operations.len(),
        );
        if fault == StateSaveFault::FailAfterCommit(index + 1) {
            return Err(format!(
                "Injected commit failure after {} operation(s)",
                index + 1
            ));
        }
        if fault == StateSaveFault::LeaveInterruptedAfterCommit(index + 1) {
            return Ok(CommitResult::Interrupted);
        }
    }
    Ok(CommitResult::Completed)
}

fn verify_operation_source(operation: &ResolvedOperation) -> Result<(), String> {
    match operation {
        ResolvedOperation::Modify {
            final_path, before, ..
        }
        | ResolvedOperation::Remove {
            final_path, before, ..
        } => verify_exact_file(final_path, before),
        ResolvedOperation::Create { final_path, .. } => {
            if final_path.exists() {
                Err(format!(
                    "Created destination appeared during Save: {}",
                    final_path.display()
                ))
            } else {
                Ok(())
            }
        }
    }
}

fn validate_saved_project(
    root: &Path,
    source: &Hoi4Project,
    edit: &StateEditSession,
    plan: &ProjectPatchPlan,
    operations: &[ResolvedOperation],
    map_snapshot: &[(PathBuf, Vec<u8>)],
) -> Result<Hoi4Project, String> {
    verify_map_snapshot(map_snapshot)?;
    for operation in operations {
        match operation {
            ResolvedOperation::Modify {
                final_path, after, ..
            } => {
                verify_exact_file(final_path, after)?;
            }
            ResolvedOperation::Create {
                final_path,
                content,
                ..
            } => {
                verify_exact_file(final_path, content)?;
            }
            ResolvedOperation::Remove { final_path, .. } => {
                if final_path.exists() {
                    return Err(format!(
                        "Removed state still exists: {}",
                        final_path.display()
                    ));
                }
            }
        }
    }
    let (candidate, land_provinces) = reload_project_for_save(root)?;
    let (semantic, diagnostics) =
        compare_project_for_save(source, edit, plan, &candidate, &land_provinces);
    if !semantic.states_match
        || !semantic.indexes_match
        || !semantic.province_coverage_match
        || !semantic.victory_points_match
        || !semantic.buildings_match
        || !semantic.created_states_match
        || !semantic.removed_states_match
        || !semantic.differences.is_empty()
        || diagnostics.unexpected_diagnostics != 0
    {
        return Err(format!(
            "Post-save reload mismatch: {} semantic difference(s), {} unexpected diagnostic(s)",
            semantic.differences.len(),
            diagnostics.unexpected_diagnostics,
        ));
    }
    Ok(candidate)
}

#[allow(clippy::too_many_arguments)]
fn rollback_after_failure(
    journal: &mut SaveTransactionJournal,
    journal_path: &Path,
    lock_path: &Path,
    operations: &[ResolvedOperation],
    backup_directory: &Path,
    counts: (usize, usize, usize),
    error: String,
    project: &Hoi4Project,
    map_snapshot: &[(PathBuf, Vec<u8>)],
) -> StateSaveReport {
    journal.failure = Some(SaveFailure {
        operation: "save".to_owned(),
        path: None,
        message: error.clone(),
        commit_started: true,
        rollback_attempted: true,
    });
    let _ = transition(
        journal,
        journal_path,
        SaveTransactionState::RollingBack,
        "rollback started",
    );
    let rollback_errors = rollback_operations(operations, journal, journal_path);
    let verification = if rollback_errors.is_empty() {
        verify_rollback(project, operations, map_snapshot)
    } else {
        Err("Rollback operations reported errors.".to_owned())
    };
    let transaction_id = journal.transaction_id.clone();
    if rollback_errors.is_empty() && verification.is_ok() {
        let _ = transition(
            journal,
            journal_path,
            SaveTransactionState::RolledBack,
            "rollback verified",
        );
        let _ = fs::remove_file(lock_path);
        StateSaveReport {
            outcome: StateSaveOutcome::RolledBack,
            transaction_id: Some(transaction_id),
            state: SaveTransactionState::RolledBack,
            backup_path: Some(backup_directory.to_owned()),
            journal_path: Some(journal_path.to_owned()),
            modified_files: counts.0,
            created_files: counts.1,
            removed_files: counts.2,
            error: Some(error),
            rollback_errors,
            reloaded_project: None,
        }
    } else {
        let mut all_errors = rollback_errors;
        if let Err(error) = verification {
            all_errors.push(error);
        }
        let _ = transition(
            journal,
            journal_path,
            SaveTransactionState::RollbackFailed,
            "rollback incomplete; manual recovery required",
        );
        StateSaveReport {
            outcome: StateSaveOutcome::RollbackFailed,
            transaction_id: Some(transaction_id),
            state: SaveTransactionState::RollbackFailed,
            backup_path: Some(backup_directory.to_owned()),
            journal_path: Some(journal_path.to_owned()),
            modified_files: counts.0,
            created_files: counts.1,
            removed_files: counts.2,
            error: Some(error),
            rollback_errors: all_errors,
            reloaded_project: None,
        }
    }
}

fn rollback_operations(
    operations: &[ResolvedOperation],
    journal: &mut SaveTransactionJournal,
    journal_path: &Path,
) -> Vec<String> {
    let mut errors = Vec::new();
    for (index, operation) in operations.iter().enumerate().rev() {
        let progress = journal.operations[index].progress;
        let result = match operation {
            ResolvedOperation::Modify {
                final_path,
                rollback_path,
                before,
                after,
                ..
            } => restore_modified(final_path, rollback_path, before, after, progress),
            ResolvedOperation::Create {
                final_path,
                content,
                ..
            } => rollback_created(final_path, content, progress),
            ResolvedOperation::Remove {
                final_path,
                rollback_path,
                before,
                ..
            } => restore_removed(final_path, rollback_path, before, progress),
        };
        if let Err(error) = result {
            errors.push(format!("{}: {error}", operation.relative().display()));
        } else {
            journal.operations[index].progress = FileOperationProgress::RolledBack;
            if let Err(error) = write_journal(journal_path, journal) {
                errors.push(format!(
                    "journal update for {}: {error}",
                    operation.relative().display()
                ));
            }
        }
        if let Err(error) = remove_if_exists(operation.stage_path()) {
            errors.push(format!(
                "stage cleanup for {}: {error}",
                operation.relative().display()
            ));
        }
    }
    errors
}

fn restore_modified(
    final_path: &Path,
    rollback_path: &Path,
    before: &[u8],
    candidate: &[u8],
    progress: FileOperationProgress,
) -> Result<(), String> {
    if rollback_path.exists() {
        if final_path.exists() {
            verify_exact_file(final_path, candidate).map_err(|_| {
                format!(
                    "Refusing to overwrite externally changed candidate {} during rollback",
                    final_path.display()
                )
            })?;
            fs::remove_file(final_path).map_err(|error| {
                format!(
                    "Cannot remove failed candidate {}: {error}",
                    final_path.display()
                )
            })?;
        }
        return fs::rename(rollback_path, final_path)
            .map_err(|error| rename_error("restore rollback", rollback_path, error));
    }
    if matches!(
        progress,
        FileOperationProgress::Pending | FileOperationProgress::Staged
    ) {
        verify_exact_file(final_path, before)
    } else {
        Err(format!(
            "Missing rollback file for committed modification {}",
            final_path.display()
        ))
    }
}

fn rollback_created(
    final_path: &Path,
    candidate: &[u8],
    progress: FileOperationProgress,
) -> Result<(), String> {
    if final_path.exists() {
        verify_exact_file(final_path, candidate).map_err(|_| {
            format!(
                "Refusing to delete externally changed created state {}",
                final_path.display()
            )
        })?;
        fs::remove_file(final_path).map_err(|error| {
            format!(
                "Cannot remove created state {}: {error}",
                final_path.display()
            )
        })
    } else if matches!(
        progress,
        FileOperationProgress::Pending
            | FileOperationProgress::Staged
            | FileOperationProgress::CandidateInstalled
            | FileOperationProgress::RolledBack
    ) {
        Ok(())
    } else {
        Err(format!(
            "Unexpected create rollback state for {}",
            final_path.display()
        ))
    }
}

fn restore_removed(
    final_path: &Path,
    rollback_path: &Path,
    before: &[u8],
    progress: FileOperationProgress,
) -> Result<(), String> {
    if rollback_path.exists() {
        if final_path.exists() {
            return Err(format!(
                "Removed destination unexpectedly exists during rollback: {}",
                final_path.display()
            ));
        }
        return fs::rename(rollback_path, final_path)
            .map_err(|error| rename_error("restore removed state", rollback_path, error));
    }
    if matches!(
        progress,
        FileOperationProgress::Pending | FileOperationProgress::Staged
    ) {
        verify_exact_file(final_path, before)
    } else {
        Err(format!(
            "Missing rollback file for committed removal {}",
            final_path.display()
        ))
    }
}

fn verify_rollback(
    project: &Hoi4Project,
    operations: &[ResolvedOperation],
    map_snapshot: &[(PathBuf, Vec<u8>)],
) -> Result<(), String> {
    verify_map_snapshot(map_snapshot)?;
    for operation in operations {
        match operation {
            ResolvedOperation::Modify {
                final_path, before, ..
            }
            | ResolvedOperation::Remove {
                final_path, before, ..
            } => {
                verify_exact_file(final_path, before)?;
            }
            ResolvedOperation::Create { final_path, .. } => {
                if final_path.exists() {
                    return Err(format!(
                        "Created state remains after rollback: {}",
                        final_path.display()
                    ));
                }
            }
        }
    }
    reload_project_for_save(&project.paths.root)
        .map(|_| ())
        .map_err(|error| format!("Rolled-back project did not reload: {error}"))
}

fn fail_before_commit(
    journal: &mut SaveTransactionJournal,
    journal_path: &Path,
    lock_path: &Path,
    operations: &[ResolvedOperation],
    backup_directory: &Path,
    counts: (usize, usize, usize),
    error: String,
) -> StateSaveReport {
    for operation in operations {
        let _ = remove_if_exists(operation.stage_path());
    }
    journal.failure = Some(SaveFailure {
        operation: journal.state.label().to_owned(),
        path: None,
        message: error.clone(),
        commit_started: false,
        rollback_attempted: false,
    });
    let _ = transition(
        journal,
        journal_path,
        SaveTransactionState::FailedBeforeCommit,
        "failed before first final-file rename",
    );
    let _ = fs::remove_file(lock_path);
    StateSaveReport {
        outcome: StateSaveOutcome::FailedBeforeCommit,
        transaction_id: Some(journal.transaction_id.clone()),
        state: SaveTransactionState::FailedBeforeCommit,
        backup_path: backup_directory
            .exists()
            .then(|| backup_directory.to_owned()),
        journal_path: Some(journal_path.to_owned()),
        modified_files: counts.0,
        created_files: counts.1,
        removed_files: counts.2,
        error: Some(error),
        rollback_errors: Vec::new(),
        reloaded_project: None,
    }
}

fn cancel_before_commit(
    journal: &mut SaveTransactionJournal,
    journal_path: &Path,
    lock_path: &Path,
    operations: &[ResolvedOperation],
    backup_directory: &Path,
    counts: (usize, usize, usize),
) -> StateSaveReport {
    for operation in operations {
        let _ = remove_if_exists(operation.stage_path());
    }
    let _ = transition(
        journal,
        journal_path,
        SaveTransactionState::Cancelled,
        "cancelled before commit",
    );
    let _ = fs::remove_file(lock_path);
    StateSaveReport {
        outcome: StateSaveOutcome::Cancelled,
        transaction_id: Some(journal.transaction_id.clone()),
        state: SaveTransactionState::Cancelled,
        backup_path: backup_directory
            .exists()
            .then(|| backup_directory.to_owned()),
        journal_path: Some(journal_path.to_owned()),
        modified_files: counts.0,
        created_files: counts.1,
        removed_files: counts.2,
        error: None,
        rollback_errors: Vec::new(),
        reloaded_project: None,
    }
}

pub fn detect_state_save_recovery(root: &Path) -> Option<RecoveryInfo> {
    let root = canonical_root(root).ok()?;
    let lock_path = root.join(INTERNAL_DIRECTORY).join(LOCK_FILE);
    let lock: SaveLock = read_toml(&lock_path).ok()?;
    let state = read_toml::<SaveTransactionJournal>(&lock.journal_path)
        .ok()
        .map(|journal| journal.state);
    let transaction_id = lock.transaction_id.clone();
    Some(RecoveryInfo {
        lock_path,
        journal_path: lock.journal_path,
        transaction_id: transaction_id.clone(),
        state,
        message: match state {
            Some(state) => format!(
                "Interrupted state save {} is in {}. Verified rollback is required.",
                transaction_id,
                state.label(),
            ),
            None => format!(
                "State save lock {} exists but its journal could not be read.",
                transaction_id
            ),
        },
    })
}

pub fn recover_interrupted_state_save(root: &Path) -> StateSaveReport {
    let root = match canonical_root(root) {
        Ok(root) => root,
        Err(error) => return bare_report(StateSaveOutcome::RollbackFailed, (0, 0, 0), error),
    };
    let lock_path = root.join(INTERNAL_DIRECTORY).join(LOCK_FILE);
    let lock: SaveLock = match read_toml(&lock_path) {
        Ok(lock) => lock,
        Err(error) => {
            return bare_report(
                StateSaveOutcome::RollbackFailed,
                (0, 0, 0),
                format!("Cannot load save lock: {error}"),
            );
        }
    };
    let mut journal: SaveTransactionJournal = match read_toml(&lock.journal_path) {
        Ok(journal) => journal,
        Err(error) => {
            return bare_report(
                StateSaveOutcome::RollbackFailed,
                (0, 0, 0),
                format!("Cannot load interrupted save journal: {error}"),
            );
        }
    };
    let counts = journal
        .operations
        .iter()
        .fold((0, 0, 0), |mut counts, operation| {
            match operation.kind {
                SaveFileKind::Modify => counts.0 += 1,
                SaveFileKind::Create => counts.1 += 1,
                SaveFileKind::Remove => counts.2 += 1,
            }
            counts
        });
    if journal.state == SaveTransactionState::Completed {
        let verified = verify_backup(&journal.backup_manifest_path)
            .and_then(|_| reload_project_for_save(&root));
        return match verified {
            Ok((project, _)) => {
                let cleanup = journal.operations.iter().try_for_each(|operation| {
                    validate_recovery_operation(&root, &journal.transaction_id, operation)?;
                    remove_if_exists(operation.stage_path.as_deref())?;
                    remove_if_exists(operation.rollback_path.as_deref())
                });
                if let Err(error) = cleanup {
                    return contextual_report(
                        StateSaveOutcome::RecoveryRequired,
                        counts,
                        &journal.transaction_id,
                        SaveTransactionState::Completed,
                        journal.backup_manifest_path.parent(),
                        Some(&lock.journal_path),
                        format!("Completed Save cleanup is still required: {error}"),
                    );
                }
                let _ = fs::remove_file(&lock_path);
                StateSaveReport {
                    outcome: StateSaveOutcome::Completed,
                    transaction_id: Some(journal.transaction_id),
                    state: SaveTransactionState::Completed,
                    backup_path: journal.backup_manifest_path.parent().map(Path::to_owned),
                    journal_path: Some(lock.journal_path),
                    modified_files: counts.0,
                    created_files: counts.1,
                    removed_files: counts.2,
                    error: Some("Recovered cleanup after an already completed Save.".to_owned()),
                    rollback_errors: Vec::new(),
                    reloaded_project: Some(project),
                }
            }
            Err(error) => contextual_report(
                StateSaveOutcome::RollbackFailed,
                counts,
                &journal.transaction_id,
                SaveTransactionState::RollbackFailed,
                journal.backup_manifest_path.parent(),
                Some(&lock.journal_path),
                format!("Completed Save recovery verification failed: {error}"),
            ),
        };
    }
    let _ = transition(
        &mut journal,
        &lock.journal_path,
        SaveTransactionState::RollingBack,
        "startup recovery rollback started",
    );
    let mut errors = Vec::new();
    if let Err(error) = verify_backup(&journal.backup_manifest_path) {
        errors.push(format!(
            "Backup verification failed before recovery: {error}"
        ));
    }
    for (index, operation) in journal.operations.clone().into_iter().enumerate().rev() {
        if !errors.is_empty() {
            break;
        }
        let result = validate_recovery_operation(&root, &journal.transaction_id, &operation)
            .and_then(|final_path| rollback_journal_operation(&final_path, &operation));
        if let Err(error) = result {
            errors.push(format!("{}: {error}", operation.relative_path.display()));
        } else {
            journal.operations[index].progress = FileOperationProgress::RolledBack;
            let _ = write_journal(&lock.journal_path, &journal);
        }
        if let Some(stage_path) = operation.stage_path.as_deref()
            && let Err(error) = remove_if_exists(Some(stage_path))
        {
            errors.push(format!("{}: {error}", stage_path.display()));
        }
    }
    if errors.is_empty()
        && let Err(error) = reload_project_for_save(&root)
    {
        errors.push(format!("Recovered project did not reload: {error}"));
    }
    if errors.is_empty() {
        let _ = transition(
            &mut journal,
            &lock.journal_path,
            SaveTransactionState::RolledBack,
            "startup recovery rollback completed",
        );
        let _ = fs::remove_file(&lock_path);
        let reloaded_project = reload_project_for_save(&root)
            .ok()
            .map(|(project, _)| project);
        StateSaveReport {
            outcome: StateSaveOutcome::RolledBack,
            transaction_id: Some(journal.transaction_id),
            state: SaveTransactionState::RolledBack,
            backup_path: journal.backup_manifest_path.parent().map(Path::to_owned),
            journal_path: Some(lock.journal_path),
            modified_files: counts.0,
            created_files: counts.1,
            removed_files: counts.2,
            error: Some("Recovered an interrupted save by rollback.".to_owned()),
            rollback_errors: errors,
            reloaded_project,
        }
    } else {
        let _ = transition(
            &mut journal,
            &lock.journal_path,
            SaveTransactionState::RollbackFailed,
            "startup recovery rollback incomplete",
        );
        StateSaveReport {
            outcome: StateSaveOutcome::RollbackFailed,
            transaction_id: Some(journal.transaction_id),
            state: SaveTransactionState::RollbackFailed,
            backup_path: journal.backup_manifest_path.parent().map(Path::to_owned),
            journal_path: Some(lock.journal_path),
            modified_files: counts.0,
            created_files: counts.1,
            removed_files: counts.2,
            error: Some("CRITICAL: Save failed and rollback was incomplete.".to_owned()),
            rollback_errors: errors,
            reloaded_project: None,
        }
    }
}

fn validate_recovery_operation(
    root: &Path,
    transaction_id: &str,
    operation: &SaveFileOperationJournal,
) -> Result<PathBuf, String> {
    validate_plan_path(&operation.relative_path)?;
    let final_path = resolve_final_path(root, &operation.relative_path)?;
    match operation.kind {
        SaveFileKind::Modify => {
            let expected_stage = internal_sibling(&final_path, "stage", transaction_id)?;
            let expected_rollback = internal_sibling(&final_path, "rollback", transaction_id)?;
            if operation.stage_path.as_deref() != Some(expected_stage.as_path())
                || operation.rollback_path.as_deref() != Some(expected_rollback.as_path())
            {
                return Err(format!(
                    "Journal internal paths do not match {}",
                    operation.relative_path.display()
                ));
            }
        }
        SaveFileKind::Create => {
            let expected_stage = internal_sibling(&final_path, "stage", transaction_id)?;
            if operation.stage_path.as_deref() != Some(expected_stage.as_path())
                || operation.rollback_path.is_some()
            {
                return Err(format!(
                    "Journal create paths do not match {}",
                    operation.relative_path.display()
                ));
            }
        }
        SaveFileKind::Remove => {
            let expected_rollback = internal_sibling(&final_path, "rollback", transaction_id)?;
            if operation.rollback_path.as_deref() != Some(expected_rollback.as_path())
                || operation.stage_path.is_some()
            {
                return Err(format!(
                    "Journal remove paths do not match {}",
                    operation.relative_path.display()
                ));
            }
        }
    }
    Ok(final_path)
}

fn rollback_journal_operation(
    final_path: &Path,
    operation: &SaveFileOperationJournal,
) -> Result<(), String> {
    match operation.kind {
        SaveFileKind::Modify => {
            let rollback = operation
                .rollback_path
                .as_deref()
                .ok_or_else(|| "Missing modification rollback path".to_owned())?;
            if rollback.exists() {
                if final_path.exists() {
                    verify_persisted_file(
                        final_path,
                        operation.candidate_fingerprint.as_ref(),
                        "candidate",
                    )?;
                    fs::remove_file(final_path).map_err(|error| {
                        format!(
                            "Cannot remove failed candidate {}: {error}",
                            final_path.display()
                        )
                    })?;
                }
                fs::rename(rollback, final_path)
                    .map_err(|error| rename_error("restore modification", rollback, error))?;
                verify_persisted_file(
                    final_path,
                    operation.original_fingerprint.as_ref(),
                    "restored original",
                )
            } else if matches!(
                operation.progress,
                FileOperationProgress::Pending | FileOperationProgress::Staged
            ) {
                verify_persisted_file(
                    final_path,
                    operation.original_fingerprint.as_ref(),
                    "untouched original",
                )
            } else {
                Err(format!(
                    "Missing rollback file for committed modification {}",
                    final_path.display()
                ))
            }
        }
        SaveFileKind::Create => {
            if final_path.exists() {
                verify_persisted_file(
                    final_path,
                    operation.candidate_fingerprint.as_ref(),
                    "created candidate",
                )?;
                fs::remove_file(final_path).map_err(|error| {
                    format!(
                        "Cannot remove created state {}: {error}",
                        final_path.display()
                    )
                })?;
            }
            Ok(())
        }
        SaveFileKind::Remove => {
            let rollback = operation
                .rollback_path
                .as_deref()
                .ok_or_else(|| "Missing removal rollback path".to_owned())?;
            if rollback.exists() {
                if final_path.exists() {
                    return Err(format!(
                        "Removed destination unexpectedly exists: {}",
                        final_path.display()
                    ));
                }
                fs::rename(rollback, final_path)
                    .map_err(|error| rename_error("restore removed state", rollback, error))?;
                verify_persisted_file(
                    final_path,
                    operation.original_fingerprint.as_ref(),
                    "restored removed state",
                )
            } else if matches!(
                operation.progress,
                FileOperationProgress::Pending | FileOperationProgress::Staged
            ) {
                verify_persisted_file(
                    final_path,
                    operation.original_fingerprint.as_ref(),
                    "untouched original",
                )
            } else {
                Err(format!(
                    "Missing rollback file for committed removal {}",
                    final_path.display()
                ))
            }
        }
    }
}

fn verify_persisted_file(
    path: &Path,
    expected: Option<&PersistedFingerprint>,
    description: &str,
) -> Result<(), String> {
    let expected = expected
        .ok_or_else(|| format!("Missing {description} fingerprint for {}", path.display()))?;
    let bytes = fs::read(path)
        .map_err(|error| format!("Cannot read {description} {}: {error}", path.display()))?;
    if expected.matches(&bytes) {
        Ok(())
    } else {
        Err(format!(
            "{description} fingerprint mismatch for {}",
            path.display()
        ))
    }
}

fn transition(
    journal: &mut SaveTransactionJournal,
    journal_path: &Path,
    state: SaveTransactionState,
    step: &str,
) -> Result<(), String> {
    if !valid_transition(journal.state, state) {
        return Err(format!(
            "Invalid save transition: {} -> {}",
            journal.state.label(),
            state.label()
        ));
    }
    journal.state = state;
    journal.completed_steps.push(step.to_owned());
    write_journal(journal_path, journal)
}

fn valid_transition(from: SaveTransactionState, to: SaveTransactionState) -> bool {
    use SaveTransactionState::*;
    from == to
        || matches!(
            (from, to),
            (Preparing, VerifyingSource | FailedBeforeCommit | Cancelled)
                | (VerifyingSource, BackingUp | FailedBeforeCommit | Cancelled)
                | (BackingUp, BackupVerified | FailedBeforeCommit | Cancelled)
                | (BackupVerified, Staging | FailedBeforeCommit | Cancelled)
                | (Staging, Staged | FailedBeforeCommit | Cancelled)
                | (Staged, Committing | FailedBeforeCommit | Cancelled)
                | (
                    Committing,
                    ValidatingSavedProject | RollingBack | RecoveryRequired
                )
                | (
                    ValidatingSavedProject,
                    Completed | RollingBack | RecoveryRequired
                )
                | (RollingBack, RolledBack | RollbackFailed | RecoveryRequired)
                | (RecoveryRequired, RollingBack | RollbackFailed)
        )
}

fn snapshot_map_files(project: &Hoi4Project) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    [
        project.paths.provinces_bmp.clone(),
        project.paths.definition_csv.clone(),
    ]
    .into_iter()
    .map(|path| {
        fs::read(&path)
            .map(|bytes| (path.clone(), bytes))
            .map_err(|error| {
                format!(
                    "Cannot snapshot read-only map file {}: {error}",
                    path.display()
                )
            })
    })
    .collect()
}

fn verify_map_snapshot(snapshot: &[(PathBuf, Vec<u8>)]) -> Result<(), String> {
    for (path, expected) in snapshot {
        verify_exact_file(path, expected)?;
    }
    Ok(())
}

fn create_lock(path: &Path, lock: &SaveLock) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Lock has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
    let bytes =
        toml::to_vec(lock).map_err(|error| format!("Cannot serialize save lock: {error}"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "Cannot acquire exclusive state save lock {}: {error}. Recovery may be required.",
                path.display()
            )
        })?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Cannot persist save lock {}: {error}", path.display()))
}

fn write_journal(path: &Path, journal: &SaveTransactionJournal) -> Result<(), String> {
    write_toml_atomic(path, journal)
}

fn write_toml_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = toml::to_vec(value)
        .map_err(|error| format!("Cannot serialize {}: {error}", path.display()))?;
    let filename = path
        .file_name()
        .ok_or_else(|| format!("Metadata path has no filename: {}", path.display()))?
        .to_string_lossy();
    let temporary = path.with_file_name(format!("{filename}.tmp"));
    let previous = path.with_file_name(format!("{filename}.previous"));
    let _ = fs::remove_file(&temporary);
    let _ = fs::remove_file(&previous);
    write_new_synced(&temporary, &bytes)?;
    if path.exists() {
        fs::rename(path, &previous)
            .map_err(|error| format!("Cannot preserve metadata {}: {error}", path.display()))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if previous.exists() {
            let _ = fs::rename(&previous, path);
        }
        return Err(rename_error("publish metadata", &temporary, error));
    }
    let _ = fs::remove_file(previous);
    Ok(())
}

fn read_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let filename = path
                .file_name()
                .ok_or_else(|| format!("Metadata path has no filename: {}", path.display()))?
                .to_string_lossy();
            let previous = path.with_file_name(format!("{filename}.previous"));
            fs::read(&previous).map_err(|fallback| {
                format!(
                    "Cannot read {} ({error}) or recovery copy {} ({fallback})",
                    path.display(),
                    previous.display(),
                )
            })?
        }
        Err(error) => return Err(format!("Cannot read {}: {error}", path.display())),
    };
    toml::from_slice(&bytes).map_err(|error| format!("Cannot parse {}: {error}", path.display()))
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Cannot create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Cannot persist {}: {error}", path.display()))
}

fn remove_if_exists(path: Option<&Path>) -> Result<(), String> {
    let Some(path) = path else { return Ok(()) };
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot remove {}: {error}", path.display())),
    }
}

fn ensure_absent(path: &Path) -> Result<(), String> {
    if path.exists() {
        Err(format!(
            "Internal transaction path already exists: {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}

fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    dunce::canonicalize(root).map_err(|error| {
        format!(
            "Cannot canonicalize project root {}: {error}",
            root.display()
        )
    })
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn transaction_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:032x}-{counter:08x}-{:08x}", std::process::id())
}

fn rename_error(operation: &str, path: &Path, error: std::io::Error) -> String {
    format!("{operation} failed for {}: {error}", path.display())
}

fn bare_report(
    outcome: StateSaveOutcome,
    counts: (usize, usize, usize),
    error: String,
) -> StateSaveReport {
    let state = match outcome {
        StateSaveOutcome::Completed => SaveTransactionState::Completed,
        StateSaveOutcome::RolledBack => SaveTransactionState::RolledBack,
        StateSaveOutcome::RollbackFailed => SaveTransactionState::RollbackFailed,
        StateSaveOutcome::Cancelled => SaveTransactionState::Cancelled,
        StateSaveOutcome::FailedBeforeCommit => SaveTransactionState::FailedBeforeCommit,
        StateSaveOutcome::RecoveryRequired => SaveTransactionState::RecoveryRequired,
    };
    StateSaveReport {
        outcome,
        transaction_id: None,
        state,
        backup_path: None,
        journal_path: None,
        modified_files: counts.0,
        created_files: counts.1,
        removed_files: counts.2,
        error: Some(error),
        rollback_errors: Vec::new(),
        reloaded_project: None,
    }
}

fn contextual_report(
    outcome: StateSaveOutcome,
    counts: (usize, usize, usize),
    transaction_id: &str,
    state: SaveTransactionState,
    backup_path: Option<&Path>,
    journal_path: Option<&Path>,
    error: String,
) -> StateSaveReport {
    StateSaveReport {
        outcome,
        transaction_id: Some(transaction_id.to_owned()),
        state,
        backup_path: backup_path.map(Path::to_owned),
        journal_path: journal_path.map(Path::to_owned),
        modified_files: counts.0,
        created_files: counts.1,
        removed_files: counts.2,
        error: Some(error),
        rollback_errors: Vec::new(),
        reloaded_project: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::map::{Bundle, ProvinceKind, write_rgb_bmp_image};
    use crate::app::project::{
        EditableProvinceData, EditableStateProperties, ProjectPaths, RoundTripCancellation,
        RoundTripValidationPolicy, RoundTripValidator, StateRemovalPolicy, plan_state_patches,
    };
    use crate::config::Config;
    use crate::util::files::Location;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn transaction_state_rejects_shortcut_to_completed() {
        assert!(!valid_transition(
            SaveTransactionState::Preparing,
            SaveTransactionState::Completed,
        ));
        assert!(valid_transition(
            SaveTransactionState::Staged,
            SaveTransactionState::Committing,
        ));
    }

    #[test]
    fn final_state_paths_are_narrow_and_windows_safe() {
        assert!(validate_plan_path(Path::new("history/states/1-State_1.txt")).is_ok());
        assert!(validate_plan_path(Path::new("../states/1.txt")).is_err());
        assert!(validate_plan_path(Path::new("history/states/sub/1.txt")).is_err());
        assert!(validate_plan_path(Path::new("history/states/CON.txt")).is_err());
        assert!(validate_plan_path(Path::new("history/states/1-State_1.txt.exe")).is_err());
    }

    #[test]
    fn persisted_fingerprint_detects_byte_change() {
        let fingerprint = PersistedFingerprint::from(&SourceFingerprint::from_bytes(b"before"));
        assert!(fingerprint.matches(b"before"));
        assert!(!fingerprint.matches(b"after"));
        assert_ne!(fingerprint.content_hash, 0);
    }

    #[test]
    fn review_required_save_needs_explicit_approved_validation() {
        let (root, project, mut edit) = test_project("eligibility");
        change_manpower(&project, &mut edit, 2);
        let mut plan = plan_state_patches(&project, &edit);
        plan.modified_files[0].safety = PatchSafety::ReviewRequired;
        plan.summary.safe_files = 0;
        plan.summary.review_required_files = 1;
        let report = RoundTripValidator {
            policy: RoundTripValidationPolicy {
                allow_review_required: true,
                ..Default::default()
            },
        }
        .validate(
            &project,
            &edit,
            &plan,
            &RoundTripCancellation::default(),
            |_| {},
        );
        assert_eq!(report.status, RoundTripStatus::PassedWithReview);
        let eligibility = state_save_eligibility(
            &project,
            &edit,
            Some(&plan),
            Some(&report),
            StateSaveConditions::default(),
        );
        assert!(!eligibility.eligible);
        assert!(
            eligibility
                .reasons
                .contains(&StateSaveBlockReason::ReviewRequired)
        );
        assert!(
            eligibility
                .reasons
                .contains(&StateSaveBlockReason::RoundTripNotPassed)
        );
        let approved = state_save_eligibility(
            &project,
            &edit,
            Some(&plan),
            Some(&report),
            StateSaveConditions {
                allow_review_required: true,
                ..Default::default()
            },
        );
        assert!(approved.eligible);
        let authorization = approved.authorization.unwrap();
        assert!(authorization.allow_review_required);
        assert!(validate_authorization(&edit, &plan, &report, &authorization).is_ok());
        let mut denied = authorization;
        denied.allow_review_required = false;
        assert!(validate_authorization(&edit, &plan, &report, &denied).is_err());
        cleanup(&root);
    }

    #[test]
    fn net_zero_plan_creates_no_save_metadata() {
        let (root, project, edit) = test_project("net-zero");
        let plan = plan_state_patches(&project, &edit);
        let report = validate(&project, &edit, &plan);
        let eligibility = state_save_eligibility(
            &project,
            &edit,
            Some(&plan),
            Some(&report),
            StateSaveConditions::default(),
        );
        assert!(!eligibility.eligible);
        assert!(
            eligibility
                .reasons
                .contains(&StateSaveBlockReason::NoChanges)
        );
        assert!(!root.join(INTERNAL_DIRECTORY).exists());
        cleanup(&root);
    }

    #[test]
    fn source_change_invalidates_authorization_before_backup() {
        let (root, project, mut edit) = test_project("source-change");
        change_manpower(&project, &mut edit, 2);
        let plan = plan_state_patches(&project, &edit);
        let report = validate(&project, &edit, &plan);
        fs::write(
            root.join("history/states/1-Test.txt"),
            "state={id=1 state_category=rural provinces={1} manpower=99 history={owner=TAG}}",
        )
        .unwrap();
        let eligibility = state_save_eligibility(
            &project,
            &edit,
            Some(&plan),
            Some(&report),
            StateSaveConditions::default(),
        );
        assert!(!eligibility.eligible);
        assert!(
            eligibility
                .reasons
                .contains(&StateSaveBlockReason::SourceChanged)
        );
        assert!(!root.join(INTERNAL_DIRECTORY).exists());
        cleanup(&root);
    }

    #[test]
    fn modified_state_save_creates_verified_backup_and_new_baseline_candidate() {
        let (root, project, mut edit) = test_project("modified-save");
        let state_path = root.join("history/states/1-Test.txt");
        let before = fs::read(&state_path).unwrap();
        change_manpower(&project, &mut edit, 2);
        let plan = plan_state_patches(&project, &edit);
        let report = validate(&project, &edit, &plan);
        let authorization = authorize(&project, &edit, &plan, &report);
        let result = execute_state_save(
            &project,
            &edit,
            &plan,
            &report,
            &authorization,
            &StateSaveCancellation::default(),
            StateSaveFault::None,
            |_, _, _| {},
        );
        assert_eq!(
            result.outcome,
            StateSaveOutcome::Completed,
            "{}",
            result.summary_text()
        );
        assert_ne!(fs::read(&state_path).unwrap(), before);
        assert!(result.reloaded_project.is_some());
        let manifest_path = result.backup_path.as_ref().unwrap().join("manifest.toml");
        verify_backup(&manifest_path).unwrap();
        let manifest: BackupManifest = read_toml(&manifest_path).unwrap();
        let backup = manifest.entries[0].backup_path.as_ref().unwrap();
        assert_eq!(fs::read(backup).unwrap(), before);
        assert!(!root.join(INTERNAL_DIRECTORY).join(LOCK_FILE).exists());
        assert_no_transaction_siblings(&root);
        cleanup(&root);
    }

    #[test]
    fn staging_failure_never_changes_final_file() {
        let (root, project, mut edit) = test_project("staging-failure");
        let state_path = root.join("history/states/1-Test.txt");
        let before = fs::read(&state_path).unwrap();
        change_manpower(&project, &mut edit, 2);
        let plan = plan_state_patches(&project, &edit);
        let report = validate(&project, &edit, &plan);
        let authorization = authorize(&project, &edit, &plan, &report);
        let result = execute_state_save(
            &project,
            &edit,
            &plan,
            &report,
            &authorization,
            &StateSaveCancellation::default(),
            StateSaveFault::FailStagingAt(0),
            |_, _, _| {},
        );
        assert_eq!(result.outcome, StateSaveOutcome::FailedBeforeCommit);
        assert_eq!(fs::read(&state_path).unwrap(), before);
        assert!(!root.join(INTERNAL_DIRECTORY).join(LOCK_FILE).exists());
        assert_no_transaction_siblings(&root);
        cleanup(&root);
    }

    #[test]
    fn mid_commit_and_post_validation_failures_restore_original_bytes() {
        for (name, fault) in [
            ("mid-commit", StateSaveFault::FailAfterCommit(1)),
            ("post-validation", StateSaveFault::FailPostValidation),
        ] {
            let (root, project, mut edit) = test_project(name);
            let state_path = root.join("history/states/1-Test.txt");
            let before = fs::read(&state_path).unwrap();
            change_manpower(&project, &mut edit, 2);
            let plan = plan_state_patches(&project, &edit);
            let report = validate(&project, &edit, &plan);
            let authorization = authorize(&project, &edit, &plan, &report);
            let result = execute_state_save(
                &project,
                &edit,
                &plan,
                &report,
                &authorization,
                &StateSaveCancellation::default(),
                fault,
                |_, _, _| {},
            );
            assert_eq!(
                result.outcome,
                StateSaveOutcome::RolledBack,
                "{}",
                result.summary_text()
            );
            assert_eq!(fs::read(&state_path).unwrap(), before);
            assert!(
                result
                    .backup_path
                    .as_ref()
                    .is_some_and(|path| path.exists())
            );
            assert!(!root.join(INTERNAL_DIRECTORY).join(LOCK_FILE).exists());
            assert_no_transaction_siblings(&root);
            cleanup(&root);
        }
    }

    #[test]
    fn interrupted_commit_is_detected_and_recovered_from_journal() {
        let (root, project, mut edit) = test_project("recovery");
        let state_path = root.join("history/states/1-Test.txt");
        let before = fs::read(&state_path).unwrap();
        change_manpower(&project, &mut edit, 2);
        let plan = plan_state_patches(&project, &edit);
        let report = validate(&project, &edit, &plan);
        let authorization = authorize(&project, &edit, &plan, &report);
        let interrupted = execute_state_save(
            &project,
            &edit,
            &plan,
            &report,
            &authorization,
            &StateSaveCancellation::default(),
            StateSaveFault::LeaveInterruptedAfterCommit(1),
            |_, _, _| {},
        );
        assert_eq!(interrupted.outcome, StateSaveOutcome::RecoveryRequired);
        assert_ne!(fs::read(&state_path).unwrap(), before);
        assert!(detect_state_save_recovery(&root).is_some());
        let recovered = recover_interrupted_state_save(&root);
        assert_eq!(
            recovered.outcome,
            StateSaveOutcome::RolledBack,
            "{}",
            recovered.summary_text()
        );
        assert_eq!(fs::read(&state_path).unwrap(), before);
        assert!(recovered.reloaded_project.is_some());
        assert!(detect_state_save_recovery(&root).is_none());
        assert_no_transaction_siblings(&root);
        cleanup(&root);
    }

    #[test]
    fn completed_save_recovery_finishes_temporary_cleanup() {
        let (root, project, mut edit) = test_project("completed-cleanup-recovery");
        change_manpower(&project, &mut edit, 2);
        let plan = plan_state_patches(&project, &edit);
        let report = validate(&project, &edit, &plan);
        let authorization = authorize(&project, &edit, &plan, &report);
        let saved = execute_state_save(
            &project,
            &edit,
            &plan,
            &report,
            &authorization,
            &StateSaveCancellation::default(),
            StateSaveFault::None,
            |_, _, _| {},
        );
        assert_eq!(
            saved.outcome,
            StateSaveOutcome::Completed,
            "{}",
            saved.summary_text()
        );

        let journal_path = saved.journal_path.clone().unwrap();
        let journal: SaveTransactionJournal = read_toml(&journal_path).unwrap();
        let operation = &journal.operations[0];
        let stage_path = operation.stage_path.as_ref().unwrap();
        let rollback_path = operation.rollback_path.as_ref().unwrap();
        fs::write(stage_path, b"stale stage").unwrap();
        fs::write(rollback_path, b"stale rollback").unwrap();
        let lock_path = root.join(INTERNAL_DIRECTORY).join(LOCK_FILE);
        create_lock(
            &lock_path,
            &SaveLock {
                transaction_id: journal.transaction_id.clone(),
                timestamp: timestamp(),
                pid: std::process::id(),
                project_root: canonical_root(&root).unwrap(),
                journal_path,
                application_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        )
        .unwrap();

        let recovered = recover_interrupted_state_save(&root);
        assert_eq!(
            recovered.outcome,
            StateSaveOutcome::Completed,
            "{}",
            recovered.summary_text()
        );
        assert!(!stage_path.exists());
        assert!(!rollback_path.exists());
        assert!(!lock_path.exists());
        cleanup(&root);
    }

    #[test]
    fn created_and_removed_files_follow_manifest_and_commit_rules() {
        let (create_root, create_project, mut create_edit) = test_project("created");
        create_edit
            .create_state(2, EditableStateProperties::default(), false)
            .unwrap();
        let create_plan = plan_state_patches(&create_project, &create_edit);
        let create_report = validate(&create_project, &create_edit, &create_plan);
        let create_authorization =
            authorize(&create_project, &create_edit, &create_plan, &create_report);
        let created = execute_state_save(
            &create_project,
            &create_edit,
            &create_plan,
            &create_report,
            &create_authorization,
            &StateSaveCancellation::default(),
            StateSaveFault::None,
            |_, _, _| {},
        );
        assert_eq!(
            created.outcome,
            StateSaveOutcome::Completed,
            "{}",
            created.summary_text()
        );
        assert!(create_root.join("history/states/2-State_2.txt").exists());
        let manifest: BackupManifest =
            read_toml(&created.backup_path.unwrap().join("manifest.toml")).unwrap();
        let entry = manifest
            .entries
            .iter()
            .find(|entry| entry.kind == SaveFileKind::Create)
            .unwrap();
        assert!(entry.original_fingerprint.is_none());
        assert!(entry.backup_path.is_none());
        cleanup(&create_root);

        let (remove_root, remove_project, mut remove_edit) = test_project("removed");
        let removed_path = remove_root.join("history/states/1-Test.txt");
        let before = fs::read(&removed_path).unwrap();
        remove_edit
            .remove_state(1, StateRemovalPolicy::Unassign)
            .unwrap();
        let remove_plan = plan_state_patches(&remove_project, &remove_edit);
        let remove_report = validate(&remove_project, &remove_edit, &remove_plan);
        let remove_authorization =
            authorize(&remove_project, &remove_edit, &remove_plan, &remove_report);
        let removed = execute_state_save(
            &remove_project,
            &remove_edit,
            &remove_plan,
            &remove_report,
            &remove_authorization,
            &StateSaveCancellation::default(),
            StateSaveFault::None,
            |_, _, _| {},
        );
        assert_eq!(
            removed.outcome,
            StateSaveOutcome::Completed,
            "{}",
            removed.summary_text()
        );
        assert!(!removed_path.exists());
        let manifest: BackupManifest =
            read_toml(&removed.backup_path.unwrap().join("manifest.toml")).unwrap();
        let backup = manifest
            .entries
            .iter()
            .find(|entry| entry.kind == SaveFileKind::Remove)
            .and_then(|entry| entry.backup_path.as_ref())
            .unwrap();
        assert_eq!(fs::read(backup).unwrap(), before);
        assert_no_transaction_siblings(&remove_root);
        cleanup(&remove_root);
    }

    #[test]
    #[ignore = "requires HOI4_STATE_EDITOR_REAL_MOD_ROOT and writes only controlled TEMP copies"]
    fn real_mod_phase4c_transactional_save_smoke_on_controlled_copies() {
        let original = std::env::var_os("HOI4_STATE_EDITOR_REAL_MOD_ROOT")
            .map(PathBuf::from)
            .expect("set HOI4_STATE_EDITOR_REAL_MOD_ROOT");
        let original = dunce::canonicalize(original).unwrap();
        let original_snapshot = snapshot_original(&original);
        assert!(
            original_snapshot.len() > 2,
            "expected map files and state files"
        );

        with_real_copy(&original, "a-properties", |root, project, mut edit| {
            let mut properties = EditableStateProperties::from_state(&edit.state_data(1).unwrap());
            assert_eq!(properties.manpower, Some(142_000));
            assert_eq!(properties.state_category.as_deref(), Some("rural"));
            assert_eq!(properties.resources.get("oil"), Some(&8));
            properties.manpower = Some(150_000);
            properties.state_category = Some("town".to_owned());
            properties.resources.insert("oil".to_owned(), 10);
            edit.update_state_properties(1, properties).unwrap();
            let saved = save_real(&project, &edit, StateSaveFault::None);
            assert_eq!(
                saved.outcome,
                StateSaveOutcome::Completed,
                "{}",
                saved.summary_text()
            );
            assert!(saved.backup_path.as_ref().is_some_and(|path| path.exists()));
            assert_promotable_clean_baseline(saved, root);
        });

        with_real_copy(&original, "b-province-5144", |root, project, mut edit| {
            assert_eq!(edit.province_state_id(5144), Some(1));
            assert_eq!(edit.province_data(5144).unwrap().victory_point, Some(5));
            let target = edit
                .valid_state_ids()
                .iter()
                .copied()
                .find(|id| *id != 1)
                .unwrap();
            edit.reassign_provinces(&[5144], Some(target)).unwrap();
            let saved = save_real(&project, &edit, StateSaveFault::None);
            assert_eq!(
                saved.outcome,
                StateSaveOutcome::Completed,
                "{}",
                saved.summary_text()
            );
            let reloaded = saved.reloaded_project.as_ref().unwrap();
            assert_eq!(reloaded.state_by_province.get(&5144), Some(&target));
            let state = reloaded
                .state_document(target)
                .unwrap()
                .data
                .as_ref()
                .unwrap();
            assert!(
                state
                    .history
                    .victory_points
                    .iter()
                    .any(|vp| { vp.province_id == 5144 && vp.value == 5 })
            );
            assert_promotable_clean_baseline(saved, root);
        });

        with_real_copy(&original, "c-create-state", |root, project, mut edit| {
            let new_state_id = edit.suggest_next_state_id();
            let new_state_name = format!("STATE_{new_state_id}");
            edit.toggle_selected_province(5144).unwrap();
            edit.create_state(
                new_state_id,
                EditableStateProperties {
                    name: Some(new_state_name.clone()),
                    manpower: Some(1_000),
                    state_category: Some("rural".to_owned()),
                    ..Default::default()
                },
                true,
            )
            .unwrap();
            let saved = save_real(&project, &edit, StateSaveFault::None);
            assert_eq!(
                saved.outcome,
                StateSaveOutcome::Completed,
                "{}",
                saved.summary_text()
            );
            assert!(
                root.join(format!(
                    "history/states/{new_state_id}-State_{new_state_id}.txt"
                ))
                .exists()
            );
            assert!(
                saved
                    .reloaded_project
                    .as_ref()
                    .unwrap()
                    .state_document(new_state_id)
                    .is_some()
            );
            let manifest: BackupManifest =
                read_toml(&saved.backup_path.as_ref().unwrap().join("manifest.toml")).unwrap();
            assert!(manifest.entries.iter().any(|entry| {
                entry.kind == SaveFileKind::Create
                    && entry.original_fingerprint.is_none()
                    && entry.backup_path.is_none()
            }));
            assert_promotable_clean_baseline(saved, root);
        });

        with_real_copy(&original, "d-remove", |root, project, mut edit| {
            let removal_id = removable_state(&edit);
            let removed_path = project.state_document(removal_id).unwrap().path.clone();
            edit.remove_state(removal_id, StateRemovalPolicy::MoveToState(1))
                .unwrap();
            let saved = save_real(&project, &edit, StateSaveFault::None);
            assert_eq!(
                saved.outcome,
                StateSaveOutcome::Completed,
                "{}",
                saved.summary_text()
            );
            assert!(!removed_path.exists());
            assert!(
                saved
                    .reloaded_project
                    .as_ref()
                    .unwrap()
                    .state_document(removal_id)
                    .is_none()
            );
            assert_no_transaction_siblings(root);
            assert_promotable_clean_baseline(saved, root);
        });

        with_real_copy(&original, "e-combined", |root, project, mut edit| {
            let mut properties = EditableStateProperties::from_state(&edit.state_data(1).unwrap());
            properties.manpower = Some(150_000);
            properties.state_category = Some("town".to_owned());
            properties.resources.insert("oil".to_owned(), 10);
            edit.update_state_properties(1, properties).unwrap();
            let provincial_id = edit
                .state_data(1)
                .unwrap()
                .provinces
                .iter()
                .copied()
                .find(|province_id| *province_id != 5144)
                .unwrap();
            edit.update_province_data(
                provincial_id,
                1,
                EditableProvinceData {
                    victory_point: Some(1),
                    buildings: BTreeMap::from([("bunker".to_owned(), 1)]),
                },
            )
            .unwrap();
            let new_state_id = edit.suggest_next_state_id();
            edit.toggle_selected_province(5144).unwrap();
            edit.create_state(
                new_state_id,
                EditableStateProperties {
                    name: Some(format!("STATE_{new_state_id}")),
                    state_category: Some("rural".to_owned()),
                    ..Default::default()
                },
                true,
            )
            .unwrap();
            let removal_id = edit
                .valid_state_ids()
                .iter()
                .copied()
                .filter(|id| *id != 1 && *id != new_state_id)
                .find(|id| {
                    let mut probe = edit.clone();
                    probe
                        .remove_state(*id, StateRemovalPolicy::MoveToState(1))
                        .is_ok()
                })
                .unwrap();
            edit.remove_state(removal_id, StateRemovalPolicy::MoveToState(1))
                .unwrap();
            let plan = plan_state_patches(&project, &edit);
            assert!(!plan.modified_files.is_empty());
            assert_eq!(plan.created_files.len(), 1);
            assert_eq!(plan.removed_files.len(), 1);
            let saved = save_real_with_plan(&project, &edit, &plan, StateSaveFault::None);
            assert_eq!(
                saved.outcome,
                StateSaveOutcome::Completed,
                "{}",
                saved.summary_text()
            );
            assert_promotable_clean_baseline(saved, root);
        });

        for (label, fault, expected) in [
            (
                "f-staging-failure",
                StateSaveFault::FailStagingAt(0),
                StateSaveOutcome::FailedBeforeCommit,
            ),
            (
                "g-mid-commit",
                StateSaveFault::FailAfterCommit(1),
                StateSaveOutcome::RolledBack,
            ),
            (
                "h-post-validation",
                StateSaveFault::FailPostValidation,
                StateSaveOutcome::RolledBack,
            ),
        ] {
            with_real_copy(&original, label, |root, project, mut edit| {
                let before = snapshot_original(root);
                let mut properties =
                    EditableStateProperties::from_state(&edit.state_data(1).unwrap());
                properties.manpower = Some(150_000);
                edit.update_state_properties(1, properties).unwrap();
                let result = save_real(&project, &edit, fault);
                assert_eq!(result.outcome, expected, "{}", result.summary_text());
                assert_eq!(snapshot_original(root), before);
                assert_no_transaction_siblings(root);
            });
        }

        with_real_copy(&original, "i-recovery", |root, project, mut edit| {
            let before = snapshot_original(root);
            let mut properties = EditableStateProperties::from_state(&edit.state_data(1).unwrap());
            properties.manpower = Some(150_000);
            edit.update_state_properties(1, properties).unwrap();
            let interrupted = save_real(
                &project,
                &edit,
                StateSaveFault::LeaveInterruptedAfterCommit(1),
            );
            assert_eq!(interrupted.outcome, StateSaveOutcome::RecoveryRequired);
            assert!(detect_state_save_recovery(root).is_some());
            let recovered = recover_interrupted_state_save(root);
            assert_eq!(
                recovered.outcome,
                StateSaveOutcome::RolledBack,
                "{}",
                recovered.summary_text()
            );
            assert_eq!(snapshot_original(root), before);
            assert!(detect_state_save_recovery(root).is_none());
        });

        with_real_copy(&original, "j-source-changed", |root, project, mut edit| {
            let mut properties = EditableStateProperties::from_state(&edit.state_data(1).unwrap());
            properties.manpower = Some(150_000);
            edit.update_state_properties(1, properties).unwrap();
            let plan = plan_state_patches(&project, &edit);
            let report = validate(&project, &edit, &plan);
            let path = project.state_document(1).unwrap().path.clone();
            let mut changed = fs::read(&path).unwrap();
            changed.extend_from_slice(b"\n# external change");
            fs::write(&path, changed).unwrap();
            let eligibility = state_save_eligibility(
                &project,
                &edit,
                Some(&plan),
                Some(&report),
                StateSaveConditions::default(),
            );
            assert!(
                eligibility
                    .reasons
                    .contains(&StateSaveBlockReason::SourceChanged)
            );
            assert!(!root.join(INTERNAL_DIRECTORY).exists());
        });

        with_real_copy(&original, "k-review", |_, project, mut edit| {
            let mut properties = EditableStateProperties::from_state(&edit.state_data(1).unwrap());
            properties.manpower = Some(150_000);
            edit.update_state_properties(1, properties).unwrap();
            let mut plan = plan_state_patches(&project, &edit);
            plan.modified_files[0].safety = PatchSafety::ReviewRequired;
            plan.summary.safe_files -= 1;
            plan.summary.review_required_files += 1;
            let report = RoundTripValidator {
                policy: RoundTripValidationPolicy {
                    allow_review_required: true,
                    ..Default::default()
                },
            }
            .validate(
                &project,
                &edit,
                &plan,
                &RoundTripCancellation::default(),
                |_| {},
            );
            assert_eq!(report.status, RoundTripStatus::PassedWithReview);
            let eligibility = state_save_eligibility(
                &project,
                &edit,
                Some(&plan),
                Some(&report),
                StateSaveConditions::default(),
            );
            assert!(
                eligibility
                    .reasons
                    .contains(&StateSaveBlockReason::ReviewRequired)
            );
        });

        with_real_copy(&original, "l-net-zero", |root, project, edit| {
            let plan = plan_state_patches(&project, &edit);
            let report = validate(&project, &edit, &plan);
            let eligibility = state_save_eligibility(
                &project,
                &edit,
                Some(&plan),
                Some(&report),
                StateSaveConditions::default(),
            );
            assert!(
                eligibility
                    .reasons
                    .contains(&StateSaveBlockReason::NoChanges)
            );
            assert!(!root.join(INTERNAL_DIRECTORY).exists());
        });

        assert_eq!(snapshot_original(&original), original_snapshot);
        println!(
            "Phase 4C controlled-copy smoke A-L passed; original root remained byte-identical: {}",
            original.display()
        );
    }

    fn with_real_copy(
        original: &Path,
        label: &str,
        run: impl FnOnce(&Path, Hoi4Project, StateEditSession),
    ) {
        let base = std::env::temp_dir()
            .join("hoi4-state-editor")
            .join("phase4c-save-smoke");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!(
            "RealModCopy-{label}-{}-{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        copy_real_project(original, &root);
        let canonical_original = dunce::canonicalize(original).unwrap();
        let canonical_copy = dunce::canonicalize(&root).unwrap();
        assert_ne!(canonical_original, canonical_copy);
        assert!(canonical_copy.starts_with(dunce::canonicalize(&base).unwrap()));
        let (project, edit) = load_real_project(&root);
        run(&root, project, edit);
        assert!(canonical_copy.starts_with(dunce::canonicalize(&base).unwrap()));
        fs::remove_dir_all(&root).unwrap();
    }

    fn copy_real_project(original: &Path, destination: &Path) {
        fs::create_dir_all(destination.join("map")).unwrap();
        fs::create_dir_all(destination.join("history/states")).unwrap();
        for relative in ["map/provinces.bmp", "map/definition.csv"] {
            fs::copy(original.join(relative), destination.join(relative)).unwrap();
        }
        let mut states = fs::read_dir(original.join("history/states"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
            })
            .collect::<Vec<_>>();
        states.sort_by_key(|path| normalized_path(path));
        assert!(!states.is_empty(), "expected at least one state file");
        for source in states {
            fs::copy(
                &source,
                destination
                    .join("history/states")
                    .join(source.file_name().unwrap()),
            )
            .unwrap();
        }
    }

    fn load_real_project(root: &Path) -> (Hoi4Project, StateEditSession) {
        let paths = ProjectPaths::discover(root).unwrap();
        let config = Config {
            preserve_ids: true,
            ..Config::default()
        };
        let bundle =
            Bundle::load(&Location::Directory(paths.map_directory.clone()), config).unwrap();
        let province_ids = bundle.map.province_ids().collect::<BTreeSet<_>>();
        let land_province_ids = bundle
            .map
            .iter_province_data()
            .filter(|(_, province)| province.kind == ProvinceKind::Land)
            .filter_map(|(_, province)| province.preserved_id)
            .collect::<BTreeSet<_>>();
        let mut project = Hoi4Project::new(paths);
        project.load_states(&province_ids, &land_province_ids);
        let edit = StateEditSession::new(&project, &bundle.map);
        (project, edit)
    }

    fn snapshot_original(root: &Path) -> BTreeMap<PathBuf, SourceFingerprint> {
        let mut paths = vec![
            PathBuf::from("map/provinces.bmp"),
            PathBuf::from("map/definition.csv"),
        ];
        let mut states = fs::read_dir(root.join("history/states"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
            })
            .map(|path| PathBuf::from("history/states").join(path.file_name().unwrap()))
            .collect::<Vec<_>>();
        states.sort_by_key(|path| normalized_path(path));
        paths.extend(states);
        paths
            .into_iter()
            .map(|relative| {
                let bytes = fs::read(root.join(&relative)).unwrap();
                (relative, SourceFingerprint::from_bytes(&bytes))
            })
            .collect()
    }

    fn save_real(
        project: &Hoi4Project,
        edit: &StateEditSession,
        fault: StateSaveFault,
    ) -> StateSaveReport {
        let plan = plan_state_patches(project, edit);
        save_real_with_plan(project, edit, &plan, fault)
    }

    fn save_real_with_plan(
        project: &Hoi4Project,
        edit: &StateEditSession,
        plan: &ProjectPatchPlan,
        fault: StateSaveFault,
    ) -> StateSaveReport {
        let report = validate(project, edit, plan);
        let authorization = authorize(project, edit, plan, &report);
        execute_state_save(
            project,
            edit,
            plan,
            &report,
            &authorization,
            &StateSaveCancellation::default(),
            fault,
            |_, _, _| {},
        )
    }

    fn removable_state(edit: &StateEditSession) -> u32 {
        edit.valid_state_ids()
            .iter()
            .copied()
            .filter(|id| *id != 1)
            .find(|id| {
                let mut probe = edit.clone();
                probe
                    .remove_state(*id, StateRemovalPolicy::MoveToState(1))
                    .is_ok()
            })
            .expect("removable loaded state")
    }

    fn assert_promotable_clean_baseline(mut saved: StateSaveReport, root: &Path) {
        let reloaded = saved.reloaded_project.take().expect("post-save reload");
        let paths = ProjectPaths::discover(root).unwrap();
        let config = Config {
            preserve_ids: true,
            ..Config::default()
        };
        let bundle = Bundle::load(&Location::Directory(paths.map_directory), config).unwrap();
        let promoted = StateEditSession::new(&reloaded, &bundle.map);
        assert!(!promoted.is_dirty());
        assert_eq!(promoted.summary().commands, 0);
        assert_eq!(promoted.summary().modified_states, 0);
        assert_no_transaction_siblings(root);
    }

    fn test_project(name: &str) -> (PathBuf, Hoi4Project, StateEditSession) {
        let root = std::env::temp_dir().join(format!(
            "phase4c-{name}-{}-{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("map")).unwrap();
        fs::create_dir_all(root.join("history/states")).unwrap();
        let image = image::RgbImage::from_pixel(1, 1, image::Rgb([1, 2, 3]));
        let mut bmp = Vec::new();
        write_rgb_bmp_image(&mut bmp, &image).unwrap();
        fs::write(root.join("map/provinces.bmp"), bmp).unwrap();
        fs::write(
            root.join("map/definition.csv"),
            "0;0;0;0;land;false;unknown;0\n1;1;2;3;land;false;plains;1\n",
        )
        .unwrap();
        fs::write(
            root.join("history/states/1-Test.txt"),
            "state={id=1 state_category=rural provinces={1} manpower=1 history={owner=TAG}}",
        )
        .unwrap();
        let paths = ProjectPaths::discover(&root).unwrap();
        let config = Config {
            preserve_ids: true,
            ..Config::default()
        };
        let bundle =
            Bundle::load(&Location::Directory(paths.map_directory.clone()), config).unwrap();
        let valid = bundle.map.province_ids().collect::<BTreeSet<_>>();
        let land = BTreeSet::from([1]);
        let mut project = Hoi4Project::new(paths);
        project.load_states(&valid, &land);
        let edit = StateEditSession::new(&project, &bundle.map);
        (root, project, edit)
    }

    fn change_manpower(project: &Hoi4Project, edit: &mut StateEditSession, manpower: u64) {
        let mut properties = EditableStateProperties::from_state(
            project.state_document(1).unwrap().data.as_ref().unwrap(),
        );
        properties.manpower = Some(manpower);
        assert!(edit.update_state_properties(1, properties).unwrap());
    }

    fn validate(
        project: &Hoi4Project,
        edit: &StateEditSession,
        plan: &ProjectPatchPlan,
    ) -> RoundTripValidationReport {
        let report = RoundTripValidator::default().validate(
            project,
            edit,
            plan,
            &RoundTripCancellation::default(),
            |_| {},
        );
        assert_eq!(
            report.status,
            RoundTripStatus::Passed,
            "{}",
            report.full_text()
        );
        report
    }

    fn authorize(
        project: &Hoi4Project,
        edit: &StateEditSession,
        plan: &ProjectPatchPlan,
        report: &RoundTripValidationReport,
    ) -> StateSaveAuthorization {
        state_save_eligibility(
            project,
            edit,
            Some(plan),
            Some(report),
            StateSaveConditions::default(),
        )
        .authorization
        .expect("safe current plan should be authorized")
    }

    fn assert_no_transaction_siblings(root: &Path) {
        let entries = fs::read_dir(root.join("history/states")).unwrap();
        for entry in entries {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(!name.contains(".hse-stage-"), "{name}");
            assert!(!name.contains(".hse-rollback-"), "{name}");
        }
    }

    fn cleanup(root: &Path) {
        fs::remove_dir_all(root).unwrap();
    }
}
