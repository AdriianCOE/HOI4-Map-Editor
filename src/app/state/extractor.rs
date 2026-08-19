use super::{
    DatedHistoryBlock, PdxBlock, PdxDocument, PdxEntry, PdxScalar, PdxScalarKind, PdxValue,
    StateData, SyntaxDiagnosticKind, VictoryPoint,
};
use crate::app::project::{DiagnosticSeverity, ProjectDiagnostic, ProjectDiagnosticKind};

#[derive(Debug, Clone)]
pub struct ExtractStateResult {
    pub data: Option<StateData>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

pub fn extract_state(document: &PdxDocument) -> ExtractStateResult {
    let extractor = Extractor::new(document);
    extractor.extract()
}

struct Extractor<'a> {
    document: &'a PdxDocument,
    diagnostics: Vec<ProjectDiagnostic>,
}

impl<'a> Extractor<'a> {
    fn new(document: &'a PdxDocument) -> Self {
        Self {
            document,
            diagnostics: Vec::new(),
        }
    }

    fn extract(mut self) -> ExtractStateResult {
        for diagnostic in &self.document.diagnostics {
            self.push(
                if diagnostic.kind == SyntaxDiagnosticKind::EmptyFile {
                    ProjectDiagnosticKind::EmptyStateFile
                } else {
                    ProjectDiagnosticKind::SyntaxError
                },
                DiagnosticSeverity::Error,
                Some(diagnostic.span),
                diagnostic.message.clone(),
            );
        }

        let state_blocks: Vec<usize> = self
            .document
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry
                    .key_text()
                    .is_some_and(|key| key.eq_ignore_ascii_case("state"))
                    && entry.value.as_block().is_some()
            })
            .map(|(index, _)| index)
            .collect();

        let first_block = state_blocks
            .first()
            .and_then(|index| self.document.entries.get(*index))
            .and_then(|entry| entry.value.as_block())
            .cloned();

        let multiple_spans: Vec<_> = state_blocks
            .iter()
            .skip(1)
            .filter_map(|index| self.document.entries.get(*index))
            .map(|entry| entry.span)
            .collect();

        if state_blocks.is_empty() {
            self.push(
                ProjectDiagnosticKind::MissingStateBlock,
                DiagnosticSeverity::Error,
                Some(self.document.span),
                "missing root state block",
            );
            return ExtractStateResult {
                data: None,
                diagnostics: self.diagnostics,
            };
        }

        if state_blocks.len() > 1 {
            for span in multiple_spans {
                self.push(
                    ProjectDiagnosticKind::MultipleStateBlocks,
                    DiagnosticSeverity::Error,
                    Some(span),
                    "multiple root state blocks",
                );
            }
        }

        let data = first_block
            .as_ref()
            .map(|block| self.extract_state_block(block));

        ExtractStateResult {
            data,
            diagnostics: self.diagnostics,
        }
    }

    fn extract_state_block(&mut self, block: &PdxBlock) -> StateData {
        let mut state = StateData::default();
        let mut has_history = false;

        for entry in &block.entries {
            match entry.key_text() {
                Some("id") => {
                    self.read_u32(entry, ProjectDiagnosticKind::InvalidStateId, |value| {
                        state.id = Some(value);
                    })
                }
                Some("name") => {
                    if let Some(value) = scalar_text(entry) {
                        state.name = Some(value.to_string());
                    }
                }
                Some("provinces") => self.extract_provinces(entry, &mut state),
                Some("manpower") => {
                    self.read_u64(entry, ProjectDiagnosticKind::InvalidManpower, |value| {
                        state.manpower = Some(value);
                    });
                }
                Some("buildings_max_level_factor") => {
                    self.read_f64(entry, ProjectDiagnosticKind::InvalidField, |value| {
                        state.buildings_max_level_factor = Some(value);
                    });
                }
                Some("state_category") => {
                    if let Some(value) = scalar_text(entry) {
                        state.state_category = Some(value.to_string());
                    }
                }
                Some("local_supplies") => {
                    self.read_f64(entry, ProjectDiagnosticKind::InvalidField, |value| {
                        state.local_supplies = Some(value);
                    });
                }
                Some("impassable") => {
                    state.impassable = self.read_bool(entry);
                }
                Some("resources") => self.extract_resources(entry, &mut state),
                Some("history") => {
                    has_history = true;
                    if let Some(history) = entry.value.as_block() {
                        self.extract_history(history, &mut state);
                    }
                }
                _ => {}
            }
        }

        if state.id == Some(0) {
            self.push(
                ProjectDiagnosticKind::ZeroStateId,
                DiagnosticSeverity::Error,
                field_span(block, "id"),
                "state id must be greater than zero",
            );
        }
        if state.id.is_none() {
            self.push(
                ProjectDiagnosticKind::MissingStateId,
                DiagnosticSeverity::Error,
                Some(block.span),
                "missing state id",
            );
        }
        if state.name.is_none() {
            self.push(
                ProjectDiagnosticKind::MissingStateName,
                DiagnosticSeverity::Warning,
                Some(block.span),
                "missing state name",
            );
        }
        if state.state_category.is_none() {
            self.push(
                ProjectDiagnosticKind::MissingStateCategory,
                DiagnosticSeverity::Warning,
                Some(block.span),
                "missing state category",
            );
        }
        if !has_history {
            self.push(
                ProjectDiagnosticKind::MissingHistory,
                DiagnosticSeverity::Warning,
                Some(block.span),
                "missing history block",
            );
        } else if state.history.owner.is_none() {
            self.push(
                ProjectDiagnosticKind::MissingOwner,
                DiagnosticSeverity::Warning,
                field_span(block, "history").or(Some(block.span)),
                "missing history owner",
            );
        }
        if state.provinces.is_empty() {
            let kind = if field_span(block, "provinces").is_some() {
                ProjectDiagnosticKind::EmptyProvinces
            } else {
                ProjectDiagnosticKind::MissingProvinces
            };
            self.push(
                kind,
                DiagnosticSeverity::Error,
                Some(block.span),
                "missing provinces",
            );
        }

        state
    }

    fn extract_provinces(&mut self, entry: &PdxEntry, state: &mut StateData) {
        let Some(block) = entry.value.as_block() else {
            self.push(
                ProjectDiagnosticKind::InvalidField,
                DiagnosticSeverity::Error,
                Some(entry.span),
                "provinces must be a block",
            );
            return;
        };

        for item in &block.entries {
            let Some(scalar) = item.value.as_scalar() else {
                self.push(
                    ProjectDiagnosticKind::InvalidField,
                    DiagnosticSeverity::Error,
                    Some(item.span),
                    "province id must be an integer",
                );
                continue;
            };
            let Some(province_id) = parse_u32(&scalar.text) else {
                self.push(
                    ProjectDiagnosticKind::InvalidField,
                    DiagnosticSeverity::Error,
                    Some(scalar.span),
                    "province id must be an integer",
                );
                continue;
            };
            if state.provinces.contains(&province_id) {
                self.push(
                    ProjectDiagnosticKind::DuplicateProvinceInState,
                    DiagnosticSeverity::Warning,
                    Some(scalar.span),
                    format!("duplicate province {province_id} in state"),
                );
            }
            state.provinces.insert(province_id);
        }
    }

    fn extract_resources(&mut self, entry: &PdxEntry, state: &mut StateData) {
        let Some(block) = entry.value.as_block() else {
            self.push(
                ProjectDiagnosticKind::InvalidResource,
                DiagnosticSeverity::Error,
                Some(entry.span),
                "resources must be a block",
            );
            return;
        };

        for item in &block.entries {
            let Some(resource) = item.key_text() else {
                continue;
            };
            let Some(amount) = scalar_text(item).and_then(parse_i64) else {
                self.push(
                    ProjectDiagnosticKind::InvalidResource,
                    DiagnosticSeverity::Error,
                    Some(item.span),
                    format!("resource {resource} must have an integer value"),
                );
                continue;
            };
            *state.resources.entry(resource.to_string()).or_insert(0) += amount;
        }
    }

    fn extract_history(&mut self, block: &PdxBlock, state: &mut StateData) {
        for entry in &block.entries {
            match entry.key_text() {
                Some("owner") => {
                    if let Some(value) = scalar_text(entry) {
                        state.history.owner = Some(value.to_string());
                    }
                }
                Some("controller") => {
                    if let Some(value) = scalar_text(entry) {
                        state.history.controller = Some(value.to_string());
                    }
                }
                Some("add_core_of") => {
                    if let Some(value) = scalar_text(entry) {
                        state.history.cores.insert(value.to_string());
                    }
                }
                Some("add_claim_by") => {
                    if let Some(value) = scalar_text(entry) {
                        state.history.claims.insert(value.to_string());
                    }
                }
                Some("remove_core_of") => {
                    if let Some(value) = scalar_text(entry) {
                        state.history.removed_cores.insert(value.to_string());
                    }
                }
                Some("remove_claim_by") => {
                    if let Some(value) = scalar_text(entry) {
                        state.history.removed_claims.insert(value.to_string());
                    }
                }
                Some("victory_points") => self.extract_victory_points(entry, state),
                Some("buildings") => self.extract_buildings(entry, state),
                Some(date) if is_date(date) => state.history.dated_blocks.push(DatedHistoryBlock {
                    date: date.to_string(),
                    span: entry.span,
                }),
                _ => {}
            }
        }
    }

    fn extract_victory_points(&mut self, entry: &PdxEntry, state: &mut StateData) {
        let Some(block) = entry.value.as_block() else {
            self.push(
                ProjectDiagnosticKind::InvalidField,
                DiagnosticSeverity::Error,
                Some(entry.span),
                "victory_points must be a block",
            );
            return;
        };

        let scalars: Vec<&PdxScalar> = block
            .entries
            .iter()
            .filter_map(|item| item.value.as_scalar())
            .collect();

        for pair in scalars.chunks(2) {
            let Some(province_scalar) = pair.first().copied() else {
                continue;
            };
            let Some(value_scalar) = pair.get(1).copied() else {
                self.push(
                    ProjectDiagnosticKind::InvalidField,
                    DiagnosticSeverity::Error,
                    Some(province_scalar.span),
                    "victory_points needs province/value pairs",
                );
                continue;
            };
            let Some(province_id) = parse_u32(&province_scalar.text) else {
                self.push(
                    ProjectDiagnosticKind::InvalidField,
                    DiagnosticSeverity::Error,
                    Some(province_scalar.span),
                    "victory point province must be an integer",
                );
                continue;
            };
            let Some(value) = parse_i64(&value_scalar.text) else {
                self.push(
                    ProjectDiagnosticKind::InvalidField,
                    DiagnosticSeverity::Error,
                    Some(value_scalar.span),
                    "victory point value must be an integer",
                );
                continue;
            };
            if !state.provinces.is_empty() && !state.provinces.contains(&province_id) {
                self.push(
                    ProjectDiagnosticKind::VictoryPointOutsideState,
                    DiagnosticSeverity::Warning,
                    Some(province_scalar.span),
                    format!("victory point province {province_id} is outside this state"),
                );
            }
            state
                .history
                .victory_points
                .push(VictoryPoint { province_id, value });
        }
    }

    fn extract_buildings(&mut self, entry: &PdxEntry, state: &mut StateData) {
        let Some(block) = entry.value.as_block() else {
            self.push(
                ProjectDiagnosticKind::InvalidBuilding,
                DiagnosticSeverity::Error,
                Some(entry.span),
                "buildings must be a block",
            );
            return;
        };

        for item in &block.entries {
            let Some(key) = item.key_text() else {
                continue;
            };
            if let Some(province_id) = parse_u32(key) {
                self.extract_province_buildings(item, province_id, state);
            } else {
                let Some(level) = building_level(item) else {
                    self.push(
                        ProjectDiagnosticKind::InvalidBuilding,
                        DiagnosticSeverity::Error,
                        Some(item.span),
                        format!("building {key} must have an integer level"),
                    );
                    continue;
                };
                *state
                    .history
                    .state_buildings
                    .entry(key.to_string())
                    .or_insert(0) += level;
            }
        }
    }

    fn extract_province_buildings(
        &mut self,
        entry: &PdxEntry,
        province_id: u32,
        state: &mut StateData,
    ) {
        if !state.provinces.is_empty() && !state.provinces.contains(&province_id) {
            self.push(
                ProjectDiagnosticKind::ProvinceBuildingOutsideState,
                DiagnosticSeverity::Warning,
                entry.key.as_ref().map(|key| key.span),
                format!("building province {province_id} is outside this state"),
            );
        }

        let Some(block) = entry.value.as_block() else {
            self.push(
                ProjectDiagnosticKind::InvalidBuilding,
                DiagnosticSeverity::Error,
                Some(entry.span),
                "province buildings must be a block",
            );
            return;
        };

        let buildings = state
            .history
            .province_buildings
            .entry(province_id)
            .or_default();

        for item in &block.entries {
            let Some(building) = item.key_text() else {
                continue;
            };
            let Some(level) = building_level(item) else {
                self.push(
                    ProjectDiagnosticKind::InvalidBuilding,
                    DiagnosticSeverity::Error,
                    Some(item.span),
                    format!("building {building} must have an integer level"),
                );
                continue;
            };
            *buildings.entry(building.to_string()).or_insert(0) += level;
        }
    }

    fn read_u32<F>(&mut self, entry: &PdxEntry, kind: ProjectDiagnosticKind, mut apply: F)
    where
        F: FnMut(u32),
    {
        match scalar_text(entry).and_then(parse_u32) {
            Some(value) => apply(value),
            None => self.push(
                kind,
                DiagnosticSeverity::Error,
                Some(entry.span),
                "expected unsigned integer",
            ),
        }
    }

    fn read_u64<F>(&mut self, entry: &PdxEntry, kind: ProjectDiagnosticKind, mut apply: F)
    where
        F: FnMut(u64),
    {
        match scalar_text(entry).and_then(parse_u64) {
            Some(value) => apply(value),
            None => self.push(
                kind,
                DiagnosticSeverity::Error,
                Some(entry.span),
                "expected unsigned integer",
            ),
        }
    }

    fn read_f64<F>(&mut self, entry: &PdxEntry, kind: ProjectDiagnosticKind, mut apply: F)
    where
        F: FnMut(f64),
    {
        match scalar_text(entry).and_then(parse_f64) {
            Some(value) => apply(value),
            None => self.push(
                kind,
                DiagnosticSeverity::Error,
                Some(entry.span),
                "expected number",
            ),
        }
    }

    fn read_bool(&mut self, entry: &PdxEntry) -> Option<bool> {
        match scalar_text(entry) {
            Some("yes") => Some(true),
            Some("no") => Some(false),
            Some(_) => {
                self.push(
                    ProjectDiagnosticKind::InvalidField,
                    DiagnosticSeverity::Error,
                    Some(entry.span),
                    "expected yes or no",
                );
                None
            }
            None => None,
        }
    }

    fn push(
        &mut self,
        kind: ProjectDiagnosticKind,
        severity: DiagnosticSeverity,
        span: Option<super::TextSpan>,
        message: impl Into<String>,
    ) {
        self.diagnostics.push(ProjectDiagnostic::new(
            kind,
            severity,
            Some(self.document.source.path.clone()),
            span,
            message,
        ));
    }
}

