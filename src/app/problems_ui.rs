//! Project Problems UI state and read-only action requests.
//!
//! This module turns project validation diagnostics into rows, details, and
//! requests. It deliberately does not know about `Canvas`, camera state,
//! selection, or platform source handling; those side effects remain at the
//! application boundary.

use std::path::PathBuf;

use crate::app::project::{
    DiagnosticAction, DiagnosticSeverity, ProjectValidationChange, ProjectValidationDiagnostic,
    ProjectValidationDomain, ProjectValidationReport, SourceGeneration, diagnostic_actions,
};
use crate::localization::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ValidationSeverityFilter {
    #[default]
    All,
    Errors,
    Warnings,
    Information,
}

impl ValidationSeverityFilter {
    pub(crate) fn cycle(self) -> Self {
        match self {
            Self::All => Self::Errors,
            Self::Errors => Self::Warnings,
            Self::Warnings => Self::Information,
            Self::Information => Self::All,
        }
    }

    fn matches(self, severity: DiagnosticSeverity) -> bool {
        match self {
            Self::All => true,
            Self::Errors => severity == DiagnosticSeverity::Error,
            Self::Warnings => severity == DiagnosticSeverity::Warning,
            Self::Information => severity == DiagnosticSeverity::Information,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        tr(match self {
            Self::All => "project_validation.all",
            Self::Errors => "project_validation.errors",
            Self::Warnings => "project_validation.warnings",
            Self::Information => "project_validation.information",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ValidationSourceFilter {
    #[default]
    All,
    New,
    Aggravated,
    Unchanged,
    Resolved,
    Improved,
}

impl ValidationSourceFilter {
    pub(crate) fn cycle(self) -> Self {
        match self {
            Self::All => Self::New,
            Self::New => Self::Aggravated,
            Self::Aggravated => Self::Unchanged,
            Self::Unchanged => Self::Resolved,
            Self::Resolved => Self::Improved,
            Self::Improved => Self::All,
        }
    }

    fn matches(self, source: Self) -> bool {
        self == Self::All || self == source
    }

    pub(crate) fn label(self) -> &'static str {
        tr(match self {
            Self::All => "project_validation.all",
            Self::New => "project_validation.new",
            Self::Aggravated => "project_validation.aggravated",
            Self::Unchanged => "project_validation.unchanged",
            Self::Resolved => "project_validation.resolved",
            Self::Improved => "project_validation.improved",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ValidationDomainFilter {
    #[default]
    All,
    ProvinceMap,
    Definition,
    States,
    CrossDomain,
}

impl ValidationDomainFilter {
    pub(crate) fn cycle(self) -> Self {
        match self {
            Self::All => Self::ProvinceMap,
            Self::ProvinceMap => Self::Definition,
            Self::Definition => Self::States,
            Self::States => Self::CrossDomain,
            Self::CrossDomain => Self::All,
        }
    }

    fn matches(self, domain: ProjectValidationDomain) -> bool {
        match self {
            Self::All => true,
            Self::ProvinceMap => domain == ProjectValidationDomain::Province,
            Self::Definition => domain == ProjectValidationDomain::Definition,
            Self::States => matches!(
                domain,
                ProjectValidationDomain::State
                    | ProjectValidationDomain::Syntax
                    | ProjectValidationDomain::Resource
                    | ProjectValidationDomain::Building
            ),
            Self::CrossDomain => domain == ProjectValidationDomain::CrossDomain,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => tr("project_validation.all"),
            Self::ProvinceMap => "Province Map",
            Self::Definition => "Definition",
            Self::States => "States",
            Self::CrossDomain => "Cross Domain",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProblemsUiState {
    pub(crate) severity: ValidationSeverityFilter,
    pub(crate) source: ValidationSourceFilter,
    pub(crate) domain: ValidationDomainFilter,
    pub(crate) selected: usize,
    pub(crate) offset: usize,
    pub(crate) filters_expanded: bool,
    pub(crate) show_technical_details: bool,
    pub(crate) blocking_only: bool,
    pub(crate) action_index: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectProblemsController {
    pub(crate) state: ProblemsUiState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProblemsRequest {
    SelectProvince(u32),
    SelectState(u32),
    FocusLocation([u32; 2]),
    OpenSource(PathBuf),
    RevealSource(PathBuf),
    CopyDetails(String),
}

impl From<DiagnosticAction> for ProblemsRequest {
    fn from(action: DiagnosticAction) -> Self {
        match action {
            DiagnosticAction::GoToProvince(id) => Self::SelectProvince(id),
            DiagnosticAction::GoToState(id) => Self::SelectState(id),
            DiagnosticAction::GoToLocation(location) => Self::FocusLocation(location),
            DiagnosticAction::OpenSource(path) => Self::OpenSource(path),
            DiagnosticAction::RevealSource(path) => Self::RevealSource(path),
            DiagnosticAction::CopySourcePath(path) => Self::CopyDetails(path),
        }
    }
}

impl ProjectProblemsController {
    pub(crate) fn reset(&mut self) {
        self.state = ProblemsUiState::default();
    }

    pub(crate) fn open(&mut self, blocking_only: bool, source: ValidationSourceFilter) {
        self.state = ProblemsUiState {
            blocking_only,
            source,
            ..ProblemsUiState::default()
        };
    }

    pub(crate) fn visible<'a>(
        &self,
        report: Option<&'a ProjectValidationReport>,
    ) -> Vec<(ValidationSourceFilter, &'a ProjectValidationDiagnostic)> {
        report
            .map(validation_delta_items)
            .unwrap_or_default()
            .into_iter()
            .filter(|(source, diagnostic)| {
                validation_problem_matches(&self.state, *source, diagnostic)
            })
            .collect()
    }

    pub(crate) fn selected<'a>(
        &self,
        report: Option<&'a ProjectValidationReport>,
    ) -> Option<(ValidationSourceFilter, &'a ProjectValidationDiagnostic)> {
        let problems = self.visible(report);
        problems
            .get(self.state.selected.min(problems.len().saturating_sub(1)))
            .copied()
    }

    pub(crate) fn selected_requests(
        &self,
        report: Option<&ProjectValidationReport>,
        generation: SourceGeneration,
    ) -> Vec<ProblemsRequest> {
        self.selected(report)
            .map(|(_, diagnostic)| {
                diagnostic_actions(diagnostic, generation)
                    .into_iter()
                    .map(ProblemsRequest::from)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn selected_request(
        &self,
        report: Option<&ProjectValidationReport>,
        generation: SourceGeneration,
    ) -> Option<ProblemsRequest> {
        let requests = self.selected_requests(report, generation);
        requests
            .get(self.state.action_index % requests.len().max(1))
            .cloned()
    }

    pub(crate) fn next_action_or_details(
        &mut self,
        report: Option<&ProjectValidationReport>,
        generation: SourceGeneration,
    ) {
        let request_count = self.selected_requests(report, generation).len();
        if request_count > 1 {
            self.state.action_index = (self.state.action_index + 1) % request_count;
        } else {
            self.state.show_technical_details = !self.state.show_technical_details;
        }
    }

    pub(crate) fn cycle_severity(&mut self) {
        self.state.severity = self.state.severity.cycle();
        self.reset_selection();
    }

    pub(crate) fn cycle_source(&mut self) {
        self.state.source = self.state.source.cycle();
        self.reset_selection();
    }

    pub(crate) fn cycle_domain(&mut self) {
        self.state.domain = self.state.domain.cycle();
        self.reset_selection();
    }

    fn reset_selection(&mut self) {
        self.state.selected = 0;
        self.state.offset = 0;
        self.state.action_index = 0;
    }
}

pub(crate) fn validation_problem_summary(
    source: &ValidationSourceFilter,
    diagnostic: &ProjectValidationDiagnostic,
) -> String {
    let severity = match diagnostic.severity {
        DiagnosticSeverity::Information => tr("project_validation.severity_info"),
        DiagnosticSeverity::Warning => tr("project_validation.severity_warning"),
        DiagnosticSeverity::Error => tr("project_validation.severity_error"),
    };
    let mut context = vec![severity.to_owned(), source.label().to_owned()];
    if let Some(id) = diagnostic.province_id {
        context.push(format!("Province {id}"));
    }
    if let Some(id) = diagnostic.state_id {
        context.push(format!("State {id}"));
    }
    if diagnostic.blocks_save {
        context.push("Blocks Save".to_owned());
    }
    format!("{} — {}", context.join(" · "), diagnostic.message)
}

pub(crate) fn validation_problem_details(
    source: ValidationSourceFilter,
    diagnostic: &ProjectValidationDiagnostic,
) -> String {
    format!(
        "Source: {}\nSeverity: {:?}\nBlocks Save: {}\nDomain: {:?}\nCode: {}\nMessage: {}\nPath: {}\nProvince: {}\nRelated Provinces: {}\nState: {}\nMap coordinate: {}\nResolved source: {}",
        source.label(),
        diagnostic.severity,
        if diagnostic.blocks_save { "Yes" } else { "No" },
        diagnostic.domain,
        diagnostic.code,
        diagnostic.message,
        diagnostic
            .path
            .as_ref()
            .map_or_else(|| "-".to_owned(), |path| path.display().to_string()),
        diagnostic
            .province_id
            .map_or_else(|| "-".to_owned(), |id| id.to_string()),
        if diagnostic.related_province_ids.is_empty() {
            "-".to_owned()
        } else {
            diagnostic
                .related_province_ids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        },
        diagnostic
            .state_id
            .map_or_else(|| "-".to_owned(), |id| id.to_string()),
        diagnostic
            .map_location
            .map_or_else(|| "-".to_owned(), |[x, y]| format!("{x},{y}")),
        diagnostic.source.as_ref().map_or_else(
            || "-".to_owned(),
            |source| format!(
                "{} ({:?}, {:?})",
                source.logical_path.display(),
                source.source_kind,
                source.location
            )
        ),
    )
}

pub(crate) fn problems_request_label(request: &ProblemsRequest) -> &'static str {
    match request {
        ProblemsRequest::SelectProvince(_) => tr("project_validation.go_to_province"),
        ProblemsRequest::SelectState(_) => tr("project_validation.go_to_state"),
        ProblemsRequest::FocusLocation(_) => tr("project_validation.go_to_location"),
        ProblemsRequest::OpenSource(_) => tr("project_validation.open_source_file"),
        ProblemsRequest::RevealSource(_) => tr("project_validation.reveal_source"),
        ProblemsRequest::CopyDetails(_) => tr("project_validation.copy_source_path"),
    }
}

pub(crate) fn project_validation_blockers(
    report: Option<&ProjectValidationReport>,
) -> Vec<(ValidationSourceFilter, &ProjectValidationDiagnostic)> {
    report
        .map(validation_delta_items)
        .unwrap_or_default()
        .into_iter()
        .filter(|(source, diagnostic)| {
            diagnostic.severity == DiagnosticSeverity::Error
                && matches!(
                    source,
                    ValidationSourceFilter::New | ValidationSourceFilter::Aggravated
                )
        })
        .collect()
}

fn validation_delta_items(
    report: &ProjectValidationReport,
) -> Vec<(ValidationSourceFilter, &ProjectValidationDiagnostic)> {
    fn append<'a>(
        output: &mut Vec<(ValidationSourceFilter, &'a ProjectValidationDiagnostic)>,
        source: ValidationSourceFilter,
        changes: &'a [ProjectValidationChange],
        use_before: bool,
    ) {
        output.extend(changes.iter().filter_map(|change| {
            let diagnostic = if use_before {
                change.before.as_ref()
            } else {
                change.after.as_ref().or(change.before.as_ref())
            }?;
            Some((source, diagnostic))
        }));
    }

    let mut output = Vec::with_capacity(report.diagnostics.len());
    append(
        &mut output,
        ValidationSourceFilter::New,
        &report.delta.new,
        false,
    );
    append(
        &mut output,
        ValidationSourceFilter::Aggravated,
        &report.delta.aggravated,
        false,
    );
    append(
        &mut output,
        ValidationSourceFilter::Unchanged,
        &report.delta.unchanged,
        false,
    );
    append(
        &mut output,
        ValidationSourceFilter::Resolved,
        &report.delta.resolved,
        true,
    );
    append(
        &mut output,
        ValidationSourceFilter::Improved,
        &report.delta.improved,
        false,
    );
    output.sort_by_key(|(_, diagnostic)| match diagnostic.severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Information => 2,
    });
    output
}

fn validation_problem_matches(
    state: &ProblemsUiState,
    source: ValidationSourceFilter,
    diagnostic: &ProjectValidationDiagnostic,
) -> bool {
    source != ValidationSourceFilter::Resolved
        && state.source.matches(source)
        && state.severity.matches(diagnostic.severity)
        && state.domain.matches(diagnostic.domain)
        && (!state.blocking_only
            || diagnostic.blocks_save
                && matches!(
                    source,
                    ValidationSourceFilter::New | ValidationSourceFilter::Aggravated
                ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::project::{
        ProjectDiagnosticKind, ResolvedLocation, ResolvedSource, SourceKind,
    };

    fn diagnostic() -> ProjectValidationDiagnostic {
        ProjectValidationDiagnostic {
            kind: ProjectDiagnosticKind::UnknownProvince,
            severity: DiagnosticSeverity::Error,
            domain: ProjectValidationDomain::State,
            code: "unknown-province".to_owned(),
            message_key: "unknown-province".to_owned(),
            path: None,
            related_path: None,
            span: None,
            province_id: Some(501),
            state_id: Some(123),
            strategic_region_ids: Vec::new(),
            map_location: Some([7, 3]),
            related_province_ids: vec![42],
            source: None,
            blocks_save: true,
            message: "Province reference is invalid".to_owned(),
        }
    }

    #[test]
    fn request_conversion_keeps_navigation_and_source_shapes() {
        assert_eq!(
            ProblemsRequest::from(DiagnosticAction::GoToProvince(7)),
            ProblemsRequest::SelectProvince(7)
        );
        assert_eq!(
            ProblemsRequest::from(DiagnosticAction::GoToState(4)),
            ProblemsRequest::SelectState(4)
        );
        assert_eq!(
            ProblemsRequest::from(DiagnosticAction::GoToLocation([2, 3])),
            ProblemsRequest::FocusLocation([2, 3])
        );
    }

    #[test]
    fn archive_source_request_never_invents_an_open_action() {
        let mut value = diagnostic();
        value.source = Some(ResolvedSource {
            logical_path: "map/rivers.bmp".into(),
            location: ResolvedLocation::ArchiveEntry {
                archive_path: "dlc.zip".into(),
                entry_path: "map/rivers.bmp".into(),
            },
            source_kind: SourceKind::Dlc,
            project_generation: SourceGeneration::new(1),
        });
        let requests = diagnostic_actions(&value, SourceGeneration::new(1))
            .into_iter()
            .map(ProblemsRequest::from)
            .collect::<Vec<_>>();
        assert!(
            !requests
                .iter()
                .any(|request| matches!(request, ProblemsRequest::OpenSource(_)))
        );
        assert!(
            requests
                .iter()
                .any(|request| matches!(request, ProblemsRequest::RevealSource(_)))
        );
    }

    #[test]
    fn filters_keep_current_issues_and_blocking_scope() {
        let state = ProblemsUiState::default();
        let value = diagnostic();
        assert!(validation_problem_matches(
            &state,
            ValidationSourceFilter::Unchanged,
            &value
        ));
        assert!(!validation_problem_matches(
            &state,
            ValidationSourceFilter::Resolved,
            &value
        ));
        let blocking = ProblemsUiState {
            blocking_only: true,
            ..ProblemsUiState::default()
        };
        assert!(validation_problem_matches(
            &blocking,
            ValidationSourceFilter::New,
            &value
        ));
        assert!(!validation_problem_matches(
            &blocking,
            ValidationSourceFilter::Unchanged,
            &value
        ));
    }

    #[test]
    fn summary_keeps_severity_source_and_navigation_context() {
        let summary = validation_problem_summary(&ValidationSourceFilter::New, &diagnostic());
        assert!(summary.contains("Province 501"));
        assert!(summary.contains("State 123"));
        assert!(summary.contains("Province reference is invalid"));
    }

    #[test]
    fn reset_clears_selection_and_filters() {
        let mut controller = ProjectProblemsController::default();
        controller.state.filters_expanded = true;
        controller.state.selected = 4;
        controller.open(true, ValidationSourceFilter::Aggravated);
        assert!(controller.state.blocking_only);
        assert_eq!(controller.state.source, ValidationSourceFilter::Aggravated);
        controller.reset();
        assert_eq!(controller.state.selected, 0);
        assert_eq!(controller.state.source, ValidationSourceFilter::All);
        assert!(!controller.state.blocking_only);
    }
}
