use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::app::format::{Definition, ParseCsv};
use crate::app::map::{Bundle, Color, ProvinceKind};
use crate::app::state::{PdxEntry, PdxValue, TextSpan, parse_text};

use super::diagnostics::DiagnosticDomain;
use super::province_geometry::ProvinceGeometryAnalysis;
use super::river_topology::{IndexedRiverBitmap, RiverTopologyAnalysis};
use super::{
    DiagnosticSeverity, Hoi4Project, ProjectDiagnostic, ProjectDiagnosticKind, ResolvedSource,
};

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
    Definition,
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
    pub map_location: Option<[u32; 2]>,
    pub related_province_ids: Vec<u32>,
    pub source: Option<ResolvedSource>,
    pub blocks_save: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectValidationReport {
    pub target: ProjectValidationTarget,
    pub diagnostics: Vec<ProjectValidationDiagnostic>,
    pub summary: ProjectValidationSummary,
    pub baseline_summary: Option<ProjectValidationSummary>,
    pub delta: ProjectValidationDelta,
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

    validate_definition_consistency(bundle, project, &mut diagnostics);
    validate_provinces(bundle, project, &mut diagnostics);
    validate_province_geometry(bundle, project, &mut diagnostics);
    validate_terrain_catalog(bundle, project, &mut diagnostics);
    validate_continent_catalog(bundle, project, &mut diagnostics);
    validate_states(bundle, project, &mut diagnostics);
    validate_adjacencies(bundle, project, &mut diagnostics);
    validate_river_topology(bundle, project, &mut diagnostics);
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
        baseline_summary: None,
        delta: match target {
            ProjectValidationTarget::CurrentProject => {
                ProjectValidationDelta::from_unchanged_diagnostics(&diagnostics)
            }
            ProjectValidationTarget::PendingChanges => {
                ProjectValidationDelta::from_new_diagnostics(&diagnostics)
            }
        },
        diagnostics,
        summary,
    }
}