trait EntryExt {
    fn key_text(&self) -> Option<&str>;
}

impl EntryExt for PdxEntry {
    fn key_text(&self) -> Option<&str> {
        self.key.as_ref().map(|key| key.text.as_str())
    }
}

trait ValueExt {
    fn as_block(&self) -> Option<&PdxBlock>;
    fn as_scalar(&self) -> Option<&PdxScalar>;
}

impl ValueExt for PdxValue {
    fn as_block(&self) -> Option<&PdxBlock> {
        match self {
            PdxValue::Block(block) => Some(block),
            PdxValue::Scalar(_) => None,
        }
    }

    fn as_scalar(&self) -> Option<&PdxScalar> {
        match self {
            PdxValue::Scalar(scalar) => Some(scalar),
            PdxValue::Block(_) => None,
        }
    }
}

fn scalar_text(entry: &PdxEntry) -> Option<&str> {
    entry.value.as_scalar().map(|scalar| {
        if scalar.kind == PdxScalarKind::String {
            scalar
                .text
                .strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
                .unwrap_or(&scalar.text)
        } else {
            &scalar.text
        }
    })
}

fn field_span(block: &PdxBlock, field: &str) -> Option<super::TextSpan> {
    block
        .entries
        .iter()
        .find(|entry| entry.key_text() == Some(field))
        .map(|entry| entry.span)
}

