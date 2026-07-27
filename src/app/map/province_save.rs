use super::bridge::{
  deconstruct_map_data, read_rgb_bmp_image, write_adjacencies_table, write_definition_table,
  write_id_changes,
};
use super::{Bundle, SaveOperation};
use crate::app::format::{Definition, ParseCsv};
use crate::util::files::{Location, ZipArchiveFilesMap};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::{
  Arc,
  atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TRANSACTION_COUNTER: AtomicU64 = AtomicU64::new(0);
const INTERNAL_DIRECTORY: &str = ".hoi4-state-editor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvinceSaveMode {
  Save,
  Export,
}

impl ProvinceSaveMode {
  pub fn clears_dirty(self) -> bool {
    self == Self::Save
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvinceSaveStage {
  Preparing,
  EncodingBmp,
  WritingDefinition,
  Staging,
  Validating,
  BackingUp,
  Committing,
  Reloading,
  Verifying,
  Completed,
  RolledBack,
}

impl ProvinceSaveStage {
  pub fn label(self) -> &'static str {
    match self {
      Self::Preparing => "Preparing province data",
      Self::EncodingBmp => "Encoding provinces.bmp",
      Self::WritingDefinition => "Writing definition.csv",
      Self::Staging => "Writing staged files",
      Self::Validating => "Validating staged files",
      Self::BackingUp => "Creating verified backup",
      Self::Committing => "Applying validated files",
      Self::Reloading => "Reloading saved province map",
      Self::Verifying => "Verifying saved province map",
      Self::Completed => "Province map save completed",
      Self::RolledBack => "Province map save rolled back",
    }
  }

  pub fn cancellable(self) -> bool {
    matches!(
      self,
      Self::Preparing
        | Self::EncodingBmp
        | Self::WritingDefinition
        | Self::Staging
        | Self::Validating
        | Self::BackingUp
    )
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProvinceSaveProgress {
  pub stage: ProvinceSaveStage,
  pub current: u64,
  pub total: u64,
}

impl ProvinceSaveProgress {
  pub fn percent(self) -> Option<u64> {
    (self.total != 0).then(|| self.current.saturating_mul(100) / self.total)
  }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProvinceSaveTimings {
  pub preparing: Duration,
  pub encoding_bmp: Duration,
  pub writing_definition: Duration,
  pub staging: Duration,
  pub validating: Duration,
  pub backup: Duration,
  pub commit: Duration,
  pub reload: Duration,
  pub verify: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvinceSavedFile {
  pub path: PathBuf,
  pub size: u64,
  pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct ProvinceSaveReport {
  pub mode: ProvinceSaveMode,
  pub transaction_id: String,
  pub destination: PathBuf,
  pub backup_id: Option<String>,
  pub files: Vec<ProvinceSavedFile>,
  pub timings: ProvinceSaveTimings,
  pub had_id_changes: bool,
}

impl ProvinceSaveReport {
  pub fn summary_text(&self) -> String {
    let verb = match self.mode {
      ProvinceSaveMode::Save => "Province map saved",
      ProvinceSaveMode::Export => "Province map exported",
    };
    let files = self
      .files
      .iter()
      .map(|file| {
        format!(
          "{}: {} bytes, SHA-256 {}",
          file.path.display(),
          file.size,
          file.sha256
        )
      })
      .collect::<Vec<_>>()
      .join("\n");
    let backup = self
      .backup_id
      .as_deref()
      .map_or_else(|| "Backup: not applicable".to_owned(), |id| format!("Backup: {id}"));
    format!(
      "{verb} at {}\n{backup}\n{files}\nTimings (ms): prepare {} | BMP encode {} | definition {} | stage {} | validate {} | backup {} | commit {} | reload {} | verify {}",
      self.destination.display(),
      self.timings.preparing.as_millis(),
      self.timings.encoding_bmp.as_millis(),
      self.timings.writing_definition.as_millis(),
      self.timings.staging.as_millis(),
      self.timings.validating.as_millis(),
      self.timings.backup.as_millis(),
      self.timings.commit.as_millis(),
      self.timings.reload.as_millis(),
      self.timings.verify.as_millis(),
    )
  }
}

#[derive(Debug, Clone, Default)]
pub struct ProvinceSaveCancellation(Arc<AtomicBool>);

impl ProvinceSaveCancellation {
  pub fn cancel(&self) {
    self.0.store(true, Ordering::SeqCst);
  }

  fn cancelled(&self) -> bool {
    self.0.load(Ordering::SeqCst)
  }
}

#[derive(Debug)]
struct Candidate {
  files: BTreeMap<PathBuf, Vec<u8>>,
  definitions: Vec<Definition>,
  image: image::RgbImage,
  had_id_changes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fingerprint {
  size: u64,
  sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestEntry {
  relative_path: PathBuf,
  backup_path: Option<PathBuf>,
  original: Option<Fingerprint>,
  candidate: Fingerprint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProvinceBackupManifest {
  transaction_id: String,
  destination: PathBuf,
  entries: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProvinceSaveJournal {
  transaction_id: String,
  destination: PathBuf,
  state: ProvinceJournalState,
  committed_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ProvinceJournalState {
  Prepared,
  Validated,
  BackupComplete,
  Committing,
  Committed,
  Reloaded,
  Verified,
  RolledBack,
  Cancelled,
  FailedBeforeCommit,
  RecoveryRequired,
}

#[derive(Debug)]
struct PreparedFile {
  relative: PathBuf,
  destination: PathBuf,
  stage: PathBuf,
  rollback: PathBuf,
  before: Option<Vec<u8>>,
}

struct PreparedCleanup {
  paths: Vec<(PathBuf, PathBuf)>,
}

impl PreparedCleanup {
  fn new(files: &[PreparedFile]) -> Self {
    Self {
      paths: files
        .iter()
        .map(|file| (file.stage.clone(), file.rollback.clone()))
        .collect(),
    }
  }
}

impl Drop for PreparedCleanup {
  fn drop(&mut self) {
    for (stage, rollback) in &self.paths {
      let _ = remove_if_exists(stage);
      let _ = remove_if_exists(rollback);
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
  None,
  #[cfg(test)]
  Encoding,
  #[cfg(test)]
  Staging,
  #[cfg(test)]
  Validation,
  #[cfg(test)]
  Backup,
  #[cfg(test)]
  BeforeCommit,
  #[cfg(test)]
  AfterCommit(usize),
  #[cfg(test)]
  IncompleteRollback,
  #[cfg(test)]
  Reload,
  #[cfg(test)]
  Verification,
}

pub fn execute_province_save(
  location: &Location,
  bundle: &Bundle,
  mode: ProvinceSaveMode,
  cancellation: &ProvinceSaveCancellation,
  mut progress: impl FnMut(ProvinceSaveProgress),
) -> Result<ProvinceSaveReport, String> {
  execute_province_save_with_fault(
    location,
    bundle,
    mode,
    cancellation,
    Fault::None,
    &mut progress,
  )
}

fn execute_province_save_with_fault(
  location: &Location,
  bundle: &Bundle,
  mode: ProvinceSaveMode,
  cancellation: &ProvinceSaveCancellation,
  fault: Fault,
  progress: &mut impl FnMut(ProvinceSaveProgress),
) -> Result<ProvinceSaveReport, String> {
  let transaction_id = transaction_id();
  let mut timings = ProvinceSaveTimings::default();
  emit(progress, ProvinceSaveStage::Preparing, 0, 1);
  let started = Instant::now();
  let (definitions, adjacencies, id_changes) =
    deconstruct_map_data(bundle).map_err(|error| error.to_string())?;
  timings.preparing = started.elapsed();
  check_cancel(cancellation)?;
  let mut output_paths = vec![
    PathBuf::from("provinces.bmp"),
    PathBuf::from("definition.csv"),
  ];
  if !adjacencies.is_empty() {
    output_paths.push(PathBuf::from("adjacencies.csv"));
  }
  if id_changes.is_some() {
    output_paths.push(PathBuf::from("id_changes.txt"));
  }
  let directory_baseline = match location {
    Location::Directory(root) => Some(capture_directory_baseline(root, &output_paths)?),
    Location::ZipArchive(_) => None,
  };
  let archive_baseline = match location {
    Location::ZipArchive(path) => Some(read_optional(path)?),
    Location::Directory(_) => None,
  };

  let started = Instant::now();
  let expected_bmp_size = expected_bmp_size(
    bundle.map.base.color_buffer.width(),
    bundle.map.base.color_buffer.height(),
  )?;
  let mut bmp = ProgressWriter::new(Vec::with_capacity(expected_bmp_size as usize), expected_bmp_size, |current, total| {
    emit(progress, ProvinceSaveStage::EncodingBmp, current, total);
  });
  #[cfg(test)]
  if fault == Fault::Encoding {
    return Err("Injected failure during BMP encoding".to_owned());
  }
  super::bridge::write_rgb_bmp_image(&mut bmp, &bundle.map.base.color_buffer)
    .map_err(|error| format!("Cannot encode provinces.bmp: {error}"))?;
  let bmp = bmp.into_inner();
  timings.encoding_bmp = started.elapsed();
  check_cancel(cancellation)?;

  let started = Instant::now();
  emit(progress, ProvinceSaveStage::WritingDefinition, 0, 1);
  let mut definition_csv = Vec::new();
  write_definition_table(&mut definition_csv, definitions.clone())
    .map_err(|error| format!("Cannot encode definition.csv: {error}"))?;
  emit(progress, ProvinceSaveStage::WritingDefinition, 1, 1);
  timings.writing_definition = started.elapsed();

  let had_id_changes = id_changes.is_some();
  let mut files = BTreeMap::from([
    (PathBuf::from("provinces.bmp"), bmp),
    (PathBuf::from("definition.csv"), definition_csv),
  ]);
  if !adjacencies.is_empty() {
    let mut bytes = Vec::new();
    write_adjacencies_table(&mut bytes, adjacencies)
      .map_err(|error| format!("Cannot encode adjacencies.csv: {error}"))?;
    files.insert(PathBuf::from("adjacencies.csv"), bytes);
  }
  if let Some(id_changes) = id_changes {
    let mut bytes = Vec::new();
    write_id_changes(&mut bytes, id_changes)
      .map_err(|error| format!("Cannot encode id_changes.txt: {error}"))?;
    files.insert(PathBuf::from("id_changes.txt"), bytes);
  }
  let candidate = Candidate {
    files,
    definitions,
    image: (*bundle.map.base.color_buffer).clone(),
    had_id_changes,
  };

  match location {
    Location::Directory(root) => execute_directory(
      root,
      candidate,
      mode,
      cancellation,
      fault,
      directory_baseline.expect("Directory save has a destination baseline"),
      transaction_id.clone(),
      timings,
      progress,
    ),
    Location::ZipArchive(path) => {
      if mode == ProvinceSaveMode::Save {
        return Err("Saving the current Province workspace to an archive is not supported.".to_owned());
      }
      execute_archive(
        path,
        candidate,
        cancellation,
        fault,
        archive_baseline.expect("Archive export has a destination baseline"),
        transaction_id,
        timings,
        progress,
      )
    }
  }
}

#[allow(clippy::too_many_arguments)]
fn execute_directory(
  root: &Path,
  candidate: Candidate,
  mode: ProvinceSaveMode,
  cancellation: &ProvinceSaveCancellation,
  fault: Fault,
  destination_baseline: BTreeMap<PathBuf, Option<Vec<u8>>>,
  transaction_id: String,
  mut timings: ProvinceSaveTimings,
  progress: &mut impl FnMut(ProvinceSaveProgress),
) -> Result<ProvinceSaveReport, String> {
  fs::create_dir_all(root)
    .map_err(|error| format!("Cannot create export directory {}: {error}", root.display()))?;
  let internal_root = internal_root(root);
  let lock_path = mode
    .clears_dirty()
    .then(|| internal_root.join(INTERNAL_DIRECTORY).join("province-save.lock"));
  if let Some(lock) = &lock_path {
    create_lock(lock, &transaction_id)?;
  }
  let journal_path = mode.clears_dirty().then(|| {
    internal_root
      .join(INTERNAL_DIRECTORY)
      .join("province-save-journal.toml")
  });
  let result = (|| {
    if let Some(path) = journal_path.as_deref() {
      write_journal(
        path,
        &ProvinceSaveJournal {
          transaction_id: transaction_id.clone(),
          destination: root.to_owned(),
          state: ProvinceJournalState::Prepared,
          committed_files: Vec::new(),
        },
      )?;
    }
    let started = Instant::now();
    let mut prepared = prepare_files(
      root,
      &candidate.files,
      &destination_baseline,
      &transaction_id,
    )?;
    let _cleanup = PreparedCleanup::new(&prepared);
    emit(progress, ProvinceSaveStage::Staging, 0, prepared.len() as u64);
    for (index, file) in prepared.iter().enumerate() {
      #[cfg(test)]
      if fault == Fault::Staging && index == 0 {
        return Err("Injected failure during staging write".to_owned());
      }
      write_new_synced(&file.stage, &candidate.files[&file.relative])?;
      emit(
        progress,
        ProvinceSaveStage::Staging,
        index as u64 + 1,
        prepared.len() as u64,
      );
      check_cancel(cancellation)?;
    }
    timings.staging = started.elapsed();

    let started = Instant::now();
    emit(progress, ProvinceSaveStage::Validating, 0, 1);
    #[cfg(test)]
    if fault == Fault::Validation {
      return Err("Injected failure during staged validation".to_owned());
    }
    validate_candidate_files(&candidate, |relative| {
      let file = prepared
        .iter()
        .find(|file| file.relative == relative)
        .ok_or_else(|| format!("Missing staged path for {}", relative.display()))?;
      fs::read(&file.stage)
        .map_err(|error| format!("Cannot read staged {}: {error}", file.stage.display()))
    })?;
    emit(progress, ProvinceSaveStage::Validating, 1, 1);
    timings.validating = started.elapsed();
    if let Some(path) = journal_path.as_deref() {
      write_journal(
        path,
        &ProvinceSaveJournal {
          transaction_id: transaction_id.clone(),
          destination: root.to_owned(),
          state: ProvinceJournalState::Validated,
          committed_files: Vec::new(),
        },
      )?;
    }
    check_cancel(cancellation)?;

    let backup_id = if mode.clears_dirty() {
      let started = Instant::now();
      emit(progress, ProvinceSaveStage::BackingUp, 0, prepared.len() as u64);
      #[cfg(test)]
      if fault == Fault::Backup {
        return Err("Injected failure during backup".to_owned());
      }
      let backup_directory = internal_root
        .join(INTERNAL_DIRECTORY)
        .join("backups")
        .join(&transaction_id)
        .join("province-map");
      let manifest = create_backup(
        &backup_directory,
        root,
        &candidate.files,
        &prepared,
        &transaction_id,
        progress,
      )?;
      verify_backup(&manifest)?;
      timings.backup = started.elapsed();
      write_journal(
        journal_path.as_deref().expect("Save has a journal"),
        &ProvinceSaveJournal {
          transaction_id: transaction_id.clone(),
          destination: root.to_owned(),
          state: ProvinceJournalState::BackupComplete,
          committed_files: Vec::new(),
        },
      )?;
      Some(transaction_id.clone())
    } else {
      None
    };

    #[cfg(test)]
    if fault == Fault::BeforeCommit {
      return Err("Injected failure before commit".to_owned());
    }
    check_cancel(cancellation)?;
    verify_destinations_unchanged(&prepared)?;

    let started = Instant::now();
    let commit_result = commit_files(
      &mut prepared,
      journal_path.as_deref(),
      &transaction_id,
      fault,
      progress,
    );
    timings.commit = started.elapsed();
    if let Err(error) = commit_result {
      #[cfg(test)]
      if fault == Fault::IncompleteRollback
        && let Some(committed) = prepared.iter().find(|file| file.rollback.exists())
      {
        remove_if_exists(&committed.rollback)?;
      }
      let rollback = rollback_files(&prepared);
      return match rollback {
        Ok(()) => Err(format!("{error}. Original files were restored and verified.")),
        Err(rollback) => Err(format!(
          "{error}. CRITICAL: rollback was incomplete: {rollback}. Recovery is required."
        )),
      };
    }
    if let Some(path) = journal_path.as_deref() {
      write_journal(
        path,
        &ProvinceSaveJournal {
          transaction_id: transaction_id.clone(),
          destination: root.to_owned(),
          state: ProvinceJournalState::Committed,
          committed_files: prepared
            .iter()
            .map(|file| file.relative.clone())
            .collect(),
        },
      )?;
    }

    let post_commit = (|| {
      let started = Instant::now();
      emit(progress, ProvinceSaveStage::Reloading, 0, 1);
      #[cfg(test)]
      if fault == Fault::Reload {
        return Err("Injected failure during reload".to_owned());
      }
      validate_candidate_files(&candidate, |relative| {
        let path = root.join(relative);
        fs::read(&path).map_err(|error| format!("Cannot reload {}: {error}", path.display()))
      })?;
      emit(progress, ProvinceSaveStage::Reloading, 1, 1);
      timings.reload = started.elapsed();
      if let Some(path) = journal_path.as_deref() {
        write_journal(
          path,
          &ProvinceSaveJournal {
            transaction_id: transaction_id.clone(),
            destination: root.to_owned(),
            state: ProvinceJournalState::Reloaded,
            committed_files: prepared
              .iter()
              .map(|file| file.relative.clone())
              .collect(),
          },
        )?;
      }

      let started = Instant::now();
      emit(progress, ProvinceSaveStage::Verifying, 0, prepared.len() as u64);
      #[cfg(test)]
      if fault == Fault::Verification {
        return Err("Injected failure during final verification".to_owned());
      }
      let reports = verify_published_files(root, &candidate.files, progress)?;
      timings.verify = started.elapsed();
      if let Some(path) = journal_path.as_deref() {
        write_journal(
          path,
          &ProvinceSaveJournal {
            transaction_id: transaction_id.clone(),
            destination: root.to_owned(),
            state: ProvinceJournalState::Verified,
            committed_files: prepared
              .iter()
              .map(|file| file.relative.clone())
              .collect(),
          },
        )?;
      }
      Ok::<_, String>(reports)
    })();
    let reports = match post_commit {
      Ok(reports) => reports,
      Err(error) => {
        let rollback = rollback_files(&prepared);
        return match rollback {
          Ok(()) => Err(format!(
            "{error}. Final verification failed; original files were restored and verified."
          )),
          Err(rollback) => Err(format!(
            "{error}. CRITICAL: final verification failed and rollback was incomplete: {rollback}. Recovery is required."
          )),
        };
      }
    };
    emit(progress, ProvinceSaveStage::Completed, 1, 1);
    Ok(ProvinceSaveReport {
      mode,
      transaction_id: transaction_id.clone(),
      destination: root.to_owned(),
      backup_id,
      files: reports,
      timings,
      had_id_changes: candidate.had_id_changes,
    })
  })();

  if mode.clears_dirty()
    && let Err(error) = &result
  {
    let journal_path = internal_root
      .join(INTERNAL_DIRECTORY)
      .join("province-save-journal.toml");
    if journal_path.exists() {
      let state = if error.contains("CRITICAL") {
        ProvinceJournalState::RecoveryRequired
      } else if error.contains("restored and verified") {
        ProvinceJournalState::RolledBack
      } else if cancellation.cancelled() {
        ProvinceJournalState::Cancelled
      } else {
        ProvinceJournalState::FailedBeforeCommit
      };
      let _ = write_journal(
        &journal_path,
        &ProvinceSaveJournal {
          transaction_id: transaction_id.clone(),
          destination: root.to_owned(),
          state,
          committed_files: Vec::new(),
        },
      );
      if state == ProvinceJournalState::RolledBack {
        emit(progress, ProvinceSaveStage::RolledBack, 1, 1);
      }
    }
  }
  let recovery_required = result
    .as_ref()
    .is_err_and(|error| error.contains("CRITICAL"));
  if let Some(lock) = lock_path
    && !recovery_required
  {
    let _ = fs::remove_file(lock);
  }
  result
}

#[allow(clippy::too_many_arguments)]
fn execute_archive(
  path: &Path,
  candidate: Candidate,
  cancellation: &ProvinceSaveCancellation,
  fault: Fault,
  before: Option<Vec<u8>>,
  transaction_id: String,
  mut timings: ProvinceSaveTimings,
  progress: &mut impl FnMut(ProvinceSaveProgress),
) -> Result<ProvinceSaveReport, String> {
  #[cfg(not(test))]
  let _ = fault;
  let parent = path
    .parent()
    .ok_or_else(|| format!("Archive has no parent: {}", path.display()))?;
  fs::create_dir_all(parent)
    .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
  let stage = sibling_path(path, "stage", &transaction_id)?;
  let rollback = sibling_path(path, "rollback", &transaction_id)?;
  remove_if_exists(&stage)?;
  remove_if_exists(&rollback)?;
  let result = (|| {
    let started = Instant::now();
    emit(progress, ProvinceSaveStage::Staging, 0, 1);
    let mut archive = ZipArchiveFilesMap::with_capacity_and_comment(
      candidate.files.len(),
      format!("Generated by {}", crate::APPNAME),
    );
    for (relative, bytes) in &candidate.files {
      archive.get_or_insert_new(relative).extend_from_slice(bytes);
    }
    let mut cursor = Cursor::new(Vec::new());
    archive
      .to_writer(&mut cursor)
      .map_err(|error| format!("Cannot encode province archive: {error}"))?;
    write_new_synced(&stage, &cursor.into_inner())?;
    timings.staging = started.elapsed();
    emit(progress, ProvinceSaveStage::Staging, 1, 1);
    check_cancel(cancellation)?;

    let started = Instant::now();
    emit(progress, ProvinceSaveStage::Validating, 0, 1);
    let staged_archive = ZipArchiveFilesMap::from_fs(&stage)
      .map_err(|error| format!("Cannot validate staged archive: {error}"))?;
    let names = staged_archive
      .iter()
      .map(|(name, _)| name.to_owned())
      .collect::<BTreeSet<_>>();
    let expected = candidate.files.keys().cloned().collect::<BTreeSet<_>>();
    if names != expected {
      return Err("Staged archive contains files outside the province export allowlist.".to_owned());
    }
    validate_candidate_files(&candidate, |relative| {
      staged_archive
        .get(relative)
        .cloned()
        .ok_or_else(|| format!("Staged archive is missing {}", relative.display()))
    })?;
    timings.validating = started.elapsed();
    emit(progress, ProvinceSaveStage::Validating, 1, 1);
    check_cancel(cancellation)?;

    #[cfg(test)]
    if fault == Fault::BeforeCommit {
      return Err("Injected failure before commit".to_owned());
    }
    if read_optional(path)? != before {
      return Err(format!(
        "{} changed outside the editor while the export was being prepared. Export was cancelled.",
        path.display()
      ));
    }

    let started = Instant::now();
    emit(progress, ProvinceSaveStage::Committing, 0, 1);
    publish_file(&stage, path, &rollback, before.is_some())?;
    timings.commit = started.elapsed();
    emit(progress, ProvinceSaveStage::Committing, 1, 1);

    #[cfg(test)]
    if fault == Fault::AfterCommit(1) {
      rollback_one(path, &rollback, before.is_some())?;
      return Err("Injected failure after archive commit; original archive restored.".to_owned());
    }

    let started = Instant::now();
    emit(progress, ProvinceSaveStage::Verifying, 0, 1);
    let final_bytes =
      fs::read(path).map_err(|error| format!("Cannot verify {}: {error}", path.display()))?;
    let final_archive = ZipArchiveFilesMap::from_fs(path)
      .map_err(|error| format!("Cannot reopen final archive: {error}"))?;
    validate_candidate_files(&candidate, |relative| {
      final_archive
        .get(relative)
        .cloned()
        .ok_or_else(|| format!("Final archive is missing {}", relative.display()))
    })?;
    timings.verify = started.elapsed();
    emit(progress, ProvinceSaveStage::Verifying, 1, 1);
    remove_if_exists(&rollback)?;
    emit(progress, ProvinceSaveStage::Completed, 1, 1);
    Ok(ProvinceSaveReport {
      mode: ProvinceSaveMode::Export,
      transaction_id,
      destination: path.to_owned(),
      backup_id: None,
      files: vec![ProvinceSavedFile {
        path: path.to_owned(),
        size: final_bytes.len() as u64,
        sha256: sha256(&final_bytes),
      }],
      timings,
      had_id_changes: candidate.had_id_changes,
    })
  })();

  let _ = remove_if_exists(&stage);
  if result.is_err() && path.exists() && rollback.exists() {
    let _ = rollback_one(path, &rollback, before.is_some());
  }
  let _ = remove_if_exists(&rollback);
  result
}

fn prepare_files(
  root: &Path,
  files: &BTreeMap<PathBuf, Vec<u8>>,
  baseline: &BTreeMap<PathBuf, Option<Vec<u8>>>,
  transaction_id: &str,
) -> Result<Vec<PreparedFile>, String> {
  files
    .keys()
    .map(|relative| {
      let destination = root.join(relative);
      Ok(PreparedFile {
        relative: relative.clone(),
        stage: sibling_path(&destination, "stage", transaction_id)?,
        rollback: sibling_path(&destination, "rollback", transaction_id)?,
        before: baseline
          .get(relative)
          .cloned()
          .ok_or_else(|| format!("Missing destination baseline for {}", relative.display()))?,
        destination,
      })
    })
    .collect()
}

fn capture_directory_baseline(
  root: &Path,
  paths: &[PathBuf],
) -> Result<BTreeMap<PathBuf, Option<Vec<u8>>>, String> {
  paths
    .iter()
    .map(|relative| Ok((relative.clone(), read_optional(&root.join(relative))?)))
    .collect()
}

fn validate_candidate_files(
  candidate: &Candidate,
  mut read: impl FnMut(&Path) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
  let bmp = read(Path::new("provinces.bmp"))?;
  validate_bmp(&bmp, &candidate.image)?;
  let csv = read(Path::new("definition.csv"))?;
  let definitions = Definition::read_records(csv.as_slice())
    .map_err(|error| format!("Staged definition.csv is invalid: {error}"))?;
  if definitions != candidate.definitions {
    return Err("Staged definition.csv does not match the working province model.".to_owned());
  }
  validate_definition_colors(&definitions, &candidate.image)
}

fn validate_bmp(bytes: &[u8], expected: &image::RgbImage) -> Result<(), String> {
  if bytes.len() < 54 || &bytes[..2] != b"BM" {
    return Err("Staged provinces.bmp has an invalid BMP header.".to_owned());
  }
  let file_size = le_u32(bytes, 2)? as usize;
  let pixel_offset = le_u32(bytes, 10)? as usize;
  let dib_size = le_u32(bytes, 14)?;
  let width = le_i32(bytes, 18)?;
  let height = le_i32(bytes, 22)?;
  let planes = le_u16(bytes, 26)?;
  let bit_depth = le_u16(bytes, 28)?;
  let compression = le_u32(bytes, 30)?;
  if dib_size != 40
    || width <= 0
    || height <= 0
    || planes != 1
    || bit_depth != 24
    || compression != 0
  {
    return Err(format!(
      "Staged provinces.bmp format mismatch: DIB {dib_size}, {width}x{height}, planes {planes}, {bit_depth}-bit, compression {compression}."
    ));
  }
  if (width as u32, height as u32) != expected.dimensions() {
    return Err(format!(
      "Staged provinces.bmp is {}x{}, expected {}x{}.",
      width,
      height,
      expected.width(),
      expected.height()
    ));
  }
  let stride = (width as u64 * 3).div_ceil(4) * 4;
  let expected_size = pixel_offset as u64 + stride * height as u64;
  if file_size != bytes.len() || expected_size != bytes.len() as u64 {
    return Err(format!(
      "Staged provinces.bmp size mismatch: header {file_size}, actual {}, expected {expected_size}.",
      bytes.len()
    ));
  }
  let decoded = read_rgb_bmp_image(bytes)
    .map_err(|error| format!("Staged provinces.bmp cannot be reloaded: {error}"))?;
  if decoded != *expected {
    return Err("Staged provinces.bmp pixels do not match the working province model.".to_owned());
  }
  Ok(())
}

fn validate_definition_colors(
  definitions: &[Definition],
  image: &image::RgbImage,
) -> Result<(), String> {
  let ids = definitions.iter().map(|definition| definition.id).collect::<BTreeSet<_>>();
  let defined = definitions.iter().map(|definition| definition.rgb).collect::<BTreeSet<_>>();
  let pixels = image.pixels().map(|pixel| pixel.0).collect::<BTreeSet<_>>();
  if ids.len() != definitions.len() {
    return Err("Staged definition.csv contains duplicate province IDs.".to_owned());
  }
  if defined.len() != definitions.len() {
    return Err("Staged definition.csv contains duplicate province colors.".to_owned());
  }
  if defined != pixels {
    return Err("Staged BMP colors and definition.csv province colors do not match.".to_owned());
  }
  Ok(())
}

fn create_backup(
  backup_directory: &Path,
  root: &Path,
  candidate: &BTreeMap<PathBuf, Vec<u8>>,
  prepared: &[PreparedFile],
  transaction_id: &str,
  progress: &mut impl FnMut(ProvinceSaveProgress),
) -> Result<PathBuf, String> {
  let files_directory = backup_directory.join("files");
  fs::create_dir_all(&files_directory)
    .map_err(|error| format!("Cannot create {}: {error}", files_directory.display()))?;
  let mut entries = Vec::with_capacity(prepared.len());
  for (index, file) in prepared.iter().enumerate() {
    let backup_path = file.before.as_ref().map(|bytes| {
      let path = files_directory.join(&file.relative);
      (path, bytes)
    });
    if let Some((path, bytes)) = &backup_path {
      write_new_synced(path, bytes)?;
    }
    entries.push(ManifestEntry {
      relative_path: file.relative.clone(),
      backup_path: backup_path.as_ref().map(|(path, _)| path.clone()),
      original: file.before.as_deref().map(fingerprint),
      candidate: fingerprint(&candidate[&file.relative]),
    });
    emit(
      progress,
      ProvinceSaveStage::BackingUp,
      index as u64 + 1,
      prepared.len() as u64,
    );
  }
  let manifest_path = backup_directory.join("manifest.toml");
  write_toml_atomic(
    &manifest_path,
    &ProvinceBackupManifest {
      transaction_id: transaction_id.to_owned(),
      destination: root.to_owned(),
      entries,
    },
  )?;
  Ok(manifest_path)
}

fn verify_backup(manifest_path: &Path) -> Result<(), String> {
  let bytes = fs::read(manifest_path)
    .map_err(|error| format!("Cannot read backup manifest {}: {error}", manifest_path.display()))?;
  let manifest: ProvinceBackupManifest = toml::from_slice(&bytes)
    .map_err(|error| format!("Cannot parse backup manifest {}: {error}", manifest_path.display()))?;
  for entry in manifest.entries {
    match (entry.original, entry.backup_path) {
      (Some(expected), Some(path)) => {
        let bytes =
          fs::read(&path).map_err(|error| format!("Cannot verify {}: {error}", path.display()))?;
        if fingerprint(&bytes) != expected {
          return Err(format!("Backup fingerprint mismatch for {}", entry.relative_path.display()));
        }
      }
      (None, None) => {}
      _ => return Err(format!("Incomplete backup for {}", entry.relative_path.display())),
    }
  }
  Ok(())
}

fn verify_destinations_unchanged(prepared: &[PreparedFile]) -> Result<(), String> {
  for file in prepared {
    if read_optional(&file.destination)? != file.before {
      return Err(format!(
        "{} changed outside the editor while the save was being prepared. Save was cancelled.",
        file.destination.display()
      ));
    }
  }
  Ok(())
}

fn commit_files(
  prepared: &mut [PreparedFile],
  journal_path: Option<&Path>,
  transaction_id: &str,
  fault: Fault,
  progress: &mut impl FnMut(ProvinceSaveProgress),
) -> Result<(), String> {
  #[cfg(not(test))]
  let _ = fault;
  emit(progress, ProvinceSaveStage::Committing, 0, prepared.len() as u64);
  if let (Some(path), Some(first)) = (journal_path, prepared.first()) {
    write_journal(
      path,
      &ProvinceSaveJournal {
        transaction_id: transaction_id.to_owned(),
        destination: first
          .destination
          .parent()
          .unwrap_or_else(|| Path::new(""))
          .to_owned(),
        state: ProvinceJournalState::Committing,
        committed_files: Vec::new(),
      },
    )?;
  }
  for index in 0..prepared.len() {
    let file = &prepared[index];
    publish_file(
      &file.stage,
      &file.destination,
      &file.rollback,
      file.before.is_some(),
    )?;
    if let Some(path) = journal_path {
      write_journal(
        path,
        &ProvinceSaveJournal {
          transaction_id: transaction_id.to_owned(),
          destination: file
            .destination
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_owned(),
          state: ProvinceJournalState::Committing,
          committed_files: prepared[..=index]
            .iter()
            .map(|file| file.relative.clone())
            .collect(),
        },
      )?;
    }
    emit(
      progress,
      ProvinceSaveStage::Committing,
      index as u64 + 1,
      prepared.len() as u64,
    );
    #[cfg(test)]
    if fault == Fault::AfterCommit(index + 1) {
      return Err(format!("Injected failure after {} committed file(s)", index + 1));
    }
    #[cfg(test)]
    if fault == Fault::IncompleteRollback && index == 0 {
      return Err("Injected commit failure before incomplete rollback".to_owned());
    }
  }
  Ok(())
}

fn rollback_files(prepared: &[PreparedFile]) -> Result<(), String> {
  let mut errors = Vec::new();
  for file in prepared.iter().rev() {
    let committed = file.rollback.exists() || (file.before.is_none() && file.destination.exists() && !file.stage.exists());
    if committed
      && let Err(error) = rollback_one(&file.destination, &file.rollback, file.before.is_some())
    {
      errors.push(error);
    }
  }
  for file in prepared {
    match &file.before {
      Some(expected) => match fs::read(&file.destination) {
        Ok(actual) if actual == *expected => {}
        Ok(_) => errors.push(format!("Rollback verification failed for {}", file.destination.display())),
        Err(error) => errors.push(format!("Cannot verify rollback {}: {error}", file.destination.display())),
      },
      None if file.destination.exists() => {
        errors.push(format!("Rollback left created file {}", file.destination.display()))
      }
      None => {}
    }
  }
  if errors.is_empty() {
    Ok(())
  } else {
    Err(errors.join("; "))
  }
}

fn rollback_one(destination: &Path, rollback: &Path, existed: bool) -> Result<(), String> {
  if existed {
    if !rollback.exists() {
      return Err(format!("Missing rollback file {}", rollback.display()));
    }
    let discard = sibling_path(destination, "failed", "rollback")?;
    remove_if_exists(&discard)?;
    publish_file(rollback, destination, &discard, destination.exists())?;
    remove_if_exists(&discard)
  } else {
    remove_if_exists(destination)
  }
}

fn verify_published_files(
  root: &Path,
  candidate: &BTreeMap<PathBuf, Vec<u8>>,
  progress: &mut impl FnMut(ProvinceSaveProgress),
) -> Result<Vec<ProvinceSavedFile>, String> {
  let mut reports = Vec::with_capacity(candidate.len());
  for (index, (relative, expected)) in candidate.iter().enumerate() {
    let path = root.join(relative);
    let actual =
      fs::read(&path).map_err(|error| format!("Cannot verify {}: {error}", path.display()))?;
    if actual != *expected {
      return Err(format!("Final file differs from validated candidate: {}", path.display()));
    }
    reports.push(ProvinceSavedFile {
      path,
      size: actual.len() as u64,
      sha256: sha256(&actual),
    });
    emit(
      progress,
      ProvinceSaveStage::Verifying,
      index as u64 + 1,
      candidate.len() as u64,
    );
  }
  Ok(reports)
}

fn create_lock(path: &Path, transaction_id: &str) -> Result<(), String> {
  let parent = path
    .parent()
    .ok_or_else(|| format!("Lock has no parent: {}", path.display()))?;
  fs::create_dir_all(parent)
    .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
  let mut file = OpenOptions::new()
    .write(true)
    .create_new(true)
    .open(path)
    .map_err(|error| format!("Another Province Save is active ({}): {error}", path.display()))?;
  file
    .write_all(transaction_id.as_bytes())
    .and_then(|_| file.sync_all())
    .map_err(|error| format!("Cannot persist {}: {error}", path.display()))
}

fn write_journal(path: &Path, journal: &ProvinceSaveJournal) -> Result<(), String> {
  write_toml_atomic(path, journal)
}

fn write_toml_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
  let bytes =
    toml::to_vec(value).map_err(|error| format!("Cannot serialize {}: {error}", path.display()))?;
  let temporary = sibling_path(path, "tmp", "metadata")?;
  remove_if_exists(&temporary)?;
  write_new_synced(&temporary, &bytes)?;
  if path.exists() {
    atomic_replace_existing(&temporary, path, None)?;
  } else {
    atomic_move_new(&temporary, path)?;
  }
  Ok(())
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)
      .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
  }
  let mut file = OpenOptions::new()
    .write(true)
    .create_new(true)
    .open(path)
    .map_err(|error| format!("Cannot create {}: {error}", path.display()))?;
  file
    .write_all(bytes)
    .and_then(|_| file.flush())
    .and_then(|_| file.sync_all())
    .map_err(|error| format!("Cannot persist {}: {error}", path.display()))
}

fn publish_file(
  stage: &Path,
  destination: &Path,
  rollback: &Path,
  destination_exists: bool,
) -> Result<(), String> {
  remove_if_exists(rollback)?;
  if destination_exists {
    atomic_replace_existing(stage, destination, Some(rollback))
  } else {
    atomic_move_new(stage, destination)
  }
}

#[cfg(windows)]
fn atomic_replace_existing(
  replacement: &Path,
  destination: &Path,
  backup: Option<&Path>,
) -> Result<(), String> {
  use std::os::windows::ffi::OsStrExt;
  use std::ptr;

  #[link(name = "kernel32")]
  unsafe extern "system" {
    fn ReplaceFileW(
      replaced: *const u16,
      replacement: *const u16,
      backup: *const u16,
      flags: u32,
      exclude: *mut core::ffi::c_void,
      reserved: *mut core::ffi::c_void,
    ) -> i32;
  }
  const REPLACEFILE_WRITE_THROUGH: u32 = 0x0000_0001;
  fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
  }
  let destination_wide = wide(destination);
  let replacement_wide = wide(replacement);
  let backup_wide = backup.map(wide);
  let backup_ptr = backup_wide.as_ref().map_or(ptr::null(), |path| path.as_ptr());
  // SAFETY: all pointers reference NUL-terminated UTF-16 buffers for the duration of the call.
  let result = unsafe {
    ReplaceFileW(
      destination_wide.as_ptr(),
      replacement_wide.as_ptr(),
      backup_ptr,
      REPLACEFILE_WRITE_THROUGH,
      ptr::null_mut(),
      ptr::null_mut(),
    )
  };
  if result != 0 {
    Ok(())
  } else {
    Err(format!(
      "Cannot atomically replace {} with {}: {}",
      destination.display(),
      replacement.display(),
      io::Error::last_os_error()
    ))
  }
}

#[cfg(windows)]
fn atomic_move_new(source: &Path, destination: &Path) -> Result<(), String> {
  use std::os::windows::ffi::OsStrExt;

  #[link(name = "kernel32")]
  unsafe extern "system" {
    fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
  }
  const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
  fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
  }
  let source_wide = wide(source);
  let destination_wide = wide(destination);
  // SAFETY: both pointers reference NUL-terminated UTF-16 buffers for the duration of the call.
  let result = unsafe {
    MoveFileExW(
      source_wide.as_ptr(),
      destination_wide.as_ptr(),
      MOVEFILE_WRITE_THROUGH,
    )
  };
  if result != 0 {
    Ok(())
  } else {
    Err(format!(
      "Cannot atomically publish {} as {}: {}",
      source.display(),
      destination.display(),
      io::Error::last_os_error()
    ))
  }
}

#[cfg(not(windows))]
fn atomic_replace_existing(
  replacement: &Path,
  destination: &Path,
  backup: Option<&Path>,
) -> Result<(), String> {
  if let Some(backup) = backup {
    fs::copy(destination, backup)
      .map_err(|error| format!("Cannot create rollback {}: {error}", backup.display()))?;
    sync_file(backup)?;
  }
  fs::rename(replacement, destination).map_err(|error| {
    format!(
      "Cannot atomically replace {} with {}: {error}",
      destination.display(),
      replacement.display()
    )
  })
}

#[cfg(not(windows))]
fn atomic_move_new(source: &Path, destination: &Path) -> Result<(), String> {
  fs::rename(source, destination).map_err(|error| {
    format!(
      "Cannot atomically publish {} as {}: {error}",
      source.display(),
      destination.display()
    )
  })
}

#[cfg(not(windows))]
fn sync_file(path: &Path) -> Result<(), String> {
  OpenOptions::new()
    .read(true)
    .open(path)
    .and_then(|file| file.sync_all())
    .map_err(|error| format!("Cannot sync {}: {error}", path.display()))
}

fn sibling_path(path: &Path, kind: &str, transaction_id: &str) -> Result<PathBuf, String> {
  let filename = path
    .file_name()
    .ok_or_else(|| format!("Path has no filename: {}", path.display()))?
    .to_string_lossy();
  Ok(path.with_file_name(format!(".{filename}.hoi4me-{kind}-{transaction_id}")))
}

fn internal_root(map_directory: &Path) -> PathBuf {
  if map_directory
    .file_name()
    .is_some_and(|name| name.eq_ignore_ascii_case("map"))
  {
    map_directory
      .parent()
      .unwrap_or(map_directory)
      .to_owned()
  } else {
    map_directory.to_owned()
  }
}

fn expected_bmp_size(width: u32, height: u32) -> Result<u64, String> {
  let stride = (u64::from(width) * 3).div_ceil(4) * 4;
  54u64
    .checked_add(
      stride
        .checked_mul(u64::from(height))
        .ok_or_else(|| "BMP size overflow".to_owned())?,
    )
    .ok_or_else(|| "BMP size overflow".to_owned())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
  match fs::read(path) {
    Ok(bytes) => Ok(Some(bytes)),
    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
    Err(error) => Err(format!("Cannot read {}: {error}", path.display())),
  }
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
  match fs::remove_file(path) {
    Ok(()) => Ok(()),
    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
    Err(error) => Err(format!("Cannot remove {}: {error}", path.display())),
  }
}

fn check_cancel(cancellation: &ProvinceSaveCancellation) -> Result<(), String> {
  if cancellation.cancelled() {
    Err("Province map save cancelled before commit; destination files were not changed.".to_owned())
  } else {
    Ok(())
  }
}

fn fingerprint(bytes: &[u8]) -> Fingerprint {
  Fingerprint {
    size: bytes.len() as u64,
    sha256: sha256(bytes),
  }
}

fn sha256(bytes: &[u8]) -> String {
  format!("{:X}", Sha256::digest(bytes))
}

fn transaction_id() -> String {
  let millis = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis();
  let counter = TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed);
  format!("{millis}-{}-{counter}", std::process::id())
}

fn emit(
  progress: &mut impl FnMut(ProvinceSaveProgress),
  stage: ProvinceSaveStage,
  current: u64,
  total: u64,
) {
  progress(ProvinceSaveProgress {
    stage,
    current,
    total,
  });
}

fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
  bytes
    .get(offset..offset + 2)
    .and_then(|bytes| bytes.try_into().ok())
    .map(u16::from_le_bytes)
    .ok_or_else(|| "Truncated BMP header.".to_owned())
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
  bytes
    .get(offset..offset + 4)
    .and_then(|bytes| bytes.try_into().ok())
    .map(u32::from_le_bytes)
    .ok_or_else(|| "Truncated BMP header.".to_owned())
}

fn le_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
  bytes
    .get(offset..offset + 4)
    .and_then(|bytes| bytes.try_into().ok())
    .map(i32::from_le_bytes)
    .ok_or_else(|| "Truncated BMP header.".to_owned())
}

struct ProgressWriter<W, F> {
  inner: W,
  total: u64,
  written: u64,
  last_percent: u64,
  progress: F,
}

impl<W, F> ProgressWriter<W, F> {
  fn new(inner: W, total: u64, progress: F) -> Self {
    Self {
      inner,
      total,
      written: 0,
      last_percent: 0,
      progress,
    }
  }

  fn into_inner(self) -> W {
    self.inner
  }
}

impl<W: Write, F: FnMut(u64, u64)> Write for ProgressWriter<W, F> {
  fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
    let written = self.inner.write(bytes)?;
    self.written += written as u64;
    let percent = self
      .written
      .saturating_mul(100)
      .checked_div(self.total)
      .unwrap_or(100);
    if percent != self.last_percent || self.written == self.total {
      self.last_percent = percent;
      (self.progress)(self.written, self.total);
    }
    Ok(written)
  }

  fn flush(&mut self) -> io::Result<()> {
    self.inner.flush()
  }
}

pub(super) fn save_bundle_compat(
  location: &Location,
  bundle: &Bundle,
) -> Result<SaveOperation, crate::error::Error> {
  let report = execute_province_save(
    location,
    bundle,
    ProvinceSaveMode::Save,
    &ProvinceSaveCancellation::default(),
    |_| {},
  )
  .map_err(crate::error::Error::from)?;
  Ok(SaveOperation {
    had_id_changes: report.had_id_changes,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::format::DefinitionKind;
  use crate::app::map::bridge::construct_map_data;
  use crate::config::Config;
  use image::{Rgb, RgbImage};

  #[test]
  fn only_project_save_clears_province_dirty_state() {
    assert!(ProvinceSaveMode::Save.clears_dirty());
    assert!(!ProvinceSaveMode::Export.clears_dirty());
  }

  fn bundle() -> Bundle {
    bundle_with_colors([[10, 20, 30], [40, 50, 60]])
  }

  fn bundle_with_colors(colors: [[u8; 3]; 2]) -> Bundle {
    let image = RgbImage::from_fn(3, 2, |x, _| Rgb(if x < 2 { colors[0] } else { colors[1] }));
    let definitions = colors
      .into_iter()
      .enumerate()
      .map(|(index, rgb)| Definition {
        id: index as u32 + 1,
        rgb,
        kind: DefinitionKind::Land,
        coastal: false,
        terrain: "plains".to_owned(),
        continent: 1,
      })
      .collect();
    construct_map_data(
      image,
      definitions,
      Vec::new(),
      None,
      Config {
        preserve_ids: true,
        ..Config::default()
      },
    )
    .unwrap()
  }

  fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
      "hoi4-map-editor-{name}-{}-{}",
      std::process::id(),
      TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
  }

  #[test]
  fn bmp_validation_requires_24_bit_padded_complete_output() {
    let bundle = bundle();
    let mut bytes = Vec::new();
    super::super::bridge::write_rgb_bmp_image(&mut bytes, &bundle.map.base.color_buffer).unwrap();
    assert_eq!(bytes.len() as u64, expected_bmp_size(3, 2).unwrap());
    validate_bmp(&bytes, &bundle.map.base.color_buffer).unwrap();

    let mut truncated = bytes.clone();
    truncated.pop();
    assert!(validate_bmp(&truncated, &bundle.map.base.color_buffer).is_err());
    let mut wrong_depth = bytes;
    wrong_depth[28..30].copy_from_slice(&32u16.to_le_bytes());
    assert!(validate_bmp(&wrong_depth, &bundle.map.base.color_buffer).is_err());
  }

  #[test]
  fn directory_save_publishes_complete_candidates_and_writes_verified_backup() {
    let root = root("atomic-success");
    let map = root.join("map");
    fs::create_dir_all(&map).unwrap();
    fs::write(map.join("provinces.bmp"), b"old-bmp").unwrap();
    fs::write(map.join("definition.csv"), b"old-csv").unwrap();
    let report = execute_province_save(
      &Location::Directory(map.clone()),
      &bundle(),
      ProvinceSaveMode::Save,
      &ProvinceSaveCancellation::default(),
      |_| {},
    )
    .unwrap();

    assert_eq!(report.mode, ProvinceSaveMode::Save);
    assert!(report.backup_id.is_some());
    let journal: ProvinceSaveJournal = toml::from_slice(
      &fs::read(root.join(".hoi4-state-editor/province-save-journal.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(journal.state, ProvinceJournalState::Verified);
    assert_eq!(fs::read(root.join(".hoi4-state-editor/backups").join(report.backup_id.unwrap()).join("province-map/files/provinces.bmp")).unwrap(), b"old-bmp");
    assert!(read_rgb_bmp_image(fs::read(map.join("provinces.bmp")).unwrap().as_slice()).is_ok());
    assert!(!fs::read(map.join("definition.csv")).unwrap().is_empty());
    assert!(fs::read_dir(&map).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().contains("hoi4me-")));
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn failure_after_first_commit_rolls_back_every_destination() {
    let root = root("atomic-rollback");
    let map = root.join("map");
    fs::create_dir_all(&map).unwrap();
    let originals = BTreeMap::from([
      (PathBuf::from("provinces.bmp"), b"old-bmp".to_vec()),
      (PathBuf::from("definition.csv"), b"old-csv".to_vec()),
    ]);
    for (relative, bytes) in &originals {
      fs::write(map.join(relative), bytes).unwrap();
    }
    let result = execute_province_save_with_fault(
      &Location::Directory(map.clone()),
      &bundle(),
      ProvinceSaveMode::Save,
      &ProvinceSaveCancellation::default(),
      Fault::AfterCommit(1),
      &mut |_| {},
    );
    assert!(result.is_err());
    for (relative, bytes) in originals {
      assert_eq!(fs::read(map.join(relative)).unwrap(), bytes);
    }
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn failures_across_the_save_pipeline_preserve_originals_and_clean_staging() {
    let faults = [
      Fault::Encoding,
      Fault::Staging,
      Fault::Validation,
      Fault::Backup,
      Fault::BeforeCommit,
      Fault::AfterCommit(1),
      Fault::Reload,
      Fault::Verification,
    ];
    for (index, fault) in faults.into_iter().enumerate() {
      let root = root(&format!("pipeline-failure-{index}"));
      let map = root.join("map");
      fs::create_dir_all(&map).unwrap();
      let originals = [
        ("provinces.bmp", b"old-bmp".as_slice()),
        ("definition.csv", b"old-csv".as_slice()),
      ];
      for (name, bytes) in originals {
        fs::write(map.join(name), bytes).unwrap();
      }

      let result = execute_province_save_with_fault(
        &Location::Directory(map.clone()),
        &bundle(),
        ProvinceSaveMode::Save,
        &ProvinceSaveCancellation::default(),
        fault,
        &mut |_| {},
      );

      assert!(result.is_err(), "{fault:?} unexpectedly succeeded");
      for (name, bytes) in originals {
        assert_eq!(fs::read(map.join(name)).unwrap(), bytes, "{fault:?}");
      }
      assert!(fs::read_dir(&map).unwrap().all(|entry| {
        !entry
          .unwrap()
          .file_name()
          .to_string_lossy()
          .contains("hoi4me-")
      }));
      assert!(!root
        .join(".hoi4-state-editor/province-save.lock")
        .exists());
      fs::remove_dir_all(root).unwrap();
    }
  }

  #[test]
  fn incomplete_rollback_keeps_lock_and_marks_recovery_required() {
    let root = root("recovery-required");
    let map = root.join("map");
    fs::create_dir_all(&map).unwrap();
    fs::write(map.join("provinces.bmp"), b"old-bmp").unwrap();
    fs::write(map.join("definition.csv"), b"old-csv").unwrap();

    let result = execute_province_save_with_fault(
      &Location::Directory(map),
      &bundle(),
      ProvinceSaveMode::Save,
      &ProvinceSaveCancellation::default(),
      Fault::IncompleteRollback,
      &mut |_| {},
    );

    assert!(result.unwrap_err().contains("CRITICAL"));
    assert!(root
      .join(".hoi4-state-editor/province-save.lock")
      .is_file());
    let journal: ProvinceSaveJournal = toml::from_slice(
      &fs::read(root.join(".hoi4-state-editor/province-save-journal.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(journal.state, ProvinceJournalState::RecoveryRequired);
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn export_keeps_source_and_dirty_semantics_outside_the_transaction() {
    let root = root("export");
    fs::create_dir_all(&root).unwrap();
    let report = execute_province_save(
      &Location::Directory(root.clone()),
      &bundle(),
      ProvinceSaveMode::Export,
      &ProvinceSaveCancellation::default(),
      |_| {},
    )
    .unwrap();
    assert_eq!(report.mode, ProvinceSaveMode::Export);
    assert!(report.backup_id.is_none());
    assert!(root.join("provinces.bmp").is_file());
    assert!(root.join("definition.csv").is_file());
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn destination_is_always_an_old_or_complete_new_file_during_save() {
    let root = root("atomic-observer");
    let map = root.join("map");
    let candidate_root = root.join("candidate");
    fs::create_dir_all(&map).unwrap();
    fs::create_dir_all(&candidate_root).unwrap();
    execute_province_save(
      &Location::Directory(map.clone()),
      &bundle(),
      ProvinceSaveMode::Export,
      &ProvinceSaveCancellation::default(),
      |_| {},
    )
    .unwrap();
    execute_province_save(
      &Location::Directory(candidate_root.clone()),
      &bundle_with_colors([[11, 21, 31], [41, 51, 61]]),
      ProvinceSaveMode::Export,
      &ProvinceSaveCancellation::default(),
      |_| {},
    )
    .unwrap();
    let old = ["provinces.bmp", "definition.csv"]
      .into_iter()
      .map(|name| (name, fs::read(map.join(name)).unwrap()))
      .collect::<BTreeMap<_, _>>();
    let new = ["provinces.bmp", "definition.csv"]
      .into_iter()
      .map(|name| (name, fs::read(candidate_root.join(name)).unwrap()))
      .collect::<BTreeMap<_, _>>();

    execute_province_save(
      &Location::Directory(map.clone()),
      &bundle_with_colors([[11, 21, 31], [41, 51, 61]]),
      ProvinceSaveMode::Save,
      &ProvinceSaveCancellation::default(),
      |progress| {
        for name in ["provinces.bmp", "definition.csv"] {
          let observed = fs::read(map.join(name)).unwrap();
          assert!(
            observed == old[name] || observed == new[name],
            "{name} exposed partial bytes during {:?}",
            progress.stage
          );
          if progress.stage.cancellable() {
            assert_eq!(observed, old[name]);
          }
        }
      },
    )
    .unwrap();
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn cancellation_before_commit_preserves_destination_and_cleans_staging() {
    let root = root("cancel");
    let map = root.join("map");
    fs::create_dir_all(&map).unwrap();
    fs::write(map.join("provinces.bmp"), b"old-bmp").unwrap();
    fs::write(map.join("definition.csv"), b"old-csv").unwrap();
    let cancellation = ProvinceSaveCancellation::default();
    let result = execute_province_save(
      &Location::Directory(map.clone()),
      &bundle(),
      ProvinceSaveMode::Save,
      &cancellation,
      |progress| {
        if progress.stage == ProvinceSaveStage::Validating {
          cancellation.cancel();
        }
      },
    );
    assert!(result.unwrap_err().contains("cancelled before commit"));
    assert_eq!(fs::read(map.join("provinces.bmp")).unwrap(), b"old-bmp");
    assert_eq!(fs::read(map.join("definition.csv")).unwrap(), b"old-csv");
    assert!(fs::read_dir(&map).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().contains("hoi4me-")));
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn external_change_cancels_without_overwriting_the_external_bytes() {
    let root = root("external-change");
    let map = root.join("map");
    fs::create_dir_all(&map).unwrap();
    fs::write(map.join("provinces.bmp"), b"old-bmp").unwrap();
    fs::write(map.join("definition.csv"), b"old-csv").unwrap();
    let mut changed = false;
    let result = execute_province_save(
      &Location::Directory(map.clone()),
      &bundle(),
      ProvinceSaveMode::Save,
      &ProvinceSaveCancellation::default(),
      |progress| {
        if !changed
          && progress.stage == ProvinceSaveStage::EncodingBmp
          && progress.current > 0
        {
          fs::write(map.join("definition.csv"), b"external-change").unwrap();
          changed = true;
        }
      },
    );
    assert!(result.unwrap_err().contains("changed outside the editor"));
    assert_eq!(fs::read(map.join("provinces.bmp")).unwrap(), b"old-bmp");
    assert_eq!(fs::read(map.join("definition.csv")).unwrap(), b"external-change");
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn archive_export_uses_an_explicit_province_file_allowlist() {
    let root = root("archive");
    fs::create_dir_all(&root).unwrap();
    let archive_path = root.join("province-map.zip");
    execute_province_save(
      &Location::ZipArchive(archive_path.clone()),
      &bundle(),
      ProvinceSaveMode::Export,
      &ProvinceSaveCancellation::default(),
      |_| {},
    )
    .unwrap();
    let archive = ZipArchiveFilesMap::from_fs(&archive_path).unwrap();
    let names = archive
      .iter()
      .map(|(name, _)| name.to_string_lossy().into_owned())
      .collect::<BTreeSet<_>>();
    assert_eq!(
      names,
      BTreeSet::from(["definition.csv".to_owned(), "provinces.bmp".to_owned()])
    );
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn progress_stages_follow_prepare_encode_stage_validate_backup_commit_reload_verify() {
    let root = root("progress");
    let map = root.join("map");
    fs::create_dir_all(&map).unwrap();
    fs::write(map.join("provinces.bmp"), b"old-bmp").unwrap();
    fs::write(map.join("definition.csv"), b"old-csv").unwrap();
    let mut stages = Vec::new();
    execute_province_save(
      &Location::Directory(map),
      &bundle(),
      ProvinceSaveMode::Save,
      &ProvinceSaveCancellation::default(),
      |progress| {
        if stages.last() != Some(&progress.stage) {
          stages.push(progress.stage);
        }
      },
    )
    .unwrap();
    assert_eq!(
      stages,
      vec![
        ProvinceSaveStage::Preparing,
        ProvinceSaveStage::EncodingBmp,
        ProvinceSaveStage::WritingDefinition,
        ProvinceSaveStage::Staging,
        ProvinceSaveStage::Validating,
        ProvinceSaveStage::BackingUp,
        ProvinceSaveStage::Committing,
        ProvinceSaveStage::Reloading,
        ProvinceSaveStage::Verifying,
        ProvinceSaveStage::Completed,
      ]
    );
    fs::remove_dir_all(root).unwrap();
  }

  #[test]
  #[ignore = "requires HOI4_STATE_EDITOR_REAL_MOD_ROOT and writes only controlled TEMP copies"]
  fn real_mod_province_atomic_save_smoke_on_controlled_copy() {
    let original = std::env::var_os("HOI4_STATE_EDITOR_REAL_MOD_ROOT")
      .map(PathBuf::from)
      .expect("set HOI4_STATE_EDITOR_REAL_MOD_ROOT");
    let source_map = original.join("map");
    let original_bytes = ["provinces.bmp", "definition.csv"]
      .into_iter()
      .map(|name| (name, fs::read(source_map.join(name)).unwrap()))
      .collect::<BTreeMap<_, _>>();
    let root = root("real-province-atomic");
    let map = root.join("map");
    let candidate = root.join("candidate");
    fs::create_dir_all(&map).unwrap();
    fs::create_dir_all(&candidate).unwrap();
    for (name, bytes) in &original_bytes {
      fs::write(map.join(name), bytes).unwrap();
    }
    let bundle = Bundle::load(
      &Location::Directory(source_map.clone()),
      Config {
        preserve_ids: true,
        ..Config::default()
      },
    )
    .unwrap();
    execute_province_save(
      &Location::Directory(candidate.clone()),
      &bundle,
      ProvinceSaveMode::Export,
      &ProvinceSaveCancellation::default(),
      |_| {},
    )
    .unwrap();
    let candidate_bytes = ["provinces.bmp", "definition.csv"]
      .into_iter()
      .map(|name| (name, fs::read(candidate.join(name)).unwrap()))
      .collect::<BTreeMap<_, _>>();

    let report = execute_province_save(
      &Location::Directory(map.clone()),
      &bundle,
      ProvinceSaveMode::Save,
      &ProvinceSaveCancellation::default(),
      |progress| {
        for name in ["provinces.bmp", "definition.csv"] {
          let observed = fs::read(map.join(name)).unwrap();
          assert!(observed == original_bytes[name] || observed == candidate_bytes[name]);
          if progress.stage.cancellable() {
            assert_eq!(observed, original_bytes[name]);
          }
        }
      },
    )
    .unwrap();

    println!("{}", report.summary_text());
    for name in ["provinces.bmp", "definition.csv"] {
      assert_eq!(fs::read(source_map.join(name)).unwrap(), original_bytes[name]);
      assert_eq!(fs::read(map.join(name)).unwrap(), candidate_bytes[name]);
    }
    fs::remove_dir_all(root).unwrap();
  }
}