pub fn validate_project_against_baseline(
    bundle: &Bundle,
    project: &Hoi4Project,
    target: ProjectValidationTarget,
    baseline: &ProjectValidationReport,
    baseline_root: &Path,
) -> ProjectValidationReport {
    let mut report = validate_project(bundle, project, target);
    report.baseline_summary = Some(baseline.summary.clone());
    report.delta = ProjectValidationDelta::new(
        &baseline.diagnostics,
        &report.diagnostics,
        baseline_root,
        &project.paths.root,
    );
    report.blocks_save = report.delta.blocks_save();
    report.requires_warning_review = report.delta.requires_warning_review();
    report
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectValidationDelta {
    pub new: Vec<ProjectValidationChange>,
    pub aggravated: Vec<ProjectValidationChange>,
    pub unchanged: Vec<ProjectValidationChange>,
    pub resolved: Vec<ProjectValidationChange>,
    pub improved: Vec<ProjectValidationChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectValidationChange {
    pub before: Option<ProjectValidationDiagnostic>,
    pub after: Option<ProjectValidationDiagnostic>,
}

impl ProjectValidationDelta {
    pub fn new(
        baseline: &[ProjectValidationDiagnostic],
        candidate: &[ProjectValidationDiagnostic],
        baseline_root: &Path,
        candidate_root: &Path,
    ) -> Self {
        let mut before = grouped_by_identity(baseline, baseline_root);
        let after = grouped_by_identity(candidate, candidate_root);
        let mut delta = Self::default();
        for (identity, mut after_values) in after {
            let mut before_values = before.remove(&identity).unwrap_or_default();
            while let Some(after) = after_values.pop() {
                match before_values.pop() {
                    Some(before) => {
                        let change = ProjectValidationChange {
                            before: Some(before.clone()),
                            after: Some(after.clone()),
                        };
                        match severity_level(after.severity).cmp(&severity_level(before.severity)) {
                            std::cmp::Ordering::Greater => delta.aggravated.push(change),
                            std::cmp::Ordering::Less => delta.improved.push(change),
                            std::cmp::Ordering::Equal => delta.unchanged.push(change),
                        }
                    }
                    None => delta.new.push(ProjectValidationChange {
                        before: None,
                        after: Some(after),
                    }),
                }
            }
            for before in before_values {
                delta.resolved.push(ProjectValidationChange {
                    before: Some(before),
                    after: None,
                });
            }
        }
        for before_values in before.into_values() {
            for before in before_values {
                delta.resolved.push(ProjectValidationChange {
                    before: Some(before),
                    after: None,
                });
            }
        }
        delta.sort();
        delta
    }

    pub fn from_new_diagnostics(diagnostics: &[ProjectValidationDiagnostic]) -> Self {
        let mut delta = Self {
            new: diagnostics
                .iter()
                .cloned()
                .map(|diagnostic| ProjectValidationChange {
                    before: None,
                    after: Some(diagnostic),
                })
                .collect(),
            ..Self::default()
        };
        delta.sort();
        delta
    }

    pub fn from_unchanged_diagnostics(diagnostics: &[ProjectValidationDiagnostic]) -> Self {
        let mut delta = Self {
            unchanged: diagnostics
                .iter()
                .cloned()
                .map(|diagnostic| ProjectValidationChange {
                    before: Some(diagnostic.clone()),
                    after: Some(diagnostic),
                })
                .collect(),
            ..Self::default()
        };
        delta.sort();
        delta
    }

    pub fn new_errors(&self) -> usize {
        self.new
            .iter()
            .filter(|change| change.after_is_error())
            .count()
    }

    pub fn new_warnings(&self) -> usize {
        self.new
            .iter()
            .filter(|change| change.after_is_warning())
            .count()
    }

    pub fn aggravated_to_error(&self) -> usize {
        self.aggravated
            .iter()
            .filter(|change| change.after_is_error())
            .count()
    }

    pub fn blocks_save(&self) -> bool {
        self.new_errors() != 0 || self.aggravated_to_error() != 0
    }

    pub fn requires_warning_review(&self) -> bool {
        self.new_warnings() != 0
            || self
                .aggravated
                .iter()
                .any(|change| !change.after_is_error())
    }

    pub fn has_preexisting_errors(&self) -> bool {
        self.unchanged.iter().any(|change| change.after_is_error())
            || self.improved.iter().any(|change| {
                change
                    .before
                    .as_ref()
                    .is_some_and(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            })
    }

    pub fn has_review_items(&self) -> bool {
        self.requires_warning_review() || self.has_preexisting_errors()
    }

    fn sort(&mut self) {
        for values in [
            &mut self.new,
            &mut self.aggravated,
            &mut self.unchanged,
            &mut self.resolved,
            &mut self.improved,
        ] {
            values.sort_by_key(change_key);
        }
    }
}

impl ProjectValidationChange {
    fn after_is_error(&self) -> bool {
        self.after.as_ref().is_some_and(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Error && diagnostic.blocks_save
        })
    }

    fn after_is_warning(&self) -> bool {
        self.after
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
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
            map_location: None,
            related_province_ids: Vec::new(),
            source: None,
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

    fn with_optional_province_id(mut self, province_id: Option<u32>) -> Self {
        self.province_id = province_id;
        self
    }

    fn with_state_id(mut self, state_id: Option<u32>) -> Self {
        self.state_id = state_id;
        self
    }

    fn with_map_location(mut self, location: [u32; 2]) -> Self {
        self.map_location = Some(location);
        self
    }

    fn with_related_province_ids(mut self, mut ids: Vec<u32>) -> Self {
        ids.sort_unstable();
        ids.dedup();
        self.related_province_ids = ids;
        self
    }

    fn with_source(mut self, source: ResolvedSource) -> Self {
        self.path = source
            .filesystem_path()
            .map(Path::to_path_buf)
            .or_else(|| Some(source.logical_path.clone()));
        self.source = Some(source);
        self
    }

    fn with_blocks_save(mut self, blocks_save: bool) -> Self {
        self.blocks_save = blocks_save;
        self
    }
}

impl ProjectValidationDomain {
    fn from_project(domain: DiagnosticDomain) -> Self {
        match domain {
            DiagnosticDomain::Project => Self::Project,
            DiagnosticDomain::ProvinceMap => Self::Province,
            DiagnosticDomain::Definition => Self::Definition,
            DiagnosticDomain::States => Self::State,
            DiagnosticDomain::CrossDomain => Self::CrossDomain,
            DiagnosticDomain::Transaction => Self::Transaction,
        }
    }
}

fn validate_definition_consistency(
    bundle: &Bundle,
    project: &Hoi4Project,
    diagnostics: &mut Vec<ProjectValidationDiagnostic>,
) {
    let source = project.paths.sources.source_files().definition_csv.clone();
    let definitions = match project.paths.sources.read_resolved(&source) {
        Ok(bytes) => Definition::read_records(std::io::Cursor::new(bytes)),
        Err(error) => {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::InvalidDefinition,
                    DiagnosticSeverity::Error,
                    None,
                    format!("definition table cannot be read: {error}"),
                )
                .with_domain(ProjectValidationDomain::Definition)
                .with_source(source)
                .with_blocks_save(true),
            );
            return;
        }
    };
    let definitions = match definitions {
        Ok(definitions) => definitions,
        Err(error) => {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::InvalidDefinition,
                    DiagnosticSeverity::Error,
                    None,
                    format!("definition table cannot be parsed: {error}"),
                )
                .with_domain(ProjectValidationDomain::Definition)
                .with_source(source)
                .with_blocks_save(true),
            );
            return;
        }
    };
    let bitmap_colors = bundle
        .map
        .iter_province_data()
        .map(|(color, _)| color)
        .collect::<BTreeSet<_>>();
    let mut ids = BTreeMap::<u32, Color>::new();
    let mut colors = BTreeMap::<Color, u32>::new();
    for definition in definitions {
        if let Some(first_color) = ids.insert(definition.id, definition.rgb) {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::DuplicateProvinceId,
                    DiagnosticSeverity::Error,
                    None,
                    format!(
                        "province id {} is used by colors {} and {}",
                        definition.id,
                        color_text(first_color),
                        color_text(definition.rgb)
                    ),
                )
                .with_domain(ProjectValidationDomain::Definition)
                .with_province_id(definition.id)
                .with_source(source.clone())
                .with_blocks_save(true),
            );
        }
        if let Some(first_id) = colors.insert(definition.rgb, definition.id) {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::DuplicateProvinceRgb,
                    DiagnosticSeverity::Error,
                    None,
                    format!(
                        "province color {} is used by ids {} and {}",
                        color_text(definition.rgb),
                        first_id,
                        definition.id
                    ),
                )
                .with_domain(ProjectValidationDomain::Definition)
                .with_province_id(definition.id)
                .with_related_province_ids(vec![first_id, definition.id])
                .with_source(source.clone())
                .with_blocks_save(true),
            );
        }
        if definition.id != 0 && !bitmap_colors.contains(&definition.rgb) {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::UnusedDefinition,
                    DiagnosticSeverity::Error,
                    None,
                    format!(
                        "province definition {} ({}) is absent from provinces bitmap",
                        definition.id,
                        color_text(definition.rgb)
                    ),
                )
                .with_domain(ProjectValidationDomain::CrossDomain)
                .with_province_id(definition.id)
                .with_source(source.clone())
                .with_blocks_save(false),
            );
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
        if province.kind == ProvinceKind::Land && province.continent == 0 {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::LandProvinceNoContinent,
                    DiagnosticSeverity::Warning,
                    Some(project.paths.definition_csv.clone()),
                    format!("{province_label} is land but has no continent"),
                )
                .with_domain(ProjectValidationDomain::Definition)
                .with_optional_province_id(province_id)
                .with_blocks_save(false),
            );
        } else if !province.kind.valid_continent_id(province.continent) {
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

fn validate_province_geometry(
    bundle: &Bundle,
    project: &Hoi4Project,
    diagnostics: &mut Vec<ProjectValidationDiagnostic>,
) {
    let source = project.paths.sources.source_files().provinces_bmp.clone();
    let analysis = ProvinceGeometryAnalysis::analyze(bundle);
    for info in analysis.provinces.values() {
        if info.pixel_count == 1 {
            let location = info.representative_locations[0];
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::ProvinceOnePixel,
                    DiagnosticSeverity::Warning,
                    None,
                    format!(
                        "province {} occupies one pixel at {},{}",
                        province_label(info.province_id, info.color),
                        location[0],
                        location[1]
                    ),
                )
                .with_domain(ProjectValidationDomain::Province)
                .with_optional_province_id(info.province_id)
                .with_map_location(location)
                .with_source(source.clone())
                .with_blocks_save(false),
            );
        }
        if info.component_count > 1 {
            let location = info.representative_locations[1];
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::ProvinceDisconnectedComponents,
                    DiagnosticSeverity::Warning,
                    None,
                    format!(
                        "province {} has {} disconnected components; additional component starts at {},{}",
                        province_label(info.province_id, info.color),
                        info.component_count,
                        location[0],
                        location[1]
                    ),
                )
                .with_domain(ProjectValidationDomain::Province)
                .with_optional_province_id(info.province_id)
                .with_map_location(location)
                .with_source(source.clone())
                .with_blocks_save(false),
            );
        }
    }
    for crossing in analysis.x_crossings {
        let ids = crossing
            .colors
            .into_iter()
            .filter_map(|color| bundle.map.province_id_for_color(color))
            .collect::<Vec<_>>();
        diagnostics.push(
            ProjectValidationDiagnostic::custom(
                ProjectDiagnosticKind::MapXCrossing,
                DiagnosticSeverity::Error,
                None,
                format!(
                    "four-province X crossing at {},{} affects {}",
                    crossing.location[0],
                    crossing.location[1],
                    ids.iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .with_domain(ProjectValidationDomain::Province)
            .with_map_location(crossing.location)
            .with_related_province_ids(ids)
            .with_source(source.clone())
            .with_blocks_save(false),
        );
    }
}

fn validate_terrain_catalog(
    bundle: &Bundle,
    project: &Hoi4Project,
    diagnostics: &mut Vec<ProjectValidationDiagnostic>,
) {
    let catalog = match terrain_catalog(bundle, project) {
        Ok(catalog) => catalog,
        Err(detail) => {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::TerrainCatalogUnavailable,
                    DiagnosticSeverity::Warning,
                    None,
                    format!("terrain catalog is unavailable: {detail}"),
                )
                .with_domain(ProjectValidationDomain::Project)
                .with_blocks_save(false),
            );
            return;
        }
    };
    let Some(catalog) = catalog else {
        return;
    };
    let source = project.paths.sources.source_files().definition_csv.clone();
    for (color, province) in sorted_provinces(bundle) {
        if province.terrain.trim().is_empty()
            || province.terrain == "unknown"
            || catalog.contains(&province.terrain)
        {
            continue;
        }
        diagnostics.push(
            ProjectValidationDiagnostic::custom(
                ProjectDiagnosticKind::ProvinceTerrainUndefined,
                DiagnosticSeverity::Error,
                None,
                format!(
                    "province {} references undefined terrain '{}'",
                    province_label(province.preserved_id, color),
                    province.terrain
                ),
            )
            .with_domain(ProjectValidationDomain::Definition)
            .with_optional_province_id(province.preserved_id)
            .with_source(source.clone())
            .with_blocks_save(false),
        );
    }
}