/// Current HOI4 state files use both `building = 1` and the DLC-aware
/// `building = { level = 1 allowed = { ... } }` form. The latter still has a
/// normal editable level; metadata such as `allowed` remains in the source
/// document and is preserved by patch generation.
fn building_level(entry: &PdxEntry) -> Option<i64> {
    if let Some(level) = scalar_text(entry).and_then(parse_i64) {
        return Some(level);
    }
    let block = entry.value.as_block()?;
    match block
        .entries
        .iter()
        .find(|item| item.key_text() == Some("level"))
    {
        Some(level) => scalar_text(level).and_then(parse_i64),
        // Some scripted landmark blocks intentionally rely on the default
        // level. Treat the presence of the building as level one instead of
        // rejecting the entire State document.
        None => Some(1),
    }
}

fn parse_u32(text: &str) -> Option<u32> {
    text.parse::<u32>().ok()
}

fn parse_u64(text: &str) -> Option<u64> {
    text.parse::<u64>().ok()
}

fn parse_i64(text: &str) -> Option<i64> {
    text.parse::<i64>().ok().or_else(|| {
        let (whole, fraction) = text.split_once('.')?;
        (!fraction.is_empty() && fraction.bytes().all(|byte| byte == b'0'))
            .then(|| whole.parse::<i64>().ok())
            .flatten()
    })
}

