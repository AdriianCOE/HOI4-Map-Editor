//! Read-only Strategic Region discovery and parsing.
//!
//! Strategic Region files are directory-merged through `ProjectSources`; this
//! module deliberately has neither editing nor save authority.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use crate::app::state::{PdxBlock, PdxValue, TextSpan, parse_text};

use super::{ProjectSources, ResolvedSource};

pub const STRATEGIC_REGIONS_DIRECTORY: &str = "map/strategicregions";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategicRegion {
    pub id: u32,
    /// The exact script token, retained for a future editor/save implementation.
    pub name_key: Option<String>,
    /// A best-effort source-aware localization lookup. `None` means callers
    /// should display `name_key` directly.
    pub display_name: Option<String>,
    pub has_provinces_field: bool,
    /// Raw, ordered IDs as written. Invalid IDs are preserved for validation.
    pub provinces: Vec<u32>,
    pub naval_terrain: Option<String>,
    pub source: Arc<ResolvedSource>,
    pub span: TextSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategicRegionCoverage {
    NotPresent,
    Complete,
    Incomplete {
        files_visible: usize,
        files_failed: usize,
    },
}

impl StrategicRegionCoverage {
    pub fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategicRegionLoadIssueKind {
    Discovery,
    Read,
    Syntax,
    MissingRegionBlock,
    MissingId,
    InvalidId,
    InvalidProvince,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategicRegionLoadIssue {
    pub kind: StrategicRegionLoadIssueKind,
    pub source: Option<ResolvedSource>,
    pub span: Option<TextSpan>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategicRegionLoadResult {
    pub regions: Vec<StrategicRegion>,
    pub issues: Vec<StrategicRegionLoadIssue>,
    pub coverage: StrategicRegionCoverage,
    pub files_seen: usize,
    pub province_references: usize,
    pub loading_ms: u128,
}

impl Default for StrategicRegionLoadResult {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
            issues: Vec::new(),
            coverage: StrategicRegionCoverage::NotPresent,
            files_seen: 0,
            province_references: 0,
            loading_ms: 0,
        }
    }
}

pub fn load_strategic_regions(sources: &ProjectSources) -> StrategicRegionLoadResult {
    let started = Instant::now();
    let mut result = StrategicRegionLoadResult::default();
    let listing = match sources.list_files(STRATEGIC_REGIONS_DIRECTORY) {
        Ok(listing) => listing,
        Err(error) => {
            result.issues.push(StrategicRegionLoadIssue {
                kind: StrategicRegionLoadIssueKind::Discovery,
                source: None,
                span: None,
                message: format!("Strategic Region discovery failed: {error}"),
            });
            result.coverage = StrategicRegionCoverage::Incomplete {
                files_visible: 0,
                files_failed: 1,
            };
            result.loading_ms = started.elapsed().as_millis();
            return result;
        }
    };
    let files = listing
        .files
        .into_iter()
        .filter(|source| {
            source
                .logical_path
                .extension()
                .is_some_and(|ext| ext == "txt")
        })
        .collect::<Vec<_>>();
    result.files_seen = files.len();
    if files.is_empty() {
        result.loading_ms = started.elapsed().as_millis();
        return result;
    }

    let mut failed_files = 0usize;
    for source in files {
        let issues_before = result.issues.len();
        let text = match sources.read_resolved(&source) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) => {
                    failed_files += 1;
                    result.issues.push(StrategicRegionLoadIssue {
                        kind: StrategicRegionLoadIssueKind::Read,
                        source: Some(source),
                        span: None,
                        message: format!("Strategic Region source is not UTF-8: {error}"),
                    });
                    continue;
                }
            },
            Err(error) => {
                failed_files += 1;
                result.issues.push(StrategicRegionLoadIssue {
                    kind: StrategicRegionLoadIssueKind::Read,
                    source: Some(source),
                    span: None,
                    message: format!("Strategic Region source cannot be read: {error}"),
                });
                continue;
            }
        };
        let document = parse_text(source.logical_path.clone(), text);
        if let Some(diagnostic) = document.diagnostics.first() {
            failed_files += 1;
            result.issues.push(StrategicRegionLoadIssue {
                kind: StrategicRegionLoadIssueKind::Syntax,
                source: Some(source),
                span: Some(diagnostic.span),
                message: format!("Strategic Region syntax error: {}", diagnostic.message),
            });
            continue;
        }
        let source = Arc::new(source);
        let mut found = false;
        for entry in document.entries {
            if entry
                .key
                .as_ref()
                .is_some_and(|key| key.text == "strategic_region")
            {
                found = true;
                if let PdxValue::Block(block) = entry.value {
                    parse_region_block(&source, &block, &mut result);
                } else {
                    result.issues.push(StrategicRegionLoadIssue {
                        kind: StrategicRegionLoadIssueKind::MissingRegionBlock,
                        source: Some((*source).clone()),
                        span: Some(entry.span),
                        message: "strategic_region must contain a block".to_owned(),
                    });
                }
            }
        }
        if !found {
            result.issues.push(StrategicRegionLoadIssue {
                kind: StrategicRegionLoadIssueKind::MissingRegionBlock,
                source: Some((*source).clone()),
                span: None,
                message: "Strategic Region file contains no strategic_region block".to_owned(),
            });
        }
        if result.issues.len() != issues_before {
            failed_files += 1;
        }
    }
    result.province_references = result
        .regions
        .iter()
        .map(|region| region.provinces.len())
        .sum();
    result.coverage = if failed_files == 0 {
        StrategicRegionCoverage::Complete
    } else {
        StrategicRegionCoverage::Incomplete {
            files_visible: result.files_seen,
            files_failed: failed_files,
        }
    };
    resolve_display_names(sources, &mut result.regions);
    result.loading_ms = started.elapsed().as_millis();
    result
}

