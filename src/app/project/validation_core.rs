use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::app::map::{Bundle, Color, ProvinceKind};
use crate::app::state::TextSpan;

use super::diagnostics::DiagnosticDomain;
use super::{DiagnosticSeverity, Hoi4Project, ProjectDiagnostic, ProjectDiagnosticKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectValidationTarget {
    CurrentProject,
    PendingChanges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProjectValidationDomain {
    Project,
    File,
    Syntax,
    State,
    Province,
    Resource,
    Building,
    CrossDomain,
    Session,
    Transaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectValidationSummary {
    pub total: usize,
    pub information: usize,
    pub warnings: usize,
    pub errors: usize,
    pub blocks_save: usize,
    pub domains: BTreeMap<ProjectValidationDomain, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectValidationDiagnostic {
    pub kind: ProjectDiagnosticKind,
    pub severity: DiagnosticSeverity,
    pub domain: ProjectValidationDomain,
    pub code: String,
    pub message_key: String,
    pub path: Option<PathBuf>,
    pub related_path: Option<PathBuf>,
    pub span: Option<TextSpan>,
    pub province_id: Option<u32>,
    pub state_id: Option<u32>,
    pub blocks_save: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectValidationReport {
    pub target: ProjectValidationTarget,
    pub diagnostics: Vec<ProjectValidationDiagnostic>,
    pub summary: ProjectValidationSummary,
    pub information: usize,
    pub warnings: usize,
    pub errors: usize,
    pub total: usize,
    pub blocks_save: bool,
    pub requires_warning_review: bool,
}

pub fn validate_project(
    bundle: &Bundle,
    project: &Hoi4Project,
    target: ProjectValidationTarget,
) -> ProjectValidationReport {
    let mut diagnostics = project
        .diagnostics
        .iter()
        .map(ProjectValidationDiagnostic::from_project)
        .collect::<Vec<_>>();

    validate_provinces(bundle, project, &mut diagnostics);
    validate_states(bundle, project, &mut diagnostics);
    sort_and_dedup(&mut diagnostics);

    let summary = summarize(&diagnostics);
    ProjectValidationReport {
        target,
        information: summary.information,
        warnings: summary.warnings,
        errors: summary.errors,
        total: summary.total,
        blocks_save: summary.blocks_save > 0,
        requires_warning_review: summary.warnings > 0,
        diagnostics,
        summary,
    }
}

impl ProjectValidationDiagnostic {
    fn from_project(diagnostic: &ProjectDiagnostic) -> Self {
        Self {
            kind: diagnostic.kind,
            severity: diagnostic.severity,
            domain: ProjectValidationDomain::from_project(diagnostic.domain),
            code: diagnostic.code.clone(),
            message_key: diagnostic.message_key.clone(),
            path: diagnostic.path.clone(),
            related_path: diagnostic.related_path.clone(),
            span: diagnostic.span,
            province_id: diagnostic
                .province_id
                .or_else(|| extract_message_id(&diagnostic.message, "province")),
            state_id: diagnostic
                .state_id
                .or_else(|| extract_message_id(&diagnostic.message, "state")),
            blocks_save: diagnostic.blocks_save,
            message: diagnostic.message.clone(),
        }
    }

    fn custom(
        kind: ProjectDiagnosticKind,
        severity: DiagnosticSeverity,
        path: Option<PathBuf>,
        message: String,
    ) -> Self {
        Self::from_project(&ProjectDiagnostic::new(kind, severity, path, None, message))
    }

    fn with_domain(mut self, domain: ProjectValidationDomain) -> Self {
        self.domain = domain;
        self
    }

    fn with_province_id(mut self, province_id: u32) -> Self {
        self.province_id = Some(province_id);
        self
    }

    fn with_state_id(mut self, state_id: Option<u32>) -> Self {
        self.state_id = state_id;
        self
    }
}

impl ProjectValidationDomain {
    fn from_project(domain: DiagnosticDomain) -> Self {
        match domain {
            DiagnosticDomain::Project => Self::Project,
            DiagnosticDomain::ProvinceMap | DiagnosticDomain::Definition => Self::Province,
            DiagnosticDomain::States => Self::State,
            DiagnosticDomain::CrossDomain => Self::CrossDomain,
            DiagnosticDomain::Transaction => Self::Transaction,
        }
    }
}

fn validate_provinces(
    bundle: &Bundle,
    project: &Hoi4Project,
    diagnostics: &mut Vec<ProjectValidationDiagnostic>,
) {
    let mut ids = BTreeMap::<u32, Color>::new();
    for (color, province) in sorted_provinces(bundle) {
        let province_id = province.preserved_id;
        let province_label = province_id.map_or_else(
            || format!("province color {}", color_text(color)),
            |province_id| format!("province {province_id}"),
        );
        if province_id.is_none() {
            diagnostics.push(province_error(
                project,
                ProjectDiagnosticKind::MissingDefinitionForColor,
                None,
                format!("province color {} has no province id", color_text(color)),
            ));
        }

        if let Some(province_id) = province_id {
            if province_id == 0 {
                diagnostics.push(province_error(
                    project,
                    ProjectDiagnosticKind::InvalidDefinition,
                    Some(province_id),
                    format!("province color {} uses id 0", color_text(color)),
                ));
            } else if let Some(first_color) = ids.insert(province_id, color) {
                diagnostics.push(province_error(
                    project,
                    ProjectDiagnosticKind::DuplicateProvinceId,
                    Some(province_id),
                    format!(
                        "province id {province_id} is used by colors {} and {}",
                        color_text(first_color),
                        color_text(color)
                    ),
                ));
            }
        }

        if province.kind == ProvinceKind::Unknown {
            diagnostics.push(province_error(
                project,
                ProjectDiagnosticKind::InvalidProvinceType,
                province_id,
                format!("{province_label} has invalid kind unknown"),
            ));
        }
        if province.coastal.is_none() {
            diagnostics.push(province_error(
                project,
                ProjectDiagnosticKind::InvalidCoastal,
                province_id,
                format!("{province_label} has no coastal value"),
            ));
        }
        if !province.kind.valid_continent_id(province.continent) {
            diagnostics.push(province_error(
                project,
                ProjectDiagnosticKind::InvalidContinent,
                province_id,
                format!(
                    "{province_label} has invalid continent {} for {}",
                    province.continent,
                    province.kind.to_str()
                ),
            ));
        }
        if province.terrain.trim().is_empty() || province.terrain == "unknown" {
            diagnostics.push(province_error(
                project,
                ProjectDiagnosticKind::InvalidDefinition,
                province_id,
                format!("{province_label} has no required terrain"),
            ));
        }
    }
}

fn validate_states(
    bundle: &Bundle,
    project: &Hoi4Project,
    diagnostics: &mut Vec<ProjectValidationDiagnostic>,
) {
    let provinces = province_lookup(bundle);

    for document in &project.states {
        let Some(state) = &document.data else {
            continue;
        };
        let state_id = state.id;

        if state.provinces.is_empty() {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::EmptyProvinces,
                    DiagnosticSeverity::Warning,
                    Some(document.path.clone()),
                    "state has no provinces".to_owned(),
                )
                .with_state_id(state_id),
            );
        }

        for &province_id in &state.provinces {
            if matches!(
                provinces.get(&province_id).map(|(_, kind, _)| kind),
                Some(ProvinceKind::Sea | ProvinceKind::Lake)
            ) {
                diagnostics.push(
                    ProjectValidationDiagnostic::custom(
                        ProjectDiagnosticKind::SeaOrLakeAssigned,
                        DiagnosticSeverity::Error,
                        Some(document.path.clone()),
                        format!("state references non-land province {province_id}"),
                    )
                    .with_domain(ProjectValidationDomain::CrossDomain)
                    .with_province_id(province_id)
                    .with_state_id(state_id),
                );
            }
        }

        for vp in &state.history.victory_points {
            if !state.provinces.contains(&vp.province_id) {
                diagnostics.push(
                    ProjectValidationDiagnostic::custom(
                        ProjectDiagnosticKind::VictoryPointOutsideState,
                        DiagnosticSeverity::Warning,
                        Some(document.path.clone()),
                        format!(
                            "victory point province {} is outside this state",
                            vp.province_id
                        ),
                    )
                    .with_province_id(vp.province_id)
                    .with_state_id(state_id),
                );
            }
        }

        for (&province_id, buildings) in &state.history.province_buildings {
            if !state.provinces.contains(&province_id) {
                diagnostics.push(
                    ProjectValidationDiagnostic::custom(
                        ProjectDiagnosticKind::ProvinceBuildingOutsideState,
                        DiagnosticSeverity::Warning,
                        Some(document.path.clone()),
                        format!("building province {province_id} is outside this state"),
                    )
                    .with_province_id(province_id)
                    .with_state_id(state_id),
                );
            }

            let coastal_land =
                provinces.get(&province_id) == Some(&(true, ProvinceKind::Land, true));
            if buildings.contains_key("naval_base") && !coastal_land {
                diagnostics.push(
                    ProjectValidationDiagnostic::custom(
                        ProjectDiagnosticKind::NavalBaseNonCoastal,
                        DiagnosticSeverity::Error,
                        Some(document.path.clone()),
                        format!("naval_base province {province_id} is not coastal land"),
                    )
                    .with_province_id(province_id)
                    .with_state_id(state_id),
                );
            }
        }
    }
}

fn province_lookup(bundle: &Bundle) -> BTreeMap<u32, (bool, ProvinceKind, bool)> {
    bundle
        .map
        .iter_province_data()
        .filter_map(|(_, province)| {
            province
                .preserved_id
                .map(|id| (id, (true, province.kind, province.coastal == Some(true))))
        })
        .collect()
}

fn sorted_provinces(bundle: &Bundle) -> Vec<(Color, &crate::app::map::ProvinceData)> {
    let mut provinces = bundle.map.iter_province_data().collect::<Vec<_>>();
    provinces.sort_by_key(|(color, province)| (province.preserved_id, *color));
    provinces
}

fn province_error(
    project: &Hoi4Project,
    kind: ProjectDiagnosticKind,
    province_id: Option<u32>,
    message: String,
) -> ProjectValidationDiagnostic {
    let mut diagnostic = ProjectValidationDiagnostic::custom(
        kind,
        DiagnosticSeverity::Error,
        Some(project.paths.definition_csv.clone()),
        message,
    )
    .with_domain(ProjectValidationDomain::Province);
    diagnostic.province_id = province_id;
    diagnostic
}

fn sort_and_dedup(diagnostics: &mut Vec<ProjectValidationDiagnostic>) {
    diagnostics.sort_by(cmp_stable);
    diagnostics.dedup_by(|right, left| same_identity(left, right));
}

fn summarize(diagnostics: &[ProjectValidationDiagnostic]) -> ProjectValidationSummary {
    let mut summary = ProjectValidationSummary::default();
    for diagnostic in diagnostics {
        summary.total += 1;
        *summary.domains.entry(diagnostic.domain).or_default() += 1;
        match diagnostic.severity {
            DiagnosticSeverity::Info => summary.information += 1,
            DiagnosticSeverity::Warning => summary.warnings += 1,
            DiagnosticSeverity::Error => summary.errors += 1,
        }
        if diagnostic.blocks_save {
            summary.blocks_save += 1;
        }
    }
    summary
}

fn same_identity(left: &ProjectValidationDiagnostic, right: &ProjectValidationDiagnostic) -> bool {
    left.kind == right.kind
        && left.severity == right.severity
        && left.domain == right.domain
        && left.code == right.code
        && left.message_key == right.message_key
        && left.path == right.path
        && left.related_path == right.related_path
        && left.span == right.span
        && left.province_id == right.province_id
        && left.state_id == right.state_id
        && left.blocks_save == right.blocks_save
}

fn cmp_stable(
    left: &ProjectValidationDiagnostic,
    right: &ProjectValidationDiagnostic,
) -> std::cmp::Ordering {
    (
        severity_rank(left.severity),
        left.domain,
        left.path.as_ref(),
        left.state_id,
        left.province_id,
        span_key(left.span),
        left.kind,
        left.related_path.as_ref(),
        left.blocks_save,
        left.code.as_str(),
        left.message.as_str(),
    )
        .cmp(&(
            severity_rank(right.severity),
            right.domain,
            right.path.as_ref(),
            right.state_id,
            right.province_id,
            span_key(right.span),
            right.kind,
            right.related_path.as_ref(),
            right.blocks_save,
            right.code.as_str(),
            right.message.as_str(),
        ))
}

fn severity_rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Info => 2,
    }
}

fn span_key(span: Option<TextSpan>) -> Option<(usize, usize)> {
    span.map(|span| (span.start, span.len))
}

fn extract_message_id(message: &str, label: &str) -> Option<u32> {
    let mut words = message.split(|ch: char| !ch.is_ascii_alphanumeric());
    while let Some(word) = words.next() {
        if word.eq_ignore_ascii_case(label) {
            return words.next().and_then(|value| value.parse().ok());
        }
    }
    None
}

fn color_text([r, g, b]: Color) -> String {
    format!("{r},{g},{b}")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use crate::app::map::{Bundle, write_rgb_bmp_image};
    use crate::app::project::ProjectPaths;
    use crate::app::state::{StateData, StateDocument, StateHistory, VictoryPoint, parse_text};
    use crate::config::Config;
    use crate::util::files::Location;

    use super::*;

    struct TempProject(PathBuf);

    impl TempProject {
        fn new(name: &str, definition: &str, pixels: &[[u8; 3]]) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hoi4-validation-core-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("map")).unwrap();
            fs::create_dir_all(root.join("history/states")).unwrap();

            let mut image = image::RgbImage::new(pixels.len() as u32, 1);
            for (x, pixel) in pixels.iter().enumerate() {
                image.put_pixel(x as u32, 0, image::Rgb(*pixel));
            }
            let mut bmp = Vec::new();
            write_rgb_bmp_image(&mut bmp, &image).unwrap();
            fs::write(root.join("map/provinces.bmp"), bmp).unwrap();
            fs::write(root.join("map/definition.csv"), definition).unwrap();
            Self(root)
        }

        fn paths(&self) -> ProjectPaths {
            ProjectPaths::discover(&self.0).unwrap()
        }

        fn bundle(&self) -> Bundle {
            Bundle::load(
                &Location::Directory(self.0.join("map")),
                Config {
                    preserve_ids: true,
                    ..Config::default()
                },
            )
            .unwrap()
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn project(temp: &TempProject, states: Vec<StateDocument>) -> Hoi4Project {
        let mut project = Hoi4Project::new(temp.paths());
        project.states = states;
        project
    }

    fn document(path: &str, data: StateData, diagnostics: Vec<ProjectDiagnostic>) -> StateDocument {
        StateDocument {
            path: Path::new(path).to_owned(),
            original_bytes: Arc::from([]),
            exact_utf8: true,
            syntax: parse_text(path, ""),
            data: Some(data),
            diagnostics,
            modified: false,
        }
    }

    fn state(id: u32, provinces: &[u32]) -> StateData {
        StateData {
            id: Some(id),
            provinces: BTreeSet::from_iter(provinces.iter().copied()),
            ..Default::default()
        }
    }

    fn valid_fixture(name: &str) -> TempProject {
        TempProject::new(
            name,
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;true;plains;1\n",
            &[[1, 0, 0]],
        )
    }

    #[test]
    fn valid_project_has_empty_report() {
        let temp = valid_fixture("valid");
        let bundle = temp.bundle();
        let project = project(&temp, vec![document("1.txt", state(1, &[1]), Vec::new())]);

        let report = validate_project(&bundle, &project, ProjectValidationTarget::CurrentProject);

        assert_eq!(report.total, 0);
        assert!(!report.blocks_save);
        assert!(!report.requires_warning_review);
    }

    #[test]
    fn warning_report_requires_review_without_blocking_save() {
        let temp = valid_fixture("warning");
        let bundle = temp.bundle();
        let project = project(&temp, vec![document("1.txt", state(1, &[]), Vec::new())]);

        let report = validate_project(&bundle, &project, ProjectValidationTarget::PendingChanges);

        assert_eq!(report.target, ProjectValidationTarget::PendingChanges);
        assert_eq!(report.warnings, 1);
        assert_eq!(report.errors, 0);
        assert!(!report.blocks_save);
        assert!(report.requires_warning_review);
        assert_eq!(report.diagnostics[0].code, "states.provinces.empty");
    }

    #[test]
    fn province_metadata_errors_block_save() {
        let temp = TempProject::new(
            "metadata",
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;true;plains;1\n",
            &[[1, 0, 0], [2, 0, 0]],
        );
        let bundle = temp.bundle();
        let project = project(&temp, vec![document("1.txt", state(1, &[1]), Vec::new())]);

        let report = validate_project(&bundle, &project, ProjectValidationTarget::CurrentProject);
        let codes = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<BTreeSet<_>>();

        assert!(report.blocks_save);
        assert!(codes.contains("cross.bitmap_color.missing_definition"));
        assert!(codes.contains("definition.type.invalid"));
        assert!(codes.contains("definition.coastal.invalid"));
        assert!(codes.contains("definition.invalid"));
    }

    #[test]
    fn cross_domain_rules_catch_state_map_conflicts() {
        let temp = TempProject::new(
            "cross",
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n2;2;0;0;sea;false;ocean;0\n",
            &[[1, 0, 0], [2, 0, 0]],
        );
        let bundle = temp.bundle();
        let mut data = state(1, &[1, 2]);
        data.history = StateHistory {
            victory_points: vec![VictoryPoint {
                province_id: 99,
                value: 1,
            }],
            province_buildings: BTreeMap::from([
                (1, BTreeMap::from([("naval_base".to_owned(), 1)])),
                (99, BTreeMap::from([("bunker".to_owned(), 1)])),
            ]),
            ..Default::default()
        };
        let project = project(&temp, vec![document("1.txt", data, Vec::new())]);

        let report = validate_project(&bundle, &project, ProjectValidationTarget::CurrentProject);
        let codes = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<BTreeSet<_>>();

        assert!(codes.contains("cross.non_land.assigned"));
        assert!(codes.contains("cross.victory_point.outside_state"));
        assert!(codes.contains("cross.building.outside_state"));
        assert!(codes.contains("cross.naval_base.non_coastal"));
    }

    #[test]
    fn summary_order_and_dedup_are_deterministic() {
        let temp = valid_fixture("dedup");
        let bundle = temp.bundle();
        let duplicate = ProjectDiagnostic::new(
            ProjectDiagnosticKind::MissingOwner,
            DiagnosticSeverity::Warning,
            Some(PathBuf::from("b.txt")),
            None,
            "missing owner",
        );
        let same_duplicate = ProjectDiagnostic::new(
            ProjectDiagnosticKind::MissingOwner,
            DiagnosticSeverity::Warning,
            Some(PathBuf::from("b.txt")),
            None,
            "texto localizado diferente",
        );
        let mut project = project(&temp, vec![document("1.txt", state(1, &[1]), Vec::new())]);
        project.diagnostics = vec![same_duplicate, duplicate];

        let report = validate_project(&bundle, &project, ProjectValidationTarget::CurrentProject);

        assert_eq!(report.total, 1);
        assert_eq!(report.warnings, 1);
        assert_eq!(report.summary.blocks_save, 0);
        assert_eq!(report.diagnostics[0].path, Some(PathBuf::from("b.txt")));
    }
}