fn parse_f64(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn is_date(text: &str) -> bool {
    let mut parts = text.split('.');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };

    !year.is_empty()
        && !month.is_empty()
        && !day.is_empty()
        && year.chars().all(|ch| ch.is_ascii_digit())
        && month.chars().all(|ch| ch.is_ascii_digit())
        && day.chars().all(|ch| ch.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::extract_state;
    use crate::app::project::{DiagnosticSeverity, ProjectDiagnosticKind};
    use crate::app::state::parse_text;

    #[test]
    fn extracts_valverde_exact_values() {
        let result = extract_state(&parse_text(
            "731-Valverde.txt",
            r#"state={
  id=731
  name="STATE_731"
  manpower=142000
  buildings_max_level_factor=1.000
  state_category=town
  local_supplies=0.25
  impassable=no
  provinces={ 1405 5144 5145 }
  resources={ steel=4 chromium=2 }
  history={
    owner=URG
    controller=URG
    add_core_of=URG
    add_claim_by=ARG
    victory_points={ 5144 5 }
    buildings={
      infrastructure=3
      industrial_complex=1
      5144={ naval_base=1 bunker=2 }
    }

  }
}"#,
        ));

        let state = result.data.expect("state data");
        assert!(result.diagnostics.is_empty());
        assert_eq!(state.id, Some(731));
        assert_eq!(state.name.as_deref(), Some("STATE_731"));
        assert_eq!(state.manpower, Some(142000));
        assert_eq!(state.buildings_max_level_factor, Some(1.0));
        assert_eq!(state.state_category.as_deref(), Some("town"));
        assert_eq!(state.local_supplies, Some(0.25));
        assert_eq!(state.impassable, Some(false));
        assert!(state.provinces.contains(&1405));
        assert_eq!(state.resources.get("steel"), Some(&4));
        assert_eq!(state.resources.get("chromium"), Some(&2));
        assert_eq!(state.history.owner.as_deref(), Some("URG"));
        assert_eq!(state.history.controller.as_deref(), Some("URG"));
        assert!(state.history.cores.contains("URG"));
        assert!(state.history.claims.contains("ARG"));
        assert_eq!(state.history.victory_points.len(), 1);
        assert_eq!(state.history.victory_points[0].province_id, 5144);
        assert_eq!(state.history.victory_points[0].value, 5);
        assert_eq!(
            state.history.state_buildings.get("infrastructure"),
            Some(&3)
        );
        assert_eq!(
            state
                .history
                .province_buildings
                .get(&5144)
                .and_then(|buildings| buildings.get("naval_base")),
            Some(&1)
        );
    }

    #[test]
    fn accepts_dlc_aware_buildings_with_a_nested_level() {
        let result = extract_state(&parse_text(
            "vanilla-landmark.txt",
            r#"state={
                id=70
                provinces={ 3484 }
                history={
                    owner=CZE
                    buildings={
                        3484={
                            landmark_bojnice_castle={
                                level=1
                                allowed={ has_dlc="Peace For Our Time" }
                            }
                            naval_headquarters={ level=2 allowed={ has_dlc="No Compromise, No Surrender" } }
                        }
                    }
                }
            }"#,
        ));
        let state = result.data.expect("nested building state must load");
        assert_eq!(
            state.history.province_buildings[&3484]["landmark_bojnice_castle"],
            1
        );
        assert_eq!(
            state.history.province_buildings[&3484]["naval_headquarters"],
            2
        );
        assert!(!result.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == ProjectDiagnosticKind::InvalidBuilding
                && diagnostic.severity == DiagnosticSeverity::Error
        }));
    }

    #[test]
    fn tolerates_custom_fields_and_accumulates_repeats() {
        let result = extract_state(&parse_text(
            "custom.txt",
            "state={ id=1 name=a state_category=city provinces={ 1 } resources={ steel=2.000 steel=3 } custom=yes history={ owner=TAG add_core_of=TAG add_core_of=ABC buildings={ arms_factory=1 arms_factory=2 1={ bunker=1 bunker=2 } } } }",
        ));

        let state = result.data.expect("state data");
        assert!(result.diagnostics.is_empty());
        assert_eq!(state.resources.get("steel"), Some(&5));
        assert!(state.history.cores.contains("TAG"));
        assert!(state.history.cores.contains("ABC"));
        assert_eq!(state.history.state_buildings.get("arms_factory"), Some(&3));
        assert_eq!(
            state
                .history
                .province_buildings
                .get(&1)
                .and_then(|buildings| buildings.get("bunker")),
            Some(&3)
        );
    }

    #[test]
    fn preserves_dated_blocks() {
        let result = extract_state(&parse_text(
            "dated.txt",
            "state={ id=2 name=a state_category=city provinces={ 2 } history={ owner=TAG 1939.1.1={ add_core_of=ABC buildings={ infrastructure=1 } } } }",
        ));

        let state = result.data.expect("state data");
        assert!(result.diagnostics.is_empty());
        assert_eq!(state.history.dated_blocks.len(), 1);
        assert_eq!(state.history.dated_blocks[0].date, "1939.1.1");
    }

    #[test]
    fn reports_malformed_state_cases() {
        let missing = extract_state(&parse_text("missing.txt", "foo={ id=1 }"));
        assert!(missing.data.is_none());
        assert!(
            missing
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == ProjectDiagnosticKind::MissingStateBlock)
        );

        let multiple = extract_state(&parse_text(
            "multiple.txt",
            "state={ id=1 name=a state_category=city provinces={ 1 } history={ owner=TAG } } state={ id=2 }",
        ));
        assert!(multiple.data.is_some());
        assert!(
            multiple
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == ProjectDiagnosticKind::MultipleStateBlocks)
        );

        let bad = extract_state(&parse_text(
            "bad.txt",
            "state={ id=abc provinces={ 1 1 nope } resources={ steel=x } history={ victory_points={ 2 1 1 } buildings={ 2={ bunker=1 } dockyard=x } } }",
        ));
        let kinds: Vec<ProjectDiagnosticKind> = bad
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind)
            .collect();
        assert!(kinds.contains(&ProjectDiagnosticKind::InvalidStateId));
        assert!(kinds.contains(&ProjectDiagnosticKind::DuplicateProvinceInState));
        assert!(kinds.contains(&ProjectDiagnosticKind::InvalidResource));
        assert!(kinds.contains(&ProjectDiagnosticKind::VictoryPointOutsideState));
        assert!(kinds.contains(&ProjectDiagnosticKind::ProvinceBuildingOutsideState));
        assert!(kinds.contains(&ProjectDiagnosticKind::InvalidBuilding));
        assert!(kinds.contains(&ProjectDiagnosticKind::MissingStateName));
        assert!(kinds.contains(&ProjectDiagnosticKind::MissingStateCategory));
        assert!(kinds.contains(&ProjectDiagnosticKind::MissingOwner));
    }
}
