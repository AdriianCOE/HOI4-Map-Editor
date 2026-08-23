//! Read-only Save Project review models and UI action orchestration.
//!
//! The project save engine retains candidate construction, validation,
//! freshness checks, transactions, rollback, and recovery. This module only
//! translates those authoritative results into dialog presentation and
//! explicit requests for the Canvas/App boundary to fulfill.

use crate::app::problems_ui::{project_validation_blockers, validation_problem_summary};
use crate::app::project::{
    ProjectGeneration, ProjectSavePlan, ProjectValidationReport, RoundTripStatus, StateSaveOutcome,
    StateSaveReport,
};
use crate::localization::{tr, tr_args};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProjectSavePresentationSummary {
    pub(crate) province_files: usize,
    pub(crate) state_files: usize,
    pub(crate) coastal_flags_recalculated: usize,
}

impl ProjectSavePresentationSummary {
    pub(crate) fn from_plan(plan: &ProjectSavePlan) -> Self {
        let dirty = plan.dirty();
        Self {
            province_files: dirty.province_files,
            state_files: dirty.state_files,
            coastal_flags_recalculated: plan.coastal_flags_recalculated(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SaveUiRequest {
    CommitPreparedProject,
    ViewBlockingProblems,
    ViewIntegrityProblem,
    ViewExistingProblems,
    None,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SaveUiController {
    generation: Option<ProjectGeneration>,
    commit_requested: bool,
}

impl SaveUiController {
    pub(crate) fn reset_for_generation(&mut self, generation: ProjectGeneration) {
        self.generation = Some(generation);
        self.commit_requested = false;
    }

    pub(crate) fn reset_commit_request(&mut self) {
        self.commit_requested = false;
    }

    pub(crate) fn request_primary(&mut self, model: &SaveReviewModel) -> SaveUiRequest {
        match model.primary_request() {
            SaveUiRequest::CommitPreparedProject if self.commit_requested => SaveUiRequest::None,
            SaveUiRequest::CommitPreparedProject => {
                self.commit_requested = true;
                SaveUiRequest::CommitPreparedProject
            }
            request => request,
        }
    }

    pub(crate) fn request_secondary(&self, model: &SaveReviewModel) -> SaveUiRequest {
        model.secondary_request()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SaveReviewModel {
    province_files: usize,
    state_files: usize,
    files: usize,
    coastal_flags_recalculated: usize,
    validation_blocked: bool,
    round_trip_failed: bool,
    blocking_count: usize,
    blocking_lines: Vec<String>,
    integrity_summary: Option<String>,
    baseline_errors: Option<usize>,
    baseline_warnings: Option<usize>,
}

impl SaveReviewModel {
    pub(crate) fn from_engine(
        plan: Option<&ProjectSavePlan>,
        report: Option<&ProjectValidationReport>,
        round_trip_status: Option<RoundTripStatus>,
        integrity_summary: Option<&str>,
    ) -> Self {
        let dirty = plan.map(ProjectSavePlan::dirty).unwrap_or_default();
        let validation_blocked = report.is_some_and(|report| report.delta.blocks_save());
        let round_trip_failed = !matches!(
            round_trip_status,
            Some(RoundTripStatus::Passed | RoundTripStatus::PassedWithReview)
        );
        let blocking = project_validation_blockers(report);
        let blocking_count = blocking.len();
        let blocking_lines = blocking
            .into_iter()
            .take(3)
            .map(|(source, diagnostic)| validation_problem_summary(&source, diagnostic))
            .collect();
        let baseline = report.and_then(|report| report.baseline_summary.as_ref());
        Self {
            province_files: dirty.province_files,
            state_files: dirty.state_files,
            files: plan.map_or(0, |plan| plan.patch_plan().files_len()),
            coastal_flags_recalculated: plan.map_or(0, ProjectSavePlan::coastal_flags_recalculated),
            validation_blocked,
            round_trip_failed,
            blocking_count,
            blocking_lines,
            integrity_summary: integrity_summary.map(str::to_owned),
            baseline_errors: baseline.map(|summary| summary.errors),
            baseline_warnings: baseline.map(|summary| summary.warnings),
        }
    }

    pub(crate) fn primary_request(&self) -> SaveUiRequest {
        if self.validation_blocked {
            SaveUiRequest::ViewBlockingProblems
        } else if self.round_trip_failed {
            SaveUiRequest::ViewIntegrityProblem
        } else {
            SaveUiRequest::CommitPreparedProject
        }
    }

    pub(crate) fn secondary_request(&self) -> SaveUiRequest {
        SaveUiRequest::ViewExistingProblems
    }

    pub(crate) fn presentation(&self) -> SaveDialogPresentation {
        let blocked = self.validation_blocked || self.round_trip_failed;
        let mut lines = vec![
            tr("project_validation.changes").to_owned(),
            tr_args(
                "project_validation.map_provinces_files",
                &[("count", &self.province_files.to_string())],
            ),
            tr_args(
                "project_validation.states_files",
                &[("count", &self.state_files.to_string())],
            ),
            if self.coastal_flags_recalculated != 0 {
                tr_args(
                    "project_validation.automatic_coastal",
                    &[("count", &self.coastal_flags_recalculated.to_string())],
                )
            } else {
                String::new()
            },
            tr_args(
                "project_validation.files_will_update",
                &[("count", &self.files.to_string())],
            ),
            tr("project_validation.validation").to_owned(),
        ];
        if blocked {
            if !self.blocking_lines.is_empty() {
                lines.push(tr_args(
                    "project_validation.blocking_problems_count",
                    &[("count", &self.blocking_count.to_string())],
                ));
                lines.extend(self.blocking_lines.iter().cloned());
            } else if self.round_trip_failed {
                lines.push("Round-trip verification failed.".to_owned());
                lines.push(self.integrity_summary.clone().unwrap_or_else(|| {
                    tr("project_validation.round_trip_save_blocked").to_owned()
                }));
            }
        } else {
            lines.push(tr("project_validation.no_new_blockers").to_owned());
        }
        lines.push(match (self.baseline_errors, self.baseline_warnings) {
            (Some(errors), Some(warnings)) => tr_args(
                "project_validation.existing_project_issues",
                &[
                    ("errors", &errors.to_string()),
                    ("warnings", &warnings.to_string()),
                ],
            ),
            _ => tr("project_validation.current_project_issues_unclassified").to_owned(),
        });
        SaveDialogPresentation {
            title: if blocked {
                tr("project_validation.save_blocked")
            } else {
                tr("project_validation.ready_to_save")
            },
            primary: match self.primary_request() {
                SaveUiRequest::ViewBlockingProblems => {
                    tr("project_validation.view_blocking_problems")
                }
                SaveUiRequest::ViewIntegrityProblem => "View Integrity Problem",
                SaveUiRequest::CommitPreparedProject => tr("workspace.save_project"),
                _ => "",
            },
            secondary: if blocked {
                tr("project_validation.view_existing_issues")
            } else {
                tr("project_validation.view_problems")
            },
            close: tr("project_validation.close"),
            lines,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SaveDialogPresentation {
    pub(crate) title: &'static str,
    pub(crate) primary: &'static str,
    pub(crate) secondary: &'static str,
    pub(crate) close: &'static str,
    pub(crate) lines: Vec<String>,
}

pub(crate) fn integrity_problem_presentation(details: Option<&str>) -> SaveDialogPresentation {
    let details = details.unwrap_or(
        "Round-trip verification failed before a detailed report could be retained. Prepare Save Project again to capture a fresh report.",
    );
    SaveDialogPresentation {
        title: "ROUND-TRIP INTEGRITY PROBLEM",
        primary: "Close",
        secondary: "Copy Details",
        close: "",
        lines: details.lines().map(str::to_owned).collect(),
    }
}

pub(crate) fn progress_presentation(
    validation_running: bool,
    state_save_running: bool,
    status: Option<&str>,
) -> SaveDialogPresentation {
    SaveDialogPresentation {
        title: if validation_running {
            "VALIDATING CHANGES"
        } else {
            "APPLYING CHANGES"
        },
        primary: "Cancel Safely",
        secondary: "View Details",
        close: "",
        lines: vec![
            "[x] Preparing patch".to_owned(),
            "[x] Checking current files".to_owned(),
            if validation_running {
                "[>] Validating temporary copy".to_owned()
            } else {
                "[x] Validating temporary copy".to_owned()
            },
            if state_save_running {
                "[>] Backup, apply, reload and verification".to_owned()
            } else {
                "[ ] Backup, apply, reload and verification".to_owned()
            },
            String::new(),
            status.unwrap_or("Preparing...").to_owned(),
        ],
    }
}

pub(crate) fn result_presentation(
    report: Option<&StateSaveReport>,
    project_summary: Option<ProjectSavePresentationSummary>,
    round_trip_status: Option<&str>,
) -> SaveDialogPresentation {
    let (title, lines) = if let Some(report) = report {
        if report.outcome == StateSaveOutcome::Completed {
            let mut lines = vec![tr("project_validation.changes").to_owned()];
            if let Some(summary) = project_summary {
                if summary.province_files != 0 {
                    lines.push(tr_args(
                        "project_validation.map_provinces_files",
                        &[("count", &summary.province_files.to_string())],
                    ));
                }
                if summary.state_files != 0 {
                    lines.push(tr_args(
                        "project_validation.states_files",
                        &[("count", &summary.state_files.to_string())],
                    ));
                }
                if summary.coastal_flags_recalculated != 0 {
                    lines.push(tr_args(
                        "project_validation.coastal_flags_recalculated",
                        &[("count", &summary.coastal_flags_recalculated.to_string())],
                    ));
                }
            }
            lines.extend([
                tr_args(
                    "project_validation.files_updated",
                    &[(
                        "count",
                        &(report.modified_files + report.created_files + report.removed_files)
                            .to_string(),
                    )],
                ),
                tr("project_validation.safety").to_owned(),
                tr("project_validation.validation_passed").to_owned(),
                if report.backup_path.is_some() {
                    tr("project_validation.backup_created").to_owned()
                } else {
                    tr("project_validation.backup_status_unavailable").to_owned()
                },
                tr("project_validation.round_trip_verified").to_owned(),
            ]);
            (tr("project_validation.project_saved"), lines)
        } else if report.outcome == StateSaveOutcome::RolledBack {
            (
                tr("project_validation.save_failed_restored"),
                vec![
                    report
                        .error
                        .clone()
                        .unwrap_or_else(|| tr("project_validation.commit_failure").to_owned()),
                    tr("project_validation.original_files_restored").to_owned(),
                    tr("project_validation.no_partial_changes").to_owned(),
                ],
            )
        } else {
            (
                tr("project_validation.save_blocked"),
                vec![
                    report
                        .error
                        .clone()
                        .unwrap_or_else(|| report.state.label().to_owned()),
                    tr("project_validation.no_changes_committed").to_owned(),
                ],
            )
        }
    } else {
        (
            tr("project_validation.validation_result"),
            vec![
                round_trip_status
                    .unwrap_or(tr("project_validation.validation_incomplete"))
                    .to_owned(),
            ],
        )
    };
    SaveDialogPresentation {
        title,
        primary: "Done",
        secondary: "View Report",
        close: "Close",
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(validation_blocked: bool, round_trip_failed: bool) -> SaveReviewModel {
        SaveReviewModel {
            province_files: 1,
            state_files: 2,
            files: 3,
            coastal_flags_recalculated: 0,
            validation_blocked,
            round_trip_failed,
            blocking_count: 0,
            blocking_lines: Vec::new(),
            integrity_summary: None,
            baseline_errors: None,
            baseline_warnings: None,
        }
    }

    #[test]
    fn primary_action_preserves_blocker_integrity_and_ready_states() {
        assert_eq!(
            model(true, false).primary_request(),
            SaveUiRequest::ViewBlockingProblems
        );
        assert_eq!(
            model(false, true).primary_request(),
            SaveUiRequest::ViewIntegrityProblem
        );
        assert_eq!(
            model(false, false).primary_request(),
            SaveUiRequest::CommitPreparedProject
        );
    }

    #[test]
    fn commit_request_is_idempotent_until_the_engine_resets_it() {
        let mut controller = SaveUiController::default();
        let model = model(false, false);
        assert_eq!(
            controller.request_primary(&model),
            SaveUiRequest::CommitPreparedProject
        );
        assert_eq!(controller.request_primary(&model), SaveUiRequest::None);
        controller.reset_commit_request();
        assert_eq!(
            controller.request_primary(&model),
            SaveUiRequest::CommitPreparedProject
        );
    }

    #[test]
    fn project_replacement_resets_pending_commit_state() {
        let mut controller = SaveUiController::default();
        let model = model(false, false);
        let _ = controller.request_primary(&model);
        controller.reset_for_generation(ProjectGeneration(2));
        assert_eq!(
            controller.request_primary(&model),
            SaveUiRequest::CommitPreparedProject
        );
    }
}
