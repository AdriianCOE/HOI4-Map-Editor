use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use image::RgbaImage;

use super::map::Color;
use super::map_layers::{MapBaseView, political_fallback_color};

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
    roots: Vec<PathBuf>,
    country_source_layers: Vec<CountrySourceLayer>,
    country_histories: BTreeMap<String, CountryHistory>,
    localized_names: BTreeMap<String, LocalizedName>,
    known_tags: BTreeSet<String>,
    metadata: BTreeMap<String, CountryMetadata>,
}

impl PoliticalCountryCatalog {
    pub fn load(project_root: &Path, base_game_root: Option<&Path>) -> Self {
        let mut roots = base_game_root
            .map(Path::to_owned)
            .into_iter()
            .collect::<Vec<_>>();
        roots.push(project_root.to_owned());
        let mut country_histories = BTreeMap::new();
        let mut localized_names = BTreeMap::new();
        for root in &roots {
            country_histories.extend(scan_country_histories(root));
            localized_names.extend(scan_localized_country_names(root));
        }
        // A mod layer wins as a whole: colors.txt, then its per-country files,
        // before continuing to the next (base-game) layer.
        let country_source_layers = roots
            .iter()
            .rev()
            .map(|root| CountrySourceLayer {
                colors_file: scan_country_colors_files(root).into_iter().next(),
                country_files: scan_country_files(root),
            })
            .collect::<Vec<_>>();
        let mut known_tags = country_source_layers
            .iter()
            .flat_map(|layer| layer.country_files.keys().cloned())
            .collect::<BTreeSet<_>>();
        known_tags.extend(country_histories.keys().cloned());
        known_tags.extend(
            localized_names
                .keys()
                .filter_map(|key| localization_tag(key).map(str::to_owned)),
        );
        for layer in &country_source_layers {
            if let Some(file) = &layer.colors_file {
                known_tags.extend(file.colors.keys().cloned());
            }
        }
        Self {
            roots,
            country_source_layers,
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
            };
        }
        for layer in &self.country_source_layers {
            if let Some(file) = &layer.colors_file
                && let Some(result) = file.colors.get(tag)
            {
                return result.to_resolution(tag, &file.path, "common/countries/colors.txt");
            }
            if let Some(path) = layer.country_files.get(tag) {
                return parse_country_color(path).to_resolution(
                    tag,
                    path,
                    "per-country definition",
                );
            }
        }
        CountryColorResolution {
            kind: CountryColorResolutionKind::ColorMissing,
            tag: tag.to_owned(),
            rgb: None,
            source_path: None,
            source_type: None,
        }
    }

    fn load_flag(&self, tag: &str) -> Option<RgbaImage> {
        for root in self.roots.iter().rev() {
            for extension in ["tga", "dds", "png", "bmp"] {
                let path = root.join("gfx/flags").join(format!("{tag}.{extension}"));
                if path.is_file()
                    && let Ok(image) = image::open(path)
                {
                    return Some(image.to_rgba8());
                }
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
    let provinces_by_id = provinces
        .iter()
        .copied()
        .map(|province| (province.id, province))
        .collect::<BTreeMap<_, _>>();
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
            provinces_by_id
                .get(province_id)
                .is_some_and(|province| province.is_land)
        }));
    }
    let mut neighbors = BTreeMap::<u32, BTreeSet<u32>>::new();
    for &(left, right) in adjacency_pairs {
        if left != right {
            neighbors.entry(left).or_default().insert(right);
            neighbors.entry(right).or_default().insert(left);
        }
    }

    owned_by_country
        .into_iter()
        .filter_map(|(tag, owned)| {
            let main_component = largest_component(&owned, &neighbors, &provinces_by_id)?;
            let territory_pixels = main_component
                .iter()
                .filter_map(|province_id| provinces_by_id.get(province_id))
                .map(|province| province.pixel_count)
                .sum();
            let anchor = anchor_for_component(&main_component, &provinces_by_id)?;
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
    neighbors: &BTreeMap<u32, BTreeSet<u32>>,
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

/// Stable largest-connected-territory anchor shared by non-political overlays.
pub(crate) fn territory_anchor(
    owned: &BTreeSet<u32>,
    provinces: &[PoliticalProvince],
    adjacency_pairs: &[(u32, u32)],
) -> Option<([f64; 2], u64)> {
    let provinces_by_id = provinces
        .iter()
        .copied()
        .map(|province| (province.id, province))
        .collect::<BTreeMap<_, _>>();
    let owned = owned
        .iter()
        .copied()
        .filter(|id| {
            provinces_by_id
                .get(id)
                .is_some_and(|province| province.is_land)
        })
        .collect::<BTreeSet<_>>();
    let mut neighbors = BTreeMap::<u32, BTreeSet<u32>>::new();
    for &(left, right) in adjacency_pairs {
        if left != right {
            neighbors.entry(left).or_default().insert(right);
            neighbors.entry(right).or_default().insert(left);
        }
    }
    let component = largest_component(&owned, &neighbors, &provinces_by_id)?;
    let pixels = component
        .iter()
        .filter_map(|id| provinces_by_id.get(id))
        .map(|province| province.pixel_count)
        .sum();
    Some((anchor_for_component(&component, &provinces_by_id)?, pixels))
}

fn squared_distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    dx * dx + dy * dy
}

fn scan_country_files(root: &Path) -> BTreeMap<String, PathBuf> {
    let mut files = BTreeMap::new();
    for path in text_files_recursive(&root.join("common/country_tags")) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines().map(strip_comment) {
            let Some((tag, target)) = line.split_once('=') else {
                continue;
            };
            let tag = tag.trim();
            let target = target.trim().trim_matches('"');
            if is_country_tag(tag) && !target.is_empty() {
                files.insert(tag.to_owned(), root.join("common").join(target));
            }
        }
    }
    for path in text_files_recursive(&root.join("common/countries")) {
        let Some(tag) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.split_whitespace().next())
            .filter(|tag| is_country_tag(tag))
        else {
            continue;
        };
        files.entry(tag.to_owned()).or_insert(path);
    }
    files
}

