use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{Hoi4Project, StateEditSession, WorkingStateOrigin};
use crate::app::state::{
  extract_state, parse, NewlineStyle, PdxBlock, PdxDocument, PdxEntry, PdxValue, SourceText,
  StateData, StateDocument, TextSpan, TokenKind, VictoryPoint,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PatchSafety {
  Safe,
  ReviewRequired,
  Blocked,
}

impl PatchSafety {
  pub fn label(self) -> &'static str {
    match self {
      Self::Safe => "Safe",
      Self::ReviewRequired => "Review required",
      Self::Blocked => "Blocked",
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchDiagnosticKind {
  SourceChanged,
  InvalidEncoding,
  MissingBinding,
  DuplicateBinding,
  AmbiguousHistory,
  DatedHistoryConflict,
  UnsafeCommentAssociation,
  OverlappingPatch,
  PathCollision,
  ParseFailure,
  SemanticMismatch,
  UnsupportedStructure,
}

#[derive(Debug, Clone)]
pub struct PatchDiagnostic {
  pub kind: PatchDiagnosticKind,
  pub safety: PatchSafety,
  pub path: PathBuf,
  pub state_id: Option<u32>,
  pub field: Option<String>,
  pub span: Option<TextSpan>,
  pub message: String,
  pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFingerprint {
  pub byte_len: u64,
  pub content_hash: u64,
}

impl SourceFingerprint {
  pub fn from_bytes(bytes: &[u8]) -> Self {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
      hash ^= u64::from(*byte);
      hash = hash.wrapping_mul(0x100000001b3);
    }
    Self {
      byte_len: bytes.len() as u64,
      content_hash: hash,
    }
  }

  pub fn digest_hex(&self) -> String {
    format!("{:016X}-{:016X}", self.byte_len, self.content_hash)
  }
}

#[derive(Debug, Clone)]
pub enum TextPatchOperation {
  Replace {
    range: Range<usize>,
    expected: Vec<u8>,
    replacement: Vec<u8>,
    state_id: u32,
    field: String,
    description: String,
    safety: PatchSafety,
  },
  Insert {
    offset: usize,
    content: Vec<u8>,
    state_id: u32,
    field: String,
    description: String,
    safety: PatchSafety,
  },
  Delete {
    range: Range<usize>,
    expected: Vec<u8>,
    state_id: u32,
    field: String,
    description: String,
    safety: PatchSafety,
  },
}

impl TextPatchOperation {
  fn start(&self) -> usize {
    match self {
      Self::Replace { range, .. } | Self::Delete { range, .. } => range.start,
      Self::Insert { offset, .. } => *offset,
    }
  }

  fn end(&self) -> usize {
    match self {
      Self::Replace { range, .. } | Self::Delete { range, .. } => range.end,
      Self::Insert { offset, .. } => *offset,
    }
  }

  pub fn safety(&self) -> PatchSafety {
    match self {
      Self::Replace { safety, .. }
      | Self::Insert { safety, .. }
      | Self::Delete { safety, .. } => *safety,
    }
  }

  pub fn description(&self) -> &str {
    match self {
      Self::Replace { description, .. }
      | Self::Insert { description, .. }
      | Self::Delete { description, .. } => description,
    }
  }
}

#[derive(Debug, Clone)]
pub struct PlannedFileModification {
  pub path: PathBuf,
  pub state_id: u32,
  pub operations: Vec<TextPatchOperation>,
  pub before: Vec<u8>,
  pub after: Option<Vec<u8>>,
  pub unified_diff: String,
  pub semantic_changes: Vec<String>,
  pub diagnostics: Vec<PatchDiagnostic>,
  pub safety: PatchSafety,
}

#[derive(Debug, Clone)]
pub struct PlannedFileCreation {
  pub path: PathBuf,
  pub state_id: u32,
  pub content: Vec<u8>,
  pub unified_diff: String,
  pub semantic_changes: Vec<String>,
  pub diagnostics: Vec<PatchDiagnostic>,
  pub safety: PatchSafety,
}

#[derive(Debug, Clone)]
pub struct PlannedFileRemoval {
  pub path: PathBuf,
  pub state_id: u32,
  pub before: Vec<u8>,
  pub unified_diff: String,
  pub semantic_changes: Vec<String>,
  pub diagnostics: Vec<PatchDiagnostic>,
  pub safety: PatchSafety,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PatchPlanSummary {
  pub modified_files: usize,
  pub created_files: usize,
  pub removed_files: usize,
  pub safe_files: usize,
  pub review_required_files: usize,
  pub blocked_files: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PatchPlanTimings {
  pub semantic_diff_ms: u128,
  pub provenance_resolution_ms: u128,
  pub planning_ms: u128,
  pub in_memory_application_ms: u128,
  pub preview_validation_ms: u128,
  pub diff_generation_ms: u128,
  pub total_ms: u128,
}

#[derive(Debug, Clone)]
pub struct ProjectPatchPlan {
  pub generation: u64,
  pub source_fingerprints: BTreeMap<PathBuf, SourceFingerprint>,
  pub modified_files: Vec<PlannedFileModification>,
  pub created_files: Vec<PlannedFileCreation>,
  pub removed_files: Vec<PlannedFileRemoval>,
  pub diagnostics: Vec<PatchDiagnostic>,
  pub summary: PatchPlanSummary,
  pub timings: PatchPlanTimings,
}

impl ProjectPatchPlan {
  pub fn is_stale(&self, revision: u64) -> bool {
    self.generation != revision
  }

  pub fn content_fingerprint(&self) -> SourceFingerprint {
    fn append(bytes: &mut Vec<u8>, value: &[u8]) {
      bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
      bytes.extend_from_slice(value);
    }
    fn append_path(bytes: &mut Vec<u8>, path: &Path) {
      append(bytes, path.to_string_lossy().replace('\\', "/").as_bytes());
    }
    fn safety(value: PatchSafety) -> u8 {
      match value {
        PatchSafety::Safe => 0,
        PatchSafety::ReviewRequired => 1,
        PatchSafety::Blocked => 2,
      }
    }

    let mut bytes = b"HOI4_STATE_EDITOR_PATCH_PLAN_V2".to_vec();
    bytes.extend_from_slice(&self.generation.to_le_bytes());
    for (path, fingerprint) in &self.source_fingerprints {
      bytes.push(b'S');
      append_path(&mut bytes, path);
      bytes.extend_from_slice(&fingerprint.byte_len.to_le_bytes());
      bytes.extend_from_slice(&fingerprint.content_hash.to_le_bytes());
    }
    let mut modified = self.modified_files.iter().collect::<Vec<_>>();
    modified.sort_by_key(|file| file.path.to_string_lossy().replace('\\', "/"));
    for file in modified {
      bytes.push(b'M');
      append_path(&mut bytes, &file.path);
      bytes.extend_from_slice(&file.state_id.to_le_bytes());
      bytes.push(safety(file.safety));
      append(&mut bytes, &file.before);
      append(&mut bytes, file.after.as_deref().unwrap_or_default());
      for operation in &file.operations {
        match operation {
          TextPatchOperation::Replace {
            range,
            expected,
            replacement,
            state_id,
            field,
            safety: operation_safety,
            ..
          } => {
            bytes.push(b'R');
            bytes.extend_from_slice(&(range.start as u64).to_le_bytes());
            bytes.extend_from_slice(&(range.end as u64).to_le_bytes());
            append(&mut bytes, expected);
            append(&mut bytes, replacement);
            bytes.extend_from_slice(&state_id.to_le_bytes());
            append(&mut bytes, field.as_bytes());
            bytes.push(safety(*operation_safety));
          },
          TextPatchOperation::Insert {
            offset,
            content,
            state_id,
            field,
            safety: operation_safety,
            ..
          } => {
            bytes.push(b'I');
            bytes.extend_from_slice(&(*offset as u64).to_le_bytes());
            append(&mut bytes, content);
            bytes.extend_from_slice(&state_id.to_le_bytes());
            append(&mut bytes, field.as_bytes());
            bytes.push(safety(*operation_safety));
          },
          TextPatchOperation::Delete {
            range,
            expected,
            state_id,
            field,
            safety: operation_safety,
            ..
          } => {
            bytes.push(b'D');
            bytes.extend_from_slice(&(range.start as u64).to_le_bytes());
            bytes.extend_from_slice(&(range.end as u64).to_le_bytes());
            append(&mut bytes, expected);
            bytes.extend_from_slice(&state_id.to_le_bytes());
            append(&mut bytes, field.as_bytes());
            bytes.push(safety(*operation_safety));
          },
        }
      }
    }
    let mut created = self.created_files.iter().collect::<Vec<_>>();
    created.sort_by_key(|file| file.path.to_string_lossy().replace('\\', "/"));
    for file in created {
      bytes.push(b'C');
      append_path(&mut bytes, &file.path);
      bytes.extend_from_slice(&file.state_id.to_le_bytes());
      bytes.push(safety(file.safety));
      append(&mut bytes, &file.content);
    }
    let mut removed = self.removed_files.iter().collect::<Vec<_>>();
    removed.sort_by_key(|file| file.path.to_string_lossy().replace('\\', "/"));
    for file in removed {
      bytes.push(b'X');
      append_path(&mut bytes, &file.path);
      bytes.extend_from_slice(&file.state_id.to_le_bytes());
      bytes.push(safety(file.safety));
      append(&mut bytes, &file.before);
    }
    SourceFingerprint::from_bytes(&bytes)
  }

  pub fn files_len(&self) -> usize {
    self.modified_files.len() + self.created_files.len() + self.removed_files.len()
  }

  pub fn summary_text(&self) -> String {
    let mut text = format!(
      "PATCH PREVIEW\nPreview only - no files will be written.\n\
       Modified files: {} | Created files: {} | Removed files: {}\n\
       Safe: {} | Review required: {} | Blocked: {}\n\
       Timings: semantic diff {} ms | provenance {} ms | patch plan {} ms | \
       apply {} ms | validate {} ms | diff {} ms | total {} ms",
      self.summary.modified_files,
      self.summary.created_files,
      self.summary.removed_files,
      self.summary.safe_files,
      self.summary.review_required_files,
      self.summary.blocked_files,
      self.timings.semantic_diff_ms,
      self.timings.provenance_resolution_ms,
      self.timings.planning_ms,
      self.timings.in_memory_application_ms,
      self.timings.preview_validation_ms,
      self.timings.diff_generation_ms,
      self.timings.total_ms,
    );
    if self.summary.blocked_files != 0 {
      text.push_str("\nThis project cannot be saved safely while blocked patches exist.");
    }
    text
  }

  pub fn file_report(&self, index: usize) -> Option<String> {
    if let Some(file) = self.modified_files.get(index) {
      return Some(format_file_report(
        "M",
        &file.path,
        file.state_id,
        file.safety,
        &file.semantic_changes,
        file.operations.iter().map(TextPatchOperation::description),
        &file.diagnostics,
        &file.unified_diff,
      ));
    }
    let index = index.checked_sub(self.modified_files.len())?;
    if let Some(file) = self.created_files.get(index) {
      return Some(format_file_report(
        "C",
        &file.path,
        file.state_id,
        file.safety,
        &file.semantic_changes,
        std::iter::once("Create canonical state document"),
        &file.diagnostics,
        &file.unified_diff,
      ));
    }
    let index = index.checked_sub(self.created_files.len())?;
    self.removed_files.get(index).map(|file| {
      format_file_report(
        "D",
        &file.path,
        file.state_id,
        file.safety,
        &file.semantic_changes,
        std::iter::once("Remove loaded state file"),
        &file.diagnostics,
        &file.unified_diff,
      )
    })
  }
}

pub fn plan_state_patches(project: &Hoi4Project, edit: &StateEditSession) -> ProjectPatchPlan {
  let total_started = Instant::now();
  let semantic_started = Instant::now();
  let removed_state_ids = edit.removed_state_ids().clone();
  let dirty_state_ids = edit.dirty_state_ids().clone();
  let valid_state_ids = edit.valid_state_ids().clone();
  let semantic_diff_ms = semantic_started.elapsed().as_millis();
  let planning_started = Instant::now();
  let mut phase_timings = PhaseTimings::default();
  let mut source_fingerprints = BTreeMap::new();
  let mut modified_files = Vec::new();
  let mut created_files = Vec::new();
  let mut removed_files = Vec::new();
  let mut diagnostics = Vec::new();

  for &state_id in &removed_state_ids {
    let Some(origin) = edit.state_origin(state_id) else { continue };
    if matches!(origin, WorkingStateOrigin::CreatedInSession) {
      continue;
    }
    let Some(document) = project.state_document(state_id) else { continue };
    let fingerprint = SourceFingerprint::from_bytes(document.original_bytes());
    source_fingerprints.insert(document.path.clone(), fingerprint);
    let mut file_diagnostics = source_safety_diagnostics(document, state_id);
    let safety = maximum_safety(file_diagnostics.iter().map(|diagnostic| diagnostic.safety));
    let path = relative_path(project, &document.path);
    let diff_started = Instant::now();
    let unified_diff = removed_diff(&path, document.source());
    phase_timings.diff_generation_ms += diff_started.elapsed().as_millis();
    removed_files.push(PlannedFileRemoval {
      path,
      state_id,
      before: document.original_bytes().to_vec(),
      unified_diff,
      semantic_changes: vec![format!("State {state_id}: remove loaded state file")],
      diagnostics: file_diagnostics.clone(),
      safety,
    });
    diagnostics.append(&mut file_diagnostics);
  }

  for &state_id in &dirty_state_ids {
    if removed_state_ids.contains(&state_id) {
      continue;
    }
    match edit.state_origin(state_id) {
      Some(WorkingStateOrigin::CreatedInSession) => {
        if let Some(working) = edit.state_data(state_id) {
          let creation = plan_creation(project, state_id, &working, &mut phase_timings);
          diagnostics.extend(creation.diagnostics.iter().cloned());
          created_files.push(creation);
        }
      },
      Some(WorkingStateOrigin::Loaded { .. }) => {
        let Some(document) = project.state_document(state_id) else { continue };
        let Some(baseline) = document.data.as_ref() else { continue };
        let Some(working) = edit.state_data(state_id) else { continue };
        source_fingerprints.insert(
          document.path.clone(),
          SourceFingerprint::from_bytes(document.original_bytes()),
        );
        let modification =
          plan_modification(
            project,
            edit,
            document,
            state_id,
            baseline,
            &working,
            &mut phase_timings,
          );
        diagnostics.extend(modification.diagnostics.iter().cloned());
        modified_files.push(modification);
      },
      None => {},
    }
  }

  for &state_id in &valid_state_ids {
    if edit.state_origin(state_id) == Some(&WorkingStateOrigin::CreatedInSession)
      && !dirty_state_ids.contains(&state_id)
      && let Some(working) = edit.state_data(state_id)
    {
      let creation = plan_creation(project, state_id, &working, &mut phase_timings);
      diagnostics.extend(creation.diagnostics.iter().cloned());
      created_files.push(creation);
    }
  }

  modified_files.sort_by(|a, b| a.path.cmp(&b.path));
  created_files.sort_by(|a, b| a.path.cmp(&b.path));
  removed_files.sort_by(|a, b| a.path.cmp(&b.path));
  let planning_ms = planning_started.elapsed().as_millis();
  let provenance_resolution_ms = planning_ms.saturating_sub(
    phase_timings.in_memory_application_ms
      + phase_timings.preview_validation_ms
      + phase_timings.diff_generation_ms
  );
  let summary = summarize(&modified_files, &created_files, &removed_files);
  let timings = PatchPlanTimings {
    semantic_diff_ms,
    provenance_resolution_ms,
    planning_ms,
    in_memory_application_ms: phase_timings.in_memory_application_ms,
    preview_validation_ms: phase_timings.preview_validation_ms,
    diff_generation_ms: phase_timings.diff_generation_ms,
    total_ms: total_started.elapsed().as_millis(),
  };
  ProjectPatchPlan {
    generation: edit.revision(),
    source_fingerprints,
    modified_files,
    created_files,
    removed_files,
    diagnostics,
    summary,
    timings,
  }
}

#[derive(Default)]
struct PhaseTimings {
  in_memory_application_ms: u128,
  preview_validation_ms: u128,
  diff_generation_ms: u128,
}

struct FileBuilder<'a> {
  document: &'a StateDocument,
  state_id: u32,
  operations: Vec<TextPatchOperation>,
  changes: Vec<String>,
  diagnostics: Vec<PatchDiagnostic>,
}

impl<'a> FileBuilder<'a> {
  fn new(document: &'a StateDocument, state_id: u32) -> Self {
    Self {
      document,
      state_id,
      operations: Vec::new(),
      changes: Vec::new(),
      diagnostics: source_safety_diagnostics(document, state_id),
    }
  }

  fn diagnostic(
    &mut self,
    kind: PatchDiagnosticKind,
    safety: PatchSafety,
    field: &str,
    span: Option<TextSpan>,
    message: impl Into<String>,
    action: impl Into<String>,
  ) {
    self.diagnostics.push(PatchDiagnostic {
      kind,
      safety,
      path: self.document.path.clone(),
      state_id: Some(self.state_id),
      field: Some(field.to_owned()),
      span,
      message: message.into(),
      action: action.into(),
    });
  }

  fn replace(
    &mut self,
    field: &str,
    span: TextSpan,
    replacement: impl Into<Vec<u8>>,
    description: impl Into<String>,
  ) {
    let Some(expected) = bytes_at(self.document.original_bytes(), span) else {
      self.diagnostic(
        PatchDiagnosticKind::MissingBinding,
        PatchSafety::Blocked,
        field,
        Some(span),
        "The value span is outside the original source bytes.",
        "Reload the project before regenerating the preview.",
      );
      return;
    };
    self.operations.push(TextPatchOperation::Replace {
      range: span.start..span.end(),
      expected: expected.to_vec(),
      replacement: replacement.into(),
      state_id: self.state_id,
      field: field.to_owned(),
      description: description.into(),
      safety: PatchSafety::Safe,
    });
  }

  fn insert(
    &mut self,
    field: &str,
    offset: usize,
    content: impl Into<Vec<u8>>,
    description: impl Into<String>,
    safety: PatchSafety,
  ) {
    self.operations.push(TextPatchOperation::Insert {
      offset,
      content: content.into(),
      state_id: self.state_id,
      field: field.to_owned(),
      description: description.into(),
      safety,
    });
  }

  fn delete(
    &mut self,
    field: &str,
    span: TextSpan,
    description: impl Into<String>,
    safety: PatchSafety,
  ) {
    let Some(expected) = bytes_at(self.document.original_bytes(), span) else {
      self.diagnostic(
        PatchDiagnosticKind::MissingBinding,
        PatchSafety::Blocked,
        field,
        Some(span),
        "The entry span is outside the original source bytes.",
        "Reload the project before regenerating the preview.",
      );
      return;
    };
    self.operations.push(TextPatchOperation::Delete {
      range: span.start..span.end(),
      expected: expected.to_vec(),
      state_id: self.state_id,
      field: field.to_owned(),
      description: description.into(),
      safety,
    });
  }
}

fn plan_modification(
  project: &Hoi4Project,
  edit: &StateEditSession,
  document: &StateDocument,
  state_id: u32,
  baseline: &StateData,
  working: &StateData,
  timings: &mut PhaseTimings,
) -> PlannedFileModification {
  let mut builder = FileBuilder::new(document, state_id);
  let Some(root) = root_state_block(&document.syntax) else {
    builder.diagnostic(
      PatchDiagnosticKind::MissingBinding,
      PatchSafety::Blocked,
      "state",
      None,
      "The document has no unique root state block.",
      "Fix or reload the source document before planning edits.",
    );
    return finish_modification(project, builder, working, timings);
  };

  plan_optional_scalar(
    &mut builder,
    root,
    "name",
    baseline.name.as_deref(),
    working.name.as_deref(),
    quote_string,
  );
  plan_optional_scalar(
    &mut builder,
    root,
    "manpower",
    baseline.manpower,
    working.manpower,
    |value| value.to_string(),
  );
  plan_optional_scalar(
    &mut builder,
    root,
    "state_category",
    baseline.state_category.as_deref(),
    working.state_category.as_deref(),
    str::to_owned,
  );
  plan_optional_scalar(
    &mut builder,
    root,
    "buildings_max_level_factor",
    baseline.buildings_max_level_factor,
    working.buildings_max_level_factor,
    format_number,
  );
  plan_optional_scalar(
    &mut builder,
    root,
    "local_supplies",
    baseline.local_supplies,
    working.local_supplies,
    format_number,
  );
  plan_bool(&mut builder, root, baseline, working);
  plan_named_block(
    &mut builder,
    root,
    "resources",
    &baseline.resources,
    &working.resources,
  );
  plan_provinces(&mut builder, root, baseline, working);
  plan_history(&mut builder, project, edit, root, baseline, working);
  finish_modification(project, builder, working, timings)
}

fn finish_modification(
  project: &Hoi4Project,
  mut builder: FileBuilder<'_>,
  working: &StateData,
  timings: &mut PhaseTimings,
) -> PlannedFileModification {
  let path = relative_path(project, &builder.document.path);
  let before = builder.document.original_bytes().to_vec();
  if let Err(message) = validate_non_overlapping_patches(&builder.operations) {
    builder.diagnostic(
      PatchDiagnosticKind::OverlappingPatch,
      PatchSafety::Blocked,
      "document",
      None,
      message,
      "Reload and regenerate the patch plan.",
    );
  }
  let mut safety = maximum_safety(
    builder.diagnostics.iter().map(|diagnostic| diagnostic.safety)
      .chain(builder.operations.iter().map(TextPatchOperation::safety)),
  );
  let mut after = None;
  let mut unified_diff = String::new();
  if safety != PatchSafety::Blocked {
    let apply_started = Instant::now();
    let apply_result = apply_operations(&before, &builder.operations);
    timings.in_memory_application_ms += apply_started.elapsed().as_millis();
    match apply_result {
      Ok(bytes) => {
        let validation_started = Instant::now();
        if let Err(message) = validate_preview(&builder.document.path, &bytes, working) {
          builder.diagnostic(
            if message.starts_with("semantic") {
              PatchDiagnosticKind::SemanticMismatch
            } else {
              PatchDiagnosticKind::ParseFailure
            },
            PatchSafety::Blocked,
            "document",
            None,
            message,
            "Review the source structure; no applicable patch was produced.",
          );
          safety = PatchSafety::Blocked;
        }
        timings.preview_validation_ms += validation_started.elapsed().as_millis();
        let diff_started = Instant::now();
        unified_diff = modified_diff(&path, &before, &bytes);
        timings.diff_generation_ms += diff_started.elapsed().as_millis();
        after = Some(bytes);
      },
      Err(message) => {
        builder.diagnostic(
          PatchDiagnosticKind::SourceChanged,
          PatchSafety::Blocked,
          "document",
          None,
          message,
          "Reload the project before regenerating the preview.",
        );
        safety = PatchSafety::Blocked;
      },
    }
  }
  if safety == PatchSafety::Blocked {
    builder.operations.clear();
  }
  PlannedFileModification {
    path,
    state_id: builder.state_id,
    operations: builder.operations,
    before,
    after,
    unified_diff,
    semantic_changes: builder.changes,
    diagnostics: builder.diagnostics,
    safety,
  }
}

fn plan_optional_scalar<T: PartialEq + Copy>(
  builder: &mut FileBuilder<'_>,
  root: &PdxBlock,
  field: &str,
  before: Option<T>,
  after: Option<T>,
  render: impl Fn(T) -> String,
) {
  if before == after {
    return;
  }
  builder.changes.push(format!(
    "{field}: {} -> {}",
    before.map(&render).unwrap_or_else(|| "<absent>".to_owned()),
    after.map(&render).unwrap_or_else(|| "<absent>".to_owned()),
  ));
  let bindings = entries(root, field);
  if bindings.len() > 1 {
    builder.diagnostic(
      PatchDiagnosticKind::DuplicateBinding,
      PatchSafety::Blocked,
      field,
      bindings.get(1).map(|entry| entry.span),
      format!("State {} has multiple {field} assignments.", builder.state_id),
      "Resolve the duplicate assignments before patching this field.",
    );
    return;
  }
  match (bindings.first().copied(), after) {
    (Some(entry), Some(value)) => match scalar_span(entry) {
      Some(span) => builder.replace(
        field,
        span,
        render(value),
        format!("Replace State {} {field}", builder.state_id),
      ),
      None => builder.diagnostic(
        PatchDiagnosticKind::MissingBinding,
        PatchSafety::Blocked,
        field,
        Some(entry.span),
        format!("State {} {field} is not a scalar value.", builder.state_id),
        "Use a supported scalar assignment.",
      ),
    },
    (Some(entry), None) => builder.delete(
      field,
      entry.span,
      format!("Remove State {} {field}", builder.state_id),
      comment_safety(builder.document, entry.span),
    ),
    (None, Some(value)) => {
      let (offset, content) = assignment_insertion(builder.document, root, field, &render(value));
      builder.insert(
        field,
        offset,
        content,
        format!("Insert State {} {field}", builder.state_id),
        PatchSafety::Safe,
      );
    },
    (None, None) => {},
  }
}

fn plan_bool(
  builder: &mut FileBuilder<'_>,
  root: &PdxBlock,
  baseline: &StateData,
  working: &StateData,
) {
  let before = baseline.impassable.unwrap_or(false);
  let after = working.impassable.unwrap_or(false);
  if before == after {
    return;
  }
  builder.changes.push(format!("impassable: {before} -> {after}"));
  let bindings = entries(root, "impassable");
  if bindings.len() > 1 {
    builder.diagnostic(
      PatchDiagnosticKind::DuplicateBinding,
      PatchSafety::Blocked,
      "impassable",
      bindings.get(1).map(|entry| entry.span),
      "Multiple impassable assignments make the authoritative value ambiguous.",
      "Resolve duplicate impassable entries before patching.",
    );
  } else if let Some(entry) = bindings.first() {
    if after {
      if let Some(span) = scalar_span(entry) {
        builder.replace("impassable", span, "yes", "Set impassable = yes");
      }
    } else {
      builder.delete(
        "impassable",
        entry.span,
        "Remove impassable assignment",
        comment_safety(builder.document, entry.span),
      );
    }
  } else if after {
    let (offset, content) = assignment_insertion(builder.document, root, "impassable", "yes");
    builder.insert(
      "impassable",
      offset,
      content,
      "Insert impassable = yes",
      PatchSafety::Safe,
    );
  }
}

fn plan_named_block(
  builder: &mut FileBuilder<'_>,
  parent: &PdxBlock,
  field: &str,
  before: &BTreeMap<String, i64>,
  after: &BTreeMap<String, i64>,
) {
  if before == after {
    return;
  }
  let containers = entries(parent, field);
  if containers.len() > 1 {
    builder.diagnostic(
      PatchDiagnosticKind::DuplicateBinding,
      PatchSafety::Blocked,
      field,
      containers.get(1).map(|entry| entry.span),
      format!("Multiple {field} blocks are ambiguous."),
      "Merge or disambiguate the blocks before patching.",
    );
    return;
  }
  let Some(container) = containers.first().copied() else {
    if after.is_empty() {
      return;
    }
    let body = after.iter()
      .map(|(name, value)| format!("{name}={value}"))
      .collect::<Vec<_>>();
    let (offset, content) = block_insertion(builder.document, parent, field, &body);
    builder.insert(
      field,
      offset,
      content,
      format!("Insert {field} block"),
      PatchSafety::Safe,
    );
    builder.changes.push(format!("{field}: create block with {} entries", after.len()));
    return;
  };
  let Some(block) = as_block(container) else {
    builder.diagnostic(
      PatchDiagnosticKind::UnsupportedStructure,
      PatchSafety::Blocked,
      field,
      Some(container.span),
      format!("{field} is not a block."),
      "Convert the field to a normal PDXScript block before patching.",
    );
    return;
  };
  plan_named_block_contents(builder, block, field, before, after);
}

fn plan_named_block_contents(
  builder: &mut FileBuilder<'_>,
  block: &PdxBlock,
  field: &str,
  before: &BTreeMap<String, i64>,
  after: &BTreeMap<String, i64>,
) {
  let names = before.keys().chain(after.keys()).cloned().collect::<BTreeSet<_>>();
  for name in names {
    let old = before.get(&name).copied();
    let new = after.get(&name).copied();
    if old == new {
      continue;
    }
    builder.changes.push(format!(
      "{field}.{name}: {} -> {}",
      old.map_or_else(|| "<absent>".to_owned(), |value| value.to_string()),
      new.map_or_else(|| "<absent>".to_owned(), |value| value.to_string()),
    ));
    let bindings = entries(block, &name);
    if bindings.len() > 1 {
      builder.diagnostic(
        PatchDiagnosticKind::DuplicateBinding,
        PatchSafety::Blocked,
        &format!("{field}.{name}"),
        bindings.get(1).map(|entry| entry.span),
        format!("{field} contains duplicate {name} assignments."),
        "Resolve the duplicate entries before patching.",
      );
      continue;
    }
    match (bindings.first().copied(), new) {
      (Some(entry), Some(value)) => match scalar_span(entry) {
        Some(span) => builder.replace(
          &format!("{field}.{name}"),
          span,
          value.to_string(),
          format!("Replace {field} {name}"),
        ),
        None => builder.diagnostic(
          PatchDiagnosticKind::UnsupportedStructure,
          PatchSafety::Blocked,
          &format!("{field}.{name}"),
          Some(entry.span),
          format!("{field} {name} is not scalar."),
          "Use a scalar integer entry.",
        ),
      },
      (Some(entry), None) => builder.delete(
        &format!("{field}.{name}"),
        entry.span,
        format!("Remove {field} {name}"),
        comment_safety(builder.document, entry.span),
      ),
      (None, Some(value)) => {
        let (offset, content) =
          assignment_insertion(builder.document, block, &name, &value.to_string());
        builder.insert(
          &format!("{field}.{name}"),
          offset,
          content,
          format!("Insert {field} {name}"),
          PatchSafety::Safe,
        );
      },
      (None, None) => {},
    }
  }
}

fn plan_provinces(
  builder: &mut FileBuilder<'_>,
  root: &PdxBlock,
  baseline: &StateData,
  working: &StateData,
) {
  if baseline.provinces == working.provinces {
    return;
  }
  let containers = entries(root, "provinces");
  if containers.len() != 1 {
    builder.diagnostic(
      if containers.is_empty() {
        PatchDiagnosticKind::MissingBinding
      } else {
        PatchDiagnosticKind::DuplicateBinding
      },
      PatchSafety::Blocked,
      "provinces",
      containers.first().map(|entry| entry.span),
      "An existing state needs exactly one provinces block for safe patching.",
      "Fix the source structure before moving provinces.",
    );
    return;
  }
  let Some(block) = as_block(containers[0]) else {
    builder.diagnostic(
      PatchDiagnosticKind::UnsupportedStructure,
      PatchSafety::Blocked,
      "provinces",
      Some(containers[0].span),
      "provinces is not a positional block.",
      "Use a normal provinces = { ... } block.",
    );
    return;
  };
  let mut bindings = BTreeMap::<u32, Vec<TextSpan>>::new();
  for entry in &block.entries {
    if entry.key.is_none()
      && let Some(text) = scalar_text(entry)
      && let Ok(id) = text.parse::<u32>()
    {
      bindings.entry(id).or_default().push(entry.span);
    }
  }
  if let Some((id, spans)) = bindings.iter().find(|(_, spans)| spans.len() > 1) {
    builder.diagnostic(
      PatchDiagnosticKind::DuplicateBinding,
      PatchSafety::Blocked,
      "provinces",
      spans.get(1).copied(),
      format!("Province {id} appears more than once in the source block."),
      "Resolve duplicate province IDs before patching.",
    );
    return;
  }
  for id in baseline.provinces.difference(&working.provinces) {
    let Some(span) = bindings.get(id).and_then(|spans| spans.first()).copied() else {
      builder.diagnostic(
        PatchDiagnosticKind::MissingBinding,
        PatchSafety::Blocked,
        "provinces",
        None,
        format!("Province {id} has no unique source token."),
        "Reload or fix the source document.",
      );
      continue;
    };
    let safety = comment_safety(builder.document, span);
    if safety == PatchSafety::ReviewRequired {
      builder.diagnostic(
        PatchDiagnosticKind::UnsafeCommentAssociation,
        safety,
        "provinces",
        Some(span),
        format!("Province {id} has a nearby inline comment; the comment will be preserved."),
        "Review the preview to confirm the comment remains meaningful.",
      );
    }
    builder.delete(
      "provinces",
      span,
      format!("Remove Province {id}"),
      safety,
    );
    builder.changes.push(format!("remove Province {id}"));
  }
  let added = working.provinces.difference(&baseline.provinces).copied().collect::<Vec<_>>();
  if !added.is_empty() {
    let values = added.iter().map(u32::to_string).collect::<Vec<_>>().join(" ");
    let (offset, content) = positional_insertion(builder.document, block, &values);
    builder.insert(
      "provinces",
      offset,
      content,
      format!("Add {} province(s)", added.len()),
      PatchSafety::Safe,
    );
    builder.changes.extend(added.into_iter().map(|id| format!("add Province {id}")));
  }
}

fn plan_history(
  builder: &mut FileBuilder<'_>,
  project: &Hoi4Project,
  edit: &StateEditSession,
  root: &PdxBlock,
  baseline: &StateData,
  working: &StateData,
) {
  let history_changed = baseline.history.owner != working.history.owner
    || baseline.history.controller != working.history.controller
    || baseline.history.cores != working.history.cores
    || baseline.history.claims != working.history.claims
    || !victory_points_equal(&baseline.history.victory_points, &working.history.victory_points)
    || baseline.history.state_buildings != working.history.state_buildings
    || baseline.history.province_buildings != working.history.province_buildings;
  if !history_changed {
    return;
  }
  let containers = entries(root, "history");
  if containers.len() > 1 {
    builder.diagnostic(
      PatchDiagnosticKind::DuplicateBinding,
      PatchSafety::Blocked,
      "history",
      containers.get(1).map(|entry| entry.span),
      "Multiple history blocks are ambiguous.",
      "Resolve the duplicate blocks before patching.",
    );
    return;
  }
  let Some(container) = containers.first().copied() else {
    let body = render_history_lines(working);
    let (offset, content) = block_insertion(builder.document, root, "history", &body);
    builder.insert(
      "history",
      offset,
      content,
      "Insert history block",
      PatchSafety::ReviewRequired,
    );
    builder.changes.push("history: create missing block".to_owned());
    return;
  };
  let Some(history) = as_block(container) else {
    builder.diagnostic(
      PatchDiagnosticKind::UnsupportedStructure,
      PatchSafety::Blocked,
      "history",
      Some(container.span),
      "history is not a block.",
      "Use a normal history block before patching.",
    );
    return;
  };
  let political_changed = baseline.history.owner != working.history.owner
    || baseline.history.controller != working.history.controller
    || baseline.history.cores != working.history.cores
    || baseline.history.claims != working.history.claims;
  if political_changed && !baseline.history.dated_blocks.is_empty() {
    builder.diagnostic(
      PatchDiagnosticKind::DatedHistoryConflict,
      PatchSafety::Blocked,
      "history",
      baseline.history.dated_blocks.first().map(|block| block.span),
      "Political edits cannot be proven safe while dated history blocks are present.",
      "Edit dated history explicitly in a later phase.",
    );
  } else {
    plan_optional_scalar(
      builder,
      history,
      "owner",
      baseline.history.owner.as_deref(),
      working.history.owner.as_deref(),
      str::to_owned,
    );
    plan_optional_scalar(
      builder,
      history,
      "controller",
      baseline.history.controller.as_deref(),
      working.history.controller.as_deref(),
      str::to_owned,
    );
    plan_tag_set(
      builder,
      history,
      "add_core_of",
      &baseline.history.cores,
      &working.history.cores,
    );
    plan_tag_set(
      builder,
      history,
      "add_claim_by",
      &baseline.history.claims,
      &working.history.claims,
    );
  }
  plan_victory_points(builder, project, edit, history, baseline, working);
  plan_buildings(builder, project, edit, history, baseline, working);
}

fn plan_tag_set(
  builder: &mut FileBuilder<'_>,
  history: &PdxBlock,
  field: &str,
  before: &BTreeSet<String>,
  after: &BTreeSet<String>,
) {
  if before == after {
    return;
  }
  let inverse_field = match field {
    "add_core_of" => "remove_core_of",
    "add_claim_by" => "remove_claim_by",
    _ => "",
  };
  if !inverse_field.is_empty() && !entries(history, inverse_field).is_empty() {
    builder.diagnostic(
      PatchDiagnosticKind::AmbiguousHistory,
      PatchSafety::Blocked,
      field,
      entries(history, inverse_field).first().map(|entry| entry.span),
      format!("{field} cannot be changed safely while {inverse_field} entries are present."),
      "Simplify the non-dated add/remove sequence before patching it.",
    );
    return;
  }
  let mut bindings = BTreeMap::<String, Vec<&PdxEntry>>::new();
  for entry in entries(history, field) {
    if let Some(value) = scalar_text(entry) {
      bindings.entry(unquote(value).to_owned()).or_default().push(entry);
    }
  }
  for tag in before.difference(after) {
    match bindings.get(tag).map(Vec::as_slice) {
      Some([entry]) => {
        builder.delete(
          field,
          entry.span,
          format!("Remove {field} {tag}"),
          comment_safety(builder.document, entry.span),
        );
        builder.changes.push(format!("{field}: remove {tag}"));
      },
      _ => builder.diagnostic(
        PatchDiagnosticKind::DuplicateBinding,
        PatchSafety::Blocked,
        field,
        None,
        format!("{field} {tag} has no unique non-dated source entry."),
        "Resolve the history sequence before patching.",
      ),
    }
  }
  for tag in after.difference(before) {
    let (offset, content) = assignment_insertion(builder.document, history, field, tag);
    builder.insert(
      field,
      offset,
      content,
      format!("Insert {field} {tag}"),
      PatchSafety::Safe,
    );
    builder.changes.push(format!("{field}: add {tag}"));
  }
}

fn plan_victory_points(
  builder: &mut FileBuilder<'_>,
  project: &Hoi4Project,
  edit: &StateEditSession,
  history: &PdxBlock,
  baseline: &StateData,
  working: &StateData,
) {
  if victory_points_equal(&baseline.history.victory_points, &working.history.victory_points) {
    return;
  }
  let containers = entries(history, "victory_points");
  if containers.len() > 1 {
    builder.diagnostic(
      PatchDiagnosticKind::DuplicateBinding,
      PatchSafety::Blocked,
      "victory_points",
      containers.get(1).map(|entry| entry.span),
      "Multiple victory_points blocks are ambiguous.",
      "Merge the blocks before patching.",
    );
    return;
  }
  let working_map = victory_point_map(&working.history.victory_points);
  let baseline_map = victory_point_map(&baseline.history.victory_points);
  let Some(container) = containers.first().copied() else {
    if working_map.is_empty() {
      return;
    }
    let body = working_map.iter()
      .map(|(province, value)| format!("{province} {value}"))
      .collect::<Vec<_>>();
    let (offset, content) = block_insertion(builder.document, history, "victory_points", &body);
    builder.insert(
      "victory_points",
      offset,
      content,
      "Insert victory_points block",
      PatchSafety::Safe,
    );
    builder.changes.extend(
      working_map.iter().map(|(province, value)| format!("add VP {province} = {value}")),
    );
    return;
  };
  let Some(block) = as_block(container) else {
    builder.diagnostic(
      PatchDiagnosticKind::UnsupportedStructure,
      PatchSafety::Blocked,
      "victory_points",
      Some(container.span),
      "victory_points is not a positional block.",
      "Use province/value pairs in a block.",
    );
    return;
  };
  let scalar_entries = block.entries.iter()
    .filter(|entry| entry.key.is_none() && scalar_text(entry).is_some())
    .collect::<Vec<_>>();
  if scalar_entries.len() % 2 != 0 {
    builder.diagnostic(
      PatchDiagnosticKind::UnsupportedStructure,
      PatchSafety::Blocked,
      "victory_points",
      Some(block.span),
      "victory_points does not contain complete province/value pairs.",
      "Fix the malformed pair before patching.",
    );
    return;
  }
  let mut bindings = BTreeMap::<u32, Vec<(&PdxEntry, &PdxEntry)>>::new();
  for pair in scalar_entries.chunks(2) {
    if let (Ok(province), Some(_)) = (
      scalar_text(pair[0]).unwrap_or_default().parse::<u32>(),
      scalar_text(pair[1]),
    ) {
      bindings.entry(province).or_default().push((pair[0], pair[1]));
    }
  }
  if let Some((province, pairs)) = bindings.iter().find(|(_, pairs)| pairs.len() > 1) {
    builder.diagnostic(
      PatchDiagnosticKind::DuplicateBinding,
      PatchSafety::Blocked,
      "victory_points",
      pairs.get(1).map(|pair| pair.0.span),
      format!("Province {province} has duplicate victory point entries."),
      "Resolve the duplicates before patching.",
    );
    return;
  }
  for province in baseline_map.keys().chain(working_map.keys()).copied().collect::<BTreeSet<_>>() {
    match (baseline_map.get(&province), working_map.get(&province)) {
      (Some(old), Some(new)) if old != new => {
        let Some((_, value_entry)) = bindings.get(&province).and_then(|pairs| pairs.first()).copied()
        else {
          builder.diagnostic(
            PatchDiagnosticKind::MissingBinding,
            PatchSafety::Blocked,
            "victory_points",
            None,
            format!("VP {province} has no unique source pair."),
            "Reload or fix the source document.",
          );
          continue;
        };
        builder.replace(
          "victory_points",
          value_entry.span,
          new.to_string(),
          format!("Replace VP {province} value"),
        );
        builder.changes.push(format!("VP {province}: {old} -> {new}"));
      },
      (Some(old), None) => {
        let Some((province_entry, value_entry)) =
          bindings.get(&province).and_then(|pairs| pairs.first()).copied()
        else {
          continue;
        };
        let span = relocatable_span(builder.document, TextSpan::new(
          province_entry.span.start,
          value_entry.span.end().saturating_sub(province_entry.span.start),
        ));
        let destination = edit.province_state_id(province);
        if destination.is_none() {
          builder.diagnostic(
            PatchDiagnosticKind::UnsupportedStructure,
            PatchSafety::Blocked,
            "victory_points",
            Some(span),
            format!("VP {province} has no destination state in the working project."),
            "Assign the province to a state before planning persistence.",
          );
          continue;
        }
        let destination_value = edit.province_data(province)
          .and_then(|data| data.victory_point);
        if contains_comment(builder.document, span) && destination_value != Some(*old) {
          builder.diagnostic(
            PatchDiagnosticKind::UnsafeCommentAssociation,
            PatchSafety::Blocked,
            "victory_points",
            Some(span),
            format!("VP {province} changed while its source fragment carries a comment."),
            "Keep the original value or remove the ambiguous comment association manually.",
          );
          continue;
        }
        builder.delete(
          "victory_points",
          span,
          format!("Remove VP {province} = {old}"),
          PatchSafety::Safe,
        );
        builder.changes.push(format!("remove VP {province} = {old}"));
      },
      (None, Some(new)) => {
        let relocated = relocated_victory_point(project, province, *new);
        let (offset, content, safety) = match relocated {
          Some((source, span)) => {
            let raw = bytes_at(source.original_bytes(), span).unwrap_or_default();
            let (offset, content) = fragment_insertion(builder.document, block, raw);
            let safety = if newline(source) == newline(builder.document) {
              PatchSafety::Safe
            } else {
              PatchSafety::ReviewRequired
            };
            (offset, content, safety)
          },
          None => {
            let (offset, content) =
              positional_insertion(builder.document, block, &format!("{province} {new}"));
            (offset, content, PatchSafety::Safe)
          },
        };
        builder.insert(
          "victory_points",
          offset,
          content,
          format!("Add VP {province} = {new}"),
          safety,
        );
        builder.changes.push(format!("add VP {province} = {new}"));
      },
      _ => {},
    }
  }
}

fn plan_buildings(
  builder: &mut FileBuilder<'_>,
  project: &Hoi4Project,
  edit: &StateEditSession,
  history: &PdxBlock,
  baseline: &StateData,
  working: &StateData,
) {
  if baseline.history.state_buildings == working.history.state_buildings
    && baseline.history.province_buildings == working.history.province_buildings
  {
    return;
  }
  let containers = entries(history, "buildings");
  if containers.len() > 1 {
    builder.diagnostic(
      PatchDiagnosticKind::DuplicateBinding,
      PatchSafety::Blocked,
      "buildings",
      containers.get(1).map(|entry| entry.span),
      "Multiple buildings blocks are ambiguous.",
      "Merge the blocks before patching.",
    );
    return;
  }
  let Some(container) = containers.first().copied() else {
    let body = render_building_lines(working);
    let (offset, content) = block_insertion(builder.document, history, "buildings", &body);
    builder.insert(
      "buildings",
      offset,
      content,
      "Insert buildings block",
      PatchSafety::Safe,
    );
    builder.changes.push("buildings: create block".to_owned());
    return;
  };
  let Some(block) = as_block(container) else {
    builder.diagnostic(
      PatchDiagnosticKind::UnsupportedStructure,
      PatchSafety::Blocked,
      "buildings",
      Some(container.span),
      "buildings is not a block.",
      "Use a normal buildings block.",
    );
    return;
  };
  plan_named_block(
    builder,
    history,
    "buildings",
    &baseline.history.state_buildings,
    &working.history.state_buildings,
  );
  plan_province_buildings(builder, project, edit, block, baseline, working);
}

fn plan_province_buildings(
  builder: &mut FileBuilder<'_>,
  project: &Hoi4Project,
  edit: &StateEditSession,
  buildings: &PdxBlock,
  baseline: &StateData,
  working: &StateData,
) {
  let province_ids = baseline.history.province_buildings.keys()
    .chain(working.history.province_buildings.keys())
    .copied()
    .collect::<BTreeSet<_>>();
  for province_id in province_ids {
    let before = baseline.history.province_buildings.get(&province_id);
    let after = working.history.province_buildings.get(&province_id);
    if before == after {
      continue;
    }
    let key = province_id.to_string();
    let bindings = entries(buildings, &key);
    if bindings.len() > 1 {
      builder.diagnostic(
        PatchDiagnosticKind::DuplicateBinding,
        PatchSafety::Blocked,
        "province_buildings",
        bindings.get(1).map(|entry| entry.span),
        format!("Province {province_id} has duplicate buildings blocks."),
        "Resolve the duplicate blocks before patching.",
      );
      continue;
    }
    match (bindings.first().copied(), before, after) {
      (Some(entry), _, None) => {
        let span = relocatable_span(builder.document, entry.span);
        let destination = edit.province_state_id(province_id);
        let destination_values = edit.province_data(province_id).map(|data| data.buildings);
        if destination.is_none() {
          builder.diagnostic(
            PatchDiagnosticKind::UnsupportedStructure,
            PatchSafety::Blocked,
            "province_buildings",
            Some(span),
            format!("Province {province_id} buildings have no destination state."),
            "Assign the province to a state before planning persistence.",
          );
          continue;
        }
        if destination_values.as_ref() != before {
          builder.diagnostic(
            PatchDiagnosticKind::UnsupportedStructure,
            PatchSafety::Blocked,
            "province_buildings",
            Some(span),
            format!(
              "Province {province_id} buildings changed while their source fragment was relocated."
            ),
            "Apply the building edit before moving the province, or review it in a later fragment-editing phase.",
          );
          continue;
        }
        builder.delete(
          "province_buildings",
          span,
          format!("Remove Province {province_id} buildings block"),
          PatchSafety::Safe,
        );
        builder.changes.push(format!("remove Province {province_id} buildings"));
      },
      (None, None, Some(values)) | (None, Some(_), Some(values)) => {
        let relocated = relocated_province_buildings(project, province_id, values);
        let (offset, content, safety) = match relocated {
          Some((source, entry)) => {
            let span = relocatable_span(source, entry.span);
            let raw = bytes_at(source.original_bytes(), span).unwrap_or_default();
            let (offset, content) = fragment_insertion(builder.document, buildings, raw);
            let safety = if newline(source) == newline(builder.document) {
              PatchSafety::Safe
            } else {
              PatchSafety::ReviewRequired
            };
            (offset, content, safety)
          },
          None => {
            let lines =
              values.iter().map(|(name, level)| format!("{name}={level}")).collect::<Vec<_>>();
            let (offset, content) =
              nested_block_insertion(builder.document, buildings, &key, &lines);
            (offset, content, PatchSafety::Safe)
          },
        };
        builder.insert(
          "province_buildings",
          offset,
          content,
          format!("Add Province {province_id} buildings block"),
          safety,
        );
        builder.changes.push(format!("add Province {province_id} buildings"));
      },
      (Some(entry), Some(old), Some(new)) => {
        let Some(block) = as_block(entry) else {
          builder.diagnostic(
            PatchDiagnosticKind::UnsupportedStructure,
            PatchSafety::Blocked,
            "province_buildings",
            Some(entry.span),
            format!("Province {province_id} buildings is not a block."),
            "Use a normal nested buildings block.",
          );
          continue;
        };
        plan_named_block_contents(
          builder,
          block,
          &format!("province {province_id}"),
          old,
          new,
        );
      },
      _ => {},
    }
  }
}

fn plan_creation(
  project: &Hoi4Project,
  state_id: u32,
  working: &StateData,
  timings: &mut PhaseTimings,
) -> PlannedFileCreation {
  let relative = PathBuf::from(format!("history/states/{state_id}-State_{state_id}.txt"));
  let path = project.paths.root.join(&relative);
  let mut diagnostics = Vec::new();
  let collision = fs::read_dir(&project.paths.states_directory)
    .ok()
    .into_iter()
    .flatten()
    .filter_map(Result::ok)
    .any(|entry| entry.path().to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy()));
  if collision {
    diagnostics.push(PatchDiagnostic {
      kind: PatchDiagnosticKind::PathCollision,
      safety: PatchSafety::Blocked,
      path: relative.clone(),
      state_id: Some(state_id),
      field: Some("path".to_owned()),
      span: None,
      message: format!("The planned path {} already exists.", relative.display()),
      action: "Choose another state ID or resolve the path collision.".to_owned(),
    });
  }
  let (line_ending, bom) = canonical_document_style(project);
  let content = render_new_state(working, line_ending, bom);
  let validation_started = Instant::now();
  let validation = validate_preview(&relative, &content, working);
  timings.preview_validation_ms += validation_started.elapsed().as_millis();
  if let Err(message) = validation {
    diagnostics.push(PatchDiagnostic {
      kind: PatchDiagnosticKind::SemanticMismatch,
      safety: PatchSafety::Blocked,
      path: relative.clone(),
      state_id: Some(state_id),
      field: Some("document".to_owned()),
      span: None,
      message,
      action: "Review the canonical renderer output.".to_owned(),
    });
  }
  let safety = maximum_safety(diagnostics.iter().map(|diagnostic| diagnostic.safety));
  let diff_started = Instant::now();
  let unified_diff = created_diff(&relative, std::str::from_utf8(&content).unwrap_or_default());
  timings.diff_generation_ms += diff_started.elapsed().as_millis();
  PlannedFileCreation {
    unified_diff,
    path: relative,
    state_id,
    content,
    semantic_changes: vec![
      format!("create State {state_id}"),
      format!("add {} province(s)", working.provinces.len()),
      format!("add {} victory point(s)", working.history.victory_points.len()),
    ],
    diagnostics,
    safety,
  }
}

pub fn validate_non_overlapping_patches(
  operations: &[TextPatchOperation],
) -> Result<(), String> {
  let mut ranges = operations.iter()
    .filter_map(|operation| {
      let start = operation.start();
      let end = operation.end();
      (start != end).then_some((start, end))
    })
    .collect::<Vec<_>>();
  ranges.sort_unstable();
  for pair in ranges.windows(2) {
    if pair[0].1 > pair[1].0 {
      return Err(format!(
        "Patch ranges {}..{} and {}..{} overlap.",
        pair[0].0, pair[0].1, pair[1].0, pair[1].1
      ));
    }
  }
  Ok(())
}

pub fn apply_operations(
  original: &[u8],
  operations: &[TextPatchOperation],
) -> Result<Vec<u8>, String> {
  validate_non_overlapping_patches(operations)?;
  for operation in operations {
    match operation {
      TextPatchOperation::Replace { range, expected, .. }
      | TextPatchOperation::Delete { range, expected, .. } => {
        if original.get(range.clone()) != Some(expected.as_slice()) {
          return Err(format!(
            "Expected source bytes do not match at {}..{}.",
            range.start, range.end
          ));
        }
      },
      TextPatchOperation::Insert { offset, .. } if *offset > original.len() => {
        return Err(format!("Insert offset {offset} exceeds the source length."));
      },
      TextPatchOperation::Insert { .. } => {},
    }
  }
  let mut indexed = operations.iter().enumerate().collect::<Vec<_>>();
  indexed.sort_by(|(left_index, left), (right_index, right)| {
    left.start().cmp(&right.start())
      .then_with(|| left_index.cmp(right_index))
  });
  let mut bytes = original.to_vec();
  for (_, operation) in indexed.into_iter().rev() {
    match operation {
      TextPatchOperation::Replace { range, replacement, .. } => {
        bytes.splice(range.clone(), replacement.iter().copied());
      },
      TextPatchOperation::Insert { offset, content, .. } => {
        bytes.splice(*offset..*offset, content.iter().copied());
      },
      TextPatchOperation::Delete { range, .. } => {
        bytes.drain(range.clone());
      },
    }
  }
  Ok(bytes)
}

fn source_safety_diagnostics(document: &StateDocument, state_id: u32) -> Vec<PatchDiagnostic> {
  let mut diagnostics = Vec::new();
  if !document.exact_utf8 {
    diagnostics.push(PatchDiagnostic {
      kind: PatchDiagnosticKind::InvalidEncoding,
      safety: PatchSafety::Blocked,
      path: document.path.clone(),
      state_id: Some(state_id),
      field: Some("encoding".to_owned()),
      span: None,
      message: "The source required lossy UTF-8 decoding; byte spans are not safe to patch.".to_owned(),
      action: "Convert the source to valid UTF-8 without changing its semantics, then reload.".to_owned(),
    });
  }
  if document.data.is_none() || !document.syntax.diagnostics.is_empty() {
    diagnostics.push(PatchDiagnostic {
      kind: PatchDiagnosticKind::ParseFailure,
      safety: PatchSafety::Blocked,
      path: document.path.clone(),
      state_id: Some(state_id),
      field: Some("document".to_owned()),
      span: document.syntax.diagnostics.first().map(|diagnostic| diagnostic.span),
      message: "The original document is not syntactically safe to patch.".to_owned(),
      action: "Fix the source diagnostics and reload the project.".to_owned(),
    });
  }
  match fs::read(&document.path) {
    Ok(current) if current.as_slice() != document.original_bytes() => diagnostics.push(PatchDiagnostic {
      kind: PatchDiagnosticKind::SourceChanged,
      safety: PatchSafety::Blocked,
      path: document.path.clone(),
      state_id: Some(state_id),
      field: Some("source".to_owned()),
      span: None,
      message: "Source file changed after project load.".to_owned(),
      action: "Reload the project before generating a safe patch.".to_owned(),
    }),
    Err(error) if document.path.is_absolute() => diagnostics.push(PatchDiagnostic {
      kind: PatchDiagnosticKind::SourceChanged,
      safety: PatchSafety::Blocked,
      path: document.path.clone(),
      state_id: Some(state_id),
      field: Some("source".to_owned()),
      span: None,
      message: format!("The source file could not be re-read: {error}."),
      action: "Restore or reload the project source.".to_owned(),
    }),
    _ => {},
  }
  diagnostics
}

fn root_state_block(document: &PdxDocument) -> Option<&PdxBlock> {
  let roots = document.entries.iter()
    .filter(|entry| entry.key.as_ref().is_some_and(|key| key.text == "state"))
    .filter_map(as_block)
    .collect::<Vec<_>>();
  (roots.len() == 1).then_some(roots[0])
}

fn non_dated_history_block(document: &StateDocument) -> Option<&PdxBlock> {
  let root = root_state_block(&document.syntax)?;
  let history = entries(root, "history");
  (history.len() == 1).then(|| as_block(history[0])).flatten()
}

fn relocated_victory_point(
  project: &Hoi4Project,
  province_id: u32,
  value: i64,
) -> Option<(&StateDocument, TextSpan)> {
  let source_state_id = *project.state_by_province.get(&province_id)?;
  let source = project.state_document(source_state_id)?;
  if source_safety_diagnostics(source, source_state_id)
    .iter()
    .any(|diagnostic| diagnostic.safety == PatchSafety::Blocked)
  {
    return None;
  }
  let original_value = source.data.as_ref()?.history.victory_points.iter()
    .find(|point| point.province_id == province_id)?
    .value;
  if original_value != value {
    return None;
  }
  let history = non_dated_history_block(source)?;
  let containers = entries(history, "victory_points");
  let block = (containers.len() == 1).then(|| as_block(containers[0])).flatten()?;
  let scalar_entries = block.entries.iter()
    .filter(|entry| entry.key.is_none() && scalar_text(entry).is_some())
    .collect::<Vec<_>>();
  let pairs = scalar_entries.chunks_exact(2)
    .filter(|pair| scalar_text(pair[0]).and_then(|text| text.parse::<u32>().ok()) == Some(province_id))
    .collect::<Vec<_>>();
  let [pair] = pairs.as_slice() else { return None };
  let span = TextSpan::new(
    pair[0].span.start,
    pair[1].span.end().saturating_sub(pair[0].span.start),
  );
  Some((source, relocatable_span(source, span)))
}

fn relocated_province_buildings<'a>(
  project: &'a Hoi4Project,
  province_id: u32,
  values: &BTreeMap<String, i64>,
) -> Option<(&'a StateDocument, &'a PdxEntry)> {
  let source_state_id = *project.state_by_province.get(&province_id)?;
  let source = project.state_document(source_state_id)?;
  if source_safety_diagnostics(source, source_state_id)
    .iter()
    .any(|diagnostic| diagnostic.safety == PatchSafety::Blocked)
    || source.data.as_ref()?.history.province_buildings.get(&province_id) != Some(values)
  {
    return None;
  }
  let history = non_dated_history_block(source)?;
  let containers = entries(history, "buildings");
  let buildings = (containers.len() == 1).then(|| as_block(containers[0])).flatten()?;
  let bindings = entries(buildings, &province_id.to_string());
  let [entry] = bindings.as_slice() else { return None };
  Some((source, entry))
}

fn entries<'a>(block: &'a PdxBlock, key: &str) -> Vec<&'a PdxEntry> {
  block.entries.iter()
    .filter(|entry| entry.key.as_ref().is_some_and(|candidate| candidate.text == key))
    .collect()
}