fn validate_continent_catalog(
    bundle: &Bundle,
    project: &Hoi4Project,
    diagnostics: &mut Vec<ProjectValidationDiagnostic>,
) {
    let Some(source) = project.paths.sources.source_files().continent_txt.clone() else {
        return;
    };
    let catalog = match continent_catalog(project, &source) {
        Ok(catalog) => catalog,
        Err(detail) => {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::ContinentCatalogUnavailable,
                    DiagnosticSeverity::Warning,
                    None,
                    format!("continent catalog is unavailable: {detail}"),
                )
                .with_domain(ProjectValidationDomain::Project)
                .with_source(source)
                .with_blocks_save(false),
            );
            return;
        }
    };
    let definition_source = project.paths.sources.source_files().definition_csv.clone();
    for (color, province) in sorted_provinces(bundle) {
        if province.kind != ProvinceKind::Land
            || province.continent == 0
            || catalog.contains(&province.continent)
        {
            continue;
        }
        diagnostics.push(
            ProjectValidationDiagnostic::custom(
                ProjectDiagnosticKind::ProvinceContinentUndefined,
                DiagnosticSeverity::Error,
                None,
                format!(
                    "province {} references undefined continent {}",
                    province_label(province.preserved_id, color),
                    province.continent
                ),
            )
            .with_domain(ProjectValidationDomain::Definition)
            .with_optional_province_id(province.preserved_id)
            .with_source(definition_source.clone())
            .with_blocks_save(false),
        );
    }
}

fn terrain_catalog(
    bundle: &Bundle,
    project: &Hoi4Project,
) -> Result<Option<BTreeSet<String>>, String> {
    let listing = project
        .paths
        .sources
        .list_files("common/terrain")
        .map_err(|error| error.to_string())?;
    if listing.files.is_empty() {
        // The editor configuration is an intentional fallback for standalone
        // maps with no configured lower source. A real source graph always
        // takes precedence and can provide mod-defined terrains.
        return Ok(Some(bundle.config.terrains.keys().cloned().collect()));
    }
    let mut terrains = BTreeSet::new();
    for source in listing.files {
        let text = String::from_utf8(
            project
                .paths
                .sources
                .read_resolved(&source)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("{} is not UTF-8: {error}", source.logical_path.display()))?;
        let document = parse_text(&source.logical_path, text);
        if !document.diagnostics.is_empty() {
            return Err(format!(
                "{} has syntax errors",
                source.logical_path.display()
            ));
        }
        let Some(categories) = find_block(&document.entries, "categories") else {
            continue;
        };
        for entry in &categories.entries {
            if let Some(key) = &entry.key {
                terrains.insert(key.text.clone());
            }
        }
    }
    (!terrains.is_empty())
        .then_some(terrains)
        .ok_or_else(|| "no terrain categories were found".to_owned())
        .map(Some)
}

fn continent_catalog(
    project: &Hoi4Project,
    source: &ResolvedSource,
) -> Result<BTreeSet<u16>, String> {
    let text = String::from_utf8(
        project
            .paths
            .sources
            .read_resolved(source)
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("{} is not UTF-8: {error}", source.logical_path.display()))?;
    let document = parse_text(&source.logical_path, text);
    if !document.diagnostics.is_empty() {
        return Err(format!(
            "{} has syntax errors",
            source.logical_path.display()
        ));
    }
    let Some(continents) = find_block(&document.entries, "continents") else {
        return Err("continents block is missing".to_owned());
    };
    let count = continents
        .entries
        .iter()
        .filter(|entry| entry.key.is_none())
        .count();
    if count == 0 {
        return Err("continents block has no entries".to_owned());
    }
    Ok((1..=count)
        .filter_map(|id| u16::try_from(id).ok())
        .collect())
}

fn find_block<'a>(entries: &'a [PdxEntry], key: &str) -> Option<&'a crate::app::state::PdxBlock> {
    entries.iter().find_map(|entry| {
        (entry.key.as_ref()?.text == key)
            .then_some(match &entry.value {
                PdxValue::Block(block) => Some(block),
                PdxValue::Scalar(_) => None,
            })
            .flatten()
    })
}