#[derive(Debug, Clone)]
struct CountryColorFile {
    path: PathBuf,
    colors: BTreeMap<String, ParsedColor>,
}

#[derive(Debug, Clone)]
struct CountrySourceLayer {
    colors_file: Option<CountryColorFile>,
    country_files: BTreeMap<String, PathBuf>,
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
        path: &Path,
        source_type: &'static str,
    ) -> CountryColorResolution {
        match self {
            Self::Value(rgb) => CountryColorResolution {
                kind: CountryColorResolutionKind::Resolved,
                tag: tag.to_owned(),
                rgb: Some(rgb),
                source_path: Some(path.to_owned()),
                source_type: Some(source_type),
            },
            Self::Invalid => CountryColorResolution {
                kind: CountryColorResolutionKind::ColorParseFailed,
                tag: tag.to_owned(),
                rgb: None,
                source_path: Some(path.to_owned()),
                source_type: Some(source_type),
            },
            Self::Missing => CountryColorResolution {
                kind: CountryColorResolutionKind::ColorMissing,
                tag: tag.to_owned(),
                rgb: None,
                source_path: Some(path.to_owned()),
                source_type: Some(source_type),
            },
        }
    }
}

fn scan_country_colors_files(root: &Path) -> Vec<CountryColorFile> {
    let path = root.join("common/countries/colors.txt");
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    vec![CountryColorFile {
        path,
        colors: parse_multi_country_colors(&text),
    }]
}

fn scan_country_histories(root: &Path) -> BTreeMap<String, CountryHistory> {
    let mut histories = BTreeMap::new();
    for path in text_files_recursive(&root.join("history/countries")) {
        let Some(tag) = path
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
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let ruling_ideology = parse_ruling_ideology(&text);
        histories.insert(tag.to_owned(), CountryHistory { ruling_ideology });
    }
    histories
}

fn scan_localized_country_names(root: &Path) -> BTreeMap<String, LocalizedName> {
    let header = format!("l_{}:", hoi4_language());
    let mut names = BTreeMap::new();
    for path in text_files_recursive(&root.join("localisation")) {
        let Ok(text) = fs::read_to_string(path) else {
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
            if !localized.is_empty() {
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

fn parse_country_color(path: &Path) -> ParsedColor {
    fs::read_to_string(path)
        .map(|text| parse_color_block(&text))
        .unwrap_or(ParsedColor::Missing)
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

fn text_files_recursive(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            paths.extend(text_files_recursive(&path));
        } else if path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("txt") || extension.eq_ignore_ascii_case("yml")
        }) {
            paths.push(path);
        }
    }
    paths.sort();
    paths
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
                        },
                        flag: flag.then(|| RgbaImage::new(1, 1)),
                    },
                )
            })
            .collect();
        PoliticalCountryCatalog {
            roots: Vec::new(),
            country_source_layers: Vec::new(),
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

        let mut catalog = PoliticalCountryCatalog::load(&root, None);
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
        let mut catalog = PoliticalCountryCatalog::load(&root, None);
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
        let mut catalog = PoliticalCountryCatalog::load(&project, Some(&base));
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
        let mut catalog = PoliticalCountryCatalog::load(&root, None);
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
        let mut catalog = PoliticalCountryCatalog::load(&project, Some(&base));
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
        let mut catalog = PoliticalCountryCatalog::load(&project, Some(&base));
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
        let mut catalog = PoliticalCountryCatalog::load(&root, None);
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
        let mut catalog = PoliticalCountryCatalog::load(&project, Some(&base));
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
        let mut catalog = PoliticalCountryCatalog::load(&root, None);
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
