//! Development-only differential compatibility primitives.
//!
//! The editor's validators remain authoritative for editor behavior. This module
//! normalizes those findings alongside manually captured hoi4modutilities output
//! so developers can review differences without making the VS Code extension a
//! runtime, test, or CI dependency.

use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{DiagnosticSeverity, ProjectDiagnosticKind, ProjectValidationDiagnostic};

pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const HERBIX_REFERENCE_TOOL: &str = "hoi4modutilities";
pub const HERBIX_REFERENCE_VERSION: &str = "0.16.0";

type FindingIdentity = (
    String,
    FindingSeverity,
    Vec<u32>,
    Vec<u32>,
    Option<[u32; 2]>,
    Option<String>,
    Option<String>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Error,
    Warning,
    Information,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityFindingSource {
    Editor,
    Herbix,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CompatibilityFinding {
    pub rule: String,
    pub severity: FindingSeverity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub province_ids: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state_ids: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CompatibilityFinding {
    fn normalize(mut self) -> Self {
        self.province_ids.sort_unstable();
        self.province_ids.dedup();
        self.state_ids.sort_unstable();
        self.state_ids.dedup();
        self.logical_source = self.logical_source.map(normalize_logical_path);
        self
    }

    fn identity(&self) -> FindingIdentity {
        (
            self.rule.clone(),
            self.severity,
            self.province_ids.clone(),
            self.state_ids.clone(),
            self.coordinate,
            self.logical_source.clone(),
            self.detail.clone(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilitySnapshotProvenance {
    pub reference_tool: String,
    pub reference_version: String,
    pub source_identifier: String,
    pub project_fingerprint: String,
    pub normalization_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilitySnapshot {
    pub schema_version: u32,
    pub provenance: CompatibilitySnapshotProvenance,
    #[serde(default)]
    pub findings: Vec<CompatibilityFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilitySnapshotError {
    SchemaVersionMismatch { expected: u32, found: u32 },
    ReferenceToolMismatch { expected: String, found: String },
    ReferenceVersionMismatch { expected: String, found: String },
    StaleReferenceSnapshot { expected: String, found: String },
}

impl CompatibilitySnapshot {
    pub fn normalized(mut self) -> Self {
        self.findings = self
            .findings
            .into_iter()
            .map(CompatibilityFinding::normalize)
            .collect();
        self.findings.sort();
        self.findings.dedup();
        self
    }

    pub fn validate_for_project(
        &self,
        fingerprint: &str,
    ) -> Result<(), CompatibilitySnapshotError> {
        if self.schema_version != SNAPSHOT_SCHEMA_VERSION {
            return Err(CompatibilitySnapshotError::SchemaVersionMismatch {
                expected: SNAPSHOT_SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        if self.provenance.reference_tool != HERBIX_REFERENCE_TOOL {
            return Err(CompatibilitySnapshotError::ReferenceToolMismatch {
                expected: HERBIX_REFERENCE_TOOL.to_owned(),
                found: self.provenance.reference_tool.clone(),
            });
        }
        if self.provenance.reference_version != HERBIX_REFERENCE_VERSION {
            return Err(CompatibilitySnapshotError::ReferenceVersionMismatch {
                expected: HERBIX_REFERENCE_VERSION.to_owned(),
                found: self.provenance.reference_version.clone(),
            });
        }
        if self.provenance.project_fingerprint != fingerprint {
            return Err(CompatibilitySnapshotError::StaleReferenceSnapshot {
                expected: fingerprint.to_owned(),
                found: self.provenance.project_fingerprint.clone(),
            });
        }
        Ok(())
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(&self.clone().normalized())
    }

    pub fn from_toml(value: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(value).map(Self::normalized)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityCase {
    pub name: String,
    pub project_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_game_root: Option<String>,
    #[serde(default)]
    pub enabled_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewClassification {
    OurBug,
    ReferenceLimitation,
    DifferentPolicy,
    FalsePositive,
    NeedsHoi4EngineTest,
    ExpectedDifference,
    Unreviewed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedDifference {
    pub case: String,
    pub rule: String,
    #[serde(default)]
    pub province_ids: Vec<u32>,
    #[serde(default)]
    pub state_ids: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate: Option<[u32; 2]>,
    pub classification: ReviewClassification,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonClass {
    Match,
    EditorOnly,
    ReferenceOnly,
    DifferentDetails,
    UnsupportedByUs,
    UnsupportedByReference,
    PolicyDifference,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityDifference {
    pub class: ComparisonClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<CompatibilityFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herbix: Option<CompatibilityFinding>,
    pub review: ReviewClassification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityComparison {
    pub case: String,
    pub compared_domains: Vec<String>,
    pub differences: Vec<CompatibilityDifference>,
}

impl CompatibilityComparison {
    pub fn is_strictly_clean(&self) -> bool {
        self.differences
            .iter()
            .all(|difference| match difference.class {
                ComparisonClass::Match => true,
                ComparisonClass::PolicyDifference => {
                    difference.review != ReviewClassification::Unreviewed
                }
                _ => false,
            })
    }

    pub fn human_report(&self) -> String {
        let mut report = format!(
            "Case: {}\nCoverage: {}\n",
            self.case,
            self.compared_domains.join(", ")
        );
        let mut current_class = None;
        for difference in &self.differences {
            if current_class != Some(difference.class) {
                let _ = writeln!(report, "\n{:?}", difference.class);
                current_class = Some(difference.class);
            }
            let finding = difference.editor.as_ref().or(difference.herbix.as_ref());
            let rule = finding.map_or("unknown", |finding| finding.rule.as_str());
            let _ = writeln!(report, "  {rule} ({:?})", difference.review);
        }
        report
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

pub fn normalize_editor_diagnostic(
    diagnostic: &ProjectValidationDiagnostic,
    project_root: &Path,
) -> Option<CompatibilityFinding> {
    let rule = editor_rule(diagnostic.kind)?;
    let mut province_ids = diagnostic.related_province_ids.clone();
    if let Some(province_id) = diagnostic.province_id {
        province_ids.push(province_id);
    }
    Some(
        CompatibilityFinding {
            rule: rule.to_owned(),
            severity: normalize_severity(diagnostic.severity),
            province_ids,
            state_ids: diagnostic.state_id.into_iter().collect(),
            coordinate: diagnostic.map_location,
            logical_source: diagnostic
                .source
                .as_ref()
                .map(|source| source.logical_path.to_string_lossy().to_string())
                .or_else(|| {
                    diagnostic
                        .path
                        .as_ref()
                        .map(|path| redact_path(path, project_root))
                }),
            detail: Some(diagnostic.code.clone()),
        }
        .normalize(),
    )
}

/// Normalizes structured Herbix adapter output. The adapter must classify its
/// own loader warning without deriving a rule from human-readable message text.
pub fn normalize_herbix_finding(finding: CompatibilityFinding) -> CompatibilityFinding {
    finding.normalize()
}

pub fn compare_findings(
    case: impl Into<String>,
    compared_domains: impl IntoIterator<Item = String>,
    editor: impl IntoIterator<Item = CompatibilityFinding>,
    herbix: impl IntoIterator<Item = CompatibilityFinding>,
    expected: impl IntoIterator<Item = ExpectedDifference>,
) -> CompatibilityComparison {
    let case = case.into();
    let expected = expected.into_iter().collect::<Vec<_>>();
    let mut editor = editor
        .into_iter()
        .map(CompatibilityFinding::normalize)
        .collect::<Vec<_>>();
    let mut herbix = herbix
        .into_iter()
        .map(CompatibilityFinding::normalize)
        .collect::<Vec<_>>();
    editor.sort();
    herbix.sort();
    let mut differences = Vec::new();

    for editor_finding in editor {
        if let Some(reference_index) = herbix
            .iter()
            .position(|reference| reference.identity() == editor_finding.identity())
        {
            let reference_finding = herbix.remove(reference_index);
            differences.push(make_difference(
                &case,
                ComparisonClass::Match,
                Some(editor_finding),
                Some(reference_finding),
                &expected,
            ));
        } else if let Some(reference_index) = herbix
            .iter()
            .position(|reference| reference.rule == editor_finding.rule)
        {
            let reference_finding = herbix.remove(reference_index);
            differences.push(make_difference(
                &case,
                ComparisonClass::DifferentDetails,
                Some(editor_finding),
                Some(reference_finding),
                &expected,
            ));
        } else {
            differences.push(make_difference(
                &case,
                ComparisonClass::EditorOnly,
                Some(editor_finding),
                None,
                &expected,
            ));
        }
    }
    for reference_finding in herbix {
        let class = if reference_finding.rule.starts_with("strategic_region.") {
            ComparisonClass::UnsupportedByUs
        } else {
            ComparisonClass::ReferenceOnly
        };
        differences.push(make_difference(
            &case,
            class,
            None,
            Some(reference_finding),
            &expected,
        ));
    }
    differences.sort_by(|left, right| {
        (left.class, &left.editor, &left.herbix).cmp(&(right.class, &right.editor, &right.herbix))
    });
    CompatibilityComparison {
        case,
        compared_domains: compared_domains.into_iter().collect(),
        differences,
    }
}

/// Fingerprints only caller-selected project inputs and their paths relative to
/// `root`; source bytes and absolute user paths never enter a committed snapshot.
pub fn fingerprint_inputs(
    root: &Path,
    paths: impl IntoIterator<Item = PathBuf>,
) -> std::io::Result<String> {
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    let mut hasher = Sha256::new();
    for path in paths {
        let logical = redact_path(&path, root);
        hasher.update(logical.as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(path)?);
        hasher.update([0]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn make_difference(
    case: &str,
    mut class: ComparisonClass,
    editor: Option<CompatibilityFinding>,
    herbix: Option<CompatibilityFinding>,
    expected: &[ExpectedDifference],
) -> CompatibilityDifference {
    let review = finding_for_review(&editor, &herbix, case, expected)
        .map(|difference| difference.classification)
        .unwrap_or(ReviewClassification::Unreviewed);
    if matches!(
        review,
        ReviewClassification::DifferentPolicy | ReviewClassification::ExpectedDifference
    ) && !matches!(class, ComparisonClass::Match)
    {
        class = ComparisonClass::PolicyDifference;
    }
    CompatibilityDifference {
        class,
        editor,
        herbix,
        review,
    }
}

fn finding_for_review<'a>(
    editor: &Option<CompatibilityFinding>,
    herbix: &Option<CompatibilityFinding>,
    case: &str,
    expected: &'a [ExpectedDifference],
) -> Option<&'a ExpectedDifference> {
    let finding = editor.as_ref().or(herbix.as_ref())?;
    expected.iter().find(|difference| {
        difference.case == case
            && difference.rule == finding.rule
            && difference.province_ids == finding.province_ids
            && difference.state_ids == finding.state_ids
            && difference.coordinate == finding.coordinate
    })
}

fn normalize_severity(severity: DiagnosticSeverity) -> FindingSeverity {
    match severity {
        DiagnosticSeverity::Error => FindingSeverity::Error,
        DiagnosticSeverity::Warning => FindingSeverity::Warning,
        DiagnosticSeverity::Info => FindingSeverity::Information,
    }
}

fn editor_rule(kind: ProjectDiagnosticKind) -> Option<&'static str> {
    Some(match kind {
        ProjectDiagnosticKind::MapXCrossing => "province.x_crossing",
        ProjectDiagnosticKind::ProvinceOnePixel => "province.one_pixel",
        ProjectDiagnosticKind::ProvinceDisconnectedComponents => "province.disconnected",
        ProjectDiagnosticKind::DuplicateProvinceId => "province.duplicate_id",
        ProjectDiagnosticKind::DuplicateProvinceRgb => "province.duplicate_color",
        ProjectDiagnosticKind::MissingDefinitionForColor => "province.missing_definition",
        ProjectDiagnosticKind::UnusedDefinition => "province.definition_absent_from_bitmap",
        ProjectDiagnosticKind::ProvinceTerrainUndefined => "province.undefined_terrain",
        ProjectDiagnosticKind::ProvinceContinentUndefined => "province.undefined_continent",
        ProjectDiagnosticKind::UnknownProvince => "state.missing_province",
        ProjectDiagnosticKind::ProvinceInMultipleStates => "state.duplicate_membership",
        ProjectDiagnosticKind::VictoryPointOutsideState => "state.vp_outside_state",
        ProjectDiagnosticKind::SeaOrLakeAssigned => "state.sea_province",
        ProjectDiagnosticKind::AdjacencyFromProvinceMissing => "adjacency.missing_from",
        ProjectDiagnosticKind::AdjacencyToProvinceMissing => "adjacency.missing_to",
        ProjectDiagnosticKind::AdjacencyThroughProvinceMissing => "adjacency.missing_through",
        ProjectDiagnosticKind::RiverNoSource => "river.no_source",
        ProjectDiagnosticKind::RiverMultipleSources => "river.multiple_sources",
        ProjectDiagnosticKind::RiverInvalidFlowMarker => "river.invalid_flow_marker",
        ProjectDiagnosticKind::RiverPossibleLoop => "river.possible_loop",
        ProjectDiagnosticKind::RailwayProvinceCountMismatch => "railway.count_mismatch",
        ProjectDiagnosticKind::RailwayProvinceMissing => "railway.missing_province",
        ProjectDiagnosticKind::RailwaySegmentNotAdjacent => "railway.non_adjacent",
        ProjectDiagnosticKind::SupplyNodeProvinceMissing => "supply.missing_province",
        _ => return None,
    })
}

fn redact_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn normalize_logical_path(path: String) -> String {
    path.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::project::ProjectValidationDomain;

    fn finding(rule: &str) -> CompatibilityFinding {
        CompatibilityFinding {
            rule: rule.to_owned(),
            severity: FindingSeverity::Error,
            province_ids: vec![42, 7, 42],
            state_ids: Vec::new(),
            coordinate: Some([3, 2]),
            logical_source: Some("C:\\work\\map\\provinces.bmp".to_owned()),
            detail: None,
        }
    }

    #[test]
    fn exact_match_is_structured_and_deterministic() {
        let comparison = compare_findings(
            "synthetic_x_crossing",
            ["province_geometry".to_owned()],
            [finding("province.x_crossing")],
            [finding("province.x_crossing")],
            [],
        );
        assert_eq!(comparison.differences[0].class, ComparisonClass::Match);
        assert!(comparison.is_strictly_clean());
        assert_eq!(
            comparison.differences[0]
                .editor
                .as_ref()
                .unwrap()
                .province_ids,
            [7, 42]
        );
        assert_eq!(comparison.to_json().unwrap(), comparison.to_json().unwrap());
    }

    #[test]
    fn narrow_policy_approval_does_not_mask_other_findings() {
        let expected = ExpectedDifference {
            case: "tiny_map".to_owned(),
            rule: "project.dimension.non_256".to_owned(),
            province_ids: Vec::new(),
            state_ids: Vec::new(),
            coordinate: None,
            classification: ReviewClassification::DifferentPolicy,
            note: None,
        };
        let reference = CompatibilityFinding {
            rule: "project.dimension.non_256".to_owned(),
            severity: FindingSeverity::Warning,
            province_ids: Vec::new(),
            state_ids: Vec::new(),
            coordinate: None,
            logical_source: None,
            detail: None,
        };
        let comparison = compare_findings("tiny_map", [], [], [reference], [expected]);
        assert_eq!(
            comparison.differences[0].class,
            ComparisonClass::PolicyDifference
        );
        assert_eq!(
            comparison.differences[0].review,
            ReviewClassification::DifferentPolicy
        );
    }

    #[test]
    fn strategic_regions_are_explicitly_unsupported_by_editor() {
        let comparison = compare_findings(
            "reference_domain",
            ["strategic_regions".to_owned()],
            [],
            [finding("strategic_region.missing_province")],
            [],
        );
        assert_eq!(
            comparison.differences[0].class,
            ComparisonClass::UnsupportedByUs
        );
    }

    #[test]
    fn additional_or_different_details_remain_visible_for_review() {
        let mut reference = finding("river.no_source");
        reference.coordinate = Some([1, 1]);
        let comparison = compare_findings(
            "river_fixture",
            ["rivers".to_owned()],
            [finding("river.no_source"), finding("river.possible_loop")],
            [reference],
            [],
        );
        assert!(
            comparison
                .differences
                .iter()
                .any(|difference| difference.class == ComparisonClass::DifferentDetails)
        );
        assert!(
            comparison
                .differences
                .iter()
                .any(|difference| difference.class == ComparisonClass::EditorOnly)
        );
        assert!(!comparison.is_strictly_clean());
    }

    #[test]
    fn snapshots_reject_stale_or_drifted_reference_metadata() {
        let snapshot = CompatibilitySnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            provenance: CompatibilitySnapshotProvenance {
                reference_tool: HERBIX_REFERENCE_TOOL.to_owned(),
                reference_version: HERBIX_REFERENCE_VERSION.to_owned(),
                source_identifier: "local-wrapper".to_owned(),
                project_fingerprint: "sha256:known".to_owned(),
                normalization_version: 1,
            },
            findings: vec![finding("province.x_crossing")],
        };
        assert!(matches!(
            snapshot.validate_for_project("sha256:changed"),
            Err(CompatibilitySnapshotError::StaleReferenceSnapshot { .. })
        ));
        let mut mismatched = snapshot.clone();
        mismatched.provenance.reference_version = "0.17.0".to_owned();
        assert!(matches!(
            mismatched.validate_for_project("sha256:known"),
            Err(CompatibilitySnapshotError::ReferenceVersionMismatch { .. })
        ));
        let encoded = snapshot.to_toml().unwrap();
        assert_eq!(
            CompatibilitySnapshot::from_toml(&encoded)
                .unwrap()
                .to_toml()
                .unwrap(),
            encoded
        );
    }

    #[test]
    fn editor_mapping_and_fingerprint_are_semantic_and_path_safe() {
        let diagnostic = ProjectValidationDiagnostic {
            kind: ProjectDiagnosticKind::MapXCrossing,
            severity: DiagnosticSeverity::Warning,
            domain: ProjectValidationDomain::Province,
            code: "MAP_X_CROSSING".to_owned(),
            message_key: "ignored".to_owned(),
            path: Some(PathBuf::from("map/provinces.bmp")),
            related_path: None,
            span: None,
            province_id: Some(7),
            state_id: None,
            map_location: Some([3, 2]),
            related_province_ids: vec![42, 1],
            source: None,
            blocks_save: false,
            message: "human wording is not part of comparison identity".to_owned(),
        };
        let normalized = normalize_editor_diagnostic(&diagnostic, Path::new(".")).unwrap();
        assert_eq!(normalized.rule, "province.x_crossing");
        assert_eq!(normalized.province_ids, [1, 7, 42]);
        assert_eq!(
            normalized.logical_source.as_deref(),
            Some("map/provinces.bmp")
        );

        let root = std::env::temp_dir().join(format!(
            "hoi4-map-editor-compatibility-fingerprint-{}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("map")).unwrap();
        let input = root.join("map/input.txt");
        fs::write(&input, "fixture").unwrap();
        assert_eq!(
            fingerprint_inputs(&root, [input.clone()]).unwrap(),
            fingerprint_inputs(&root, [input]).unwrap()
        );
        let _ = fs::remove_dir_all(root);
    }
}