fn relocatable_span(document: &StateDocument, span: TextSpan) -> TextSpan {
  let bytes = document.original_bytes();
  let line_end = bytes.get(span.end().min(bytes.len())..)
    .and_then(|tail| tail.iter().position(|byte| matches!(byte, b'\r' | b'\n')))
    .map_or(bytes.len(), |offset| span.end() + offset);
  let comment_end = document.syntax.tokens.iter()
    .filter(|token| {
      token.kind == TokenKind::Comment
        && token.span.start >= span.end()
        && token.span.start < line_end
    })
    .map(|token| token.span.end())
    .max()
    .unwrap_or(span.end());
  TextSpan::new(span.start, comment_end.saturating_sub(span.start))
}

fn as_block(entry: &PdxEntry) -> Option<&PdxBlock> {
  match &entry.value {
    PdxValue::Block(block) => Some(block),
    PdxValue::Scalar(_) => None,
  }
}

fn scalar_span(entry: &PdxEntry) -> Option<TextSpan> {
  match &entry.value {
    PdxValue::Scalar(scalar) => Some(scalar.span),
    PdxValue::Block(_) => None,
  }
}

fn scalar_text(entry: &PdxEntry) -> Option<&str> {
  match &entry.value {
    PdxValue::Scalar(scalar) => Some(&scalar.text),
    PdxValue::Block(_) => None,
  }
}

