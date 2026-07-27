use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::Hoi4Project;
use crate::app::state::{
    PdxBlock, PdxDocument, PdxEntry, PdxScalarKind, PdxValue, SourceText, parse,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionSource {
    Fallback,
    BaseGame,
    LoadedProject,
    CurrentUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: String,
    pub source: DefinitionSource,
    pub path: Option<PathBuf>,
    pub observed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildingScope {
    State,
    Province,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildingCatalogEntry {
    pub name: String,
    pub source: DefinitionSource,
    pub path: Option<PathBuf>,
    pub observed: bool,
    pub scope: BuildingScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogDiagnosticSeverity {
    Info,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogDiagnostic {
    pub severity: CatalogDiagnosticSeverity,
    pub path: Option<PathBuf>,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct GameDefinitionCatalog {
    pub resources: BTreeMap<String, CatalogEntry>,
    pub state_categories: BTreeMap<String, CatalogEntry>,
    pub buildings: BTreeMap<String, BuildingCatalogEntry>,
    pub country_tags: BTreeMap<String, CatalogEntry>,
    pub diagnostics: Vec<CatalogDiagnostic>,
}

impl GameDefinitionCatalog {
    pub fn build(project: &Hoi4Project, base_game_root: Option<&Path>) -> Self {
        let mut catalog = Self::default();
        catalog.add_fallbacks();

        if let Some(root) = base_game_root {
            catalog.scan_root(root, DefinitionSource::BaseGame);
        }

        catalog.scan_root(&project.paths.root, DefinitionSource::LoadedProject);
        catalog.add_observed_project_values(project);
        catalog
    }

    fn add_fallbacks(&mut self) {
        for name in [
            "oil",
            "aluminium",
            "rubber",
            "tungsten",
            "steel",
            "chromium",
        ] {
            self.insert_entry(
                CatalogKind::Resource,
                name,
                DefinitionSource::Fallback,
                None,
                false,
            );
        }
        for name in [
            "wasteland",
            "enclave",
            "tiny_island",
            "pastoral",
            "rural",
            "town",
            "large_town",
            "city",
            "large_city",
            "metropolis",
            "megalopolis",
        ] {
            self.insert_entry(
                CatalogKind::StateCategory,
                name,
                DefinitionSource::Fallback,
                None,
                false,
            );
        }
        for (name, scope) in [
            ("infrastructure", BuildingScope::State),
            ("arms_factory", BuildingScope::State),
            ("industrial_complex", BuildingScope::State),
            ("dockyard", BuildingScope::State),
            ("air_base", BuildingScope::State),
            ("anti_air_building", BuildingScope::State),
            ("radar_station", BuildingScope::State),
            ("synthetic_refinery", BuildingScope::State),
            ("fuel_silo", BuildingScope::State),
            ("rocket_site", BuildingScope::State),
            ("nuclear_reactor", BuildingScope::State),
            ("naval_base", BuildingScope::Province),
            ("bunker", BuildingScope::Province),
            ("coastal_bunker", BuildingScope::Province),
            ("supply_node", BuildingScope::Province),
            ("rail_way", BuildingScope::Province),
        ] {
            self.insert_building(name, DefinitionSource::Fallback, None, false, scope);
        }
        for name in ["GER", "ENG", "SOV", "USA", "FRA", "ITA", "JAP", "CHI"] {
            self.insert_entry(
                CatalogKind::CountryTag,
                name,
                DefinitionSource::Fallback,
                None,
                false,
            );
        }
    }

    fn scan_root(&mut self, root: &Path, source: DefinitionSource) {
        if !root.is_dir() {
            self.push(
                CatalogDiagnosticSeverity::Warning,
                Some(root.to_owned()),
                format!("definition root is not a directory: {}", root.display()),
            );
            return;
        }

        self.scan_definition_dir(root.join("common/resources"), source, CatalogKind::Resource);
        self.scan_definition_dir(
            root.join("common/state_category"),
            source,
            CatalogKind::StateCategory,
        );
        self.scan_definition_dir(
            root.join("common/state_categories"),
            source,
            CatalogKind::StateCategory,
        );
        self.scan_definition_dir(root.join("common/buildings"), source, CatalogKind::Building);
        self.scan_definition_dir(
            root.join("common/country_tags"),
            source,
            CatalogKind::CountryTag,
        );
    }

    fn scan_definition_dir(&mut self, dir: PathBuf, source: DefinitionSource, kind: CatalogKind) {
        let files = match txt_files(&dir) {
            Ok(files) => files,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                self.push(
                    CatalogDiagnosticSeverity::Info,
                    Some(dir),
                    "definition directory is absent; using loaded values and fallbacks",
                );
                return;
            }
            Err(err) => {
                self.push(
                    CatalogDiagnosticSeverity::Warning,
                    Some(dir),
                    format!("failed to read definition directory: {err}"),
                );
                return;
            }
        };

        for path in files {
            self.scan_definition_file(&path, source, kind);
        }
    }

    fn scan_definition_file(&mut self, path: &Path, source: DefinitionSource, kind: CatalogKind) {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                self.push(
                    CatalogDiagnosticSeverity::Warning,
                    Some(path.to_owned()),
                    format!("failed to read definition file: {err}"),
                );
                return;
            }
        };
        let document = parse(SourceText::new(path.to_owned(), text));
        for diagnostic in &document.diagnostics {
            self.push(
                CatalogDiagnosticSeverity::Warning,
                Some(path.to_owned()),
                format!("definition parse diagnostic: {}", diagnostic.message),
            );
        }

        match kind {
            CatalogKind::Resource => self.scan_resource_document(&document, source, path),
            CatalogKind::StateCategory => {
                self.scan_named_block_document(&document, source, path, kind)
            }
            CatalogKind::Building => self.scan_building_document(&document, source, path),
            CatalogKind::CountryTag => self.scan_country_tag_document(&document, source, path),
        }
    }

    fn scan_resource_document(
        &mut self,
        document: &PdxDocument,
        source: DefinitionSource,
        path: &Path,
    ) {
        let resource_block = document
            .entries
            .iter()
            .find(|entry| entry.key_text() == Some("resources"))
            .and_then(|entry| entry.value.as_block());

        if let Some(block) = resource_block {
            self.scan_named_block_entries(block, source, path, CatalogKind::Resource);
        } else {
            self.scan_named_block_document(document, source, path, CatalogKind::Resource);
        }
    }

    fn scan_named_block_document(
        &mut self,
        document: &PdxDocument,
        source: DefinitionSource,
        path: &Path,
        kind: CatalogKind,
    ) {
        for entry in &document.entries {
            if entry.value.as_block().is_some()
                && let Some(name) = entry.key_text()
            {
                self.insert_entry(kind, name, source, Some(path.to_owned()), false);
            }
        }
    }

    fn scan_named_block_entries(
        &mut self,
        block: &PdxBlock,
        source: DefinitionSource,
        path: &Path,
        kind: CatalogKind,
    ) {
        for entry in &block.entries {
            if let Some(name) = entry.key_text() {
                self.insert_entry(kind, name, source, Some(path.to_owned()), false);
            }
        }
    }

    fn scan_building_document(
        &mut self,
        document: &PdxDocument,
        source: DefinitionSource,
        path: &Path,
    ) {
        let blocks: Vec<&PdxBlock> = document
            .entries
            .iter()
            .filter_map(|entry| {
                if entry.key_text() == Some("buildings") {
                    entry.value.as_block()
                } else {
                    None
                }
            })
            .collect();

        if blocks.is_empty() {
            for entry in &document.entries {
                if let Some(block) = entry.value.as_block()
                    && let Some(name) = entry.key_text()
                {
                    self.insert_building(
                        name,
                        source,
                        Some(path.to_owned()),
                        false,
                        building_scope(block),
                    );
                }
            }
            return;
        }

        for block in blocks {
            for entry in &block.entries {
                if let Some(name) = entry.key_text()
                    && let Some(building) = entry.value.as_block()
                {
                    self.insert_building(
                        name,
                        source,
                        Some(path.to_owned()),
                        false,
                        building_scope(building),
                    );
                }
            }
        }
    }

    fn scan_country_tag_document(
        &mut self,
        document: &PdxDocument,
        source: DefinitionSource,
        path: &Path,
    ) {
        for entry in &document.entries {
            let Some(tag) = entry.key_text() else {
                continue;
            };
            if is_country_tag(tag) {
                self.insert_entry(
                    CatalogKind::CountryTag,
                    tag,
                    source,
                    Some(path.to_owned()),
                    false,
                );
            }
        }
    }

    fn add_observed_project_values(&mut self, project: &Hoi4Project) {
        for document in &project.states {
            let Some(data) = document.data.as_ref() else {
                continue;
            };
            if let Some(category) = data.state_category.as_deref() {
                self.observe_entry(CatalogKind::StateCategory, category);
            }
            for resource in data.resources.keys() {
                self.observe_entry(CatalogKind::Resource, resource);
            }
            for building in data.history.state_buildings.keys() {
                self.observe_building(building, BuildingScope::State);
            }
            for buildings in data.history.province_buildings.values() {
                for building in buildings.keys() {
                    self.observe_building(building, BuildingScope::Province);
                }
            }
            for tag in data
                .history
                .owner
                .iter()
                .chain(data.history.controller.iter())
                .chain(data.history.cores.iter())
                .chain(data.history.claims.iter())
                .chain(data.history.removed_cores.iter())
                .chain(data.history.removed_claims.iter())
            {
                if is_country_tag(tag) {
                    self.observe_entry(CatalogKind::CountryTag, tag);
                }
            }
        }
    }

    fn observe_entry(&mut self, kind: CatalogKind, name: &str) {
        let map = self.entries_mut(kind);
        match map.get_mut(name) {
            Some(entry) => entry.observed = true,
            None => {
                map.insert(
                    name.to_owned(),
                    CatalogEntry {
                        name: name.to_owned(),
                        source: DefinitionSource::CurrentUnknown,
                        path: None,
                        observed: true,
                    },
                );
            }
        }
    }

    fn observe_building(&mut self, name: &str, observed_scope: BuildingScope) {
        match self.buildings.get_mut(name) {
            Some(entry) => {
                entry.observed = true;
                if entry.scope == BuildingScope::Unknown {
                    entry.scope = observed_scope;
                }
            }
            None => {
                self.buildings.insert(
                    name.to_owned(),
                    BuildingCatalogEntry {
                        name: name.to_owned(),
                        source: DefinitionSource::CurrentUnknown,
                        path: None,
                        observed: true,
                        scope: observed_scope,
                    },
                );
            }
        }
    }

    fn insert_entry(
        &mut self,
        kind: CatalogKind,
        name: &str,
        source: DefinitionSource,
        path: Option<PathBuf>,
        observed: bool,
    ) {
        self.entries_mut(kind).insert(
            name.to_owned(),
            CatalogEntry {
                name: name.to_owned(),
                source,
                path,
                observed,
            },
        );
    }

    fn insert_building(
        &mut self,
        name: &str,
        source: DefinitionSource,
        path: Option<PathBuf>,
        observed: bool,
        scope: BuildingScope,
    ) {
        self.buildings.insert(
            name.to_owned(),
            BuildingCatalogEntry {
                name: name.to_owned(),
                source,
                path,
                observed,
                scope,
            },
        );
    }

    fn entries_mut(&mut self, kind: CatalogKind) -> &mut BTreeMap<String, CatalogEntry> {
        match kind {
            CatalogKind::Resource => &mut self.resources,
            CatalogKind::StateCategory => &mut self.state_categories,
            CatalogKind::CountryTag => &mut self.country_tags,
            CatalogKind::Building => unreachable!("building entries use insert_building"),
        }
    }

    fn push(
        &mut self,
        severity: CatalogDiagnosticSeverity,
        path: Option<PathBuf>,
        message: impl Into<String>,
    ) {
        self.diagnostics.push(CatalogDiagnostic {
            severity,
            path,
            message: message.into(),
        });
    }
}

#[derive(Debug, Clone, Copy)]
enum CatalogKind {
    Resource,
    StateCategory,
    Building,
    CountryTag,
}

trait EntryExt {
    fn key_text(&self) -> Option<&str>;
}

impl EntryExt for PdxEntry {
    fn key_text(&self) -> Option<&str> {
        self.key
            .as_ref()
            .map(|key| scalar_text(&key.text, key.kind))
    }
}

trait ValueExt {
    fn as_block(&self) -> Option<&PdxBlock>;
    fn scalar_text(&self) -> Option<&str>;
}

impl ValueExt for PdxValue {
    fn as_block(&self) -> Option<&PdxBlock> {
        match self {
            PdxValue::Block(block) => Some(block),
            PdxValue::Scalar(_) => None,
        }
    }

    fn scalar_text(&self) -> Option<&str> {
        match self {
            PdxValue::Scalar(scalar) => Some(scalar_text(&scalar.text, scalar.kind)),
            PdxValue::Block(_) => None,
        }
    }
}

fn scalar_text(text: &str, kind: PdxScalarKind) -> &str {
    if kind == PdxScalarKind::String {
        text.strip_prefix('"')
            .and_then(|text| text.strip_suffix('"'))
            .unwrap_or(text)
    } else {
        text
    }
}

fn building_scope(block: &PdxBlock) -> BuildingScope {
    for entry in &block.entries {
        match (entry.key_text(), entry.value.scalar_text()) {
            (Some("province" | "provincial"), Some("yes")) => return BuildingScope::Province,
            (Some("province" | "provincial"), Some("no")) => return BuildingScope::State,
            (Some("state" | "statewide"), Some("yes")) => return BuildingScope::State,
            (Some("type"), Some("province" | "provincial")) => return BuildingScope::Province,
            (Some("type"), Some("state" | "statewide")) => return BuildingScope::State,
            _ => {}
        }
    }
    BuildingScope::Unknown
}

fn txt_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut pending = vec![dir.to_owned()];
    let mut files = Vec::new();
    while let Some(current) = pending.pop() {
        let mut entries = fs::read_dir(&current)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "txt")
            {
                files.push(path);
            }
        }
        pending.sort();
    }
    files.sort();
    Ok(files)
}

