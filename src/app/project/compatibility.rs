use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::bmp::BmpDecoder;
use image::{ImageDecoder, ImageFormat};

use crate::app::format::{Definition, ParseCsv};
use crate::app::state::load_state_documents;

const SAMPLE_LIMIT: usize = 16;

pub type ProvinceColor = [u8; 3];

/// A stable category for data discovered before a project is loaded into the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompatibilityCode {
    MapDirectoryMissing,
    ProvinceBitmapMissing,
    ProvinceBitmapUnreadable,
    DefinitionMissing,
    DefinitionMalformed,
    DefinitionEmpty,
    InvalidProvinceIdRange,
    SparseProvinceIds,
    DuplicateProvinceId,
    DuplicateProvinceColor,
    BitmapColorMissingDefinition,
    DefinitionColorUnused,
    StatesDirectoryMissing,
    StateReferencesMissingProvince,
    RelatedBitmapUnreadable,
    RelatedBitmapDimensionsMismatch,
}

impl CompatibilityCode {
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::MapDirectoryMissing => "map_directory_missing",
            Self::ProvinceBitmapMissing => "province_bitmap_missing",
            Self::ProvinceBitmapUnreadable => "province_bitmap_unreadable",
            Self::DefinitionMissing => "definition_missing",
            Self::DefinitionMalformed => "definition_malformed",
            Self::DefinitionEmpty => "definition_empty",
            Self::InvalidProvinceIdRange => "invalid_province_id_range",
            Self::SparseProvinceIds => "sparse_province_ids",
            Self::DuplicateProvinceId => "duplicate_province_id",
            Self::DuplicateProvinceColor => "duplicate_province_color",
            Self::BitmapColorMissingDefinition => "bitmap_color_missing_definition",
            Self::DefinitionColorUnused => "definition_color_unused",
            Self::StatesDirectoryMissing => "states_directory_missing",
            Self::StateReferencesMissingProvince => "state_references_missing_province",
            Self::RelatedBitmapUnreadable => "related_bitmap_unreadable",
            Self::RelatedBitmapDimensionsMismatch => "related_bitmap_dimensions_mismatch",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompatibilitySeverity {
    Compatible,
    Warning,
    Unsupported,
    Malformed,
}

impl CompatibilitySeverity {
    const fn rank(self) -> u8 {
        match self {
            Self::Compatible => 0,
            Self::Warning => 1,
            Self::Unsupported => 2,
            Self::Malformed => 3,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompatibilityContext {
    pub path: Option<PathBuf>,
    pub province_id: Option<u32>,
    pub color: Option<ProvinceColor>,
    pub count: Option<usize>,
    pub expected_dimensions: Option<[u32; 2]>,
    pub actual_dimensions: Option<[u32; 2]>,
    pub sample_province_ids: Vec<u32>,
    pub sample_colors: Vec<ProvinceColor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityFinding {
    pub code: CompatibilityCode,
    pub severity: CompatibilitySeverity,
    pub context: CompatibilityContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapCompatibilityMetadata {
    pub path: PathBuf,
    pub dimensions: [u32; 2],
    pub pixel_format: String,
    pub unique_color_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionCompatibilityMetadata {
    pub path: PathBuf,
    pub record_count: usize,
    pub minimum_province_id: Option<u32>,
    pub maximum_province_id: Option<u32>,
    pub occupied_province_ids: usize,
    pub ids_contiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateCompatibilityMetadata {
    pub directory: PathBuf,
    pub directory_exists: bool,
    pub state_editing_available: bool,
    pub state_file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedBitmapCompatibilityMetadata {
    pub path: PathBuf,
    pub dimensions: Option<[u32; 2]>,
    pub pixel_format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityReport {
    pub root: PathBuf,
    pub bitmap: Option<BitmapCompatibilityMetadata>,
    pub definitions: Option<DefinitionCompatibilityMetadata>,
    pub states: StateCompatibilityMetadata,
    pub related_bitmaps: Vec<RelatedBitmapCompatibilityMetadata>,
    pub findings: Vec<CompatibilityFinding>,
}

impl CompatibilityReport {
    pub fn severity(&self) -> CompatibilitySeverity {
        self.findings
            .iter()
            .map(|finding| finding.severity)
            .max_by_key(|severity| severity.rank())
            .unwrap_or(CompatibilitySeverity::Compatible)
    }

    pub fn is_editor_compatible(&self) -> bool {
        matches!(
            self.severity(),
            CompatibilitySeverity::Compatible | CompatibilitySeverity::Warning
        )
    }

    pub fn primary_blocker(&self) -> Option<&CompatibilityFinding> {
        self.findings.iter().find(|finding| {
            matches!(
                finding.severity,
                CompatibilitySeverity::Malformed | CompatibilitySeverity::Unsupported
            )
        })
    }

    fn push(
        &mut self,
        code: CompatibilityCode,
        severity: CompatibilitySeverity,
        context: CompatibilityContext,
    ) {
        self.findings.push(CompatibilityFinding {
            code,
            severity,
            context,
        });
    }
}

/// Inspects project files without constructing a mutable map or writing to the project.
pub fn scan_project(root: impl Into<PathBuf>) -> CompatibilityReport {
    let root = root.into();
    let states_directory = root.join("history").join("states");
    let mut report = CompatibilityReport {
        root: root.clone(),
        bitmap: None,
        definitions: None,
        states: StateCompatibilityMetadata {
            directory: states_directory.clone(),
            directory_exists: states_directory.is_dir(),
            state_editing_available: states_directory.is_dir(),
            state_file_count: 0,
        },
        related_bitmaps: Vec::new(),
        findings: Vec::new(),
    };

    let map_directory = root.join("map");
    if !map_directory.is_dir() {
        report.push(
            CompatibilityCode::MapDirectoryMissing,
            CompatibilitySeverity::Malformed,
            context_for_path(&map_directory),
        );
        inspect_states(&mut report, None);
        return report;
    }

    let bitmap_colors = inspect_province_bitmap(&mut report, &map_directory.join("provinces.bmp"));
    let definitions = inspect_definitions(&mut report, &map_directory.join("definition.csv"));
    if let (Some(bitmap_colors), Some(definitions)) = (&bitmap_colors, &definitions) {
        inspect_color_cross_check(&mut report, bitmap_colors, definitions);
    }
    inspect_related_bitmaps(&mut report, &map_directory);
    inspect_states(&mut report, definitions.as_ref());
    report
}

fn inspect_province_bitmap(
    report: &mut CompatibilityReport,
    path: &Path,
) -> Option<BTreeSet<ProvinceColor>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report.push(
                CompatibilityCode::ProvinceBitmapMissing,
                CompatibilitySeverity::Malformed,
                context_for_path(path),
            );
            return None;
        }
        Err(_) => {
            report.push(
                CompatibilityCode::ProvinceBitmapUnreadable,
                CompatibilitySeverity::Malformed,
                context_for_path(path),
            );
            return None;
        }
    };
    let decoder = match BmpDecoder::new(Cursor::new(&bytes)) {
        Ok(decoder) => decoder,
        Err(_) => {
            report.push(
                CompatibilityCode::ProvinceBitmapUnreadable,
                CompatibilitySeverity::Malformed,
                context_for_path(path),
            );
            return None;
        }
    };
    let dimensions = decoder.dimensions().into();
    let pixel_format = format!("{:?}", decoder.color_type());
    let image = match image::load_from_memory_with_format(&bytes, ImageFormat::Bmp) {
        Ok(image) => image.to_rgb8(),
        Err(_) => {
            report.push(
                CompatibilityCode::ProvinceBitmapUnreadable,
                CompatibilitySeverity::Malformed,
                context_for_path(path),
            );
            return None;
        }
    };
    let colors = image.pixels().map(|pixel| pixel.0).collect::<BTreeSet<_>>();
    report.bitmap = Some(BitmapCompatibilityMetadata {
        path: path.to_owned(),
        dimensions,
        pixel_format,
        unique_color_count: colors.len(),
    });
    Some(colors)
}

fn inspect_definitions(report: &mut CompatibilityReport, path: &Path) -> Option<Vec<Definition>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report.push(
                CompatibilityCode::DefinitionMissing,
                CompatibilitySeverity::Malformed,
                context_for_path(path),
            );
            return None;
        }
        Err(_) => {
            report.push(
                CompatibilityCode::DefinitionMalformed,
                CompatibilitySeverity::Malformed,
                context_for_path(path),
            );
            return None;
        }
    };
    let definitions = match Definition::read_records(bytes.as_slice()) {
        Ok(definitions) => definitions,
        Err(_) => {
            report.push(
                CompatibilityCode::DefinitionMalformed,
                CompatibilitySeverity::Malformed,
                context_for_path(path),
            );
            return None;
        }
    };
    let ids = definitions
        .iter()
        .map(|definition| definition.id)
        .collect::<BTreeSet<_>>();
    report.definitions = Some(DefinitionCompatibilityMetadata {
        path: path.to_owned(),
        record_count: definitions.len(),
        minimum_province_id: ids.first().copied(),
        maximum_province_id: ids.last().copied(),
        occupied_province_ids: ids.len(),
        ids_contiguous: ids.iter().copied().eq(1..=ids.len() as u32),
    });
    if definitions.is_empty() {
        report.push(
            CompatibilityCode::DefinitionEmpty,
            CompatibilitySeverity::Malformed,
            context_for_path(path),
        );
        return Some(definitions);
    }

    let mut ids_to_counts = BTreeMap::new();
    let mut colors_to_counts = BTreeMap::new();
    for definition in &definitions {
        *ids_to_counts.entry(definition.id).or_insert(0usize) += 1;
        *colors_to_counts.entry(definition.rgb).or_insert(0usize) += 1;
    }
    let duplicate_ids = ids_to_counts
        .iter()
        .filter_map(|(&id, &count)| (count > 1).then_some(id))
        .collect::<Vec<_>>();
    let has_duplicate_ids = !duplicate_ids.is_empty();
    if has_duplicate_ids {
        report.push(
            CompatibilityCode::DuplicateProvinceId,
            CompatibilitySeverity::Malformed,
            CompatibilityContext {
                path: Some(path.to_owned()),
                count: Some(duplicate_ids.len()),
                sample_province_ids: bounded(duplicate_ids),
                ..Default::default()
            },
        );
    }
    let duplicate_colors = colors_to_counts
        .iter()
        .filter_map(|(&color, &count)| (count > 1).then_some(color))
        .collect::<Vec<_>>();
    if !duplicate_colors.is_empty() {
        report.push(
            CompatibilityCode::DuplicateProvinceColor,
            CompatibilitySeverity::Malformed,
            CompatibilityContext {
                path: Some(path.to_owned()),
                count: Some(duplicate_colors.len()),
                sample_colors: bounded(duplicate_colors),
                ..Default::default()
            },
        );
    }
    if ids.contains(&0) {
        report.push(
            CompatibilityCode::InvalidProvinceIdRange,
            CompatibilitySeverity::Malformed,
            CompatibilityContext {
                path: Some(path.to_owned()),
                count: Some(ids.len()),
                sample_province_ids: vec![0],
                ..Default::default()
            },
        );
    } else if !has_duplicate_ids && !ids.iter().copied().eq(1..=definitions.len() as u32) {
        report.push(
            CompatibilityCode::SparseProvinceIds,
            CompatibilitySeverity::Warning,
            CompatibilityContext {
                path: Some(path.to_owned()),
                count: Some(ids.len()),
                sample_province_ids: non_contiguous_samples(&ids),
                ..Default::default()
            },
        );
    }
    Some(definitions)
}

fn non_contiguous_samples(ids: &BTreeSet<u32>) -> Vec<u32> {
    ids.iter()
        .copied()
        .enumerate()
        .filter_map(|(index, id)| (id != index as u32 + 1).then_some(id))
        .take(SAMPLE_LIMIT)
        .collect()
}

fn inspect_color_cross_check(
    report: &mut CompatibilityReport,
    bitmap_colors: &BTreeSet<ProvinceColor>,
    definitions: &[Definition],
) {
    let definition_colors = definitions
        .iter()
        .map(|definition| definition.rgb)
        .collect::<BTreeSet<_>>();
    let missing = bitmap_colors
        .difference(&definition_colors)
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        report.push(
            CompatibilityCode::BitmapColorMissingDefinition,
            CompatibilitySeverity::Malformed,
            CompatibilityContext {
                path: report.bitmap.as_ref().map(|bitmap| bitmap.path.clone()),
                count: Some(missing.len()),
                sample_colors: bounded(missing),
                ..Default::default()
            },
        );
    }
    let unused = definition_colors
        .difference(bitmap_colors)
        .copied()
        .collect::<Vec<_>>();
    if !unused.is_empty() {
        report.push(
            CompatibilityCode::DefinitionColorUnused,
            CompatibilitySeverity::Warning,
            CompatibilityContext {
                path: report
                    .definitions
                    .as_ref()
                    .map(|definitions| definitions.path.clone()),
                count: Some(unused.len()),
                sample_colors: bounded(unused),
                ..Default::default()
            },
        );
    }
}

fn inspect_related_bitmaps(report: &mut CompatibilityReport, map_directory: &Path) {
    let expected_dimensions = report.bitmap.as_ref().map(|bitmap| bitmap.dimensions);
    for name in ["rivers.bmp", "heightmap.bmp"] {
        let path = map_directory.join(name);
        if !path.is_file() {
            continue;
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                report.push(
                    CompatibilityCode::RelatedBitmapUnreadable,
                    CompatibilitySeverity::Warning,
                    context_for_path(&path),
                );
                report
                    .related_bitmaps
                    .push(RelatedBitmapCompatibilityMetadata {
                        path,
                        dimensions: None,
                        pixel_format: None,
                    });
                continue;
            }
        };
        let decoder = match BmpDecoder::new(Cursor::new(&bytes)) {
            Ok(decoder) => decoder,
            Err(_) => {
                report.push(
                    CompatibilityCode::RelatedBitmapUnreadable,
                    CompatibilitySeverity::Warning,
                    context_for_path(&path),
                );
                report
                    .related_bitmaps
                    .push(RelatedBitmapCompatibilityMetadata {
                        path,
                        dimensions: None,
                        pixel_format: None,
                    });
                continue;
            }
        };
        let dimensions = decoder.dimensions().into();
        report
            .related_bitmaps
            .push(RelatedBitmapCompatibilityMetadata {
                path: path.clone(),
                dimensions: Some(dimensions),
                pixel_format: Some(format!("{:?}", decoder.color_type())),
            });
        if let Some(expected_dimensions) =
            expected_dimensions.filter(|expected| *expected != dimensions)
        {
            report.push(
                CompatibilityCode::RelatedBitmapDimensionsMismatch,
                CompatibilitySeverity::Warning,
                CompatibilityContext {
                    path: Some(path),
                    expected_dimensions: Some(expected_dimensions),
                    actual_dimensions: Some(dimensions),
                    ..Default::default()
                },
            );
        }
    }
}

fn inspect_states(report: &mut CompatibilityReport, definitions: Option<&Vec<Definition>>) {
    if !report.states.directory_exists {
        report.push(
            CompatibilityCode::StatesDirectoryMissing,
            CompatibilitySeverity::Compatible,
            context_for_path(&report.states.directory),
        );
        return;
    }
    let batch = load_state_documents(&report.states.directory);
    report.states.state_file_count = batch.files_found;
    let Some(definitions) = definitions else {
        return;
    };
    let known_ids = definitions
        .iter()
        .map(|definition| definition.id)
        .collect::<BTreeSet<_>>();
    let missing_ids = batch
        .documents
        .iter()
        .filter_map(|document| document.data.as_ref())
        .flat_map(|state| state.provinces.iter().copied())
        .filter(|id| !known_ids.contains(id))
        .collect::<BTreeSet<_>>();
    if !missing_ids.is_empty() {
        report.push(
            CompatibilityCode::StateReferencesMissingProvince,
            CompatibilitySeverity::Malformed,
            CompatibilityContext {
                path: Some(report.states.directory.clone()),
                count: Some(missing_ids.len()),
                sample_province_ids: bounded(missing_ids),
                ..Default::default()
            },
        );
    }
}

fn context_for_path(path: &Path) -> CompatibilityContext {
    CompatibilityContext {
        path: Some(path.to_owned()),
        ..Default::default()
    }
}

fn bounded<T>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    values.into_iter().take(SAMPLE_LIMIT).collect()
}

#[cfg(test)]
mod tests {
    use super::{CompatibilityCode, CompatibilitySeverity, scan_project};
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
    use std::fs;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str, definitions: &[(u32, [u8; 3])], pixels: &[[u8; 3]]) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hoi4-compatibility-{name}-{}-{}",
                std::process::id(),
                TEST_COUNTER.fetch_add(1, Ordering::Relaxed),
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("map")).unwrap();
            write_bitmap(&root.join("map/provinces.bmp"), pixels);
            write_definitions(&root.join("map/definition.csv"), definitions);
            Self(root)
        }

        fn states(&self, contents: &str) {
            let directory = self.0.join("history/states");
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("1-Test.txt"), contents).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_bitmap(path: &Path, pixels: &[[u8; 3]]) {
        write_bitmap_with_dimensions(path, pixels.len() as u32, 1, |x, _| pixels[x as usize]);
    }

    fn write_bitmap_with_dimensions(
        path: &Path,
        width: u32,
        height: u32,
        color_at: impl Fn(u32, u32) -> [u8; 3],
    ) {
        let image = RgbImage::from_fn(width, height, |x, y| Rgb(color_at(x, y)));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, ImageFormat::Bmp)
            .unwrap();
        fs::write(path, bytes.into_inner()).unwrap();
    }

    fn write_definitions(path: &Path, definitions: &[(u32, [u8; 3])]) {
        let mut text = String::from("0;0;0;0;land;false;unknown;0\r\n");
        for (id, [r, g, b]) in definitions {
            text.push_str(&format!("{id};{r};{g};{b};land;false;plains;1\r\n"));
        }
        fs::write(path, text).unwrap();
    }

    fn has(report: &super::CompatibilityReport, code: CompatibilityCode) -> bool {
        report.findings.iter().any(|finding| finding.code == code)
    }

    #[test]
    fn contiguous_and_tiny_projects_are_compatible_without_dimension_findings() {
        let fixture = Fixture::new(
            "tiny",
            &[(1, [1, 2, 3]), (2, [4, 5, 6])],
            &[[1, 2, 3], [4, 5, 6]],
        );
        let report = scan_project(&fixture.0);
        assert_eq!(report.severity(), CompatibilitySeverity::Compatible);
        assert_eq!(report.bitmap.as_ref().unwrap().dimensions, [2, 1]);
        assert!(report.definitions.as_ref().unwrap().ids_contiguous);
        assert!(!has(&report, CompatibilityCode::SparseProvinceIds));
    }

    #[test]
    fn sparse_ids_are_supported_with_a_non_blocking_structural_finding() {
        let definitions = [
            (1, [1, 1, 1]),
            (7, [2, 2, 2]),
            (42, [3, 3, 3]),
            (500, [4, 4, 4]),
        ];
        let fixture = Fixture::new(
            "sparse",
            &definitions,
            &[[1, 1, 1], [2, 2, 2], [3, 3, 3], [4, 4, 4]],
        );
        let report = scan_project(&fixture.0);
        assert_eq!(report.severity(), CompatibilitySeverity::Warning);
        assert!(report.is_editor_compatible());
        assert!(has(&report, CompatibilityCode::SparseProvinceIds));
        assert_eq!(report.bitmap.as_ref().unwrap().dimensions, [4, 1]);
        let definitions = report.definitions.as_ref().unwrap();
        assert_eq!(definitions.record_count, 4);
        assert_eq!(definitions.minimum_province_id, Some(1));
        assert_eq!(definitions.maximum_province_id, Some(500));
        assert!(!definitions.ids_contiguous);
    }

    #[test]
    fn custom_dimensions_are_compatible_independently_of_contiguous_ids() {
        for [width, height] in [
            [3, 2],
            [64, 64],
            [128, 128],
            [256, 128],
            [192, 64],
            [130, 70],
        ] {
            let fixture = Fixture::new(
                "custom-dimensions",
                &[(1, [1, 2, 3]), (2, [4, 5, 6])],
                &[[1, 2, 3], [4, 5, 6]],
            );
            write_bitmap_with_dimensions(
                &fixture.0.join("map/provinces.bmp"),
                width,
                height,
                |x, y| {
                    if x == 0 && y == 0 {
                        [1, 2, 3]
                    } else {
                        [4, 5, 6]
                    }
                },
            );
            let report = scan_project(&fixture.0);
            assert_eq!(report.severity(), CompatibilitySeverity::Compatible);
            assert_eq!(report.bitmap.as_ref().unwrap().dimensions, [width, height]);
            assert!(report.definitions.as_ref().unwrap().ids_contiguous);
            assert!(!has(&report, CompatibilityCode::SparseProvinceIds));
        }
    }

    #[test]
    fn high_sparse_ids_and_duplicate_ids_are_reported_structurally() {
        let sparse = Fixture::new(
            "high",
            &[(1, [1, 1, 1]), (50_000, [2, 2, 2])],
            &[[1, 1, 1], [2, 2, 2]],
        );
        assert!(has(
            &scan_project(&sparse.0),
            CompatibilityCode::SparseProvinceIds
        ));
        let duplicate = Fixture::new(
            "duplicate-id",
            &[(1, [1, 1, 1]), (1, [2, 2, 2])],
            &[[1, 1, 1], [2, 2, 2]],
        );
        assert!(has(
            &scan_project(&duplicate.0),
            CompatibilityCode::DuplicateProvinceId
        ));
    }

    #[test]
    fn duplicate_colors_and_color_cross_checks_are_bounded_findings() {
        let duplicate = Fixture::new(
            "duplicate-color",
            &[(1, [1, 1, 1]), (2, [1, 1, 1])],
            &[[1, 1, 1]],
        );
        assert!(has(
            &scan_project(&duplicate.0),
            CompatibilityCode::DuplicateProvinceColor
        ));
        let missing = Fixture::new("missing-color", &[(1, [1, 1, 1])], &[[9, 9, 9]]);
        assert!(has(
            &scan_project(&missing.0),
            CompatibilityCode::BitmapColorMissingDefinition
        ));
        let unused = Fixture::new(
            "unused-color",
            &[(1, [1, 1, 1]), (2, [2, 2, 2])],
            &[[1, 1, 1]],
        );
        assert!(has(
            &scan_project(&unused.0),
            CompatibilityCode::DefinitionColorUnused
        ));
    }

    #[test]
    fn province_only_and_state_references_are_distinguished() {
        let province_only = Fixture::new("province-only", &[(1, [1, 2, 3])], &[[1, 2, 3]]);
        let report = scan_project(&province_only.0);
        assert!(!report.states.state_editing_available);
        assert!(has(&report, CompatibilityCode::StatesDirectoryMissing));

        let with_state = Fixture::new("state-reference", &[(1, [1, 2, 3])], &[[1, 2, 3]]);
        with_state.states("state={id=1 state_category=rural provinces={2} history={owner=TAG}}");
        let report = scan_project(&with_state.0);
        assert_eq!(report.states.state_file_count, 1);
        assert!(has(
            &report,
            CompatibilityCode::StateReferencesMissingProvince
        ));
    }

    #[test]
    fn malformed_inputs_and_scans_are_read_only() {
        let bitmap = Fixture::new("bad-bitmap", &[(1, [1, 2, 3])], &[[1, 2, 3]]);
        fs::write(bitmap.0.join("map/provinces.bmp"), b"not a bitmap").unwrap();
        assert!(has(
            &scan_project(&bitmap.0),
            CompatibilityCode::ProvinceBitmapUnreadable
        ));

        let definition = Fixture::new("bad-definition", &[(1, [1, 2, 3])], &[[1, 2, 3]]);
        fs::write(definition.0.join("map/definition.csv"), b"not;valid\r\n").unwrap();
        assert!(has(
            &scan_project(&definition.0),
            CompatibilityCode::DefinitionMalformed
        ));

        let stable = Fixture::new("read-only", &[(1, [1, 2, 3])], &[[1, 2, 3]]);
        let before = fs::read(stable.0.join("map/provinces.bmp")).unwrap();
        let _ = scan_project(&stable.0);
        assert_eq!(
            before,
            fs::read(stable.0.join("map/provinces.bmp")).unwrap()
        );
    }
}