fn bytes_at(bytes: &[u8], span: TextSpan) -> Option<&[u8]> {
  bytes.get(span.start..span.end())
}

fn comment_safety(document: &StateDocument, span: TextSpan) -> PatchSafety {
  let line_end = document.original_bytes()[span.end().min(document.original_bytes().len())..]
    .iter()
    .position(|byte| matches!(byte, b'\r' | b'\n'))
    .map_or(document.original_bytes().len(), |offset| span.end() + offset);
  if document.syntax.tokens.iter().any(|token| {
    token.kind == TokenKind::Comment
      && token.span.start >= span.end()
      && token.span.start < line_end
  }) {
    PatchSafety::ReviewRequired
  } else {
    PatchSafety::Safe
  }
}

fn contains_comment(document: &StateDocument, span: TextSpan) -> bool {
  document.syntax.tokens.iter().any(|token| {
    token.kind == TokenKind::Comment
      && token.span.start >= span.start
      && token.span.end() <= span.end()
  })
}

fn newline(document: &StateDocument) -> &'static str {
  match document.syntax.source.newline_style {
    NewlineStyle::Crlf => "\r\n",
    NewlineStyle::Cr => "\r",
    NewlineStyle::Lf | NewlineStyle::Mixed | NewlineStyle::None => "\n",
  }
}

