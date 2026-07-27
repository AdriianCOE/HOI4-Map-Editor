use std::fs;
use std::path::{Path, PathBuf};

use super::{extract_state, parse, SourceText, StateDocument};
use crate::app::project::{DiagnosticSeverity, ProjectDiagnostic, ProjectDiagnosticKind};

#[derive(Debug, Default)]
pub struct StateLoadBatch {
  pub documents: Vec<StateDocument>,
  pub diagnostics: Vec<ProjectDiagnostic>,
  pub files_found: usize,
  pub files_read: usize
}

pub fn load_state_documents(directory: &Path) -> StateLoadBatch {
  let mut batch = StateLoadBatch::default();
  let entries = match fs::read_dir(directory) {
    Ok(entries) => entries,
    Err(err) => {
      batch.diagnostics.push(ProjectDiagnostic::new(
        ProjectDiagnosticKind::InvalidStateFile,
        DiagnosticSeverity::Error,
        Some(directory.to_owned()),
        None,
        format!("Failed to list state directory {}: {err}", directory.display())
      ));
      return batch;
    }
  };

  let mut paths = Vec::new();
  for entry in entries {
    match entry {
      Ok(entry) => {
        let path = entry.path();
        let is_txt = path.extension()
          .and_then(|extension| extension.to_str())
          .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"));
        if is_txt && path.is_file() {
          paths.push(path);
        }
      }
      Err(err) => batch.diagnostics.push(ProjectDiagnostic::new(
        ProjectDiagnosticKind::InvalidStateFile,
        DiagnosticSeverity::Error,
        Some(directory.to_owned()),
        None,
        format!("Failed to inspect a state directory entry: {err}")
      ))
    }
  }

  paths.sort();
  batch.files_found = paths.len();
  for path in paths {
    let document = load_document(path, &mut batch);
    batch.documents.push(document);
  }

  batch
}

fn load_document(path: PathBuf, batch: &mut StateLoadBatch) -> StateDocument {
  let (original_bytes, text, exact_utf8, encoding_diagnostic) = match fs::read(&path) {
    Ok(bytes) => {
      batch.files_read = batch.files_read.saturating_add(1);
      match String::from_utf8(bytes.clone()) {
        Ok(text) => (bytes.into(), text, true, None),
        Err(err) => {
          let message = format!(
            "Failed to decode state file {} as UTF-8 at byte {}.",
            path.display(),
            err.utf8_error().valid_up_to()
          );
          (
            bytes.into(),
            String::from_utf8_lossy(err.as_bytes()).into_owned(),
            false,
            Some(message)
          )
        }
      }
    }
    Err(err) => {
      let message = format!("Failed to read state file {}: {err}", path.display());
      ([].into(), String::new(), false, Some(message))
    }
  };

  let syntax = parse(SourceText::new(path.clone(), text));
  let extracted = extract_state(&syntax);
  let mut diagnostics = extracted.diagnostics;

  if let Some(message) = encoding_diagnostic {
    diagnostics.push(ProjectDiagnostic::new(
      ProjectDiagnosticKind::InvalidStateFile,
      DiagnosticSeverity::Error,
      Some(path.clone()),
      None,
      message
    ));
  }
  StateDocument {
    path,
    original_bytes,
    exact_utf8,
    syntax,
    data: extracted.data,
    diagnostics,
    modified: false
  }
}

#[cfg(test)]
mod tests {
  use super::load_state_documents;
  use std::fs;
  use std::path::{Path, PathBuf};

  struct TempStates(PathBuf);

  impl TempStates {
    fn new(name: &str) -> Self {
      let root = std::env::temp_dir().join(format!(
        "hoi4-state-loader-{}-{name}",
        std::process::id()
      ));
      let _ = fs::remove_dir_all(&root);
      fs::create_dir_all(&root).unwrap();
      Self(root)
    }

    fn path(&self) -> &Path {
      &self.0
    }
  }

  impl Drop for TempStates {
    fn drop(&mut self) {
      let _ = fs::remove_dir_all(&self.0);
    }
  }

  #[test]
  fn loads_only_direct_txt_files_in_deterministic_order() {
    let states = TempStates::new("order");
    fs::write(states.path().join("B.TXT"), "state={id=2 provinces={2}}").unwrap();
    fs::write(states.path().join("a.txt"), "state={id=1 provinces={1}}").unwrap();
    fs::write(states.path().join("ignored.bak"), "state={id=3 provinces={3}}").unwrap();
    fs::create_dir(states.path().join("nested")).unwrap();
    fs::write(states.path().join("nested/4.txt"), "state={id=4 provinces={4}}").unwrap();

    let batch = load_state_documents(states.path());
    let names = batch.documents.iter()
      .filter_map(|document| document.path.file_name())
      .filter_map(|name| name.to_str())
      .collect::<Vec<_>>();

    assert_eq!(batch.files_found, 2);
    assert_eq!(batch.files_read, 2);
    assert_eq!(names, vec!["B.TXT", "a.txt"]);
  }

  #[test]
  fn invalid_file_does_not_block_other_documents() {
    let states = TempStates::new("recovery");
    fs::write(states.path().join("1-valid.txt"), "state={id=1 provinces={1}}").unwrap();
    fs::write(states.path().join("2-invalid.txt"), "state={id=").unwrap();

    let batch = load_state_documents(states.path());

    assert_eq!(batch.documents.len(), 2);
    assert!(batch.documents[0].data.is_some());
    assert!(!batch.documents[1].diagnostics.is_empty());
  }

  #[test]
  fn preserves_exact_source_bytes_and_marks_lossy_utf8() {
    let states = TempStates::new("source-bytes");
    let valid = b"\xEF\xBB\xBFstate={\r\n\tid=1\r\n\tprovinces={1}\r\n}\r\n";
    let invalid = b"state={id=2 name=\"\xFF\" provinces={2}}";
    fs::write(states.path().join("1-valid.txt"), valid).unwrap();
    fs::write(states.path().join("2-lossy.txt"), invalid).unwrap();

    let batch = load_state_documents(states.path());

    assert_eq!(batch.documents[0].original_bytes(), valid);
    assert!(batch.documents[0].exact_utf8);
    assert_eq!(batch.documents[1].original_bytes(), invalid);
    assert!(!batch.documents[1].exact_utf8);
  }
}