fn province_label(province_id: Option<u32>, color: Color) -> String {
    province_id.map_or_else(
        || format!("color {}", color_text(color)),
        |id| id.to_string(),
    )
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
        let state_label =
            state_id.map_or_else(|| "unknown state".to_owned(), |id| format!("State {id}"));

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
            if !provinces.contains_key(&province_id) {
                diagnostics.push(
                    ProjectValidationDiagnostic::custom(
                        ProjectDiagnosticKind::UnknownProvince,
                        DiagnosticSeverity::Error,
                        Some(document.path.clone()),
                        format!(
                            "{state_label} references removed or missing province {province_id}"
                        ),
                    )
                    .with_domain(ProjectValidationDomain::CrossDomain)
                    .with_province_id(province_id)
                    .with_state_id(state_id),
                );
                continue;
            }
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
            if !provinces.contains_key(&vp.province_id) {
                diagnostics.push(
                    ProjectValidationDiagnostic::custom(
                        ProjectDiagnosticKind::UnknownProvince,
                        DiagnosticSeverity::Error,
                        Some(document.path.clone()),
                        format!(
                            "{state_label} victory point references removed or missing province {}",
                            vp.province_id
                        ),
                    )
                    .with_domain(ProjectValidationDomain::CrossDomain)
                    .with_province_id(vp.province_id)
                    .with_state_id(state_id),
                );
            }
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
            if !provinces.contains_key(&province_id) {
                diagnostics.push(
                    ProjectValidationDiagnostic::custom(
                        ProjectDiagnosticKind::UnknownProvince,
                        DiagnosticSeverity::Error,
                        Some(document.path.clone()),
                        format!("{state_label} province buildings reference removed or missing province {province_id}"),
                    )
                    .with_domain(ProjectValidationDomain::CrossDomain)
                    .with_province_id(province_id)
                    .with_state_id(state_id),
                );
            }
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

fn validate_adjacencies(
    bundle: &Bundle,
    project: &Hoi4Project,
    diagnostics: &mut Vec<ProjectValidationDiagnostic>,
) {
    let source = project.paths.sources.source_files().adjacencies_csv.clone();
    let [width, height] = bundle.map.dimensions();
    for adjacency in bundle.map.unresolved_adjacencies() {
        for (field, province_id, kind) in [
            (
                "source",
                adjacency.from_id,
                ProjectDiagnosticKind::AdjacencyFromProvinceMissing,
            ),
            (
                "destination",
                adjacency.to_id,
                ProjectDiagnosticKind::AdjacencyToProvinceMissing,
            ),
        ]
        .into_iter()
        .chain(adjacency.through.map(|province_id| {
            (
                "through",
                province_id,
                ProjectDiagnosticKind::AdjacencyThroughProvinceMissing,
            )
        })) {
            if bundle.map.contains_province_id(province_id) {
                continue;
            }
            let diagnostic = ProjectValidationDiagnostic::custom(
                kind,
                DiagnosticSeverity::Error,
                None,
                format!("adjacency {field} references removed or missing province {province_id}"),
            )
            .with_domain(ProjectValidationDomain::CrossDomain)
            .with_province_id(province_id)
            .with_blocks_save(true);
            diagnostics.push(match source.clone() {
                Some(source) => diagnostic.with_source(source),
                None => diagnostic,
            });
        }
    }
    for adjacency in bundle.map.adjacencies() {
        for (field, location) in [("start", adjacency.start), ("stop", adjacency.stop)] {
            let Some([x, y]) = location else {
                continue;
            };
            if x < width && y < height {
                continue;
            }
            let diagnostic = ProjectValidationDiagnostic::custom(
                ProjectDiagnosticKind::AdjacencyCoordinateOutOfBounds,
                DiagnosticSeverity::Error,
                None,
                format!(
                    "adjacency {field} coordinate {x},{y} is outside map bounds {width}x{height}"
                ),
            )
            .with_domain(ProjectValidationDomain::CrossDomain)
            .with_map_location([x, y])
            .with_blocks_save(false);
            diagnostics.push(match source.clone() {
                Some(source) => diagnostic.with_source(source),
                None => diagnostic,
            });
        }
    }
}

fn validate_river_topology(
    bundle: &Bundle,
    project: &Hoi4Project,
    diagnostics: &mut Vec<ProjectValidationDiagnostic>,
) {
    let Some(source) = project.paths.sources.source_files().rivers_bmp.clone() else {
        return;
    };
    let image = match project.paths.sources.read_resolved(&source) {
        Ok(bytes) => match IndexedRiverBitmap::parse(&bytes) {
            Ok(image) => image,
            Err(detail) => {
                diagnostics.push(
                    ProjectValidationDiagnostic::custom(
                        ProjectDiagnosticKind::RiverImageFormatUnsupported,
                        DiagnosticSeverity::Error,
                        None,
                        format!("rivers bitmap format is unsupported: {detail}"),
                    )
                    .with_domain(ProjectValidationDomain::Province)
                    .with_source(source)
                    .with_blocks_save(false),
                );
                return;
            }
        },
        Err(error) => {
            diagnostics.push(
                ProjectValidationDiagnostic::custom(
                    ProjectDiagnosticKind::RiverImageFormatUnsupported,
                    DiagnosticSeverity::Error,
                    None,
                    format!("rivers bitmap cannot be read: {error}"),
                )
                .with_domain(ProjectValidationDomain::Province)
                .with_source(source)
                .with_blocks_save(false),
            );
            return;
        }
    };
    let [width, height] = bundle.map.dimensions();
    if (image.width, image.height) != (width, height) {
        diagnostics.push(
            ProjectValidationDiagnostic::custom(
                ProjectDiagnosticKind::RiverDimensionMismatch,
                DiagnosticSeverity::Error,
                None,
                format!(
                    "rivers bitmap dimensions {}x{} do not match provinces bitmap dimensions {width}x{height}",
                    image.width, image.height
                ),
            )
            .with_domain(ProjectValidationDomain::Province)
            .with_source(source)
            .with_blocks_save(false),
        );
        return;
    }

    for component in RiverTopologyAnalysis::analyze(&image).components {
        let component_label = component.id + 1;
        if component.valid_sources.is_empty() {
            diagnostics.push(river_diagnostic(
                ProjectDiagnosticKind::RiverNoSource,
                DiagnosticSeverity::Error,
                source.clone(),
                component.representative_location,
                format!("river component {component_label} has no source marker at an endpoint"),
            ));
        } else if component.valid_sources.len() > 1 {
            let locations = component
                .valid_sources
                .iter()
                .map(|[x, y]| format!("{x},{y}"))
                .collect::<Vec<_>>()
                .join("; ");
            diagnostics.push(river_diagnostic(
                ProjectDiagnosticKind::RiverMultipleSources,
                DiagnosticSeverity::Error,
                source.clone(),
                component.valid_sources[0],
                format!(
                    "river component {component_label} has multiple source markers at {locations}"
                ),
            ));
        }
        if component.valid_flow_markers.is_empty() {
            diagnostics.push(
                river_diagnostic(
                    ProjectDiagnosticKind::RiverNoFlowEndpoint,
                    DiagnosticSeverity::Error,
                    source.clone(),
                    component.representative_location,
                    format!("river component {component_label} has no flow-in or flow-out marker connected to river body"),
                ),
            );
        }
        for location in component.invalid_flow_markers {
            diagnostics.push(
                river_diagnostic(
                    ProjectDiagnosticKind::RiverInvalidFlowMarker,
                    DiagnosticSeverity::Error,
                    source.clone(),
                    location,
                    format!(
                        "river component {component_label} has a flow marker at {},{} not connected to river body",
                        location[0], location[1]
                    ),
                ),
            );
        }
        if component.edge_count >= component.pixel_count {
            diagnostics.push(river_diagnostic(
                ProjectDiagnosticKind::RiverPossibleLoop,
                DiagnosticSeverity::Warning,
                source.clone(),
                component.representative_location,
                format!("river component {component_label} contains a possible loop"),
            ));
        }
    }
}

fn river_diagnostic(
    kind: ProjectDiagnosticKind,
    severity: DiagnosticSeverity,
    source: ResolvedSource,
    location: [u32; 2],
    message: String,
) -> ProjectValidationDiagnostic {
    ProjectValidationDiagnostic::custom(kind, severity, None, message)
        .with_domain(ProjectValidationDomain::Province)
        .with_map_location(location)
        .with_source(source)
        .with_blocks_save(false)
}

fn province_lookup(bundle: &Bundle) -> BTreeMap<u32, (bool, ProvinceKind, bool)> {
    bundle
        .map
        .province_id_index()
        .iter()
        .map(|(id, color)| {
            let province = bundle.map.get_province(color);
            (id, (true, province.kind, province.coastal == Some(true)))
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
        && left.domain == right.domain
        && left.code == right.code
        && left.message_key == right.message_key
        && left.path == right.path
        && left.related_path == right.related_path
        && left.span == right.span
        && left.province_id == right.province_id
        && left.state_id == right.state_id
        && left.map_location == right.map_location
        && left.related_province_ids == right.related_province_ids
        && source_identity(left.source.as_ref()) == source_identity(right.source.as_ref())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DiagnosticIdentity {
    kind: ProjectDiagnosticKind,
    domain: ProjectValidationDomain,
    code: String,
    message_key: String,
    path: Option<String>,
    related_path: Option<String>,
    span: Option<(usize, usize)>,
    province_id: Option<u32>,
    state_id: Option<u32>,
    map_location: Option<[u32; 2]>,
    related_province_ids: Vec<u32>,
    source: Option<(String, String, u64)>,
}

fn grouped_by_identity(
    diagnostics: &[ProjectValidationDiagnostic],
    root: &Path,
) -> BTreeMap<DiagnosticIdentity, Vec<ProjectValidationDiagnostic>> {
    let mut groups = BTreeMap::<DiagnosticIdentity, Vec<ProjectValidationDiagnostic>>::new();
    for diagnostic in diagnostics {
        groups
            .entry(identity(diagnostic, root))
            .or_default()
            .push(diagnostic.clone());
    }
    for values in groups.values_mut() {
        values.sort_by(cmp_stable);
        values.reverse();
    }
    groups
}

fn identity(diagnostic: &ProjectValidationDiagnostic, root: &Path) -> DiagnosticIdentity {
    DiagnosticIdentity {
        kind: diagnostic.kind,
        domain: diagnostic.domain,
        code: diagnostic.code.clone(),
        message_key: diagnostic.message_key.clone(),
        path: diagnostic
            .path
            .as_ref()
            .map(|path| normalized_path(root, path)),
        related_path: diagnostic
            .related_path
            .as_ref()
            .map(|path| normalized_path(root, path)),
        span: span_key(diagnostic.span),
        province_id: diagnostic.province_id,
        state_id: diagnostic.state_id,
        map_location: diagnostic.map_location,
        related_province_ids: diagnostic.related_province_ids.clone(),
        source: source_identity(diagnostic.source.as_ref()),
    }
}

fn normalized_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
        .to_lowercase()
}

fn change_key(change: &ProjectValidationChange) -> DiagnosticIdentity {
    if let Some(after) = &change.after {
        identity(after, Path::new(""))
    } else {
        identity(
            change.before.as_ref().expect("change has before or after"),
            Path::new(""),
        )
    }
}

fn severity_level(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Info => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Error => 2,
    }
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
        (left.map_location, &left.related_province_ids),
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
            (right.map_location, &right.related_province_ids),
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

fn source_identity(source: Option<&ResolvedSource>) -> Option<(String, String, u64)> {
    source.map(|source| {
        (
            source.logical_path.to_string_lossy().to_string(),
            format!("{:?}", source.source_kind),
            source.project_generation.value(),
        )
    })
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

    use crate::app::format::{Adjacency, AdjacencyKind, Definition, DefinitionKind};
    use crate::app::map::{Bundle, construct_map_data_for_sparse_tests, write_rgb_bmp_image};
    use crate::app::project::{ProjectPaths, SourceLookup};
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
            &[[1, 0, 0], [1, 0, 0]],
        )
    }

    fn sparse_bundle(adjacencies: Vec<Adjacency>) -> Bundle {
        let colors = [[10, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]];
        let ids = [1, 7, 42, 500];
        let image = image::RgbImage::from_fn(4, 1, |x, _| image::Rgb(colors[x as usize]));
        let definitions = colors
            .into_iter()
            .zip(ids)
            .map(|(rgb, id)| Definition {
                id,
                rgb,
                kind: DefinitionKind::Land,
                coastal: false,
                terrain: "plains".to_owned(),
                continent: 1,
            })
            .collect();
        construct_map_data_for_sparse_tests(
            image,
            definitions,
            adjacencies,
            None,
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .unwrap()
    }

    fn adjacency(from_id: u32, to_id: u32, through: Option<u32>) -> Adjacency {
        Adjacency {
            from_id,
            to_id,
            kind: AdjacencyKind::Sea,
            through,
            start: None,
            stop: None,
            rule_name: String::new(),
            comment: String::new(),
        }
    }

    fn bundle_from_image(image: image::RgbImage, definitions: Vec<Definition>) -> Bundle {
        construct_map_data_for_sparse_tests(
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

    fn write_indexed_river_bmp(path: &Path, width: u32, height: u32, pixels: &[u8]) {
        assert_eq!(pixels.len(), width as usize * height as usize);
        let stride = (width as usize).div_ceil(4) * 4;
        let pixel_offset = 14 + 40 + 256 * 4;
        let mut bytes = vec![0; pixel_offset + stride * height as usize];
        let file_length = bytes.len() as u32;
        bytes[0..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&file_length.to_le_bytes());
        bytes[10..14].copy_from_slice(&(pixel_offset as u32).to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&8u16.to_le_bytes());
        for (index, entry) in bytes[54..pixel_offset].chunks_exact_mut(4).enumerate() {
            let shade = index as u8;
            entry.copy_from_slice(&[shade, shade, shade, 0]);
        }
        for map_y in 0..height as usize {
            let file_y = height as usize - 1 - map_y;
            let start = pixel_offset + file_y * stride;
            bytes[start..start + width as usize]
                .copy_from_slice(&pixels[map_y * width as usize..(map_y + 1) * width as usize]);
        }
        fs::write(path, bytes).unwrap();
    }

    fn river_fixture(name: &str, width: u32, height: u32, rivers: &[u8]) -> TempProject {
        let temp = TempProject::new(
            name,
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n",
            &[[1, 0, 0]],
        );
        let provinces = image::RgbImage::from_pixel(width, height, image::Rgb([1, 0, 0]));
        let mut province_bytes = Vec::new();
        write_rgb_bmp_image(&mut province_bytes, &provinces).unwrap();
        fs::write(temp.0.join("map/provinces.bmp"), province_bytes).unwrap();
        write_indexed_river_bmp(&temp.0.join("map/rivers.bmp"), width, height, rivers);
        temp
    }

    fn river_bundle(temp: &TempProject) -> Bundle {
        Bundle::load_project(
            &temp.paths(),
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .unwrap()
    }

    fn definitions(colors: &[[u8; 3]]) -> Vec<Definition> {
        colors
            .iter()
            .enumerate()
            .map(|(index, &rgb)| Definition {
                id: index as u32 + 1,
                rgb,
                kind: DefinitionKind::Land,
                coastal: false,
                terrain: "plains".to_owned(),
                continent: 1,
            })
            .collect()
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
    fn valid_indexed_river_has_no_topology_diagnostic() {
        let temp = river_fixture("river-valid", 3, 1, &[0, 3, 2]);
        let report = validate_project(
            &river_bundle(&temp),
            &project(&temp, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );

        assert!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.starts_with("RIVER_"))
        );
    }

    #[test]
    fn river_without_source_has_coordinate_provenance_and_does_not_block_save() {
        let temp = river_fixture("river-no-source", 2, 1, &[3, 2]);
        let report = validate_project(
            &river_bundle(&temp),
            &project(&temp, Vec::new()),
            ProjectValidationTarget::PendingChanges,
        );
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "RIVER_NO_SOURCE")
            .expect("no-source diagnostic");

        assert_eq!(diagnostic.map_location, Some([0, 0]));
        assert_eq!(
            diagnostic
                .source
                .as_ref()
                .unwrap()
                .logical_path
                .to_string_lossy()
                .replace('\\', "/"),
            "map/rivers.bmp"
        );
        assert!(!diagnostic.blocks_save);
        assert!(!report.blocks_save);
    }

    #[test]
    fn river_topology_reports_multiple_sources_missing_flow_and_invalid_marker() {
        let multiple = river_fixture(
            "river-multiple-sources",
            3,
            3,
            &[0, 255, 0, 3, 3, 3, 255, 2, 255],
        );
        let multiple_paths = multiple.paths();
        let multiple_source = multiple_paths
            .sources
            .source_files()
            .rivers_bmp
            .as_ref()
            .unwrap();
        let multiple_image = IndexedRiverBitmap::parse(
            &multiple_paths
                .sources
                .read_resolved(multiple_source)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            RiverTopologyAnalysis::analyze(&multiple_image).components[0]
                .valid_sources
                .len(),
            2
        );
        let multiple_report = validate_project(
            &river_bundle(&multiple),
            &project(&multiple, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        let source = multiple_report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "RIVER_MULTIPLE_SOURCES")
            .expect("multiple-source diagnostic");
        assert_eq!(source.map_location, Some([0, 0]));

        let no_endpoint = river_fixture("river-no-endpoint", 2, 1, &[0, 3]);
        let endpoint_report = validate_project(
            &river_bundle(&no_endpoint),
            &project(&no_endpoint, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert!(
            endpoint_report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "RIVER_NO_FLOW_ENDPOINT")
        );

        let invalid = river_fixture("river-invalid-marker", 4, 1, &[0, 3, 2, 1]);
        let invalid_report = validate_project(
            &river_bundle(&invalid),
            &project(&invalid, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert!(invalid_report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "RIVER_INVALID_FLOW_MARKER"
                && diagnostic.map_location == Some([3, 0])
        }));
    }

    #[test]
    fn river_loop_is_warning_and_branching_tree_is_not_a_false_positive() {
        let looped = river_fixture("river-loop", 4, 2, &[0, 3, 3, 255, 255, 3, 3, 2]);
        let loop_report = validate_project(
            &river_bundle(&looped),
            &project(&looped, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        let loop_diagnostic = loop_report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "RIVER_POSSIBLE_LOOP")
            .expect("loop diagnostic");
        assert_eq!(loop_diagnostic.severity, DiagnosticSeverity::Warning);
        assert!(!loop_diagnostic.blocks_save);

        let branch = river_fixture("river-branch", 3, 3, &[255, 0, 255, 2, 3, 2, 255, 3, 255]);
        let branch_report = validate_project(
            &river_bundle(&branch),
            &project(&branch, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert!(
            !branch_report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "RIVER_POSSIBLE_LOOP")
        );
    }

    #[test]
    fn river_format_and_dimensions_are_root_diagnostics_without_topology_cascades() {
        let format = river_fixture("river-format", 2, 1, &[0, 3]);
        let unsupported = image::RgbImage::from_pixel(2, 1, image::Rgb([0, 255, 0]));
        let mut bytes = Vec::new();
        write_rgb_bmp_image(&mut bytes, &unsupported).unwrap();
        fs::write(format.0.join("map/rivers.bmp"), bytes).unwrap();
        let format_report = validate_project(
            &river_bundle(&format),
            &project(&format, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert_eq!(
            format_report
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code.starts_with("RIVER_"))
                .count(),
            1
        );
        assert!(
            format_report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "RIVER_IMAGE_FORMAT_UNSUPPORTED")
        );

        let dimensions = river_fixture("river-dimensions", 2, 1, &[0, 3]);
        write_indexed_river_bmp(&dimensions.0.join("map/rivers.bmp"), 1, 1, &[0]);
        let dimension_report = validate_project(
            &river_bundle(&dimensions),
            &project(&dimensions, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert_eq!(
            dimension_report
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code.starts_with("RIVER_"))
                .count(),
            1
        );
        assert!(
            dimension_report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "RIVER_DIMENSION_MISMATCH")
        );
    }

    #[test]
    fn custom_default_map_river_path_and_current_project_ownership_are_respected() {
        let custom = river_fixture("river-custom-path", 3, 1, &[255, 255, 255]);
        fs::write(
            custom.0.join("map/default.map"),
            "rivers = custom_rivers.bmp",
        )
        .unwrap();
        fs::remove_file(custom.0.join("map/rivers.bmp")).unwrap();
        write_indexed_river_bmp(&custom.0.join("map/custom_rivers.bmp"), 3, 1, &[0, 3, 2]);
        let custom_report = validate_project(
            &river_bundle(&custom),
            &project(&custom, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert!(
            !custom_report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.starts_with("RIVER_"))
        );

        let owned = valid_fixture("river-replace-path");
        fs::write(owned.0.join("descriptor.mod"), "replace_path = \"map\"").unwrap();
        let base = owned.0.with_file_name(format!(
            "hoi4-validation-core-river-base-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("map")).unwrap();
        write_indexed_river_bmp(&base.join("map/rivers.bmp"), 1, 1, &[0]);
        let mut owned_project = project(&owned, Vec::new());
        owned_project
            .paths
            .set_validated_base_game_root(Some(base.clone()));
        assert!(matches!(
            owned_project
                .paths
                .sources
                .resolve("map/rivers.bmp")
                .unwrap(),
            SourceLookup::BlockedByReplacePath { .. }
        ));
        let owned_report = validate_project(
            &owned.bundle(),
            &owned_project,
            ProjectValidationTarget::CurrentProject,
        );
        assert!(
            !owned_report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.starts_with("RIVER_"))
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn river_diagnostics_do_not_cross_project_generations() {
        let project_a = river_fixture("river-project-a", 2, 1, &[3, 2]);
        let project_b = river_fixture("river-project-b", 3, 1, &[0, 3, 2]);
        let mut loaded_a = project(&project_a, Vec::new());
        loaded_a.paths.bind_project_generation(71);
        let report_a = validate_project(
            &river_bundle(&project_a),
            &loaded_a,
            ProjectValidationTarget::CurrentProject,
        );
        assert!(
            report_a
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "RIVER_NO_SOURCE")
        );

        let mut loaded_b = project(&project_b, Vec::new());
        loaded_b.paths.bind_project_generation(72);
        let report_b = validate_project(
            &river_bundle(&project_b),
            &loaded_b,
            ProjectValidationTarget::CurrentProject,
        );
        assert!(
            !report_b
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.starts_with("RIVER_"))
        );
        assert!(report_b.diagnostics.iter().all(|diagnostic| {
            diagnostic
                .source
                .as_ref()
                .is_none_or(|source| source.project_generation.value() == 72)
        }));
    }

    #[test]
    fn x_crossing_carries_coordinate_related_ids_and_is_not_a_save_blocker() {
        let colors = [[1, 0, 0], [2, 0, 0], [3, 0, 0], [4, 0, 0]];
        let temp = TempProject::new(
            "x-crossing",
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n2;2;0;0;land;false;plains;1\n3;3;0;0;land;false;plains;1\n4;4;0;0;land;false;plains;1\n",
            &colors,
        );
        let image = image::RgbImage::from_fn(2, 2, |x, y| image::Rgb(colors[(y * 2 + x) as usize]));
        let bundle = bundle_from_image(image, definitions(&colors));
        let report = validate_project(
            &bundle,
            &project(&temp, Vec::new()),
            ProjectValidationTarget::PendingChanges,
        );
        let crossing = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "MAP_X_CROSSING")
            .expect("X crossing diagnostic");
        assert_eq!(crossing.map_location, Some([0, 0]));
        assert_eq!(crossing.related_province_ids, vec![1, 2, 3, 4]);
        assert!(!crossing.blocks_save);
        assert!(!report.blocks_save);
    }

    #[test]
    fn geometry_wraps_horizontally_but_reports_true_disconnected_components() {
        let colors = [[1, 0, 0], [2, 0, 0]];
        let temp = TempProject::new(
            "components",
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n2;2;0;0;land;false;plains;1\n",
            &colors,
        );
        let seam = image::RgbImage::from_fn(4, 1, |x, _| {
            image::Rgb(if x == 0 || x == 3 {
                colors[0]
            } else {
                colors[1]
            })
        });
        let seam_report = validate_project(
            &bundle_from_image(seam, definitions(&colors)),
            &project(&temp, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert!(
            !seam_report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PROVINCE_DISCONNECTED_COMPONENTS")
        );

        let disconnected = image::RgbImage::from_fn(4, 1, |x, _| {
            image::Rgb(if x == 0 || x == 2 {
                colors[0]
            } else {
                colors[1]
            })
        });
        let report = validate_project(
            &bundle_from_image(disconnected, definitions(&colors)),
            &project(&temp, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PROVINCE_DISCONNECTED_COMPONENTS"
                && diagnostic.province_id == Some(1)
                && !diagnostic.blocks_save
        }));
    }

    #[test]
    fn raw_definition_validation_retains_unused_and_duplicate_identity_evidence() {
        let temp = valid_fixture("raw-definition");
        let bundle = temp.bundle();
        fs::write(
            temp.0.join("map/definition.csv"),
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n1;2;0;0;land;false;plains;1\n2;3;0;0;land;false;plains;1\n3;3;0;0;land;false;plains;1\n",
        )
        .unwrap();
        let report = validate_project(
            &bundle,
            &project(&temp, Vec::new()),
            ProjectValidationTarget::PendingChanges,
        );
        assert!(report.diagnostics.iter().any(|diagnostic| diagnostic.code
            == "definition.id.duplicate"
            && diagnostic.blocks_save));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "definition.rgb.duplicate"
                    && diagnostic.blocks_save)
        );
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "cross.definition.unused" && !diagnostic.blocks_save
        }));
    }

    #[test]
    fn terrain_and_custom_default_map_continent_catalogs_use_resolved_sources() {
        let temp = TempProject::new(
            "source-catalogs",
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;made_up;2\n",
            &[[1, 0, 0]],
        );
        fs::create_dir_all(temp.0.join("common/terrain")).unwrap();
        fs::write(
            temp.0.join("common/terrain/00_test.txt"),
            "categories = { plains = { } forest = { } }",
        )
        .unwrap();
        fs::write(
            temp.0.join("map/default.map"),
            "continent = custom_continents.txt",
        )
        .unwrap();
        fs::write(
            temp.0.join("map/custom_continents.txt"),
            "continents = { europe }",
        )
        .unwrap();
        let bundle = temp.bundle();
        let report = validate_project(
            &bundle,
            &project(&temp, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PROVINCE_TERRAIN_UNDEFINED")
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PROVINCE_CONTINENT_UNDEFINED")
        );
    }

    #[test]
    fn terrain_catalog_accepts_project_entries_and_replace_path_blocks_lower_entries() {
        let temp = TempProject::new(
            "terrain-replace-path",
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;mod_only;1\n",
            &[[1, 0, 0]],
        );
        let base = temp.0.with_file_name(format!(
            "hoi4-validation-core-base-terrain-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("common/terrain")).unwrap();
        fs::write(
            base.join("common/terrain/00_base.txt"),
            "categories = { lower_only = { } }",
        )
        .unwrap();
        fs::write(
            temp.0.join("descriptor.mod"),
            "replace_path = \"common/terrain\"",
        )
        .unwrap();
        fs::create_dir_all(temp.0.join("common/terrain")).unwrap();
        fs::write(
            temp.0.join("common/terrain/00_mod.txt"),
            "categories = { mod_only = { } }",
        )
        .unwrap();
        let mut definition = definitions(&[[1, 0, 0]]);
        definition[0].terrain = "mod_only".to_owned();
        let bundle = bundle_from_image(
            image::RgbImage::from_pixel(1, 1, image::Rgb([1, 0, 0])),
            definition,
        );
        let mut loaded_project = project(&temp, Vec::new());
        loaded_project
            .paths
            .set_validated_base_game_root(Some(base.clone()));
        let report = validate_project(
            &bundle,
            &loaded_project,
            ProjectValidationTarget::CurrentProject,
        );
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PROVINCE_TERRAIN_UNDEFINED")
        );

        fs::remove_dir_all(temp.0.join("common/terrain")).unwrap();
        fs::write(
            temp.0.join("map/definition.csv"),
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;lower_only;1\n",
        )
        .unwrap();
        let mut definition = definitions(&[[1, 0, 0]]);
        definition[0].terrain = "lower_only".to_owned();
        let bundle = bundle_from_image(
            image::RgbImage::from_pixel(1, 1, image::Rgb([1, 0, 0])),
            definition,
        );
        let mut project = project(&temp, Vec::new());
        project
            .paths
            .set_validated_base_game_root(Some(base.clone()));
        let report = validate_project(&bundle, &project, ProjectValidationTarget::CurrentProject);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PROVINCE_TERRAIN_UNDEFINED")
        );
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn land_without_continent_is_a_nonblocking_warning_and_water_zero_is_ignored() {
        let temp = TempProject::new(
            "land-no-continent",
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;0\n2;2;0;0;sea;false;ocean;0\n3;3;0;0;lake;false;lakes;0\n",
            &[[1, 0, 0], [2, 0, 0], [3, 0, 0]],
        );
        let report = validate_project(
            &temp.bundle(),
            &project(&temp, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "LAND_PROVINCE_NO_CONTINENT"
                && diagnostic.severity == DiagnosticSeverity::Warning
                && !diagnostic.blocks_save
        }));
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "PROVINCE_CONTINENT_UNDEFINED")
        );
    }

    #[test]
    fn adjacency_bounds_checks_do_not_assume_dense_ids() {
        let temp = valid_fixture("adjacency-bounds");
        let mut row = adjacency(7, 500, Some(42));
        row.start = Some([4, 0]);
        row.stop = Some([0, 1]);
        let report = validate_project(
            &sparse_bundle(vec![row]),
            &project(&temp, Vec::new()),
            ProjectValidationTarget::CurrentProject,
        );
        let coordinates = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "ADJACENCY_COORDINATE_OUT_OF_BOUNDS")
            .map(|diagnostic| diagnostic.map_location)
            .collect::<BTreeSet<_>>();
        assert_eq!(coordinates, BTreeSet::from([Some([4, 0]), Some([0, 1])]));
        assert!(
            report
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "ADJACENCY_COORDINATE_OUT_OF_BOUNDS")
                .all(|diagnostic| !diagnostic.blocks_save)
        );
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
    fn sparse_cross_domain_references_are_valid_while_missing_adjacency_endpoints_are_structured() {
        let temp = valid_fixture("sparse-cross-domain");
        let bundle = sparse_bundle(vec![adjacency(7, 500, Some(42))]);
        let mut valid = state(1, &[1, 42, 500]);
        valid.history = StateHistory {
            victory_points: vec![VictoryPoint {
                province_id: 500,
                value: 5,
            }],
            province_buildings: BTreeMap::from([(42, BTreeMap::from([("bunker".to_owned(), 1)]))]),
            ..Default::default()
        };
        let valid_project = project(&temp, vec![document("1.txt", valid, Vec::new())]);
        let report = validate_project(
            &bundle,
            &valid_project,
            ProjectValidationTarget::CurrentProject,
        );
        assert!(!report.blocks_save);
        assert!(
            report
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.blocks_save)
        );

        let bundle = sparse_bundle(vec![adjacency(7, 999, Some(998))]);
        let mut invalid = state(1, &[1, 999]);
        invalid.history = StateHistory {
            victory_points: vec![VictoryPoint {
                province_id: 999,
                value: 5,
            }],
            province_buildings: BTreeMap::from([(999, BTreeMap::from([("bunker".to_owned(), 1)]))]),
            ..Default::default()
        };
        let project = project(&temp, vec![document("1.txt", invalid, Vec::new())]);
        let report = validate_project(&bundle, &project, ProjectValidationTarget::CurrentProject);
        let adjacency_diagnostics = report
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.code.as_str(),
                    "ADJACENCY_FROM_PROVINCE_MISSING"
                        | "ADJACENCY_TO_PROVINCE_MISSING"
                        | "ADJACENCY_THROUGH_PROVINCE_MISSING"
                )
            })
            .map(|diagnostic| diagnostic.province_id)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            adjacency_diagnostics,
            BTreeSet::from([Some(998), Some(999)])
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "cross.province.unknown"
                    && diagnostic.province_id == Some(999))
        );
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

    #[test]
    fn current_project_delta_marks_existing_diagnostics_unchanged() {
        let temp = valid_fixture("current-delta");
        let bundle = temp.bundle();
        let project = project(&temp, vec![document("1.txt", state(1, &[]), Vec::new())]);

        let report = validate_project(&bundle, &project, ProjectValidationTarget::CurrentProject);

        assert_eq!(report.delta.unchanged.len(), 1);
        assert_eq!(report.delta.new.len(), 0);
        assert!(!report.delta.requires_warning_review());
    }

    #[test]
    fn diagnostic_delta_ignores_message_order_severity_and_normalizes_paths() {
        let source = Path::new(r"C:\mod");
        let candidate = Path::new(r"C:\temp\candidate");
        let before = validation_diagnostic(
            ProjectDiagnosticKind::MissingOwner,
            DiagnosticSeverity::Warning,
            source.join("history/states/1-Test.txt"),
            "missing owner",
        );
        let after = validation_diagnostic(
            ProjectDiagnosticKind::MissingOwner,
            DiagnosticSeverity::Error,
            candidate.join("history/states/1-Test.txt"),
            "proprietario ausente",
        );

        let delta = ProjectValidationDelta::new(&[before], &[after], source, candidate);

        assert_eq!(delta.aggravated.len(), 1);
        assert_eq!(delta.aggravated_to_error(), 1);
        assert!(delta.blocks_save());
        assert_eq!(delta.new_errors(), 0);
        assert_eq!(delta.resolved.len(), 0);
    }

    #[test]
    fn diagnostic_delta_classifies_bulk_unchanged_baseline_counts() {
        let root = Path::new("root");
        let mut before = Vec::new();
        let mut after = Vec::new();
        for index in 0..14 {
            let path = format!("history/states/{}-Error.txt", index + 1);
            before.push(validation_diagnostic(
                ProjectDiagnosticKind::MissingOwner,
                DiagnosticSeverity::Error,
                PathBuf::from(&path),
                format!("error {index}"),
            ));
            after.push(validation_diagnostic(
                ProjectDiagnosticKind::MissingOwner,
                DiagnosticSeverity::Error,
                PathBuf::from(path),
                format!("localized error {index}"),
            ));
        }
        for index in 0..128 {
            let path = format!("history/states/{}-Warning.txt", index + 100);
            before.push(validation_diagnostic(
                ProjectDiagnosticKind::EmptyProvinces,
                DiagnosticSeverity::Warning,
                PathBuf::from(&path),
                format!("warning {index}"),
            ));
            after.push(validation_diagnostic(
                ProjectDiagnosticKind::EmptyProvinces,
                DiagnosticSeverity::Warning,
                PathBuf::from(path),
                format!("localized warning {index}"),
            ));
        }

        let delta = ProjectValidationDelta::new(&before, &after, root, root);

        assert_eq!(delta.unchanged.len(), 142);
        assert_eq!(delta.new.len(), 0);
        assert_eq!(delta.new_warnings(), 0);
        assert!(!delta.blocks_save());
        assert!(!delta.requires_warning_review());
        assert!(delta.has_preexisting_errors());
        assert!(delta.has_review_items());
    }

    #[test]
    fn diagnostic_delta_handles_severity_improvement_and_new_warning_review() {
        let root = Path::new("root");
        let before = vec![validation_diagnostic(
            ProjectDiagnosticKind::MissingOwner,
            DiagnosticSeverity::Error,
            PathBuf::from("history/states/1-Test.txt"),
            "missing owner",
        )];
        let after = vec![
            validation_diagnostic(
                ProjectDiagnosticKind::MissingOwner,
                DiagnosticSeverity::Warning,
                PathBuf::from("history/states/1-Test.txt"),
                "missing owner",
            ),
            validation_diagnostic(
                ProjectDiagnosticKind::EmptyProvinces,
                DiagnosticSeverity::Warning,
                PathBuf::from("history/states/2-Test.txt"),
                "empty",
            ),
        ];

        let delta = ProjectValidationDelta::new(&before, &after, root, root);

        assert_eq!(delta.improved.len(), 1);
        assert_eq!(delta.new_warnings(), 1);
        assert!(!delta.blocks_save());
        assert!(delta.requires_warning_review());
    }

    #[test]
    fn diagnostic_delta_classifies_new_error_and_resolved_diagnostic() {
        let root = Path::new("root");
        let before = validation_diagnostic(
            ProjectDiagnosticKind::EmptyProvinces,
            DiagnosticSeverity::Warning,
            PathBuf::from("history/states/1-Resolved.txt"),
            "resolved",
        );
        let after = validation_diagnostic(
            ProjectDiagnosticKind::MissingOwner,
            DiagnosticSeverity::Error,
            PathBuf::from("history/states/2-New.txt"),
            "new error",
        );

        let delta = ProjectValidationDelta::new(&[before], &[after], root, root);

        assert_eq!(delta.new.len(), 1);
        assert_eq!(delta.resolved.len(), 1);
        assert_eq!(delta.new_errors(), 1);
        assert!(delta.blocks_save());
    }

    fn validation_diagnostic(
        kind: ProjectDiagnosticKind,
        severity: DiagnosticSeverity,
        path: PathBuf,
        message: impl Into<String>,
    ) -> ProjectValidationDiagnostic {
        ProjectValidationDiagnostic::from_project(&ProjectDiagnostic::new(
            kind,
            severity,
            Some(path),
            None,
            message,
        ))
    }
}