fn indentation(document: &StateDocument, block: &PdxBlock) -> String {
  block.entries.first()
    .and_then(|entry| indentation_before(document.original_bytes(), entry.span.start))
    .filter(|indent| !indent.is_empty())
    .unwrap_or_else(|| {
      let parent = indentation_before(document.original_bytes(), block.span.start)
        .unwrap_or_default();
      format!("{parent}    ")
    })
}

fn indentation_before(bytes: &[u8], offset: usize) -> Option<String> {
  let before = bytes.get(..offset)?;
  let start = before.iter()
    .rposition(|byte| matches!(byte, b'\r' | b'\n'))
    .map_or(0, |index| index + 1);
  let indent = before.get(start..)?;
  indent.iter()
    .all(|byte| matches!(byte, b' ' | b'\t'))
    .then(|| String::from_utf8_lossy(indent).into_owned())
}

fn insertion_at_closing(
  document: &StateDocument,
  block: &PdxBlock,
  rendered_line: &str,
) -> (usize, Vec<u8>) {
  let close = block.span.end().saturating_sub(1);
  let bytes = document.original_bytes();
  let line_start = bytes.get(..close)
    .and_then(|before| before.iter().rposition(|byte| matches!(byte, b'\r' | b'\n')))
    .map_or(0, |index| index + 1);
  let closing_prefix = bytes.get(line_start..close).unwrap_or_default();
  let closing_on_own_line = closing_prefix.iter().all(|byte| matches!(byte, b' ' | b'\t'));
  if closing_on_own_line {
    let mut content = rendered_line.as_bytes().to_vec();
    content.extend_from_slice(newline(document).as_bytes());
    (line_start, content)
  } else {
    let closing_indent = indentation_before(bytes, block.span.start).unwrap_or_default();
    (
      close,
      format!("{}{rendered_line}{}{closing_indent}", newline(document), newline(document)).into_bytes(),
    )
  }
}