fn parse_region_block(
    source: &Arc<ResolvedSource>,
    block: &PdxBlock,
    result: &mut StrategicRegionLoadResult,
) {
    let mut id = None;
    let mut name_key = None;
    let mut provinces = None;
    let mut naval_terrain = None;
    for entry in &block.entries {
        let Some(key) = entry.key.as_ref().map(|key| key.text.as_str()) else {
            continue;
        };
        match key {
            "id" => match scalar_text(&entry.value).and_then(|value| value.parse::<u32>().ok()) {
                Some(value) => id = Some(value),
                None => result.issues.push(issue(
                    StrategicRegionLoadIssueKind::InvalidId,
                    source,
                    Some(entry.span),
                    "Strategic Region id must be an unsigned integer",
                )),
            },
            "name" => name_key = scalar_text(&entry.value).map(str::to_owned),
            "naval_terrain" => naval_terrain = scalar_text(&entry.value).map(str::to_owned),
            "provinces" => {
                let mut values = Vec::new();
                match &entry.value {
                    PdxValue::Block(values_block) => {
                        for value in &values_block.entries {
                            match scalar_text(&value.value).and_then(|raw| raw.parse::<u32>().ok())
                            {
                                Some(province_id) if value.key.is_none() => {
                                    values.push(province_id)
                                }
                                _ => result.issues.push(issue(
                                    StrategicRegionLoadIssueKind::InvalidProvince,
                                    source,
                                    Some(value.span),
                                    "Strategic Region provinces must be unsigned integer IDs",
                                )),
                            }
                        }
                        provinces = Some(values);
                    }
                    PdxValue::Scalar(_) => result.issues.push(issue(
                        StrategicRegionLoadIssueKind::InvalidProvince,
                        source,
                        Some(entry.span),
                        "Strategic Region provinces must contain a block",
                    )),
                }
            }
            _ => {}
        }
    }
    let Some(id) = id else {
        result.issues.push(issue(
            StrategicRegionLoadIssueKind::MissingId,
            source,
            Some(block.span),
            "Strategic Region is missing id",
        ));
        return;
    };
    result.regions.push(StrategicRegion {
        id,
        name_key,
        display_name: None,
        has_provinces_field: provinces.is_some(),
        provinces: provinces.unwrap_or_default(),
        naval_terrain,
        source: source.clone(),
        span: block.span,
    });
}

fn scalar_text(value: &PdxValue) -> Option<&str> {
    match value {
        PdxValue::Scalar(value) => Some(value.text.as_str()),
        PdxValue::Block(_) => None,
    }
}

fn issue(
    kind: StrategicRegionLoadIssueKind,
    source: &Arc<ResolvedSource>,
    span: Option<TextSpan>,
    message: impl Into<String>,
) -> StrategicRegionLoadIssue {
    StrategicRegionLoadIssue {
        kind,
        source: Some((**source).clone()),
        span,
        message: message.into(),
    }
}

fn resolve_display_names(sources: &ProjectSources, regions: &mut [StrategicRegion]) {
    let language_header = format!("l_{}:", hoi4_language());
    let mut values = BTreeMap::<String, (ResolvedSource, String)>::new();
    let Ok(listing) = sources.list_files("localisation") else {
        return;
    };
    for source in listing.files {
        let Ok(bytes) = sources.read_resolved(&source) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if !text.contains(&language_header) {
            continue;
        }
        for line in text.lines().map(strip_comment) {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim().trim_start_matches('\u{feff}');
            let Some((_, value)) = value.split_once('"') else {
                continue;
            };
            let Some((value, _)) = value.split_once('"') else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            let replace = values.get(key).is_none_or(|(existing, _)| {
                source.source_kind.precedence_rank() >= existing.source_kind.precedence_rank()
            });
            if replace {
                values.insert(key.to_owned(), (source.clone(), value.to_owned()));
            }
        }
    }
    for region in regions {
        region.display_name = region
            .name_key
            .as_ref()
            .and_then(|key| values.get(key).map(|(_, value)| value.clone()));
    }
}

fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (index, character) in line.char_indices() {
        if character == '"' {
            quoted = !quoted;
        } else if character == '#' && !quoted {
            return &line[..index];
        }
    }
    line
}

fn hoi4_language() -> &'static str {
    match crate::localization::language() {
        "pt-BR" => "braz_por",
        "es-ES" => "spanish",
        "fr-FR" => "french",
        "ru-RU" => "russian",
        "zh-CN" => "simp_chinese",
        _ => "english",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::app::project::{ProjectSources, SourceGeneration};

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hoi4-strategic-regions-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join(STRATEGIC_REGIONS_DIRECTORY)).unwrap();
            fs::write(root.join("map/provinces.bmp"), []).unwrap();
            fs::write(root.join("map/definition.csv"), []).unwrap();
            Self(root)
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn load(root: &TempRoot) -> StrategicRegionLoadResult {
        load_strategic_regions(
            &ProjectSources::discover(&root.0, None, SourceGeneration::new(4)).unwrap(),
        )
    }

    fn load_with_base(root: &TempRoot, base: &TempRoot) -> StrategicRegionLoadResult {
        load_strategic_regions(
            &ProjectSources::discover(&root.0, Some(base.0.clone()), SourceGeneration::new(4))
                .unwrap(),
        )
    }

    #[test]
    fn parses_sparse_regions_comments_bom_and_unknown_blocks() {
        let root = TempRoot::new("valid");
        fs::write(root.0.join("map/strategicregions/one.txt"), "\u{feff}# comment\nstrategic_region = { id = 500 name = \"STRATEGICREGION_500\" provinces = { 1 7 42 } naval_terrain = ocean weather = { period = {} } }").unwrap();
        let result = load(&root);
        assert_eq!(result.coverage, StrategicRegionCoverage::Complete);
        assert_eq!(result.regions[0].id, 500);
        assert_eq!(result.regions[0].provinces, vec![1, 7, 42]);
        assert_eq!(result.regions[0].naval_terrain.as_deref(), Some("ocean"));
    }

    #[test]
    fn malformed_file_is_partial_without_discarding_other_files() {
        let root = TempRoot::new("partial");
        fs::write(
            root.0.join("map/strategicregions/good.txt"),
            "strategic_region = { id = 7 provinces = { 42 } }",
        )
        .unwrap();
        fs::write(
            root.0.join("map/strategicregions/bad.txt"),
            "strategic_region = {",
        )
        .unwrap();
        let result = load(&root);
        assert_eq!(result.regions.len(), 1);
        assert!(matches!(
            result.coverage,
            StrategicRegionCoverage::Incomplete { .. }
        ));
    }

    #[test]
    fn missing_directory_is_not_present_not_a_false_complete_domain() {
        let root = TempRoot::new("absent");
        fs::remove_dir_all(root.0.join(STRATEGIC_REGIONS_DIRECTORY)).unwrap();
        let result = load(&root);
        assert_eq!(result.coverage, StrategicRegionCoverage::NotPresent);
    }

    #[test]
    fn source_override_merge_replace_path_and_localization_use_shared_resolution() {
        let project = TempRoot::new("source-project");
        let base = TempRoot::new("source-base");
        fs::write(
            base.0.join("map/strategicregions/shared.txt"),
            "strategic_region = { id = 1 name = REGION_NAME provinces = { 1 } }",
        )
        .unwrap();
        fs::write(
            base.0.join("map/strategicregions/lower.txt"),
            "strategic_region = { id = 2 provinces = { 7 } }",
        )
        .unwrap();
        fs::create_dir_all(project.0.join("localisation")).unwrap();
        fs::create_dir_all(base.0.join("localisation")).unwrap();
        fs::write(
            base.0.join("localisation/regions.yml"),
            "l_english:\n REGION_NAME:0 \"Base Name\"",
        )
        .unwrap();
        fs::write(
            project.0.join("localisation/regions.yml"),
            "l_english:\n REGION_NAME:0 \"Project Name\"",
        )
        .unwrap();
        fs::write(
            project.0.join("map/strategicregions/shared.txt"),
            "strategic_region = { id = 7 name = REGION_NAME provinces = { 42 } }",
        )
        .unwrap();
        let merged = load_with_base(&project, &base);
        assert_eq!(
            merged
                .regions
                .iter()
                .map(|region| region.id)
                .collect::<Vec<_>>(),
            [2, 7]
        );
        assert_eq!(
            merged.regions[1].display_name.as_deref(),
            Some("Project Name")
        );
        assert_eq!(
            merged.regions[1].source.source_kind,
            super::super::SourceKind::CurrentProject
        );

        fs::write(
            project.0.join("descriptor.mod"),
            "replace_path = \"map/strategicregions\"",
        )
        .unwrap();
        let replaced = load_with_base(&project, &base);
        assert_eq!(
            replaced
                .regions
                .iter()
                .map(|region| region.id)
                .collect::<Vec<_>>(),
            [7]
        );
    }
}
