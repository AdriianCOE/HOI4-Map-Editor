use std::cmp::Ordering;

pub const INSPECTOR_EXPANDED_WIDTH: f64 = 420.0;
pub const INSPECTOR_COLLAPSED_WIDTH: f64 = 48.0;
const INSPECTOR_MIN_MAP_WIDTH: f64 = 220.0;

pub fn inspector_drawer_width(window_width: f64, sidebar_width: f64, requested: f64) -> f64 {
    let available = (window_width - sidebar_width).max(0.0);
    if requested <= 0.0 || available <= 0.0 {
        return 0.0;
    }
    if requested <= INSPECTOR_COLLAPSED_WIDTH {
        return requested.min(available);
    }
    requested.min((available - INSPECTOR_MIN_MAP_WIDTH).max(INSPECTOR_COLLAPSED_WIDTH))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapViewport {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl MapViewport {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width: width.max(0.0),
            height: height.max(0.0),
        }
    }

    pub fn right(self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(self) -> f64 {
        self.y + self.height
    }

    pub fn center(self) -> [f64; 2] {
        [self.x + self.width / 2.0, self.y + self.height / 2.0]
    }

    pub fn dimensions(self) -> [f64; 2] {
        [self.width, self.height]
    }

    pub fn contains(self, [x, y]: [f64; 2]) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateInspectorVisibility {
    Hidden,
    Collapsed,
    Expanded,
}

impl StateInspectorVisibility {
    pub fn reserved_width(self) -> f64 {
        match self {
            Self::Hidden => 0.0,
            Self::Collapsed => INSPECTOR_COLLAPSED_WIDTH,
            Self::Expanded => INSPECTOR_EXPANDED_WIDTH,
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Hidden => Self::Collapsed,
            Self::Collapsed => Self::Expanded,
            Self::Expanded => Self::Hidden,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeveloperDiagnosticsMode {
    #[default]
    Off,
    Compact,
    Detailed,
}

impl DeveloperDiagnosticsMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::Compact,
            Self::Compact => Self::Detailed,
            Self::Detailed => Self::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProvinceLabelMode {
    Off,
    #[default]
    Hovered,
    SelectedState,
    All,
}

impl ProvinceLabelMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::Hovered,
            Self::Hovered => Self::SelectedState,
            Self::SelectedState => Self::All,
            Self::All => Self::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InspectorSection {
    Overview,
    Provinces,
    History,
    Buildings,
    Resources,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectorIcon {
    pub section: InspectorSection,
    pub label: &'static str,
    pub shortcut: Option<char>,
}

impl InspectorIcon {
    pub const fn new(
        section: InspectorSection,
        label: &'static str,
        shortcut: Option<char>,
    ) -> Self {
        Self {
            section,
            label,
            shortcut,
        }
    }
}

pub const INSPECTOR_ICONS: &[InspectorIcon] = &[
    InspectorIcon::new(InspectorSection::Overview, "General", Some('1')),
    InspectorIcon::new(InspectorSection::History, "Politics", Some('2')),
    InspectorIcon::new(InspectorSection::Provinces, "Provinces", Some('3')),
    InspectorIcon::new(InspectorSection::Buildings, "Buildings", Some('4')),
    InspectorIcon::new(InspectorSection::Resources, "Resources", Some('5')),
    InspectorIcon::new(InspectorSection::Diagnostics, "Diagnostics", Some('6')),
];

#[derive(Debug, Clone, PartialEq)]
pub struct StateInspectorState {
    pub visibility: StateInspectorVisibility,
    pub scroll_y: f64,
    pub active_section: InspectorSection,
    pub search: String,
    pub diagnostics_mode: DeveloperDiagnosticsMode,
}

impl StateInspectorState {
    pub fn session_default() -> Self {
        Self::default()
    }

    pub fn set_search(&mut self, search: impl Into<String>) {
        self.search = search.into();
    }

    pub fn clear_session(&mut self) {
        *self = Self::default();
    }
}

impl Default for StateInspectorState {
    fn default() -> Self {
        Self {
            visibility: StateInspectorVisibility::Expanded,
            scroll_y: 0.0,
            active_section: InspectorSection::Overview,
            search: String::new(),
            diagnostics_mode: DeveloperDiagnosticsMode::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InspectorLayout {
    pub map_viewport: MapViewport,
    pub inspector_viewport: Option<MapViewport>,
}

impl InspectorLayout {
    pub fn calculate(
        window_size: [f64; 2],
        sidebar_width: f64,
        toolbar_height: f64,
        visibility: StateInspectorVisibility,
    ) -> Self {
        let window_width = window_size[0].max(0.0);
        let window_height = window_size[1].max(0.0);
        let left = sidebar_width.clamp(0.0, window_width);
        let top = toolbar_height.clamp(0.0, window_height);
        let reserved = inspector_drawer_width(window_width, left, visibility.reserved_width());
        let content_height = (window_height - top).max(0.0);

        let map_viewport =
            MapViewport::new(left, top, (window_width - left).max(0.0), content_height);
        let inspector_viewport = (reserved > 0.0)
            .then(|| MapViewport::new(window_width - reserved, top, reserved, content_height));

        Self {
            map_viewport,
            inspector_viewport,
        }
    }

    pub fn hit_test(self, pos: [f64; 2]) -> InspectorHit {
        if self
            .inspector_viewport
            .is_some_and(|viewport| viewport.contains(pos))
        {
            InspectorHit::Inspector
        } else if self.map_viewport.contains(pos) {
            InspectorHit::Map
        } else {
            InspectorHit::Chrome
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorHit {
    Map,
    Inspector,
    Chrome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSearchEntry {
    pub state_id: u32,
    pub name: String,
    pub owner: Option<String>,
    pub controller: Option<String>,
    pub province_ids: Vec<u32>,
}

impl StateSearchEntry {
    pub fn new(state_id: u32, name: impl Into<String>) -> Self {
        Self {
            state_id,
            name: name.into(),
            owner: None,
            controller: None,
            province_ids: Vec::new(),
        }
    }

    pub fn with_context(
        mut self,
        owner: Option<String>,
        controller: Option<String>,
        province_ids: impl IntoIterator<Item = u32>,
    ) -> Self {
        self.owner = owner;
        self.controller = controller;
        self.province_ids = province_ids.into_iter().collect();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSearchMatch {
    ExactId,
    ProvinceId,
    Owner,
    Controller,
    PartialId,
    NameSubstring,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSearchResult {
    pub state_id: u32,
    pub name: String,
    pub matched: StateSearchMatch,
}

#[derive(Debug, Clone, Default)]
pub struct StateSearchIndex {
    entries: Vec<StateSearchEntry>,
}

impl StateSearchIndex {
    pub fn new(entries: impl IntoIterator<Item = StateSearchEntry>) -> Self {
        let mut entries = entries.into_iter().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.state_id);
        Self { entries }
    }

    pub fn search(&self, query: &str) -> Vec<StateSearchResult> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }

        let query_lower = query.to_ascii_lowercase();
        let exact_id = query.parse::<u32>().ok();
        let mut results = self
            .entries
            .iter()
            .filter_map(|entry| {
                let id_text = entry.state_id.to_string();
                let matched = if exact_id == Some(entry.state_id) {
                    StateSearchMatch::ExactId
                } else if exact_id.is_some_and(|id| entry.province_ids.contains(&id)) {
                    StateSearchMatch::ProvinceId
                } else if entry
                    .owner
                    .as_deref()
                    .is_some_and(|owner| owner.eq_ignore_ascii_case(query))
                {
                    StateSearchMatch::Owner
                } else if entry
                    .controller
                    .as_deref()
                    .is_some_and(|controller| controller.eq_ignore_ascii_case(query))
                {
                    StateSearchMatch::Controller
                } else if id_text.contains(query) {
                    StateSearchMatch::PartialId
                } else if entry.name.to_ascii_lowercase().contains(&query_lower) {
                    StateSearchMatch::NameSubstring
                } else {
                    return None;
                };
                Some(StateSearchResult {
                    state_id: entry.state_id,
                    name: entry.name.clone(),
                    matched,
                })
            })
            .collect::<Vec<_>>();

        results.sort_by(|a, b| {
            search_rank(a)
                .cmp(&search_rank(b))
                .then_with(|| a.state_id.cmp(&b.state_id))
        });
        results
    }
}

fn search_rank(result: &StateSearchResult) -> u8 {
    match result.matched {
        StateSearchMatch::ExactId => 0,
        StateSearchMatch::ProvinceId => 1,
        StateSearchMatch::Owner | StateSearchMatch::Controller => 2,
        StateSearchMatch::PartialId => 3,
        StateSearchMatch::NameSubstring => 4,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvinceSearchEntry {
    pub province_id: u32,
    pub rgb: [u8; 3],
    pub kind: String,
    pub terrain: String,
    pub coastal: bool,
    pub continent: u16,
    pub state_id: Option<u32>,
    pub state_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvinceSearchMatch {
    ExactId,
    ExactRgb,
    StateId,
    Terrain,
    Kind,
    Coastal,
    Continent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvinceSearchResult {
    pub entry: ProvinceSearchEntry,
    pub matched: ProvinceSearchMatch,
}

#[derive(Debug, Clone, Default)]
pub struct ProvinceSearchIndex {
    entries: Vec<ProvinceSearchEntry>,
}

impl ProvinceSearchIndex {
    pub fn new(entries: impl IntoIterator<Item = ProvinceSearchEntry>) -> Self {
        let mut entries = entries.into_iter().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.province_id);
        Self { entries }
    }

    pub fn search(&self, query: &str) -> Vec<ProvinceSearchResult> {
        let query = query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let lower = query.to_ascii_lowercase();
        let compact = lower.replace(' ', "");
        let exact_id = query.parse::<u32>().ok();
        let state_id = lower
            .strip_prefix("state:")
            .and_then(|value| value.trim().parse::<u32>().ok());
        let continent = lower
            .strip_prefix("continent:")
            .and_then(|value| value.trim().parse::<u16>().ok());

        self.entries
            .iter()
            .filter_map(|entry| {
                let rgb = format!("{},{},{}", entry.rgb[0], entry.rgb[1], entry.rgb[2]);
                let matched = if exact_id == Some(entry.province_id) {
                    ProvinceSearchMatch::ExactId
                } else if compact == rgb {
                    ProvinceSearchMatch::ExactRgb
                } else if state_id.is_some_and(|id| entry.state_id == Some(id)) {
                    ProvinceSearchMatch::StateId
                } else if entry.terrain.eq_ignore_ascii_case(query) {
                    ProvinceSearchMatch::Terrain
                } else if entry.kind.eq_ignore_ascii_case(query) {
                    ProvinceSearchMatch::Kind
                } else if lower == "coastal" && entry.coastal {
                    ProvinceSearchMatch::Coastal
                } else if continent == Some(entry.continent) {
                    ProvinceSearchMatch::Continent
                } else {
                    return None;
                };
                Some(ProvinceSearchResult {
                    entry: entry.clone(),
                    matched,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateOpenSource {
    Loaded { path: String },
    CreatedInSession,
    NoSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickedState {
    pub state_id: u32,
    pub source: StateOpenSource,
}

impl ClickedState {
    pub fn loaded(state_id: u32, path: impl Into<String>) -> Self {
        Self {
            state_id,
            source: StateOpenSource::Loaded { path: path.into() },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateOpenBlock {
    CreatedInSession,
    NoSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoubleClickOutcome {
    Armed,
    OpenLoadedSource {
        state_id: u32,
        path: String,
    },
    Blocked {
        state_id: u32,
        reason: StateOpenBlock,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LastClick {
    state_id: u32,
    at_ms: u64,
    pos: [f64; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct DoubleClickTracker {
    interval_ms: u64,
    max_movement: f64,
    last: Option<LastClick>,
}

impl Default for DoubleClickTracker {
    fn default() -> Self {
        Self {
            interval_ms: 450,
            max_movement: 6.0,
            last: None,
        }
    }
}

impl DoubleClickTracker {
    pub fn new(interval_ms: u64, max_movement: f64) -> Self {
        Self {
            interval_ms,
            max_movement: max_movement.max(0.0),
            last: None,
        }
    }

    pub fn click(
        &mut self,
        clicked: ClickedState,
        at_ms: u64,
        pos: [f64; 2],
    ) -> DoubleClickOutcome {
        let is_double_click = self.last.is_some_and(|last| {
            last.state_id == clicked.state_id
                && at_ms.saturating_sub(last.at_ms) <= self.interval_ms
                && distance_squared(last.pos, pos) <= self.max_movement * self.max_movement
        });

        if !is_double_click {
            self.last = Some(LastClick {
                state_id: clicked.state_id,
                at_ms,
                pos,
            });
            return DoubleClickOutcome::Armed;
        }

        self.last = None;
        match clicked.source {
            StateOpenSource::Loaded { path } => DoubleClickOutcome::OpenLoadedSource {
                state_id: clicked.state_id,
                path,
            },
            StateOpenSource::CreatedInSession => DoubleClickOutcome::Blocked {
                state_id: clicked.state_id,
                reason: StateOpenBlock::CreatedInSession,
            },
            StateOpenSource::NoSource => DoubleClickOutcome::Blocked {
                state_id: clicked.state_id,
                reason: StateOpenBlock::NoSource,
            },
        }
    }
}

fn distance_squared(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    dx * dx + dy * dy
}

impl Ord for StateSearchResult {
    fn cmp(&self, other: &Self) -> Ordering {
        search_rank(self)
            .cmp(&search_rank(other))
            .then_with(|| self.state_id.cmp(&other.state_id))
            .then_with(|| self.name.cmp(&other.name))
    }
}

impl PartialOrd for StateSearchResult {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_reserves_only_session_panel_width() {
        assert_eq!(StateInspectorVisibility::Hidden.reserved_width(), 0.0);
        assert_eq!(StateInspectorVisibility::Collapsed.reserved_width(), 48.0);
        assert_eq!(StateInspectorVisibility::Expanded.reserved_width(), 420.0);
    }

    #[test]
    fn viewport_contains_points_inside_half_open_bounds() {
        let viewport = MapViewport::new(10.0, 20.0, 100.0, 50.0);

        assert!(viewport.contains([10.0, 20.0]));
        assert!(viewport.contains([109.9, 69.9]));
        assert!(!viewport.contains([110.0, 69.9]));
        assert!(!viewport.contains([109.9, 70.0]));
        assert_eq!(viewport.dimensions(), [100.0, 50.0]);
    }

    #[test]
    fn layout_keeps_map_bounds_and_overlays_the_inspector() {
        let layout = InspectorLayout::calculate(
            [1200.0, 800.0],
            36.0,
            28.0,
            StateInspectorVisibility::Expanded,
        );

        assert_eq!(
            layout.map_viewport,
            MapViewport::new(36.0, 28.0, 1164.0, 772.0)
        );
        assert_eq!(
            layout.inspector_viewport,
            Some(MapViewport::new(780.0, 28.0, 420.0, 772.0))
        );
        assert_eq!(layout.hit_test([35.0, 100.0]), InspectorHit::Chrome);
        assert_eq!(layout.hit_test([100.0, 27.0]), InspectorHit::Chrome);
        assert_eq!(layout.hit_test([100.0, 100.0]), InspectorHit::Map);
        assert_eq!(layout.hit_test([900.0, 100.0]), InspectorHit::Inspector);
    }

    #[test]
    fn drawer_width_preserves_useful_map_space_in_small_windows() {
        assert_eq!(inspector_drawer_width(1280.0, 36.0, 420.0), 420.0);
        assert_eq!(inspector_drawer_width(384.0, 36.0, 420.0), 128.0);
        assert_eq!(inspector_drawer_width(384.0, 36.0, 48.0), 48.0);
    }

    #[test]
    fn search_matches_exact_id_partial_id_and_name_substring() {
        let index = StateSearchIndex::new([
            StateSearchEntry::new(12, "North Rhine"),
            StateSearchEntry::new(120, "Southern Alps"),
            StateSearchEntry::new(301, "Rhine Delta"),
        ]);

        let exact = index.search("12");
        assert_eq!(exact[0].state_id, 12);
        assert_eq!(exact[0].matched, StateSearchMatch::ExactId);
        assert!(exact.iter().any(|result| {
            result.state_id == 120 && result.matched == StateSearchMatch::PartialId
        }));

        let name = index.search("rhine");
        assert_eq!(
            name.iter()
                .map(|result| result.state_id)
                .collect::<Vec<_>>(),
            vec![12, 301]
        );
        assert!(index.search("").is_empty());
    }

    #[test]
    fn state_search_matches_owner_controller_and_contained_province() {
        let index = StateSearchIndex::new([
            StateSearchEntry::new(516, "Ilha3").with_context(
                Some("BOM".to_owned()),
                Some("CTR".to_owned()),
                [16020, 16021],
            ),
            StateSearchEntry::new(517, "Other").with_context(Some("OTH".to_owned()), None, [17000]),
        ]);

        assert_eq!(index.search("BOM")[0].matched, StateSearchMatch::Owner);
        assert_eq!(index.search("CTR")[0].matched, StateSearchMatch::Controller);
        assert_eq!(
            index.search("16020")[0].matched,
            StateSearchMatch::ProvinceId
        );
    }

    #[test]
    fn province_search_matches_supported_context_fields() {
        let index = ProvinceSearchIndex::new([ProvinceSearchEntry {
            province_id: 16020,
            rgb: [255, 129, 66],
            kind: "land".to_owned(),
            terrain: "forest".to_owned(),
            coastal: true,
            continent: 3,
            state_id: Some(516),
            state_name: Some("Ilha3".to_owned()),
        }]);

        for query in [
            "16020",
            "255,129,66",
            "255, 129, 66",
            "forest",
            "land",
            "coastal",
            "continent:3",
            "state:516",
        ] {
            assert_eq!(index.search(query).len(), 1, "{query}");
        }
        assert!(index.search("state:999").is_empty());
    }

    #[test]
    fn double_click_opens_only_same_loaded_state_inside_limits() {
        let mut tracker = DoubleClickTracker::default();

        assert_eq!(
            tracker.click(
                ClickedState::loaded(1, "history/states/1.txt"),
                100,
                [10.0, 10.0]
            ),
            DoubleClickOutcome::Armed
        );
        assert_eq!(
            tracker.click(
                ClickedState::loaded(2, "history/states/2.txt"),
                200,
                [10.0, 10.0]
            ),
            DoubleClickOutcome::Armed
        );
        assert_eq!(
            tracker.click(
                ClickedState::loaded(2, "history/states/2.txt"),
                700,
                [10.0, 10.0]
            ),
            DoubleClickOutcome::Armed
        );
        assert_eq!(
            tracker.click(
                ClickedState::loaded(2, "history/states/2.txt"),
                800,
                [20.0, 20.0]
            ),
            DoubleClickOutcome::Armed
        );
        assert_eq!(
            tracker.click(
                ClickedState::loaded(2, "history/states/2.txt"),
                850,
                [21.0, 21.0]
            ),
            DoubleClickOutcome::OpenLoadedSource {
                state_id: 2,
                path: "history/states/2.txt".to_owned(),
            }
        );
        assert_eq!(
            tracker.click(
                ClickedState::loaded(2, "history/states/2.txt"),
                860,
                [21.0, 21.0]
            ),
            DoubleClickOutcome::Armed
        );
    }

    #[test]
    fn double_click_represents_created_and_missing_sources_without_opening() {
        let mut tracker = DoubleClickTracker::default();

        let created = ClickedState {
            state_id: 9,
            source: StateOpenSource::CreatedInSession,
        };
        assert_eq!(
            tracker.click(created.clone(), 0, [0.0, 0.0]),
            DoubleClickOutcome::Armed
        );
        assert_eq!(
            tracker.click(created, 1, [0.0, 0.0]),
            DoubleClickOutcome::Blocked {
                state_id: 9,
                reason: StateOpenBlock::CreatedInSession,
            }
        );

        let missing = ClickedState {
            state_id: 10,
            source: StateOpenSource::NoSource,
        };
        assert_eq!(
            tracker.click(missing.clone(), 2, [0.0, 0.0]),
            DoubleClickOutcome::Armed
        );
        assert_eq!(
            tracker.click(missing, 3, [0.0, 0.0]),
            DoubleClickOutcome::Blocked {
                state_id: 10,
                reason: StateOpenBlock::NoSource,
            }
        );
    }

    #[test]
    fn inspector_defaults_to_safe_session_values() {
        let state = StateInspectorState::session_default();

        assert_eq!(state.diagnostics_mode, DeveloperDiagnosticsMode::Off);
        assert_eq!(state.visibility, StateInspectorVisibility::Expanded);
        assert!(state.search.is_empty());
    }
}