fn assignment_insertion(
  document: &StateDocument,
  block: &PdxBlock,
  key: &str,
  value: &str,
) -> (usize, Vec<u8>) {
  let line = format!("{}{key}={value}", indentation(document, block));
  insertion_at_closing(document, block, &line)
}

fn positional_insertion(
  document: &StateDocument,
  block: &PdxBlock,
  value: &str,
) -> (usize, Vec<u8>) {
  let line = format!("{}{value}", indentation(document, block));
  insertion_at_closing(document, block, &line)
}

fn fragment_insertion(
  document: &StateDocument,
  block: &PdxBlock,
  fragment: &[u8],
) -> (usize, Vec<u8>) {
  let fragment = std::str::from_utf8(fragment).unwrap_or_default();
  let line = format!("{}{fragment}", indentation(document, block));
  insertion_at_closing(document, block, &line)
}

fn block_insertion(
  document: &StateDocument,
  parent: &PdxBlock,
  key: &str,
  lines: &[String],
) -> (usize, Vec<u8>) {
  let indent = indentation(document, parent);
  let child_indent = format!("{indent}    ");
  let nl = newline(document);
  let mut line = format!("{indent}{key}={{");
  for child in lines {
    line.push_str(nl);
    line.push_str(&child_indent);
    line.push_str(child);
  }
  line.push_str(nl);
  line.push_str(&indent);
  line.push('}');
  insertion_at_closing(document, parent, &line)
}