fn is_country_tag(tag: &str) -> bool {
    tag.len() == 3
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::project::ProjectPaths;
    use crate::app::state::{StateData, StateDocument, parse_text};
    use std::sync::Arc;

    struct TempProject(PathBuf);

    impl TempProject {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("hoi4-catalog-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("map")).unwrap();
            fs::create_dir_all(root.join("history/states")).unwrap();
            fs::write(root.join("map/provinces.bmp"), []).unwrap();
            fs::write(root.join("map/definition.csv"), []).unwrap();
            Self(root)
        }

        fn paths(&self) -> ProjectPaths {
            ProjectPaths {
                root: self.0.clone(),
                map_directory: self.0.join("map"),
                provinces_bmp: self.0.join("map/provinces.bmp"),
                definition_csv: self.0.join("map/definition.csv"),
                adjacencies_csv: None,
                rivers_bmp: None,
                history_directory: self.0.join("history"),
                states_directory: self.0.join("history/states"),
            }
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loads_common_definitions_and_project_overrides_deterministically() {
        let base = TempProject::new("base");
        let project_root = TempProject::new("project");
        fs::create_dir_all(base.0.join("common/resources")).unwrap();
        fs::create_dir_all(project_root.0.join("common/resources")).unwrap();
        fs::create_dir_all(project_root.0.join("common/buildings")).unwrap();
        fs::create_dir_all(project_root.0.join("common/country_tags")).unwrap();
        fs::write(
            base.0.join("common/resources/base.txt"),
            "resources={ steel={} custom_base={} }",
        )
        .unwrap();
        fs::write(
            project_root.0.join("common/resources/project.txt"),
            "resources={ steel={} unobtainium={} }",
        )
        .unwrap();
        fs::write(
            project_root.0.join("common/buildings/buildings.txt"),
            "buildings={ local_fort={ provincial=yes } local_factory={ province=no } }",
        )
        .unwrap();
        fs::write(
            project_root.0.join("common/country_tags/tags.txt"),
            "ABC=\"countries/ABC.txt\"",
        )
        .unwrap();

        let project = Hoi4Project::new(project_root.paths());
        let catalog = GameDefinitionCatalog::build(&project, Some(&base.0));

        assert_eq!(
            catalog.resources["custom_base"].source,
            DefinitionSource::BaseGame
        );
        assert_eq!(
            catalog.resources["steel"].source,
            DefinitionSource::LoadedProject
        );
        assert_eq!(
            catalog.resources["unobtainium"].source,
            DefinitionSource::LoadedProject
        );
        assert_eq!(
            catalog.buildings["local_fort"].scope,
            BuildingScope::Province
        );
        assert_eq!(
            catalog.buildings["local_factory"].scope,
            BuildingScope::State
        );
        assert_eq!(
            catalog.country_tags["ABC"].source,
            DefinitionSource::LoadedProject
        );
    }

    #[test]
    fn preserves_values_observed_in_loaded_states() {
        let root = TempProject::new("observed");
        let mut state = StateData {
            state_category: Some("custom_category".to_owned()),
            ..Default::default()
        };
        state.resources.insert("custom_resource".to_owned(), 7);
        state.history.owner = Some("XYZ".to_owned());
        state
            .history
            .state_buildings
            .insert("custom_state_building".to_owned(), 1);
        state.history.province_buildings.insert(
            42,
            BTreeMap::from([("custom_province_building".to_owned(), 2)]),
        );
        let mut project = Hoi4Project::new(root.paths());
        project.states.push(StateDocument {
            path: root.0.join("history/states/1-test.txt"),
            original_bytes: Arc::<[u8]>::from([]),
            exact_utf8: true,
            syntax: parse_text("1-test.txt", ""),
            data: Some(state),
            diagnostics: Vec::new(),
            modified: false,
        });

        let catalog = GameDefinitionCatalog::build(&project, None);

        assert_eq!(
            catalog.resources["custom_resource"].source,
            DefinitionSource::CurrentUnknown
        );
        assert!(catalog.resources["custom_resource"].observed);
        assert_eq!(
            catalog.state_categories["custom_category"].source,
            DefinitionSource::CurrentUnknown
        );
        assert_eq!(
            catalog.buildings["custom_state_building"].scope,
            BuildingScope::State
        );
        assert_eq!(
            catalog.buildings["custom_province_building"].scope,
            BuildingScope::Province
        );
        assert_eq!(
            catalog.country_tags["XYZ"].source,
            DefinitionSource::CurrentUnknown
        );
    }

    #[test]
    fn missing_common_directories_are_non_fatal() {
        let root = TempProject::new("missing-common");
        let project = Hoi4Project::new(root.paths());
        let catalog = GameDefinitionCatalog::build(&project, None);

        assert!(catalog.resources.contains_key("steel"));
        assert!(catalog.state_categories.contains_key("rural"));
        assert!(catalog.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == CatalogDiagnosticSeverity::Info
                && diagnostic
                    .message
                    .contains("definition directory is absent")
        }));
    }
}
