use std::path::PathBuf;

use crate::app::state::TextSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
  Info,
  Warning,
  Error
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProjectDiagnosticKind {
  InvalidStateFile,
  EmptyStateFile,
  SyntaxError,
  MissingStateBlock,
  MultipleStateBlocks,
  MissingStateId,
  InvalidStateId,
  ZeroStateId,
  DuplicateStateId,
  MissingProvinces,
  EmptyProvinces,
  DuplicateProvinceInState,
  ProvinceInMultipleStates,
  UnknownProvince,
  LandProvinceWithoutState,
  MissingStateCategory,
  InvalidManpower,
  InvalidResource,
  InvalidBuilding,
  InvalidField,
  MissingStateName,
  MissingOwner,
  MissingHistory,
  StateNameIdMismatch,
  ProvinceBuildingOutsideState,
  VictoryPointOutsideState,
  StateEditSession
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDiagnostic {
  pub kind: ProjectDiagnosticKind,
  pub severity: DiagnosticSeverity,
  pub path: Option<PathBuf>,
  pub related_path: Option<PathBuf>,
  pub span: Option<TextSpan>,
  pub message: String
}

impl ProjectDiagnostic {
  pub fn new(
    kind: ProjectDiagnosticKind,
    severity: DiagnosticSeverity,
    path: impl Into<Option<PathBuf>>,
    span: impl Into<Option<TextSpan>>,
    message: impl Into<String>
  ) -> Self {
    Self {
      kind,
      severity,
      path: path.into(),
      related_path: None,
      span: span.into(),
      message: message.into()
    }
  }
}
