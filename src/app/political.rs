use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

use ahash::{AHashMap, AHashSet};
use image::RgbaImage;

use super::map::Color;
use super::map_layers::{MapBaseView, political_fallback_color};
use super::project::{ProjectSources, ResolvedSource, SourceLookup};

#[derive(Debug, Clone)]
pub struct CountryMetadata {
    pub tag: String,
    pub display_name: String,
    pub color: Option<Color>,
    pub color_resolution: CountryColorResolution,
    pub flag: Option<RgbaImage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountryColorResolutionKind {
    Resolved,
    CountryTagUnknown,
    ColorMissing,
    ColorParseFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryColorResolution {
    pub kind: CountryColorResolutionKind,
    pub tag: String,
    pub rgb: Option<Color>,
    pub source_path: Option<PathBuf>,
    pub source_type: Option<&'static str>,
    pub source: Option<ResolvedSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoliticalOwnerResolution {
    OwnerMissing,
    Country(CountryColorResolution),
}

impl PoliticalOwnerResolution {
    pub fn color_for_map(&self) -> Color {
        match self {
            Self::OwnerMissing => [0x68, 0x68, 0x68],
            Self::Country(resolution) => resolution
                .rgb
                .unwrap_or_else(|| political_fallback_color(&resolution.tag)),
        }
    }

    pub fn diagnostic_lines(&self) -> Vec<String> {
        match self {
            Self::OwnerMissing => vec![
                "Owner tag: <missing>".to_owned(),
                "Resolution: owner missing (neutral grey)".to_owned(),
            ],
            Self::Country(resolution) => {
                let mut lines = vec![format!("Owner tag: {}", resolution.tag)];
                lines.push(format!("Resolved country tag: {}", resolution.tag));
                match resolution.rgb {
                    Some([red, green, blue]) => {
                        lines.push(format!("Resolved RGB: {red}, {green}, {blue}"));
                    }
                    None => lines.push("Resolved RGB: <fallback>".to_owned()),
                }
                if let Some(source_type) = resolution.source_type {
                    lines.push(format!("Resolution source type: {source_type}"));
                }
                if let Some(path) = &resolution.source_path {
                    lines.push(format!("Resolution source path: {}", path.display()));
                }
                if let Some(source) = &resolution.source {
                    lines.push(format!(
                        "Resolution source: {} ({:?}, generation {})",
                        source.logical_path.display(),
                        source.source_kind,
                        source.project_generation.value()
                    ));
                }
                if resolution.kind != CountryColorResolutionKind::Resolved {
                    lines.push(format!(
                        "Fallback reason: {}",
                        match resolution.kind {
                            CountryColorResolutionKind::CountryTagUnknown => "country tag unknown",
                            CountryColorResolutionKind::ColorMissing =>
                                "country known but color missing",
                            CountryColorResolutionKind::ColorParseFailed =>
                                "country color parse failure",
                            CountryColorResolutionKind::Resolved => unreachable!(),
                        }
                    ));
                }
                lines
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PoliticalCountryCatalog {
    sources: ProjectSources,
    colors_file: Option<CountryColorFile>,
    country_files: BTreeMap<String, ResolvedSource>,
    country_histories: BTreeMap<String, CountryHistory>,
    localized_names: BTreeMap<String, LocalizedName>,
    known_tags: BTreeSet<String>,
    metadata: BTreeMap<String, CountryMetadata>,
}

impl PoliticalCountryCatalog {
    pub fn load(sources: ProjectSources) -> Self {
        let country_histories = scan_country_histories(&sources);
        let localized_names = scan_localized_country_names(&sources);
        let colors_file = scan_country_colors_file(&sources);
        let country_files = scan_country_files(&sources);
        let mut known_tags = country_files.keys().cloned().collect::<BTreeSet<_>>();
        known_tags.extend(country_histories.keys().cloned());
        known_tags.extend(
            localized_names
                .keys()
                .filter_map(|key| localization_tag(key).map(str::to_owned)),
        );
        if let Some(file) = &colors_file {
            known_tags.extend(file.colors.keys().cloned());
        }
        Self {
            sources,
            colors_file,
            country_files,
            country_histories,
            localized_names,
            known_tags,
            metadata: BTreeMap::new(),
        }
    }

    pub fn resolve_tags(&mut self, tags: impl IntoIterator<Item = String>) {
        for tag in tags {
            if self.metadata.contains_key(&tag) {
                continue;
            }
            let color_resolution = self.resolve_color(&tag);
            let metadata = CountryMetadata {
                tag: tag.clone(),
                display_name: self.resolve_display_name(&tag),
                color: color_resolution.rgb,
                color_resolution,
                flag: self.load_flag(&tag),
            };
            self.metadata.insert(tag, metadata);
        }
    }

    pub fn metadata(&self, tag: &str) -> Option<&CountryMetadata> {
        self.metadata.get(tag)
    }

    pub fn color_for(&self, tag: &str) -> Color {
        self.owner_resolution(Some(tag)).color_for_map()
    }

    pub fn owner_resolution(&self, owner: Option<&str>) -> PoliticalOwnerResolution {
        let Some(tag) = owner.map(str::trim).filter(|tag| !tag.is_empty()) else {
            return PoliticalOwnerResolution::OwnerMissing;
        };
        PoliticalOwnerResolution::Country(
            self.metadata
                .get(tag)
                .map(|metadata| metadata.color_resolution.clone())
                .unwrap_or_else(|| self.resolve_color(tag)),
        )
    }

    fn resolve_display_name(&self, tag: &str) -> String {
        if let Some(name) = self.localized_names.get(tag) {
            return name.value.clone();
        }
        if let Some(ideology) = self
            .country_histories
            .get(tag)
            .and_then(|history| history.ruling_ideology.as_deref())
            && let Some(name) = self.localized_names.get(&format!("{tag}_{ideology}"))
        {
            return name.value.clone();
        }
        // Without a history file, use a documented stable order instead of
        // claiming an ideology: neutrality, democratic, fascism, communism.
        if !self.country_histories.contains_key(tag) {
            for ideology in ["neutrality", "democratic", "fascism", "communism"] {
                if let Some(name) = self.localized_names.get(&format!("{tag}_{ideology}")) {
                    return name.value.clone();
                }
            }
        }
        tag.to_owned()
    }

    fn resolve_color(&self, tag: &str) -> CountryColorResolution {
        if !self.known_tags.contains(tag) {
            return CountryColorResolution {
                kind: CountryColorResolutionKind::CountryTagUnknown,
                tag: tag.to_owned(),
                rgb: None,
                source_path: None,
                source_type: None,
                source: None,
            };
        }
        let colors = self.colors_file.as_ref().and_then(|file| {
            file.colors
                .get(tag)
                .copied()
                .map(|color| (color, &file.source))
        });
        let country = self
            .country_files
            .get(tag)
            .map(|source| (parse_country_color(&self.sources, source), source));
        match (colors, country) {
            (Some((color, color_source)), Some((_country, country_source)))
                if color_source.source_kind.precedence_rank()
                    >= country_source.source_kind.precedence_rank() =>
            {
                color.to_resolution(tag, color_source, "common/countries/colors.txt")
            }
            (_, Some((color, source))) => {
                color.to_resolution(tag, source, "per-country definition")
            }
            (Some((color, source)), None) => {
                color.to_resolution(tag, source, "common/countries/colors.txt")
            }
            (None, None) => CountryColorResolution {
                kind: CountryColorResolutionKind::ColorMissing,
                tag: tag.to_owned(),
                rgb: None,
                source_path: None,
                source_type: None,
                source: None,
            },
        }
    }

    fn load_flag(&self, tag: &str) -> Option<RgbaImage> {
        for extension in ["tga", "dds", "png", "bmp"] {
            let logical = PathBuf::from("gfx/flags").join(format!("{tag}.{extension}"));
            if let Ok(SourceLookup::Found(source)) = self.sources.resolve(logical)
                && let Ok(bytes) = self.sources.read_resolved(&source)
                && let Ok(image) = image::load_from_memory(&bytes)
            {
                return Some(image.to_rgba8());
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoliticalProvince {
    pub id: u32,
    pub is_land: bool,
    pub center: [f64; 2],
    pub pixel_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoliticalStateOwnership {
    pub owner: Option<String>,
    pub provinces: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PoliticalLabel {
    pub tag: String,
    pub display_name: String,
    pub anchor: [f64; 2],
    pub territory_pixels: u64,
    pub flag_available: bool,
}

/// Geometry that is independent from a view's ownership or resource data.
///
/// It is intentionally reusable by Political and Resources views: constructing
/// either view must not rebuild a province lookup and the full adjacency graph
/// for every State label.
#[derive(Debug, Clone)]
pub(crate) struct TerritoryAnchorIndex {
    provinces_by_id: BTreeMap<u32, PoliticalProvince>,
    neighbors: AHashMap<u32, AHashSet<u32>>,
}

impl TerritoryAnchorIndex {
    pub(crate) fn new(provinces: &[PoliticalProvince], adjacency_pairs: &[(u32, u32)]) -> Self {
        let provinces_by_id = provinces
            .iter()
            .copied()
            .map(|province| (province.id, province))
            .collect();
        // `iter_boundaries` reports many pixel edges for the same province
        // border. Hash sets make this one-time de-duplication linear instead
        // of repeatedly rebalancing ordered sets for every duplicate edge.
        let mut neighbors = AHashMap::<u32, AHashSet<u32>>::new();
        for &(left, right) in adjacency_pairs {
            if left != right {
                neighbors.entry(left).or_default().insert(right);
                neighbors.entry(right).or_default().insert(left);
            }
        }
        Self {
            provinces_by_id,
            neighbors,
        }
    }

    pub(crate) fn territory_anchor(&self, owned: &BTreeSet<u32>) -> Option<([f64; 2], u64)> {
        let owned = owned
            .iter()
            .copied()
            .filter(|id| {
                self.provinces_by_id
                    .get(id)
                    .is_some_and(|province| province.is_land)
            })
            .collect::<BTreeSet<_>>();
        let component = largest_component(&owned, &self.neighbors, &self.provinces_by_id)?;
        let pixels = component
            .iter()
            .filter_map(|id| self.provinces_by_id.get(id))
            .map(|province| province.pixel_count)
            .sum();
        Some((
            anchor_for_component(&component, &self.provinces_by_id)?,
            pixels,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoliticalLabelVisibility {
    Hidden,
    FlagOnly,
    NameOnly,
    NameAndFlag,
}

pub const fn political_labels_visible_in_view(view: MapBaseView) -> bool {
    matches!(view, MapBaseView::Political)
}

pub fn prepare_country_labels(
    metadata: &PoliticalCountryCatalog,
    states: &[PoliticalStateOwnership],
    provinces: &[PoliticalProvince],
    adjacency_pairs: &[(u32, u32)],
) -> Vec<PoliticalLabel> {
    let index = TerritoryAnchorIndex::new(provinces, adjacency_pairs);
    prepare_country_labels_with_index(metadata, states, &index)
}

pub(crate) fn prepare_country_labels_with_index(
    metadata: &PoliticalCountryCatalog,
    states: &[PoliticalStateOwnership],
    index: &TerritoryAnchorIndex,
) -> Vec<PoliticalLabel> {
    let mut owned_by_country = BTreeMap::<String, BTreeSet<u32>>::new();
    for state in states {
        let Some(owner) = state
            .owner
            .as_deref()
            .filter(|owner| !owner.trim().is_empty())
        else {
            continue;
        };
        let owned = owned_by_country.entry(owner.to_owned()).or_default();
        owned.extend(state.provinces.iter().copied().filter(|province_id| {
            index
                .provinces_by_id
                .get(province_id)
                .is_some_and(|province| province.is_land)
        }));
    }

    owned_by_country
        .into_iter()
        .filter_map(|(tag, owned)| {
            let main_component =
                largest_component(&owned, &index.neighbors, &index.provinces_by_id)?;
            let territory_pixels = main_component
                .iter()
                .filter_map(|province_id| index.provinces_by_id.get(province_id))
                .map(|province| province.pixel_count)
                .sum();
            let anchor = anchor_for_component(&main_component, &index.provinces_by_id)?;
            let country = metadata.metadata(&tag)?;
            Some(PoliticalLabel {
                tag,
                display_name: country.display_name.clone(),
                anchor,
                territory_pixels,
                flag_available: country.flag.is_some(),
            })
        })
        .collect()
}

pub fn political_label_visibility(
    territory_pixels: u64,
    zoom: f64,
    flag_available: bool,
) -> PoliticalLabelVisibility {
    if (territory_pixels < 4 && zoom < 2.5) || (territory_pixels < 24 && zoom < 1.25) {
        PoliticalLabelVisibility::Hidden
    } else if territory_pixels < 100 && zoom < 2.5 {
        if flag_available {
            PoliticalLabelVisibility::FlagOnly
        } else {
            PoliticalLabelVisibility::Hidden
        }
    } else if territory_pixels < 100 && !flag_available {
        PoliticalLabelVisibility::NameOnly
    } else if territory_pixels < 500 && zoom < 0.75 {
        if flag_available {
            PoliticalLabelVisibility::FlagOnly
        } else {
            PoliticalLabelVisibility::NameOnly
        }
    } else if flag_available {
        PoliticalLabelVisibility::NameAndFlag
    } else {
        PoliticalLabelVisibility::NameOnly
    }
}

fn largest_component(
    owned: &BTreeSet<u32>,
    neighbors: &AHashMap<u32, AHashSet<u32>>,
    provinces: &BTreeMap<u32, PoliticalProvince>,
) -> Option<BTreeSet<u32>> {
    let mut remaining = owned.clone();
    let mut largest = None::<(u64, BTreeSet<u32>)>;
    while let Some(&start) = remaining.iter().next() {
        let mut component = BTreeSet::new();
        let mut queue = VecDeque::from([start]);
        remaining.remove(&start);
        while let Some(province_id) = queue.pop_front() {
            component.insert(province_id);
            for neighbor in neighbors.get(&province_id).into_iter().flatten() {
                if remaining.remove(neighbor) {
                    queue.push_back(*neighbor);
                }
            }
        }
        let pixels = component
            .iter()
            .filter_map(|province_id| provinces.get(province_id))
            .map(|province| province.pixel_count)
            .sum();
        if largest
            .as_ref()
            .is_none_or(|(largest_pixels, _)| pixels > *largest_pixels)
        {
            largest = Some((pixels, component));
        }
    }
    largest.map(|(_, component)| component)
}

fn anchor_for_component(
    component: &BTreeSet<u32>,
    provinces: &BTreeMap<u32, PoliticalProvince>,
) -> Option<[f64; 2]> {
    let total_pixels = component
        .iter()
        .filter_map(|province_id| provinces.get(province_id))
        .map(|province| province.pixel_count)
        .sum::<u64>();
    if total_pixels == 0 {
        return None;
    }
    let average = component
        .iter()
        .filter_map(|province_id| provinces.get(province_id))
        .fold([0.0, 0.0], |sum, province| {
            [
                sum[0] + province.center[0] * province.pixel_count as f64,
                sum[1] + province.center[1] * province.pixel_count as f64,
            ]
        });
    let average = [
        average[0] / total_pixels as f64,
        average[1] / total_pixels as f64,
    ];
    component
        .iter()
        .filter_map(|province_id| provinces.get(province_id))
        .min_by(|left, right| {
            squared_distance(left.center, average)
                .total_cmp(&squared_distance(right.center, average))
        })
        .map(|province| province.center)
}

fn squared_distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    dx * dx + dy * dy
}

fn scan_country_files(sources: &ProjectSources) -> BTreeMap<String, ResolvedSource> {
    let mut files = BTreeMap::new();
    for source in listed_text_files(sources, "common/country_tags") {
        let Some(text) = read_text(sources, &source) else {
            continue;
        };
        for line in text.lines().map(strip_comment) {
            let Some((tag, target)) = line.split_once('=') else {
                continue;
            };
            let tag = tag.trim();
            let target = target.trim().trim_matches('"');
            if is_country_tag(tag) && !target.is_empty() {
                let logical = PathBuf::from("common").join(target);
                if let Ok(SourceLookup::Found(country)) = sources.resolve(logical) {
                    insert_preferred(&mut files, tag.to_owned(), country);
                }
            }
        }
    }
    for source in listed_text_files(sources, "common/countries") {
        let Some(tag) = source
            .logical_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split_whitespace().next())
            .filter(|tag| is_country_tag(tag))
        else {
            continue;
        };
        insert_preferred(&mut files, tag.to_owned(), source);
    }
    files
}

#[derive(Debug, Clone)]
struct CountryColorFile {
    source: ResolvedSource,
    colors: BTreeMap<String, ParsedColor>,
}

#[derive(Debug, Clone)]
struct CountryHistory {
    ruling_ideology: Option<String>,
}

#[derive(Debug, Clone)]
struct LocalizedName {
    value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParsedColor {
    Missing,
    Value(Color),
    Invalid,
}

impl ParsedColor {
    fn to_resolution(
        self,
        tag: &str,
        source: &ResolvedSource,
        source_type: &'static str,
    ) -> CountryColorResolution {
        match self {
            Self::Value(rgb) => CountryColorResolution {
                kind: CountryColorResolutionKind::Resolved,
                tag: tag.to_owned(),
                rgb: Some(rgb),
                source_path: source.filesystem_path().map(Path::to_owned),
                source_type: Some(source_type),
                source: Some(source.clone()),
            },
            Self::Invalid => CountryColorResolution {
                kind: CountryColorResolutionKind::ColorParseFailed,
                tag: tag.to_owned(),
                rgb: None,
                source_path: source.filesystem_path().map(Path::to_owned),
                source_type: Some(source_type),
                source: Some(source.clone()),
            },
            Self::Missing => CountryColorResolution {
                kind: CountryColorResolutionKind::ColorMissing,
                tag: tag.to_owned(),
                rgb: None,
                source_path: source.filesystem_path().map(Path::to_owned),
                source_type: Some(source_type),
                source: Some(source.clone()),
            },
        }
    }
}

fn scan_country_colors_file(sources: &ProjectSources) -> Option<CountryColorFile> {
    let SourceLookup::Found(source) = sources.resolve("common/countries/colors.txt").ok()? else {
        return None;
    };
    let text = read_text(sources, &source)?;
    Some(CountryColorFile {
        source,
        colors: parse_multi_country_colors(&text),
    })
}

fn scan_country_histories(sources: &ProjectSources) -> BTreeMap<String, CountryHistory> {
    let mut histories = BTreeMap::new();
    let mut origins = BTreeMap::new();
    for source in listed_text_files(sources, "history/countries") {
        let Some(tag) = source
            .logical_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| {
                name.split(|character: char| character.is_whitespace() || character == '-')
                    .next()
            })
            .filter(|tag| is_country_tag(tag))
        else {
            continue;
        };
        let tag = tag.to_owned();
        let Some(text) = read_text(sources, &source) else {
            continue;
        };
        let ruling_ideology = parse_ruling_ideology(&text);
        if should_replace(origins.get(&tag), &source) {
            origins.insert(tag.clone(), source);
            histories.insert(tag, CountryHistory { ruling_ideology });
        }
    }
    histories
}

fn scan_localized_country_names(sources: &ProjectSources) -> BTreeMap<String, LocalizedName> {
    let header = format!("l_{}:", hoi4_language());
    let mut names = BTreeMap::new();
    let mut origins = BTreeMap::new();
    for source in listed_text_files(sources, "localisation") {
        let Some(text) = read_text(sources, &source) else {
            continue;
        };
        if !text.contains(&header) {
            continue;
        }
        for line in text.lines().map(strip_comment) {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim().trim_start_matches('\u{feff}');
            if localization_tag(key).is_none() {
                continue;
            }
            let Some((_, localized)) = value.split_once('"') else {
                continue;
            };
            let Some((localized, _)) = localized.split_once('"') else {
                continue;
            };
            if !localized.is_empty() && should_replace(origins.get(key), &source) {
                origins.insert(key.to_owned(), source.clone());
                names.insert(
                    key.to_owned(),
                    LocalizedName {
                        value: localized.to_owned(),
                    },
                );
            }
        }
    }
    names
}

fn parse_country_color(sources: &ProjectSources, source: &ResolvedSource) -> ParsedColor {
    read_text(sources, source)
        .map(|text| parse_color_block(&text))
        .unwrap_or(ParsedColor::Missing)
}

fn listed_text_files(sources: &ProjectSources, logical_directory: &str) -> Vec<ResolvedSource> {
    sources
        .list_files(logical_directory)
        .map(|listing| {
            listing
                .files
                .into_iter()
                .filter(|source| {
                    source.logical_path.extension().is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("txt")
                            || extension.eq_ignore_ascii_case("yml")
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn read_text(sources: &ProjectSources, source: &ResolvedSource) -> Option<String> {
    String::from_utf8(sources.read_resolved(source).ok()?).ok()
}

fn should_replace(existing: Option<&ResolvedSource>, candidate: &ResolvedSource) -> bool {
    existing.is_none_or(|existing| {
        candidate.source_kind.precedence_rank() >= existing.source_kind.precedence_rank()
    })
}

fn insert_preferred(
    entries: &mut BTreeMap<String, ResolvedSource>,
    key: String,
    candidate: ResolvedSource,
) {
    if should_replace(entries.get(&key), &candidate) {
        entries.insert(key, candidate);
    }
}

fn parse_multi_country_colors(text: &str) -> BTreeMap<String, ParsedColor> {
    let mut colors = BTreeMap::new();
    let mut active_tag = None::<String>;
    let mut block = String::new();
    let mut depth = 0_i32;
    for line in text.lines() {
        let line = strip_comment(line);
        if active_tag.is_none() {
            let Some((tag, value)) = line.split_once('=') else {
                continue;
            };
            let tag = tag.trim();
            if is_country_tag(tag) && value.contains('{') {
                active_tag = Some(tag.to_owned());
                depth = brace_delta(value);
                block.clear();
                block.push_str(value);
                block.push('\n');
                if depth == 0 {
                    colors.insert(tag.to_owned(), parse_color_block(&block));
                    active_tag = None;
                }
            }
            continue;
        }
        depth += brace_delta(line);
        block.push_str(line);
        block.push('\n');
        if depth <= 0 {
            let tag = active_tag.take().expect("active color block has a tag");
            colors.insert(tag, parse_color_block(&block));
        }
    }
    colors
}

fn parse_color_block(text: &str) -> ParsedColor {
    let mut search_from = 0;
    while let Some(relative) = text[search_from..].find("color") {
        let start = search_from + relative;
        search_from = start + "color".len();
        let preceding_is_identifier = start > 0
            && text.as_bytes()[start - 1].is_ascii_alphanumeric()
            || start > 0 && text.as_bytes()[start - 1] == b'_';
        if preceding_is_identifier {
            continue;
        }
        let after_key = text[search_from..].trim_start();
        let Some(after_equals) = after_key.strip_prefix('=') else {
            continue;
        };
        let Some(open) = after_equals.find('{') else {
            return ParsedColor::Invalid;
        };
        let after_open = &after_equals[open + 1..];
        let Some(close) = after_open.find('}') else {
            return ParsedColor::Invalid;
        };
        let values = after_open[..close]
            .split(|character: char| !character.is_ascii_digit())
            .filter(|value| !value.is_empty())
            .map(str::parse::<u8>)
            .collect::<Result<Vec<_>, _>>();
        return match values {
            Ok(values) if values.len() == 3 => {
                ParsedColor::Value([values[0], values[1], values[2]])
            }
            _ => ParsedColor::Invalid,
        };
    }
    ParsedColor::Missing
}

fn parse_ruling_ideology(text: &str) -> Option<String> {
    let (_, after_key) = text.split_once("ruling_party")?;
    let after_equals = after_key.trim_start().strip_prefix('=')?.trim_start();
    let ideology = after_equals
        .split(|character: char| character.is_whitespace() || character == '}')
        .next()?
        .trim_matches('"');
    (!ideology.is_empty()).then(|| ideology.to_owned())
}

fn brace_delta(text: &str) -> i32 {
    text.bytes().fold(0, |depth, byte| match byte {
        b'{' => depth + 1,
        b'}' => depth - 1,
        _ => depth,
    })
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(before, _)| before)
}

fn is_country_tag(value: &str) -> bool {
    value.len() == 3
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn localization_tag(key: &str) -> Option<&str> {
    let tag = key.split_once('_').map_or(key, |(tag, _)| tag);
    is_country_tag(tag).then_some(tag)
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
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn catalog(entries: &[(&str, Option<Color>, Option<&str>, bool)]) -> PoliticalCountryCatalog {
        let metadata = entries
            .iter()
            .map(|(tag, color, name, flag)| {
                (
                    (*tag).to_owned(),
                    CountryMetadata {
                        tag: (*tag).to_owned(),
                        display_name: name.unwrap_or(tag).to_string(),
                        color: *color,
                        color_resolution: CountryColorResolution {
                            kind: color.map_or(CountryColorResolutionKind::ColorMissing, |_| {
                                CountryColorResolutionKind::Resolved
                            }),
                            tag: (*tag).to_owned(),
                            rgb: *color,
                            source_path: None,
                            source_type: None,
                            source: None,
                        },
                        flag: flag.then(|| RgbaImage::new(1, 1)),
                    },
                )
            })
            .collect();
        PoliticalCountryCatalog {
            sources: ProjectSources::for_test_placeholder(),
            colors_file: None,
            country_files: BTreeMap::new(),
            country_histories: BTreeMap::new(),
            localized_names: BTreeMap::new(),
            known_tags: entries
                .iter()
                .map(|(tag, _, _, _)| (*tag).to_owned())
                .collect(),
            metadata,
        }
    }

    fn province(id: u32, center: [f64; 2], pixels: u64) -> PoliticalProvince {
        PoliticalProvince {
            id,
            is_land: true,
            center,
            pixel_count: pixels,
        }
    }

    fn fixture_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hoi4-political-{name}-{}-{}",
            std::process::id(),
            FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn catalog_from_roots(project: &Path, base: Option<&Path>) -> PoliticalCountryCatalog {
        fs::create_dir_all(project.join("map")).unwrap();
        fs::write(project.join("map/provinces.bmp"), []).unwrap();
        fs::write(project.join("map/definition.csv"), []).unwrap();
        PoliticalCountryCatalog::load(
            ProjectSources::discover(
                project,
                base.map(Path::to_owned),
                super::super::project::SourceGeneration::new(1),
            )
            .unwrap(),
        )
    }

    #[test]
    fn reads_mod_country_color_localized_name_and_optional_flag() {
        let root = fixture_root("metadata");
        fs::create_dir_all(root.join("common/country_tags")).unwrap();
        fs::create_dir_all(root.join("common/countries")).unwrap();
        fs::create_dir_all(root.join("localisation/fixture")).unwrap();
        fs::create_dir_all(root.join("gfx/flags")).unwrap();
        fs::write(
            root.join("common/country_tags/00_tags.txt"),
            "BRA = \"countries/BRA - Brazil.txt\"\n",
        )
        .unwrap();
        fs::write(
            root.join("common/countries/BRA - Brazil.txt"),
            "color = { 12 34 56 }\n",
        )
        .unwrap();
        fs::write(
            root.join("localisation/fixture/countries.yml"),
            format!("l_{}:\n BRA:0 \"Brazil\"\n", hoi4_language()),
        )
        .unwrap();
        RgbaImage::new(2, 1)
            .save(root.join("gfx/flags/BRA.png"))
            .unwrap();

        let mut catalog = catalog_from_roots(&root, None);
        catalog.resolve_tags(["BRA".to_owned()]);
        let country = catalog.metadata("BRA").unwrap();
        assert_eq!(country.color, Some([12, 34, 56]));
        assert_eq!(country.display_name, "Brazil");
        assert_eq!(
            country.flag.as_ref().map(RgbaImage::dimensions),
            Some((2, 1))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_optional_country_assets_leave_a_tag_only_country_renderable() {
        let root = fixture_root("missing-assets");
        let mut catalog = catalog_from_roots(&root, None);
        catalog.resolve_tags(["BRA".to_owned()]);
        let country = catalog.metadata("BRA").unwrap();
        assert_eq!(country.display_name, "BRA");
        assert_eq!(country.color, None);
        assert!(country.flag.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn country_color_uses_metadata_then_deterministic_fallback() {
        let catalog = catalog(&[("BRA", Some([1, 2, 3]), Some("Brazil"), false)]);
        assert_eq!(catalog.color_for("BRA"), [1, 2, 3]);
        assert_eq!(catalog.color_for("ZZZ"), political_fallback_color("ZZZ"));
    }

    #[test]
    fn localized_name_and_missing_flag_are_independent() {
        let catalog = catalog(&[("BRA", None, Some("Brasil"), false)]);
        let country = catalog.metadata("BRA").unwrap();
        assert_eq!(country.display_name, "Brasil");
        assert!(country.flag.is_none());
    }

    #[test]
    fn territory_uses_state_owners_and_largest_connected_landmass() {
        let catalog = catalog(&[("BRA", None, Some("Brazil"), true)]);
        let states = vec![PoliticalStateOwnership {
            owner: Some("BRA".to_owned()),
            provinces: vec![1, 2, 3],
        }];
        let labels = prepare_country_labels(
            &catalog,
            &states,
            &[
                province(1, [10.0, 10.0], 90),
                province(2, [20.0, 10.0], 80),
                province(3, [200.0, 10.0], 5),
            ],
            &[(1, 2)],
        );
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].anchor, [10.0, 10.0]);
        assert_eq!(labels[0].territory_pixels, 170);
    }

    #[test]
    fn owner_change_recalculates_prepared_territory() {
        let catalog = catalog(&[("BRA", None, None, false), ("ARG", None, None, false)]);
        let provinces = [province(1, [1.0, 1.0], 20), province(2, [10.0, 1.0], 20)];
        let brazil = prepare_country_labels(
            &catalog,
            &[PoliticalStateOwnership {
                owner: Some("BRA".to_owned()),
                provinces: vec![1, 2],
            }],
            &provinces,
            &[(1, 2)],
        );
        let argentina = prepare_country_labels(
            &catalog,
            &[PoliticalStateOwnership {
                owner: Some("ARG".to_owned()),
                provinces: vec![1, 2],
            }],
            &provinces,
            &[(1, 2)],
        );
        assert_eq!(brazil[0].tag, "BRA");
        assert_eq!(argentina[0].tag, "ARG");
    }

    #[test]
    fn small_country_visibility_is_deterministic() {
        assert_eq!(
            political_label_visibility(3, 1.0, true),
            PoliticalLabelVisibility::Hidden
        );
        assert_eq!(
            political_label_visibility(20, 2.0, true),
            PoliticalLabelVisibility::FlagOnly
        );
        assert_eq!(
            political_label_visibility(20, 3.0, false),
            PoliticalLabelVisibility::NameOnly
        );
        assert_eq!(
            political_label_visibility(1_000, 1.0, true),
            PoliticalLabelVisibility::NameAndFlag
        );
    }

    #[test]
    fn labels_only_exist_for_owned_land_territory() {
        let catalog = catalog(&[("BRA", None, None, false)]);
        let labels = prepare_country_labels(
            &catalog,
            &[PoliticalStateOwnership {
                owner: Some("BRA".to_owned()),
                provinces: vec![1],
            }],
            &[PoliticalProvince {
                id: 1,
                is_land: false,
                center: [1.0, 1.0],
                pixel_count: 20,
            }],
            &[],
        );
        assert!(labels.is_empty());
    }

    #[test]
    fn missing_flag_keeps_a_large_country_name_visible() {
        let catalog = catalog(&[("BRA", None, Some("Brazil"), false)]);
        let labels = prepare_country_labels(
            &catalog,
            &[PoliticalStateOwnership {
                owner: Some("BRA".to_owned()),
                provinces: vec![1],
            }],
            &[province(1, [1.0, 1.0], 1_000)],
            &[],
        );
        assert_eq!(labels[0].display_name, "Brazil");
        assert!(!labels[0].flag_available);
        assert_eq!(
            political_label_visibility(labels[0].territory_pixels, 1.0, labels[0].flag_available),
            PoliticalLabelVisibility::NameOnly
        );
    }

    #[test]
    fn mod_metadata_overrides_base_game_metadata() {
        let base = fixture_root("base");
        let project = fixture_root("project");
        for root in [&base, &project] {
            fs::create_dir_all(root.join("common/country_tags")).unwrap();
            fs::create_dir_all(root.join("common/countries")).unwrap();
            fs::write(
                root.join("common/country_tags/00_tags.txt"),
                "BRA = \"countries/BRA - Brazil.txt\"\n",
            )
            .unwrap();
        }
        fs::write(
            base.join("common/countries/BRA - Brazil.txt"),
            "color = { 1 2 3 }\n",
        )
        .unwrap();
        fs::write(
            project.join("common/countries/BRA - Brazil.txt"),
            "color = { 4 5 6 }\n",
        )
        .unwrap();
        let mut catalog = catalog_from_roots(&project, Some(&base));
        catalog.resolve_tags(["BRA".to_owned()]);
        assert_eq!(catalog.metadata("BRA").unwrap().color, Some([4, 5, 6]));
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn reads_consolidated_multi_country_colors_without_using_color_ui() {
        let root = fixture_root("consolidated-colors");
        fs::create_dir_all(root.join("common/countries")).unwrap();
        fs::write(
            root.join("common/countries/colors.txt"),
            "THK = {\n color = rgb { 255 13 20 }\n color_ui = rgb { 1 2 3 }\n}\n\
             GYE = { color = rgb { 24 25 34 } }\n\
             CBS = { color = rgb { 255 220 105 } }\n\
             INV = { color = rgb { 105 82 56 } }\n",
        )
        .unwrap();
        let mut catalog = catalog_from_roots(&root, None);
        catalog.resolve_tags(["THK", "GYE", "CBS", "INV"].map(str::to_owned));
        assert_eq!(catalog.metadata("THK").unwrap().color, Some([255, 13, 20]));
        assert_eq!(catalog.metadata("GYE").unwrap().color, Some([24, 25, 34]));
        assert_eq!(
            catalog.metadata("CBS").unwrap().color,
            Some([255, 220, 105])
        );
        assert_eq!(catalog.metadata("INV").unwrap().color, Some([105, 82, 56]));
        assert_eq!(catalog.color_for("THK"), [255, 13, 20]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mod_consolidated_color_overrides_base_game_color() {
        let base = fixture_root("base-colors");
        let project = fixture_root("project-colors");
        for (root, color) in [(&base, "1 2 3"), (&project, "255 13 20")] {
            fs::create_dir_all(root.join("common/countries")).unwrap();
            fs::write(
                root.join("common/countries/colors.txt"),
                format!("THK = {{ color = rgb {{ {color} }} }}"),
            )
            .unwrap();
        }
        let mut catalog = catalog_from_roots(&project, Some(&base));
        catalog.resolve_tags(["THK".to_owned()]);
        let country = catalog.metadata("THK").unwrap();
        assert_eq!(country.color, Some([255, 13, 20]));
        assert!(
            country
                .color_resolution
                .source_path
                .as_ref()
                .is_some_and(|path| path.starts_with(&project))
        );
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn mod_per_country_color_overrides_base_consolidated_color() {
        let base = fixture_root("base-global-versus-mod-country");
        let project = fixture_root("project-global-versus-mod-country");
        fs::create_dir_all(base.join("common/countries")).unwrap();
        fs::write(
            base.join("common/countries/colors.txt"),
            "THK = { color = rgb { 1 2 3 } }",
        )
        .unwrap();
        fs::create_dir_all(project.join("common/country_tags")).unwrap();
        fs::create_dir_all(project.join("common/countries")).unwrap();
        fs::write(
            project.join("common/country_tags/00_tags.txt"),
            "THK = \"countries/THK.txt\"",
        )
        .unwrap();
        fs::write(
            project.join("common/countries/THK.txt"),
            "color = rgb { 255 13 20 }",
        )
        .unwrap();
        let mut catalog = catalog_from_roots(&project, Some(&base));
        catalog.resolve_tags(["THK".to_owned()]);
        assert_eq!(catalog.metadata("THK").unwrap().color, Some([255, 13, 20]));
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn localization_prefers_generic_then_history_matched_ideology_then_raw_tag() {
        let root = fixture_root("localization");
        fs::create_dir_all(root.join("history/countries")).unwrap();
        fs::create_dir_all(root.join("localisation/english/nested")).unwrap();
        fs::write(
            root.join("history/countries/INV - Ironvale.txt"),
            "set_politics = { ruling_party = neutrality }",
        )
        .unwrap();
        fs::write(
            root.join("localisation/english/nested/countries.yml"),
            format!(
                "l_{}:\n THK:1 \"Thiryn Kingdom\"\n INV_neutrality:0 \"Ironvale Confederacy\"\n INV_fascism:0 \"Wrong history\"\n",
                hoi4_language()
            ),
        )
        .unwrap();
        let mut catalog = catalog_from_roots(&root, None);
        catalog.resolve_tags(["THK", "INV", "ZZZ"].map(str::to_owned));
        assert_eq!(
            catalog.metadata("THK").unwrap().display_name,
            "Thiryn Kingdom"
        );
        assert_eq!(
            catalog.metadata("INV").unwrap().display_name,
            "Ironvale Confederacy"
        );
        assert_eq!(catalog.metadata("ZZZ").unwrap().display_name, "ZZZ");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mod_localization_overrides_base_game_and_uses_stable_no_history_order() {
        let base = fixture_root("base-localization");
        let project = fixture_root("project-localization");
        for root in [&base, &project] {
            fs::create_dir_all(root.join("localisation/english/deep")).unwrap();
        }
        fs::write(
            base.join("localisation/english/deep/countries.yml"),
            format!("l_{}:\n THK:0 \"Base Thiryn\"\n ASV_fascism:0 \"Fascist\"\n ASV_neutrality:0 \"Ash Vultures\"\n", hoi4_language()),
        )
        .unwrap();
        fs::write(
            project.join("localisation/english/deep/override.yml"),
            format!("l_{}:\n THK: \"Mod Thiryn\"\n", hoi4_language()),
        )
        .unwrap();
        let mut catalog = catalog_from_roots(&project, Some(&base));
        catalog.resolve_tags(["THK", "ASV"].map(str::to_owned));
        assert_eq!(catalog.metadata("THK").unwrap().display_name, "Mod Thiryn");
        assert_eq!(
            catalog.metadata("ASV").unwrap().display_name,
            "Ash Vultures"
        );
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn replace_path_country_tags_excludes_lower_tags_but_keeps_current_tags() {
        let base = fixture_root("replace-tags-base");
        let project = fixture_root("replace-tags-project");
        fs::create_dir_all(base.join("common/country_tags")).unwrap();
        fs::create_dir_all(base.join("common/countries")).unwrap();
        fs::write(
            base.join("common/country_tags/base.txt"),
            "BAS = \"countries/BAS.txt\"",
        )
        .unwrap();
        fs::write(base.join("common/countries/BAS.txt"), "color = { 1 2 3 }").unwrap();
        fs::create_dir_all(project.join("common/country_tags")).unwrap();
        fs::create_dir_all(project.join("common/countries")).unwrap();
        fs::write(
            project.join("descriptor.mod"),
            "replace_path = \"common/country_tags\"",
        )
        .unwrap();
        fs::write(
            project.join("common/country_tags/custom.txt"),
            "MOD = \"countries/MOD.txt\"",
        )
        .unwrap();
        fs::write(
            project.join("common/countries/MOD.txt"),
            "color = { 4 5 6 }",
        )
        .unwrap();

        let catalog = catalog_from_roots(&project, Some(&base));
        assert!(catalog.known_tags.contains("MOD"));
        assert!(!catalog.known_tags.contains("BAS"));
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn replace_path_countries_blocks_lower_country_definitions_and_colors() {
        let base = fixture_root("replace-countries-base");
        let project = fixture_root("replace-countries-project");
        fs::create_dir_all(base.join("common/countries")).unwrap();
        fs::write(
            base.join("common/countries/colors.txt"),
            "BAS = { color = { 1 2 3 } }",
        )
        .unwrap();
        fs::write(base.join("common/countries/BAS.txt"), "color = { 1 2 3 }").unwrap();
        fs::create_dir_all(project.join("common/country_tags")).unwrap();
        fs::write(
            project.join("descriptor.mod"),
            "replace_path = \"common/countries\"",
        )
        .unwrap();
        fs::write(
            project.join("common/country_tags/tags.txt"),
            "BAS = \"countries/BAS.txt\"",
        )
        .unwrap();

        let mut catalog = catalog_from_roots(&project, Some(&base));
        catalog.resolve_tags(["BAS".to_owned()]);
        assert_eq!(catalog.metadata("BAS").unwrap().color, None);
        assert_eq!(
            catalog.metadata("BAS").unwrap().color_resolution.kind,
            CountryColorResolutionKind::CountryTagUnknown
        );
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn political_catalog_reads_dlc_inputs_and_honors_localisation_and_flag_replacements() {
        let base = fixture_root("dlc-political-base");
        let project = fixture_root("dlc-political-project");
        let dlc = base.join("dlc/dlc001");
        fs::create_dir_all(dlc.join("common/country_tags")).unwrap();
        fs::create_dir_all(dlc.join("common/countries")).unwrap();
        fs::write(
            dlc.join("common/country_tags/dlc.txt"),
            "DLC = \"countries/DLC.txt\"",
        )
        .unwrap();
        fs::write(dlc.join("common/countries/DLC.txt"), "color = { 7 8 9 }").unwrap();
        fs::create_dir_all(base.join("localisation")).unwrap();
        fs::write(
            base.join("localisation/countries.yml"),
            format!("l_{}:\n DLC:0 \"Base DLC\"", hoi4_language()),
        )
        .unwrap();
        fs::create_dir_all(base.join("gfx/flags")).unwrap();
        RgbaImage::new(1, 1)
            .save(base.join("gfx/flags/DLC.png"))
            .unwrap();
        fs::create_dir_all(project.join("localisation")).unwrap();
        fs::write(
            project.join("localisation/override.yml"),
            format!("l_{}:\n DLC:0 \"Project DLC\"", hoi4_language()),
        )
        .unwrap();

        let mut catalog = catalog_from_roots(&project, Some(&base));
        catalog.resolve_tags(["DLC".to_owned()]);
        let metadata = catalog.metadata("DLC").unwrap();
        assert_eq!(metadata.color, Some([7, 8, 9]));
        assert_eq!(metadata.display_name, "Project DLC");
        assert!(metadata.flag.is_some());
        assert_eq!(
            metadata
                .color_resolution
                .source
                .as_ref()
                .unwrap()
                .source_kind,
            super::super::project::SourceKind::Dlc
        );

        fs::write(
            project.join("descriptor.mod"),
            "replace_path = \"localisation\"\nreplace_path = \"gfx/flags\"",
        )
        .unwrap();
        fs::remove_dir_all(project.join("localisation")).unwrap();
        let mut replaced = catalog_from_roots(&project, Some(&base));
        replaced.resolve_tags(["DLC".to_owned()]);
        let metadata = replaced.metadata("DLC").unwrap();
        assert_eq!(metadata.display_name, "DLC");
        assert!(metadata.flag.is_none());
        fs::remove_dir_all(base).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn owner_resolution_distinguishes_missing_owner_unknown_tag_missing_color_and_parse_failure() {
        let root = fixture_root("owner-resolution");
        fs::create_dir_all(root.join("common/country_tags")).unwrap();
        fs::create_dir_all(root.join("common/countries")).unwrap();
        fs::write(
            root.join("common/country_tags/00_tags.txt"),
            "BRA = \"countries/BRA.txt\"\nBAD = \"countries/BAD.txt\"\n",
        )
        .unwrap();
        fs::write(
            root.join("common/countries/BRA.txt"),
            "graphical_culture = western",
        )
        .unwrap();
        fs::write(
            root.join("common/countries/BAD.txt"),
            "color = rgb { 1 nope 3 }",
        )
        .unwrap();
        let mut catalog = catalog_from_roots(&root, None);
        catalog.resolve_tags(["BRA", "BAD", "ZZZ"].map(str::to_owned));
        assert_eq!(
            catalog.owner_resolution(None),
            PoliticalOwnerResolution::OwnerMissing
        );
        assert_eq!(
            catalog.owner_resolution(Some("ZZZ")),
            PoliticalOwnerResolution::Country(CountryColorResolution {
                kind: CountryColorResolutionKind::CountryTagUnknown,
                tag: "ZZZ".to_owned(),
                rgb: None,
                source_path: None,
                source_type: None,
                source: None,
            })
        );
        assert_eq!(
            catalog.metadata("BRA").unwrap().color_resolution.kind,
            CountryColorResolutionKind::ColorMissing
        );
        assert_eq!(
            catalog.metadata("BAD").unwrap().color_resolution.kind,
            CountryColorResolutionKind::ColorParseFailed
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn labels_are_restricted_to_political_view() {
        assert!(political_labels_visible_in_view(MapBaseView::Political));
        assert!(!political_labels_visible_in_view(MapBaseView::States));
        assert!(!political_labels_visible_in_view(MapBaseView::Terrain));
    }
}
