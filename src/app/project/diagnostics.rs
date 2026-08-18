use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::app::state::TextSpan;

type DiagnosticIdentity<'a> = (
    DiagnosticSeverity,
    DiagnosticDomain,
    &'a str,
    &'a Option<PathBuf>,
    Option<u32>,
    Option<u32>,
    Option<(usize, usize)>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

impl DiagnosticSeverity {
    #[allow(non_upper_case_globals)]
    pub const Information: Self = Self::Info;

    fn rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Info => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticDomain {
    Project,
    ProvinceMap,
    Definition,
    States,
    CrossDomain,
    Transaction,
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
    StateEditSession,
    InvalidProvinceBitmap,
    InvalidDefinition,
    DuplicateProvinceId,
    DuplicateProvinceRgb,
    MissingDefinitionForColor,
    UnusedDefinition,
    InvalidProvinceType,
    InvalidCoastal,
    InvalidContinent,
    SeaOrLakeAssigned,
    NavalBaseNonCoastal,
    CandidateMismatch,
    ExternalChange,
    TransactionFailure,
}

impl ProjectDiagnosticKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidStateFile => "states.file.invalid",
            Self::EmptyStateFile => "states.file.empty",
            Self::SyntaxError => "states.syntax.error",
            Self::MissingStateBlock => "states.block.missing",
            Self::MultipleStateBlocks => "states.block.multiple",
            Self::MissingStateId => "states.id.missing",
            Self::InvalidStateId => "states.id.invalid",
            Self::ZeroStateId => "states.id.zero",
            Self::DuplicateStateId => "states.id.duplicate",
            Self::MissingProvinces => "states.provinces.missing",
            Self::EmptyProvinces => "states.provinces.empty",
            Self::DuplicateProvinceInState => "states.province.duplicate",
            Self::ProvinceInMultipleStates => "cross.province.multiple_states",
            Self::UnknownProvince => "cross.province.unknown",
            Self::LandProvinceWithoutState => "cross.land.unassigned",
            Self::MissingStateCategory => "states.category.missing",
            Self::InvalidManpower => "states.manpower.invalid",
            Self::InvalidResource => "states.resource.invalid",
            Self::InvalidBuilding => "states.building.invalid",
            Self::InvalidField => "states.field.invalid",
            Self::MissingStateName => "states.name.missing",
            Self::MissingOwner => "states.owner.missing",
            Self::MissingHistory => "states.history.missing",
            Self::StateNameIdMismatch => "states.name_id.mismatch",
            Self::ProvinceBuildingOutsideState => "cross.building.outside_state",
            Self::VictoryPointOutsideState => "cross.victory_point.outside_state",
            Self::StateEditSession => "states.edit.session",
            Self::InvalidProvinceBitmap => "province.bitmap.invalid",
            Self::InvalidDefinition => "definition.invalid",
            Self::DuplicateProvinceId => "definition.id.duplicate",
            Self::DuplicateProvinceRgb => "definition.rgb.duplicate",
            Self::MissingDefinitionForColor => "cross.bitmap_color.missing_definition",
            Self::UnusedDefinition => "cross.definition.unused",
            Self::InvalidProvinceType => "definition.type.invalid",
            Self::InvalidCoastal => "definition.coastal.invalid",
            Self::InvalidContinent => "definition.continent.invalid",
            Self::SeaOrLakeAssigned => "cross.non_land.assigned",
            Self::NavalBaseNonCoastal => "cross.naval_base.non_coastal",
            Self::CandidateMismatch => "project.candidate.mismatch",
            Self::ExternalChange => "transaction.external_change",
            Self::TransactionFailure => "transaction.failure",
        }
    }

    pub fn domain(self) -> DiagnosticDomain {
        match self {
            Self::InvalidProvinceBitmap => DiagnosticDomain::ProvinceMap,
            Self::InvalidDefinition
            | Self::DuplicateProvinceId
            | Self::DuplicateProvinceRgb
            | Self::InvalidProvinceType
            | Self::InvalidCoastal
            | Self::InvalidContinent => DiagnosticDomain::Definition,
            Self::ProvinceInMultipleStates
            | Self::UnknownProvince
            | Self::LandProvinceWithoutState
            | Self::ProvinceBuildingOutsideState
            | Self::VictoryPointOutsideState
            | Self::MissingDefinitionForColor
            | Self::UnusedDefinition
            | Self::SeaOrLakeAssigned
            | Self::NavalBaseNonCoastal => DiagnosticDomain::CrossDomain,
            Self::ExternalChange | Self::TransactionFailure => DiagnosticDomain::Transaction,
            Self::CandidateMismatch => DiagnosticDomain::Project,
            _ => DiagnosticDomain::States,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiagnosticLocation {
    pub file_path: Option<PathBuf>,
    pub related_path: Option<PathBuf>,
    pub byte_span: Option<TextSpan>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub province_id: Option<u32>,
    pub state_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiagnosticSummary {
    pub total: usize,
    pub information: usize,
    pub warnings: usize,
    pub errors: usize,
    pub blocking: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDiagnostic {
    pub kind: ProjectDiagnosticKind,
    pub severity: DiagnosticSeverity,
    pub domain: DiagnosticDomain,
    pub code: String,
    pub message_key: String,
    pub message_args: BTreeMap<String, String>,
    pub path: Option<PathBuf>,
    pub related_path: Option<PathBuf>,
    pub span: Option<TextSpan>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub province_id: Option<u32>,
    pub state_id: Option<u32>,
    pub message: String,
    pub blocks_save: bool,
}

impl ProjectDiagnostic {
    pub fn new(
        kind: ProjectDiagnosticKind,
        severity: DiagnosticSeverity,
        path: impl Into<Option<PathBuf>>,
        span: impl Into<Option<TextSpan>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            severity,
            domain: kind.domain(),
            code: kind.code().to_owned(),
            message_key: format!("diagnostic.{}", kind.code()),
            message_args: BTreeMap::new(),
            path: path.into(),
            related_path: None,
            span: span.into(),
            line: None,
            column: None,
            province_id: None,
            state_id: None,
            message: message.into(),
            blocks_save: severity == DiagnosticSeverity::Error,
        }
    }

    pub fn with_domain(mut self, domain: DiagnosticDomain) -> Self {
        self.domain = domain;
        self
    }

    pub fn with_province(mut self, province_id: u32) -> Self {
        self.province_id = Some(province_id);
        self
    }

    pub fn with_state(mut self, state_id: u32) -> Self {
        self.state_id = Some(state_id);
        self
    }

    pub fn with_line_column(mut self, line: usize, column: usize) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    pub fn with_blocks_save(mut self, blocks_save: bool) -> Self {
        self.blocks_save = blocks_save;
        self
    }

    pub fn location(&self) -> DiagnosticLocation {
        DiagnosticLocation {
            file_path: self.path.clone(),
            related_path: self.related_path.clone(),
            byte_span: self.span,
            line: self.line,
            column: self.column,
            province_id: self.province_id,
            state_id: self.state_id,
        }
    }

    pub fn sort_and_dedup(diagnostics: &mut Vec<Self>) {
        diagnostics.sort_by(Self::compare);
        diagnostics.dedup_by(|right, left| left.identity() == right.identity());
    }

    pub fn summary(diagnostics: &[Self]) -> DiagnosticSummary {
        let mut summary = DiagnosticSummary {
            total: diagnostics.len(),
            ..Default::default()
        };
        for diagnostic in diagnostics {
            match diagnostic.severity {
                DiagnosticSeverity::Info => summary.information += 1,
                DiagnosticSeverity::Warning => summary.warnings += 1,
                DiagnosticSeverity::Error => summary.errors += 1,
            }
            summary.blocking += usize::from(diagnostic.blocks_save);
        }
        summary
    }

    fn compare(left: &Self, right: &Self) -> Ordering {
        (
            left.severity.rank(),
            left.domain,
            &left.path,
            left.state_id,
            left.province_id,
            &left.code,
            left.span.map(|span| (span.start, span.len)),
        )
            .cmp(&(
                right.severity.rank(),
                right.domain,
                &right.path,
                right.state_id,
                right.province_id,
                &right.code,
                right.span.map(|span| (span.start, span.len)),
            ))
    }

    fn identity(&self) -> DiagnosticIdentity<'_> {
        (
            self.severity,
            self.domain,
            &self.code,
            &self.path,
            self.state_id,
            self.province_id,
            self.span.map(|span| (span.start, span.len)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_sorted_deduplicated_and_summarized_by_stable_identity() {
        let warning = ProjectDiagnostic::new(
            ProjectDiagnosticKind::LandProvinceWithoutState,
            DiagnosticSeverity::Warning,
            None,
            None,
            "warning",
        )
        .with_province(2);
        let duplicate = ProjectDiagnostic {
            message: "translated warning".to_owned(),
            ..warning.clone()
        };
        let error = ProjectDiagnostic::new(
            ProjectDiagnosticKind::UnknownProvince,
            DiagnosticSeverity::Error,
            Some(PathBuf::from("history/states/1.txt")),
            None,
            "error",
        )
        .with_state(1)
        .with_province(9);
        let info = ProjectDiagnostic::new(
            ProjectDiagnosticKind::StateEditSession,
            DiagnosticSeverity::Information,
            None,
            None,
            "info",
        );
        let mut diagnostics = vec![warning, info, duplicate, error];

        ProjectDiagnostic::sort_and_dedup(&mut diagnostics);

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
        assert_eq!(diagnostics[0].code, "cross.province.unknown");
        assert_eq!(diagnostics[0].location().state_id, Some(1));
        assert_eq!(
            ProjectDiagnostic::summary(&diagnostics),
            DiagnosticSummary {
                total: 3,
                information: 1,
                warnings: 1,
                errors: 1,
                blocking: 1
            }
        );
    }
}