fn nested_block_insertion(
  document: &StateDocument,
  parent: &PdxBlock,
  key: &str,
  lines: &[String],
) -> (usize, Vec<u8>) {
  block_insertion(document, parent, key, lines)
}

fn quote_string(value: &str) -> String {
  format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(value: &str) -> &str {
  value.strip_prefix('"')
    .and_then(|value| value.strip_suffix('"'))
    .unwrap_or(value)
}

fn format_number(value: f64) -> String {
  value.to_string()
}

fn victory_point_map(points: &[VictoryPoint]) -> BTreeMap<u32, i64> {
  points.iter().map(|point| (point.province_id, point.value)).collect()
}

fn victory_points_equal(left: &[VictoryPoint], right: &[VictoryPoint]) -> bool {
  victory_point_map(left) == victory_point_map(right)
}

fn render_history_lines(data: &StateData) -> Vec<String> {
  let mut lines = Vec::new();
  if let Some(owner) = &data.history.owner {
    lines.push(format!("owner={owner}"));
  }
  if let Some(controller) = &data.history.controller {
    lines.push(format!("controller={controller}"));
  }
  lines.extend(data.history.cores.iter().map(|tag| format!("add_core_of={tag}")));
  lines.extend(data.history.claims.iter().map(|tag| format!("add_claim_by={tag}")));
  if !data.history.victory_points.is_empty() {
    lines.push("victory_points={".to_owned());
    lines.extend(data.history.victory_points.iter()
      .map(|point| format!("    {} {}", point.province_id, point.value)));
    lines.push("}".to_owned());
  }
  if !data.history.state_buildings.is_empty() || !data.history.province_buildings.is_empty() {
    lines.push("buildings={".to_owned());
    lines.extend(render_building_lines(data).into_iter().map(|line| format!("    {line}")));
    lines.push("}".to_owned());
  }
  lines
}

fn render_building_lines(data: &StateData) -> Vec<String> {
  let mut lines = data.history.state_buildings.iter()
    .map(|(name, level)| format!("{name}={level}"))
    .collect::<Vec<_>>();
  for (province, buildings) in &data.history.province_buildings {
    lines.push(format!("{province}={{"));
    lines.extend(buildings.iter().map(|(name, level)| format!("    {name}={level}")));
    lines.push("}".to_owned());
  }
  lines
}

fn canonical_document_style(project: &Hoi4Project) -> (NewlineStyle, bool) {
  let mut counts = [0_usize; 3];
  let mut exact = 0_usize;
  let mut bom = 0_usize;
  for document in project.states.iter().filter(|document| document.exact_utf8) {
    exact += 1;
    bom += usize::from(document.syntax.source.bom);
    match document.syntax.source.newline_style {
      NewlineStyle::Lf => counts[0] += 1,
      NewlineStyle::Crlf => counts[1] += 1,
      NewlineStyle::Cr => counts[2] += 1,
      NewlineStyle::None | NewlineStyle::Mixed => {},
    }
  }
  let line_ending = counts.iter().enumerate()
    .max_by_key(|(index, count)| (**count, usize::from(*index == 1)))
    .map(|(index, _)| match index {
      0 => NewlineStyle::Lf,
      2 => NewlineStyle::Cr,
      _ => NewlineStyle::Crlf,
    })
    .unwrap_or(NewlineStyle::Crlf);
  (line_ending, exact != 0 && bom * 2 > exact)
}

fn render_new_state(data: &StateData, style: NewlineStyle, bom: bool) -> Vec<u8> {
  let nl = match style {
    NewlineStyle::Crlf => "\r\n",
    NewlineStyle::Cr => "\r",
    _ => "\n",
  };
  let mut lines = vec!["state={".to_owned()];
  if let Some(id) = data.id {
    lines.push(format!("    id={id}"));
  }
  if let Some(name) = &data.name {
    lines.push(format!("    name={}", quote_string(name)));
  }
  if let Some(manpower) = data.manpower {
    lines.push(format!("    manpower={manpower}"));
  }
  if let Some(category) = &data.state_category {
    lines.push(format!("    state_category={category}"));
  }
  if let Some(factor) = data.buildings_max_level_factor {
    lines.push(format!("    buildings_max_level_factor={}", format_number(factor)));
  }
  if let Some(supplies) = data.local_supplies {
    lines.push(format!("    local_supplies={}", format_number(supplies)));
  }
  if data.impassable == Some(true) {
    lines.push("    impassable=yes".to_owned());
  }
  if !data.resources.is_empty() {
    lines.push("    resources={".to_owned());
    lines.extend(data.resources.iter().map(|(name, value)| format!("        {name}={value}")));
    lines.push("    }".to_owned());
  }
  lines.push("    provinces={".to_owned());
  if !data.provinces.is_empty() {
    lines.push(format!(
      "        {}",
      data.provinces.iter().map(u32::to_string).collect::<Vec<_>>().join(" ")
    ));
  }
  lines.push("    }".to_owned());
  let history = render_history_lines(data);
  if !history.is_empty() {
    lines.push("    history={".to_owned());
    lines.extend(history.into_iter().map(|line| format!("        {line}")));
    lines.push("    }".to_owned());
  }
  lines.push("}".to_owned());
  format!("{}{}{nl}", if bom { "\u{feff}" } else { "" }, lines.join(nl)).into_bytes()
}

fn validate_preview(path: &Path, bytes: &[u8], working: &StateData) -> Result<(), String> {
  let text = String::from_utf8(bytes.to_vec())
    .map_err(|_| "preview is not valid UTF-8".to_owned())?;
  let document = parse(SourceText::new(path, text));
  if !document.diagnostics.is_empty() {
    return Err(format!("preview parse failed: {}", document.diagnostics[0].message));
  }
  let extracted = extract_state(&document);
  let Some(data) = extracted.data else {
    return Err("preview parse failed: missing root state block".to_owned());
  };
  semantic_match(&data, working)
    .then_some(())
    .ok_or_else(|| "semantic preview mismatch for editable state fields".to_owned())
}

fn semantic_match(left: &StateData, right: &StateData) -> bool {
  left.id == right.id
    && left.name == right.name
    && left.provinces == right.provinces
    && left.manpower == right.manpower
    && left.buildings_max_level_factor == right.buildings_max_level_factor
    && left.state_category == right.state_category
    && left.local_supplies == right.local_supplies
    && left.impassable.unwrap_or(false) == right.impassable.unwrap_or(false)
    && left.resources == right.resources
    && left.history.owner == right.history.owner
    && left.history.controller == right.history.controller
    && left.history.cores == right.history.cores
    && left.history.claims == right.history.claims
    && victory_points_equal(&left.history.victory_points, &right.history.victory_points)
    && left.history.state_buildings == right.history.state_buildings
    && left.history.province_buildings == right.history.province_buildings
}

fn relative_path(project: &Hoi4Project, path: &Path) -> PathBuf {
  path.strip_prefix(&project.paths.root).unwrap_or(path).to_owned()
}

fn maximum_safety(values: impl IntoIterator<Item = PatchSafety>) -> PatchSafety {
  values.into_iter().max().unwrap_or(PatchSafety::Safe)
}

fn summarize(
  modified: &[PlannedFileModification],
  created: &[PlannedFileCreation],
  removed: &[PlannedFileRemoval],
) -> PatchPlanSummary {
  let safeties = modified.iter().map(|file| file.safety)
    .chain(created.iter().map(|file| file.safety))
    .chain(removed.iter().map(|file| file.safety))
    .collect::<Vec<_>>();
  PatchPlanSummary {
    modified_files: modified.len(),
    created_files: created.len(),
    removed_files: removed.len(),
    safe_files: safeties.iter().filter(|&&safety| safety == PatchSafety::Safe).count(),
    review_required_files: safeties.iter()
      .filter(|&&safety| safety == PatchSafety::ReviewRequired)
      .count(),
    blocked_files: safeties.iter().filter(|&&safety| safety == PatchSafety::Blocked).count(),
  }
}

#[allow(clippy::too_many_arguments)]
fn format_file_report<'a>(
  marker: &str,
  path: &Path,
  state_id: u32,
  safety: PatchSafety,
  changes: &[String],
  operations: impl IntoIterator<Item = &'a str>,
  diagnostics: &[PatchDiagnostic],
  diff: &str,
) -> String {
  let mut text = format!(
    "[{marker}] {} | State {state_id} | {}\nSemantic changes:",
    path.display(),
    safety.label()
  );
  for change in changes {
    text.push_str(&format!("\n- {change}"));
  }
  text.push_str("\nOperations:");
  for operation in operations {
    text.push_str(&format!("\n- {operation}"));
  }
  if !diagnostics.is_empty() {
    text.push_str("\nDiagnostics:");
    for diagnostic in diagnostics {
      text.push_str(&format!(
        "\n- {}: {} Action: {}",
        diagnostic.safety.label(),
        diagnostic.message,
        diagnostic.action
      ));
    }
  }
  if !diff.is_empty() {
    text.push_str("\nDiff:\n");
    text.push_str(diff);
  }
  text
}

fn modified_diff(path: &Path, before: &[u8], after: &[u8]) -> String {
  text_diff(
    &format!("a/{}", path.display()),
    &format!("b/{}", path.display()),
    std::str::from_utf8(before).unwrap_or_default(),
    std::str::from_utf8(after).unwrap_or_default(),
  )
}

fn created_diff(path: &Path, after: &str) -> String {
  text_diff("/dev/null", &format!("b/{}", path.display()), "", after)
}

fn removed_diff(path: &Path, before: &str) -> String {
  text_diff(&format!("a/{}", path.display()), "/dev/null", before, "")
}

fn text_diff(before_name: &str, after_name: &str, before: &str, after: &str) -> String {
  if before == after {
    return String::new();
  }
  let before_lines = before.lines().collect::<Vec<_>>();
  let after_lines = after.lines().collect::<Vec<_>>();
  let mut prefix = 0;
  while prefix < before_lines.len()
    && prefix < after_lines.len()
    && before_lines[prefix] == after_lines[prefix]
  {
    prefix += 1;
  }
  let mut suffix = 0;
  while suffix < before_lines.len().saturating_sub(prefix)
    && suffix < after_lines.len().saturating_sub(prefix)
    && before_lines[before_lines.len() - 1 - suffix] == after_lines[after_lines.len() - 1 - suffix]
  {
    suffix += 1;
  }
  let old_end = before_lines.len().saturating_sub(suffix);
  let new_end = after_lines.len().saturating_sub(suffix);
  let mut diff = format!(
    "--- {before_name}\n+++ {after_name}\n@@ -{},{} +{},{} @@\n",
    prefix + 1,
    old_end.saturating_sub(prefix),
    prefix + 1,
    new_end.saturating_sub(prefix),
  );
  for line in &before_lines[prefix..old_end] {
    diff.push('-');
    diff.push_str(line);
    diff.push('\n');
  }
  for line in &after_lines[prefix..new_end] {
    diff.push('+');
    diff.push_str(line);
    diff.push('\n');
  }
  diff
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::state::{extract_state, parse_text, StateHistory};
  use std::sync::Arc;

  #[test]
  fn expected_bytes_and_overlap_are_enforced() {
    let operations = vec![
      TextPatchOperation::Replace {
        range: 2..4,
        expected: b"cd".to_vec(),
        replacement: b"XY".to_vec(),
        state_id: 1,
        field: "test".to_owned(),
        description: "replace".to_owned(),
        safety: PatchSafety::Safe,
      },
      TextPatchOperation::Insert {
        offset: 6,
        content: b"!".to_vec(),
        state_id: 1,
        field: "test".to_owned(),
        description: "insert".to_owned(),
        safety: PatchSafety::Safe,
      },
    ];
    assert_eq!(apply_operations(b"abcdef", &operations).unwrap(), b"abXYef!");
    let mut overlapping = operations;
    overlapping.push(TextPatchOperation::Delete {
      range: 3..5,
      expected: b"de".to_vec(),
      state_id: 1,
      field: "test".to_owned(),
      description: "overlap".to_owned(),
      safety: PatchSafety::Safe,
    });
    assert!(validate_non_overlapping_patches(&overlapping).is_err());
  }

  #[test]
  fn canonical_state_preview_roundtrips_semantically() {
    let mut state = StateData {
      id: Some(512),
      name: Some("STATE_512".to_owned()),
      manpower: Some(150_000),
      state_category: Some("town".to_owned()),
      resources: BTreeMap::from([("oil".to_owned(), 10)]),
      history: StateHistory {
        owner: Some("TAG".to_owned()),
        victory_points: vec![VictoryPoint { province_id: 5144, value: 5 }],
        province_buildings: BTreeMap::from([(
          5144,
          BTreeMap::from([("naval_base".to_owned(), 3)]),
        )]),
        ..Default::default()
      },
      ..Default::default()
    };
    state.provinces.insert(5144);
    let rendered = render_new_state(&state, NewlineStyle::Crlf, false);
    validate_preview(Path::new("512-State_512.txt"), &rendered, &state).unwrap();
  }

  #[test]
  fn scalar_replace_keeps_unrelated_source_bytes() {
    let text = "state={\r\n    id=1\r\n    manpower = 142000 # keep\r\n    custom=yes\r\n    provinces={ 1 }\r\n}\r\n";
    let syntax = parse_text("1.txt", text);
    let root = root_state_block(&syntax).unwrap();
    let entry = entries(root, "manpower")[0];
    let span = scalar_span(entry).unwrap();
    let operation = TextPatchOperation::Replace {
      range: span.start..span.end(),
      expected: b"142000".to_vec(),
      replacement: b"150000".to_vec(),
      state_id: 1,
      field: "manpower".to_owned(),
      description: "replace manpower".to_owned(),
      safety: PatchSafety::Safe,
    };
    let after = apply_operations(text.as_bytes(), &[operation]).unwrap();
    assert_eq!(
      String::from_utf8(after).unwrap(),
      text.replace("142000", "150000")
    );
  }

  #[test]
  fn ambiguous_or_changed_sources_are_blocked_without_operations() {
    let text =
      "state={ id=1 provinces={ 1 } resources={ oil=4 oil=4 } history={ owner=TAG owner=ALT } }";
    let syntax = parse_text("fixture.txt", text);
    let data = extract_state(&syntax).data;
    let document = StateDocument {
      path: PathBuf::from("fixture.txt"),
      original_bytes: Arc::from(text.as_bytes()),
      exact_utf8: true,
      syntax,
      data,
      diagnostics: Vec::new(),
      modified: false,
    };
    let mut builder = FileBuilder::new(&document, 1);
    let root = root_state_block(&document.syntax).unwrap();
    plan_named_block(
      &mut builder,
      root,
      "resources",
      &BTreeMap::from([("oil".to_owned(), 8)]),
      &BTreeMap::from([("oil".to_owned(), 10)]),
    );
    assert!(builder.operations.is_empty());
    assert!(builder.diagnostics.iter().any(|diagnostic| {
      diagnostic.kind == PatchDiagnosticKind::DuplicateBinding
        && diagnostic.safety == PatchSafety::Blocked
    }));

    let path = std::env::temp_dir().join(format!(
      "hoi4-state-patch-source-change-{}.txt",
      std::process::id()
    ));
    fs::write(&path, text).unwrap();
    let mut changed = document.clone();
    changed.path = path.clone();
    fs::write(&path, format!("{text}\n# external")).unwrap();
    let diagnostics = source_safety_diagnostics(&changed, 1);
    let _ = fs::remove_file(&path);
    assert!(diagnostics.iter().any(|diagnostic| {
      diagnostic.kind == PatchDiagnosticKind::SourceChanged
        && diagnostic.safety == PatchSafety::Blocked
    }));
  }
}
