//! Generation-owned project map source resolution.
//!
//! `map/default.map` may redirect the supported map files; when it is absent,
//! conventional HOI4 names remain compatible. Core map files must be owned by
//! the current project, while the resolver exposes read-only lower-source
//! policy for future non-map domains and honors `replace_path`. DLC and
//! integrated DLC discovery is lazy and ZIP entries remain read-only sources.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;

use zip::ZipArchive;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SourceGeneration(u64);

impl SourceGeneration {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SourceKind {
    CurrentProject,
    Dlc,
    IntegratedDlc,
    BaseGame,
}

impl SourceKind {
    /// Coarse source policy rank for merge-style consumers whose entries come
    /// from distinct logical files. Exact file collisions remain resolved by
    /// `SourceResolver`; this rank only preserves project/DLC/base intent when
    /// parsers merge independent files into one catalog.
    pub(crate) const fn precedence_rank(self) -> u8 {
        match self {
            Self::CurrentProject => 4,
            Self::Dlc => 3,
            Self::IntegratedDlc => 2,
            Self::BaseGame => 1,
        }
    }
}

/// The actual read-only storage for a resolved logical source.
///
/// Archive entries deliberately remain separate from filesystem paths: callers
/// must use `SourceResolver::read_resolved` rather than accidentally treating a
/// ZIP member as a writable file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedLocation {
    Filesystem(PathBuf),
    ArchiveEntry {
        archive_path: PathBuf,
        entry_path: PathBuf,
    },
}

impl ResolvedLocation {
    pub fn filesystem_path(&self) -> Option<&Path> {
        match self {
            Self::Filesystem(path) => Some(path),
            Self::ArchiveEntry { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSource {
    pub logical_path: PathBuf,
    pub location: ResolvedLocation,
    pub source_kind: SourceKind,
    pub project_generation: SourceGeneration,
}

impl ResolvedSource {
    pub fn filesystem_path(&self) -> Option<&Path> {
        self.location.filesystem_path()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceLookup {
    Found(ResolvedSource),
    NotFound,
    BlockedByReplacePath { logical_path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceListing {
    pub files: Vec<ResolvedSource>,
    /// Lower-priority copies which were deliberately hidden by source
    /// precedence. This makes the editor policy observable without claiming it
    /// is the HOI4 engine's package collision order.
    pub shadowed: Vec<ResolvedSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSources {
    resolver: SourceResolver,
    manifest: ProjectSourceManifest,
}

impl ProjectSources {
    #[cfg(test)]
    pub(crate) fn for_test_placeholder() -> Self {
        let generation = SourceGeneration::default();
        let source = |logical_path: &str| ResolvedSource {
            logical_path: PathBuf::from(logical_path),
            location: ResolvedLocation::Filesystem(PathBuf::new()),
            source_kind: SourceKind::CurrentProject,
            project_generation: generation,
        };
        Self {
            resolver: SourceResolver::new(generation),
            manifest: ProjectSourceManifest {
                project_root: PathBuf::new(),
                map_directory: PathBuf::new(),
                base_game_root: None,
                project_generation: generation,
                replace_paths: ReplacePathSet::new(),
                descriptor: None,
                default_map: None,
                map_layout: MapFileLayout::conventional(),
                map_files: ProjectMapSourceFiles {
                    provinces_bmp: source("map/provinces.bmp"),
                    definition_csv: source("map/definition.csv"),
                    adjacencies_csv: None,
                    continent_txt: None,
                    rivers_bmp: None,
                },
            },
        }
    }

    pub fn discover(
        root: impl Into<PathBuf>,
        base_game_root: Option<PathBuf>,
        project_generation: SourceGeneration,
    ) -> Result<Self, SourceResolutionError> {
        let root = root.into();
        let map_directory = root.join("map");
        require_current_directory(Path::new("project"), &root)?;
        require_current_directory(Path::new("map"), &map_directory)?;

        let replace_paths = ReplacePathSet::discover_current_project(&root)?;
        let resolver = SourceResolver::new(project_generation)
            .with_current_project_root(root.clone())
            .with_optional_base_game_root(base_game_root.clone())
            .with_replace_paths(replace_paths.clone());
        let descriptor =
            current_project_file(&root, Path::new("descriptor.mod"), project_generation);
        let default_map =
            current_project_file(&root, &default_map_logical_path(), project_generation);
        let layout = match &default_map {
            Some(source) => {
                let bytes = resolver.read_resolved(source)?;
                let text = String::from_utf8(bytes).map_err(|error| {
                    SourceResolutionError::InvalidUtf8 {
                        logical_path: source.logical_path.clone(),
                        source: error,
                    }
                })?;
                MapFileLayout::parse(&source.logical_path, &text)?
            }
            None => MapFileLayout::conventional(),
        };
        let layout = layout.with_conventional_map_fallbacks();

        let provinces_bmp = required_current_project_file(
            &root,
            layout
                .provinces_bmp
                .as_deref()
                .unwrap_or_else(|| Path::new("map/provinces.bmp")),
            project_generation,
        )?;
        let definition_csv = required_current_project_file(
            &root,
            layout
                .definition_csv
                .as_deref()
                .unwrap_or_else(|| Path::new("map/definition.csv")),
            project_generation,
        )?;
        let adjacencies_csv = optional_current_project_file(
            &root,
            layout.adjacencies_csv.as_deref(),
            project_generation,
        );
        let continent_txt = optional_current_project_file(
            &root,
            layout.continent_txt.as_deref(),
            project_generation,
        );
        let rivers_bmp =
            optional_current_project_file(&root, layout.rivers_bmp.as_deref(), project_generation);

        let manifest = ProjectSourceManifest {
            project_root: root,
            map_directory,
            base_game_root,
            project_generation,
            replace_paths,
            descriptor,
            default_map,
            map_layout: layout,
            map_files: ProjectMapSourceFiles {
                provinces_bmp,
                definition_csv,
                adjacencies_csv,
                continent_txt,
                rivers_bmp,
            },
        };

        Ok(Self { resolver, manifest })
    }

    pub fn resolver(&self) -> &SourceResolver {
        &self.resolver
    }

    pub fn manifest(&self) -> &ProjectSourceManifest {
        &self.manifest
    }

    pub fn source_files(&self) -> &ProjectMapSourceFiles {
        &self.manifest.map_files
    }

    pub fn resolve(
        &self,
        logical_path: impl AsRef<Path>,
    ) -> Result<SourceLookup, SourceResolutionError> {
        self.resolver.resolve(logical_path)
    }

    pub fn list_files(
        &self,
        logical_directory: impl AsRef<Path>,
    ) -> Result<SourceListing, SourceResolutionError> {
        self.resolver.list_files(logical_directory)
    }

    pub fn read_resolved(&self, source: &ResolvedSource) -> Result<Vec<u8>, SourceResolutionError> {
        self.resolver.read_resolved(source)
    }

    /// Binds lower-source discovery only after Canvas has accepted the
    /// editor's validated base-game root. Core map ownership remains current
    /// project only and is not recomputed here.
    pub fn set_validated_base_game_root(&mut self, root: Option<PathBuf>) {
        self.resolver.base_game_root = root.clone();
        self.resolver.lower_source_cache = RwLock::new(LowerSourceCache::default());
        self.manifest.base_game_root = root;
    }

    /// The App binds a fully loaded project to its replacement generation only
    /// after loading succeeds. Updating this owned manifest cannot affect a
    /// previously active project or a failed candidate project.
    pub fn rebind_generation(&mut self, generation: SourceGeneration) {
        self.resolver.project_generation = generation;
        self.manifest.project_generation = generation;
        for source in [
            self.manifest.descriptor.as_mut(),
            self.manifest.default_map.as_mut(),
            Some(&mut self.manifest.map_files.provinces_bmp),
            Some(&mut self.manifest.map_files.definition_csv),
            self.manifest.map_files.adjacencies_csv.as_mut(),
            self.manifest.map_files.continent_txt.as_mut(),
            self.manifest.map_files.rivers_bmp.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            source.project_generation = generation;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSourceManifest {
    pub project_root: PathBuf,
    pub map_directory: PathBuf,
    pub base_game_root: Option<PathBuf>,
    pub project_generation: SourceGeneration,
    pub replace_paths: ReplacePathSet,
    pub descriptor: Option<ResolvedSource>,
    pub default_map: Option<ResolvedSource>,
    pub map_layout: MapFileLayout,
    pub map_files: ProjectMapSourceFiles,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMapSourceFiles {
    pub provinces_bmp: ResolvedSource,
    pub definition_csv: ResolvedSource,
    pub adjacencies_csv: Option<ResolvedSource>,
    pub continent_txt: Option<ResolvedSource>,
    pub rivers_bmp: Option<ResolvedSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapFileLayout {
    pub default_map: PathBuf,
    pub provinces_bmp: Option<PathBuf>,
    pub definition_csv: Option<PathBuf>,
    pub adjacencies_csv: Option<PathBuf>,
    pub rivers_bmp: Option<PathBuf>,
    pub heightmap_bmp: Option<PathBuf>,
    pub terrain_bmp: Option<PathBuf>,
    pub trees_bmp: Option<PathBuf>,
    pub cities_bmp: Option<PathBuf>,
    pub continent_txt: Option<PathBuf>,
    pub positions_txt: Option<PathBuf>,
    pub ambient_object_txt: Option<PathBuf>,
    pub seasons_txt: Option<PathBuf>,
    pub supply_nodes_txt: Option<PathBuf>,
    pub railways_txt: Option<PathBuf>,
}

impl MapFileLayout {
    pub fn conventional() -> Self {
        Self {
            default_map: default_map_logical_path(),
            provinces_bmp: Some(PathBuf::from("map").join("provinces.bmp")),
            definition_csv: Some(PathBuf::from("map").join("definition.csv")),
            adjacencies_csv: Some(PathBuf::from("map").join("adjacencies.csv")),
            rivers_bmp: Some(PathBuf::from("map").join("rivers.bmp")),
            heightmap_bmp: None,
            terrain_bmp: None,
            trees_bmp: None,
            cities_bmp: None,
            continent_txt: Some(PathBuf::from("map").join("continent.txt")),
            positions_txt: None,
            ambient_object_txt: None,
            seasons_txt: None,
            supply_nodes_txt: None,
            railways_txt: None,
        }
    }

    pub fn parse_default_map(text: &str) -> Result<Self, SourceResolutionError> {
        Self::parse(default_map_logical_path(), text)
    }

    pub fn parse(default_map: impl AsRef<Path>, text: &str) -> Result<Self, SourceResolutionError> {
        let default_map = normalize_logical_path(default_map)?;
        let mut layout = Self {
            default_map,
            provinces_bmp: None,
            definition_csv: None,
            adjacencies_csv: None,
            rivers_bmp: None,
            heightmap_bmp: None,
            terrain_bmp: None,
            trees_bmp: None,
            cities_bmp: None,
            continent_txt: None,
            positions_txt: None,
            ambient_object_txt: None,
            seasons_txt: None,
            supply_nodes_txt: None,
            railways_txt: None,
        };

        for assignment in parse_assignments(text) {
            let Some(field) = DefaultMapField::from_key(&assignment.key) else {
                continue;
            };
            let logical_path = map_child_logical_path(&assignment.value).map_err(|source| {
                SourceResolutionError::Parse {
                    line: assignment.line,
                    key: assignment.key.clone(),
                    value: assignment.value.clone(),
                    source,
                }
            })?;
            layout.set(field, logical_path);
        }

        Ok(layout)
    }

    pub fn with_conventional_map_fallbacks(mut self) -> Self {
        self.provinces_bmp
            .get_or_insert_with(|| PathBuf::from("map").join("provinces.bmp"));
        self.definition_csv
            .get_or_insert_with(|| PathBuf::from("map").join("definition.csv"));
        self.adjacencies_csv
            .get_or_insert_with(|| PathBuf::from("map").join("adjacencies.csv"));
        self.continent_txt
            .get_or_insert_with(|| PathBuf::from("map").join("continent.txt"));
        self.rivers_bmp
            .get_or_insert_with(|| PathBuf::from("map").join("rivers.bmp"));
        self
    }

    fn set(&mut self, field: DefaultMapField, logical_path: PathBuf) {
        match field {
            DefaultMapField::Provinces => self.provinces_bmp = Some(logical_path),
            DefaultMapField::Definitions => self.definition_csv = Some(logical_path),
            DefaultMapField::Adjacencies => self.adjacencies_csv = Some(logical_path),
            DefaultMapField::Rivers => self.rivers_bmp = Some(logical_path),
            DefaultMapField::Heightmap => self.heightmap_bmp = Some(logical_path),
            DefaultMapField::Terrain => self.terrain_bmp = Some(logical_path),
            DefaultMapField::Tree => self.trees_bmp = Some(logical_path),
            DefaultMapField::Cities => self.cities_bmp = Some(logical_path),
            DefaultMapField::Continent => self.continent_txt = Some(logical_path),
            DefaultMapField::Positions => self.positions_txt = Some(logical_path),
            DefaultMapField::AmbientObject => self.ambient_object_txt = Some(logical_path),
            DefaultMapField::Seasons => self.seasons_txt = Some(logical_path),
            DefaultMapField::SupplyNodes => self.supply_nodes_txt = Some(logical_path),
            DefaultMapField::Railways => self.railways_txt = Some(logical_path),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplacePathSet {
    paths: BTreeSet<String>,
}

impl ReplacePathSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_descriptor_text(text: &str) -> Result<Self, SourceResolutionError> {
        let mut set = Self::new();
        set.extend_descriptor_text(text)?;
        Ok(set)
    }

    pub fn discover_current_project(root: impl AsRef<Path>) -> Result<Self, SourceResolutionError> {
        let root = root.as_ref();
        let descriptor = root.join("descriptor.mod");
        if !descriptor.is_file() {
            return Ok(Self::new());
        }
        let text = fs::read_to_string(&descriptor).map_err(|source| SourceResolutionError::Io {
            path: descriptor,
            source,
        })?;
        Self::from_descriptor_text(&text)
    }

    pub fn extend_descriptor_text(&mut self, text: &str) -> Result<(), SourceResolutionError> {
        for assignment in parse_assignments(text) {
            if !assignment.key.eq_ignore_ascii_case("replace_path") {
                continue;
            }
            self.insert(&assignment.value)
                .map_err(|source| SourceResolutionError::Parse {
                    line: assignment.line,
                    key: assignment.key.clone(),
                    value: assignment.value.clone(),
                    source,
                })?;
        }
        Ok(())
    }

    pub fn insert(&mut self, logical_path: impl AsRef<str>) -> Result<bool, SourcePathError> {
        let logical_path = normalize_logical_str(logical_path.as_ref())?;
        Ok(self.paths.insert(logical_key(&logical_path)))
    }

    pub fn blocks(&self, logical_path: impl AsRef<Path>) -> Result<bool, SourcePathError> {
        let logical_path = normalize_logical_path(logical_path)?;
        let key = logical_key(&logical_path);
        Ok(self
            .paths
            .iter()
            .any(|replace| key == *replace || key.starts_with(&format!("{replace}/"))))
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.paths.iter().map(String::as_str)
    }
}

#[derive(Debug)]
pub struct SourceResolver {
    current_project_root: Option<PathBuf>,
    current_project_map_dir: Option<PathBuf>,
    base_game_root: Option<PathBuf>,
    replace_paths: ReplacePathSet,
    project_generation: SourceGeneration,
    lower_source_cache: RwLock<LowerSourceCache>,
}

impl Clone for SourceResolver {
    fn clone(&self) -> Self {
        Self {
            current_project_root: self.current_project_root.clone(),
            current_project_map_dir: self.current_project_map_dir.clone(),
            base_game_root: self.base_game_root.clone(),
            replace_paths: self.replace_paths.clone(),
            project_generation: self.project_generation,
            // Container metadata is a safe optimization, never part of source
            // identity. Clones rediscover lazily under their own lock.
            lower_source_cache: RwLock::new(LowerSourceCache::default()),
        }
    }
}

impl PartialEq for SourceResolver {
    fn eq(&self, other: &Self) -> bool {
        self.current_project_root == other.current_project_root
            && self.current_project_map_dir == other.current_project_map_dir
            && self.base_game_root == other.base_game_root
            && self.replace_paths == other.replace_paths
            && self.project_generation == other.project_generation
    }
}

impl Eq for SourceResolver {}

impl SourceResolver {
    pub fn new(project_generation: SourceGeneration) -> Self {
        Self {
            current_project_root: None,
            current_project_map_dir: None,
            base_game_root: None,
            replace_paths: ReplacePathSet::new(),
            project_generation,
            lower_source_cache: RwLock::new(LowerSourceCache::default()),
        }
    }

    pub fn with_current_project_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.current_project_root = Some(root.into());
        self
    }

    pub fn with_current_project_map_dir(mut self, map_dir: impl Into<PathBuf>) -> Self {
        self.current_project_map_dir = Some(map_dir.into());
        self
    }

    pub fn with_base_game_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.base_game_root = Some(root.into());
        self
    }

    pub fn with_optional_base_game_root(mut self, root: Option<PathBuf>) -> Self {
        self.base_game_root = root;
        self.lower_source_cache = RwLock::new(LowerSourceCache::default());
        self
    }

    pub fn with_replace_paths(mut self, replace_paths: ReplacePathSet) -> Self {
        self.replace_paths = replace_paths;
        self
    }

    pub fn project_generation(&self) -> SourceGeneration {
        self.project_generation
    }

    pub fn replace_paths(&self) -> &ReplacePathSet {
        &self.replace_paths
    }

    pub fn base_game_root(&self) -> Option<&Path> {
        self.base_game_root.as_deref()
    }

    /// Returns an explicit resolution result so source diagnostics can
    /// distinguish a missing file from `replace_path` intentionally masking
    /// lower layers.
    pub fn resolve(
        &self,
        logical_path: impl AsRef<Path>,
    ) -> Result<SourceLookup, SourceResolutionError> {
        let logical_path = normalize_logical_path(logical_path)?;

        if let Some(physical_path) = self.current_project_physical_path(&logical_path)
            && physical_path.is_file()
        {
            return Ok(SourceLookup::Found(self.filesystem_source(
                logical_path,
                physical_path,
                SourceKind::CurrentProject,
            )));
        }

        if self.replace_paths.blocks(&logical_path)? {
            return Ok(SourceLookup::BlockedByReplacePath { logical_path });
        }

        if let Some(source) = self.resolve_lower(&logical_path)? {
            return Ok(SourceLookup::Found(source));
        }

        if let Some(base_root) = &self.base_game_root {
            let physical_path = base_root.join(&logical_path);
            if physical_path.is_file() {
                return Ok(SourceLookup::Found(self.filesystem_source(
                    logical_path,
                    physical_path,
                    SourceKind::BaseGame,
                )));
            }
        }

        Ok(SourceLookup::NotFound)
    }

    pub fn resolve_existing(
        &self,
        logical_path: impl AsRef<Path>,
    ) -> Result<Option<ResolvedSource>, SourceResolutionError> {
        match self.resolve(logical_path)? {
            SourceLookup::Found(source) => Ok(Some(source)),
            SourceLookup::NotFound | SourceLookup::BlockedByReplacePath { .. } => Ok(None),
        }
    }

    /// Lists files below one logical directory, applying source precedence at
    /// the file level. `shadowed` retains the discarded provenance for source
    /// debugging; regular callers consume `files` only.
    pub fn list_files(
        &self,
        logical_directory: impl AsRef<Path>,
    ) -> Result<SourceListing, SourceResolutionError> {
        let logical_directory = normalize_logical_path(logical_directory)?;
        let mut listing = SourceListing::default();
        let mut winners = BTreeMap::<String, ResolvedSource>::new();

        if let Some(current) = self.current_project_physical_path(&logical_directory) {
            self.add_directory_files(
                &mut winners,
                &mut listing.shadowed,
                &logical_directory,
                &current,
                SourceKind::CurrentProject,
            )?;
        }

        for source in self.list_lower(&logical_directory)? {
            self.add_source(&mut winners, &mut listing.shadowed, source);
        }

        if let Some(base_root) = &self.base_game_root {
            self.add_directory_files(
                &mut winners,
                &mut listing.shadowed,
                &logical_directory,
                &base_root.join(&logical_directory),
                SourceKind::BaseGame,
            )?;
        }

        listing.files = winners.into_values().collect();
        Ok(listing)
    }

    /// Reads a generation-bound source without exposing archive mechanics to
    /// callers. This operation is intentionally read-only for every source.
    pub fn read_resolved(&self, source: &ResolvedSource) -> Result<Vec<u8>, SourceResolutionError> {
        if source.project_generation != self.project_generation {
            return Err(SourceResolutionError::StaleGeneration {
                source_generation: source.project_generation,
                resolver_generation: self.project_generation,
            });
        }
        match &source.location {
            ResolvedLocation::Filesystem(path) => {
                fs::read(path).map_err(|source| SourceResolutionError::Io {
                    path: path.clone(),
                    source,
                })
            }
            ResolvedLocation::ArchiveEntry {
                archive_path,
                entry_path,
            } => {
                let file = fs::File::open(archive_path).map_err(|source| {
                    SourceResolutionError::ArchiveUnavailable {
                        archive_path: archive_path.clone(),
                        detail: source.to_string(),
                    }
                })?;
                let mut archive = ZipArchive::new(file).map_err(|source| {
                    SourceResolutionError::ArchiveUnavailable {
                        archive_path: archive_path.clone(),
                        detail: source.to_string(),
                    }
                })?;
                let entry_name = archive_entry_name(entry_path);
                let mut entry = archive.by_name(&entry_name).map_err(|_| {
                    SourceResolutionError::ArchiveEntryMissing {
                        archive_path: archive_path.clone(),
                        entry_path: entry_path.clone(),
                    }
                })?;
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                entry
                    .read_to_end(&mut bytes)
                    .map_err(|source| SourceResolutionError::Io {
                        path: archive_path.clone(),
                        source,
                    })?;
                Ok(bytes)
            }
        }
    }

    pub fn load_map_layout(
        &self,
        default_map: impl AsRef<Path>,
    ) -> Result<Option<(ResolvedSource, MapFileLayout)>, SourceResolutionError> {
        let Some(source) = self.resolve_existing(default_map)? else {
            return Ok(None);
        };
        let text = String::from_utf8(self.read_resolved(&source)?).map_err(|error| {
            SourceResolutionError::InvalidUtf8 {
                logical_path: source.logical_path.clone(),
                source: error,
            }
        })?;
        let layout = MapFileLayout::parse(&source.logical_path, &text)?;
        Ok(Some((source, layout)))
    }

    pub fn load_default_map_layout(
        &self,
    ) -> Result<Option<(ResolvedSource, MapFileLayout)>, SourceResolutionError> {
        self.load_map_layout(default_map_logical_path())
    }

    fn current_project_physical_path(&self, logical_path: &Path) -> Option<PathBuf> {
        if let Some(root) = &self.current_project_root {
            return Some(root.join(logical_path));
        }

        let map_dir = self.current_project_map_dir.as_ref()?;
        let mut components = logical_path.components();
        match components.next() {
            Some(Component::Normal(first)) if first.eq_ignore_ascii_case("map") => {
                let mut physical = map_dir.clone();
                for component in components {
                    physical.push(component.as_os_str());
                }
                Some(physical)
            }
            _ => None,
        }
    }

    fn filesystem_source(
        &self,
        logical_path: PathBuf,
        physical_path: PathBuf,
        source_kind: SourceKind,
    ) -> ResolvedSource {
        ResolvedSource {
            logical_path,
            location: ResolvedLocation::Filesystem(physical_path),
            source_kind,
            project_generation: self.project_generation,
        }
    }

    fn resolve_lower(
        &self,
        logical_path: &Path,
    ) -> Result<Option<ResolvedSource>, SourceResolutionError> {
        let Some(base_root) = self.base_game_root.as_deref() else {
            return Ok(None);
        };
        let mut cache = self
            .lower_source_cache
            .write()
            .expect("lower-source cache lock was poisoned");
        cache.ensure_discovered(base_root)?;
        for container in &mut cache.containers {
            if let Some(location) = container.resolve(logical_path)? {
                return Ok(Some(ResolvedSource {
                    logical_path: logical_path.to_owned(),
                    location,
                    source_kind: container.source_kind,
                    project_generation: self.project_generation,
                }));
            }
        }
        Ok(None)
    }

    fn list_lower(
        &self,
        logical_directory: &Path,
    ) -> Result<Vec<ResolvedSource>, SourceResolutionError> {
        let Some(base_root) = self.base_game_root.as_deref() else {
            return Ok(Vec::new());
        };
        let mut cache = self
            .lower_source_cache
            .write()
            .expect("lower-source cache lock was poisoned");
        cache.ensure_discovered(base_root)?;
        let mut sources = Vec::new();
        for container in &mut cache.containers {
            for (logical_path, location) in container.list(logical_directory)? {
                if !self.replace_paths.blocks(&logical_path)? {
                    sources.push(ResolvedSource {
                        logical_path,
                        location,
                        source_kind: container.source_kind,
                        project_generation: self.project_generation,
                    });
                }
            }
        }
        Ok(sources)
    }

    fn add_directory_files(
        &self,
        winners: &mut BTreeMap<String, ResolvedSource>,
        shadowed: &mut Vec<ResolvedSource>,
        logical_directory: &Path,
        physical_directory: &Path,
        source_kind: SourceKind,
    ) -> Result<(), SourceResolutionError> {
        for (logical_path, physical_path) in
            filesystem_files(logical_directory, physical_directory)?
        {
            if source_kind != SourceKind::CurrentProject
                && self.replace_paths.blocks(&logical_path)?
            {
                continue;
            }
            self.add_source(
                winners,
                shadowed,
                self.filesystem_source(logical_path, physical_path, source_kind),
            );
        }
        Ok(())
    }

    fn add_source(
        &self,
        winners: &mut BTreeMap<String, ResolvedSource>,
        shadowed: &mut Vec<ResolvedSource>,
        source: ResolvedSource,
    ) {
        let key = logical_key(&source.logical_path);
        match winners.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(source);
            }
            Entry::Occupied(_) => {
                shadowed.push(source);
            }
        }
    }
}

/// Editor policy: mirrors the reference's ZIP-before-folder lookup shape and
/// `dlc`-before-`integrated_dlc` grouping, but sorts container names for
/// cross-platform determinism. HOI4 package collision semantics remain
/// `NEEDS HOI4 ENGINE TEST`.
#[derive(Debug, Clone, Default)]
struct LowerSourceCache {
    base_game_root: Option<PathBuf>,
    containers: Vec<LowerSourceContainer>,
}

impl LowerSourceCache {
    fn ensure_discovered(&mut self, base_game_root: &Path) -> Result<(), SourceResolutionError> {
        if self.base_game_root.as_deref() == Some(base_game_root) {
            return Ok(());
        }
        self.base_game_root = Some(base_game_root.to_owned());
        self.containers = discover_lower_source_containers(base_game_root)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct LowerSourceContainer {
    source_kind: SourceKind,
    storage: LowerSourceStorage,
}

#[derive(Debug, Clone)]
enum LowerSourceStorage {
    Folder {
        root: PathBuf,
    },
    Archive {
        path: PathBuf,
        index: Option<ArchiveIndex>,
    },
}

#[derive(Debug, Clone)]
struct ArchiveIndex {
    identity: ArchiveIdentity,
    entries: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchiveIdentity {
    size: u64,
    modified: Option<std::time::SystemTime>,
}

impl LowerSourceContainer {
    fn resolve(
        &mut self,
        logical_path: &Path,
    ) -> Result<Option<ResolvedLocation>, SourceResolutionError> {
        match &mut self.storage {
            LowerSourceStorage::Folder { root } => {
                let physical_path = root.join(logical_path);
                Ok(physical_path
                    .is_file()
                    .then_some(ResolvedLocation::Filesystem(physical_path)))
            }
            LowerSourceStorage::Archive { path, index } => {
                let entries = archive_index(path, index)?;
                Ok(entries
                    .entries
                    .get(&logical_key(logical_path))
                    .map(|entry_path| ResolvedLocation::ArchiveEntry {
                        archive_path: path.clone(),
                        entry_path: entry_path.clone(),
                    }))
            }
        }
    }

    fn list(
        &mut self,
        logical_directory: &Path,
    ) -> Result<Vec<(PathBuf, ResolvedLocation)>, SourceResolutionError> {
        match &mut self.storage {
            LowerSourceStorage::Folder { root } => Ok(filesystem_files(
                logical_directory,
                &root.join(logical_directory),
            )?
            .into_iter()
            .map(|(logical_path, physical_path)| {
                (logical_path, ResolvedLocation::Filesystem(physical_path))
            })
            .collect()),
            LowerSourceStorage::Archive { path, index } => {
                let entries = archive_index(path, index)?;
                let prefix = format!("{}/", logical_key(logical_directory));
                Ok(entries
                    .entries
                    .iter()
                    .filter(|(key, _)| key.starts_with(&prefix))
                    .map(|(_, entry_path)| {
                        (
                            entry_path.clone(),
                            ResolvedLocation::ArchiveEntry {
                                archive_path: path.clone(),
                                entry_path: entry_path.clone(),
                            },
                        )
                    })
                    .collect())
            }
        }
    }
}

fn discover_lower_source_containers(
    base_game_root: &Path,
) -> Result<Vec<LowerSourceContainer>, SourceResolutionError> {
    let mut archives = Vec::new();
    let mut folders = Vec::new();
    for (source_kind, directory) in [
        (SourceKind::Dlc, "dlc"),
        (SourceKind::IntegratedDlc, "integrated_dlc"),
    ] {
        let source_root = base_game_root.join(directory);
        let Ok(entries) = fs::read_dir(&source_root) else {
            continue;
        };
        let mut package_roots = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|type_| type_.is_dir()))
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .starts_with("dlc")
            })
            .collect::<Vec<_>>();
        package_roots.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());
        for package_root in package_roots {
            let package_root = package_root.path();
            let mut zip_paths = fs::read_dir(&package_root)
                .map_err(|source| SourceResolutionError::Io {
                    path: package_root.clone(),
                    source,
                })?
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.file_type().is_ok_and(|type_| type_.is_file())
                        && entry
                            .path()
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
                })
                .map(|entry| entry.path())
                .collect::<Vec<_>>();
            zip_paths.sort_by_key(|path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase()
            });
            archives.extend(zip_paths.into_iter().map(|path| LowerSourceContainer {
                source_kind,
                storage: LowerSourceStorage::Archive { path, index: None },
            }));
            folders.push(LowerSourceContainer {
                source_kind,
                storage: LowerSourceStorage::Folder { root: package_root },
            });
        }
    }
    archives.extend(folders);
    Ok(archives)
}

fn archive_index<'a>(
    path: &Path,
    cached: &'a mut Option<ArchiveIndex>,
) -> Result<&'a ArchiveIndex, SourceResolutionError> {
    let identity = archive_identity(path)?;
    if cached
        .as_ref()
        .is_some_and(|index| index.identity == identity)
    {
        return Ok(cached.as_ref().expect("checked above"));
    }
    let file =
        fs::File::open(path).map_err(|source| SourceResolutionError::ArchiveUnavailable {
            archive_path: path.to_owned(),
            detail: source.to_string(),
        })?;
    let mut archive =
        ZipArchive::new(file).map_err(|source| SourceResolutionError::ArchiveUnavailable {
            archive_path: path.to_owned(),
            detail: source.to_string(),
        })?;
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|source| {
            SourceResolutionError::ArchiveUnavailable {
                archive_path: path.to_owned(),
                detail: source.to_string(),
            }
        })?;
        if entry.is_dir() {
            continue;
        }
        let Ok(logical_path) = normalize_logical_str(entry.name()) else {
            continue;
        };
        let key = logical_key(&logical_path);
        match entries.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(logical_path);
            }
            Entry::Occupied(mut entry)
                if archive_entry_name(&logical_path) < archive_entry_name(entry.get()) =>
            {
                entry.insert(logical_path);
            }
            Entry::Occupied(_) => {}
        }
    }
    *cached = Some(ArchiveIndex { identity, entries });
    Ok(cached.as_ref().expect("index was just assigned"))
}

fn archive_identity(path: &Path) -> Result<ArchiveIdentity, SourceResolutionError> {
    let metadata =
        fs::metadata(path).map_err(|source| SourceResolutionError::ArchiveUnavailable {
            archive_path: path.to_owned(),
            detail: source.to_string(),
        })?;
    Ok(ArchiveIdentity {
        size: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn archive_entry_name(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn filesystem_files(
    logical_directory: &Path,
    physical_directory: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, SourceResolutionError> {
    let Ok(entries) = fs::read_dir(physical_directory) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let file_type = entry
            .file_type()
            .map_err(|source| SourceResolutionError::Io {
                path: entry.path(),
                source,
            })?;
        let name = entry.file_name();
        let logical_path = logical_directory.join(&name);
        if file_type.is_dir() {
            files.extend(filesystem_files(&logical_path, &entry.path())?);
        } else if file_type.is_file() {
            files.push((logical_path, entry.path()));
        }
    }
    files.sort_by_key(|(logical_path, _)| logical_key(logical_path));
    Ok(files)
}

#[derive(Debug)]
pub enum SourceResolutionError {
    InvalidPath(SourcePathError),
    MissingCurrentProjectDirectory {
        logical_path: PathBuf,
        physical_path: PathBuf,
    },
    MissingCurrentProjectFile {
        logical_path: PathBuf,
        physical_path: PathBuf,
    },
    Parse {
        line: usize,
        key: String,
        value: String,
        source: SourcePathError,
    },
    Io {
        path: PathBuf,
        source: io::Error,
    },
    InvalidUtf8 {
        logical_path: PathBuf,
        source: std::string::FromUtf8Error,
    },
    ArchiveUnavailable {
        archive_path: PathBuf,
        detail: String,
    },
    ArchiveEntryMissing {
        archive_path: PathBuf,
        entry_path: PathBuf,
    },
    StaleGeneration {
        source_generation: SourceGeneration,
        resolver_generation: SourceGeneration,
    },
}

impl fmt::Display for SourceResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(source) => write!(f, "{source}"),
            Self::MissingCurrentProjectDirectory {
                logical_path,
                physical_path,
            } => write!(
                f,
                "current project directory {} is missing at {}",
                logical_path.display(),
                physical_path.display()
            ),
            Self::MissingCurrentProjectFile {
                logical_path,
                physical_path,
            } => write!(
                f,
                "current project file {} is missing at {}",
                logical_path.display(),
                physical_path.display()
            ),
            Self::Parse {
                line,
                key,
                value,
                source,
            } => write!(
                f,
                "invalid source path for {key} on line {line} ({value:?}): {source}"
            ),
            Self::Io { path, source } => {
                write!(f, "failed to read {}: {source}", path.display())
            }
            Self::InvalidUtf8 {
                logical_path,
                source,
            } => {
                write!(f, "{} is not valid UTF-8: {source}", logical_path.display())
            }
            Self::ArchiveUnavailable {
                archive_path,
                detail,
            } => write!(
                f,
                "archive {} is unavailable: {detail}",
                archive_path.display()
            ),
            Self::ArchiveEntryMissing {
                archive_path,
                entry_path,
            } => write!(
                f,
                "archive {} no longer contains {}",
                archive_path.display(),
                entry_path.display()
            ),
            Self::StaleGeneration {
                source_generation,
                resolver_generation,
            } => write!(
                f,
                "source generation {} cannot be read by generation {}",
                source_generation.value(),
                resolver_generation.value()
            ),
        }
    }
}

impl std::error::Error for SourceResolutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPath(source) => Some(source),
            Self::MissingCurrentProjectDirectory { .. } => None,
            Self::MissingCurrentProjectFile { .. } => None,
            Self::Parse { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::InvalidUtf8 { source, .. } => Some(source),
            Self::ArchiveUnavailable { .. }
            | Self::ArchiveEntryMissing { .. }
            | Self::StaleGeneration { .. } => None,
        }
    }
}

impl From<SourcePathError> for SourceResolutionError {
    fn from(value: SourcePathError) -> Self {
        Self::InvalidPath(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePathError {
    path: String,
    reason: SourcePathErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePathErrorKind {
    Empty,
    Absolute,
    ParentTraversal,
    WindowsPrefix,
    NulByte,
}

impl SourcePathError {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn reason(&self) -> SourcePathErrorKind {
        self.reason
    }

    fn new(path: impl Into<String>, reason: SourcePathErrorKind) -> Self {
        Self {
            path: path.into(),
            reason,
        }
    }
}

impl fmt::Display for SourcePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.reason {
            SourcePathErrorKind::Empty => "path is empty",
            SourcePathErrorKind::Absolute => "path is absolute",
            SourcePathErrorKind::ParentTraversal => "path contains parent traversal",
            SourcePathErrorKind::WindowsPrefix => "path contains a Windows path prefix",
            SourcePathErrorKind::NulByte => "path contains a NUL byte",
        };
        write!(f, "{} ({reason})", self.path)
    }
}

impl std::error::Error for SourcePathError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefaultMapField {
    Provinces,
    Definitions,
    Adjacencies,
    Rivers,
    Heightmap,
    Terrain,
    Tree,
    Cities,
    Continent,
    Positions,
    AmbientObject,
    Seasons,
    SupplyNodes,
    Railways,
}

impl DefaultMapField {
    fn from_key(key: &str) -> Option<Self> {
        match key.to_ascii_lowercase().as_str() {
            "provinces" => Some(Self::Provinces),
            "definitions" => Some(Self::Definitions),
            "adjacencies" => Some(Self::Adjacencies),
            "rivers" => Some(Self::Rivers),
            "heightmap" => Some(Self::Heightmap),
            "terrain" => Some(Self::Terrain),
            "tree" => Some(Self::Tree),
            "cities" => Some(Self::Cities),
            "continent" => Some(Self::Continent),
            "positions" => Some(Self::Positions),
            "ambient_object" => Some(Self::AmbientObject),
            "seasons" => Some(Self::Seasons),
            "supply_nodes" => Some(Self::SupplyNodes),
            "railways" => Some(Self::Railways),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Assignment {
    line: usize,
    key: String,
    value: String,
}

fn default_map_logical_path() -> PathBuf {
    PathBuf::from("map").join("default.map")
}

fn require_current_directory(
    logical_path: &Path,
    physical_path: &Path,
) -> Result<(), SourceResolutionError> {
    if physical_path.is_dir() {
        return Ok(());
    }
    Err(SourceResolutionError::MissingCurrentProjectDirectory {
        logical_path: normalize_logical_path(logical_path)?,
        physical_path: physical_path.to_owned(),
    })
}

fn required_current_project_file(
    root: &Path,
    logical_path: &Path,
    project_generation: SourceGeneration,
) -> Result<ResolvedSource, SourceResolutionError> {
    let logical_path = normalize_logical_path(logical_path)?;
    let physical_path = root.join(&logical_path);
    if physical_path.is_file() {
        return Ok(ResolvedSource {
            logical_path,
            location: ResolvedLocation::Filesystem(physical_path),
            source_kind: SourceKind::CurrentProject,
            project_generation,
        });
    }
    Err(SourceResolutionError::MissingCurrentProjectFile {
        logical_path,
        physical_path,
    })
}

fn current_project_file(
    root: &Path,
    logical_path: &Path,
    project_generation: SourceGeneration,
) -> Option<ResolvedSource> {
    let logical_path = normalize_logical_path(logical_path).ok()?;
    let physical_path = root.join(&logical_path);
    physical_path.is_file().then_some(ResolvedSource {
        logical_path,
        location: ResolvedLocation::Filesystem(physical_path),
        source_kind: SourceKind::CurrentProject,
        project_generation,
    })
}

fn optional_current_project_file(
    root: &Path,
    logical_path: Option<&Path>,
    project_generation: SourceGeneration,
) -> Option<ResolvedSource> {
    current_project_file(root, logical_path?, project_generation)
}

fn map_child_logical_path(value: &str) -> Result<PathBuf, SourcePathError> {
    let child = normalize_logical_str(value)?;
    Ok(PathBuf::from("map").join(child))
}

fn normalize_logical_path(path: impl AsRef<Path>) -> Result<PathBuf, SourcePathError> {
    let raw = path.as_ref().to_string_lossy();
    normalize_logical_str(&raw)
}

fn normalize_logical_str(path: &str) -> Result<PathBuf, SourcePathError> {
    let original = path.trim().trim_matches('"').to_owned();
    if original.is_empty() {
        return Err(SourcePathError::new(original, SourcePathErrorKind::Empty));
    }
    if original.contains('\0') {
        return Err(SourcePathError::new(original, SourcePathErrorKind::NulByte));
    }

    let normalized = original.replace('\\', "/");
    if normalized.starts_with('/') || normalized.starts_with("//") {
        return Err(SourcePathError::new(
            original,
            SourcePathErrorKind::Absolute,
        ));
    }
    if normalized.as_bytes().get(1).copied() == Some(b':') || normalized.contains(':') {
        return Err(SourcePathError::new(
            original,
            SourcePathErrorKind::WindowsPrefix,
        ));
    }

    let mut logical = PathBuf::new();
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                return Err(SourcePathError::new(
                    original,
                    SourcePathErrorKind::ParentTraversal,
                ));
            }
            part => logical.push(part),
        }
    }

    if logical.as_os_str().is_empty() {
        return Err(SourcePathError::new(original, SourcePathErrorKind::Empty));
    }

    Ok(logical)
}

fn logical_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn parse_assignments(text: &str) -> Vec<Assignment> {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .enumerate()
        .filter_map(|(line_index, line)| parse_assignment_line(line_index + 1, line))
        .collect()
}

fn parse_assignment_line(line: usize, raw_line: &str) -> Option<Assignment> {
    let line_without_comment = strip_comment(raw_line);
    let equals = find_unquoted_equals(&line_without_comment)?;
    let key = line_without_comment[..equals].trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }

    let raw_value = line_without_comment[equals + 1..].trim();
    let value = parse_scalar_value(raw_value)?;
    Some(Assignment {
        line,
        key: key.to_owned(),
        value,
    })
}

fn strip_comment(line: &str) -> String {
    let mut in_quote = false;
    let mut escaped = false;
    let mut output = String::new();
    for ch in line.chars() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => {
                output.push(ch);
                escaped = true;
            }
            '"' => {
                in_quote = !in_quote;
                output.push(ch);
            }
            '#' if !in_quote => break,
            _ => output.push(ch),
        }
    }
    output
}

fn find_unquoted_equals(line: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            '=' if !in_quote => return Some(index),
            _ => {}
        }
    }
    None
}

fn parse_scalar_value(raw_value: &str) -> Option<String> {
    if raw_value.is_empty() || raw_value.starts_with('{') {
        return None;
    }

    if let Some(rest) = raw_value.strip_prefix('"') {
        let mut escaped = false;
        let mut value = String::new();
        for ch in rest.chars() {
            if escaped {
                value.push(ch);
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => return Some(value),
                _ => value.push(ch),
            }
        }
        return Some(value);
    }

    raw_value
        .split_whitespace()
        .next()
        .map(|value| value.trim_matches('"').to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        MapFileLayout, ProjectSources, ReplacePathSet, ResolvedLocation, ResolvedSource,
        SourceGeneration, SourceKind, SourceLookup, SourcePathErrorKind, SourceResolutionError,
        SourceResolver,
    };
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::Instant;
    use zip::write::FileOptions;

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hoi4-source-resolution-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("map")).unwrap();
            Self(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn write_zip(&self, relative: &str, entries: &[(&str, &str)]) {
            let path = self.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let file = fs::File::create(path).unwrap();
            let mut archive = zip::ZipWriter::new(file);
            for (name, contents) in entries {
                archive.start_file(*name, FileOptions::default()).unwrap();
                archive.write_all(contents.as_bytes()).unwrap();
            }
            archive.finish().unwrap();
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_default_map_with_bom_comments_unknown_fields_and_last_duplicate_wins() {
        let layout = MapFileLayout::parse_default_map(
            "\u{feff}
            # ignored
            provinces = \"old.bmp\"
            unknown = \"ignored.txt\"
            provinces = \"custom/provinces-custom.bmp\" # last recognized value wins
            definitions = custom-definition.csv
            adjacencies = \"custom adjacency.csv\"
            sea_starts = { 1 2 3 }
            rivers = \"rivers-custom.bmp#literal\"
            ",
        )
        .unwrap();

        assert_eq!(layout.default_map, PathBuf::from("map/default.map"));
        assert_eq!(
            layout.provinces_bmp,
            Some(PathBuf::from("map/custom/provinces-custom.bmp"))
        );
        assert_eq!(
            layout.definition_csv,
            Some(PathBuf::from("map/custom-definition.csv"))
        );
        assert_eq!(
            layout.adjacencies_csv,
            Some(PathBuf::from("map/custom adjacency.csv"))
        );
        assert_eq!(
            layout.rivers_bmp,
            Some(PathBuf::from("map/rivers-custom.bmp#literal"))
        );
    }

    #[test]
    fn rejects_traversal_in_default_map_filenames() {
        let error = MapFileLayout::parse_default_map("definitions = \"../definition.csv\"")
            .expect_err("traversal must fail");
        assert!(matches!(
            error,
            SourceResolutionError::Parse {
                source,
                ..
            } if source.reason() == SourcePathErrorKind::ParentTraversal
        ));
    }

    #[test]
    fn parses_descriptor_replace_paths_and_matches_directory_boundaries() {
        let set = ReplacePathSet::from_descriptor_text(
            "
            name = \"Fixture\"
            replace_path = \"history/states/\"
            replace_path = \"map\"
            ",
        )
        .unwrap();

        assert_eq!(set.len(), 2);
        assert!(set.blocks("history/states/1-Test.txt").unwrap());
        assert!(set.blocks("history/states").unwrap());
        assert!(set.blocks("MAP/definition.csv").unwrap());
        assert!(!set.blocks("history/stateful.txt").unwrap());
    }

    #[test]
    fn rejects_traversal_in_descriptor_replace_paths() {
        let error = ReplacePathSet::from_descriptor_text("replace_path = \"history/../common\"")
            .expect_err("replace_path traversal must fail");
        assert!(matches!(
            error,
            SourceResolutionError::Parse {
                source,
                ..
            } if source.reason() == SourcePathErrorKind::ParentTraversal
        ));
    }

    #[test]
    fn replacement_blocks_base_fallback_when_current_project_file_is_absent() {
        let project = TempRoot::new("project-replace");
        let base = TempRoot::new("base-replace");
        base.write("history/states/1-Base.txt", "state={id=1}");
        base.write("history/stateful.txt", "not blocked");

        let replace_paths =
            ReplacePathSet::from_descriptor_text("replace_path = \"history/states\"").unwrap();
        let resolver = SourceResolver::new(SourceGeneration::new(7))
            .with_current_project_root(project.path())
            .with_base_game_root(base.path())
            .with_replace_paths(replace_paths);

        assert!(
            resolver
                .resolve_existing("history/states/1-Base.txt")
                .unwrap()
                .is_none()
        );
        let sibling = resolver
            .resolve_existing("history/stateful.txt")
            .unwrap()
            .expect("sibling path is outside replace_path boundary");
        assert_eq!(sibling.source_kind, SourceKind::BaseGame);
        assert_eq!(sibling.project_generation, SourceGeneration::new(7));
    }

    #[test]
    fn current_project_file_wins_before_replace_path_blocks_base() {
        let project = TempRoot::new("project-current-first");
        let base = TempRoot::new("base-current-first");
        project.write("history/states/1-Project.txt", "state={id=1}");
        base.write("history/states/1-Project.txt", "state={id=1}");

        let replace_paths =
            ReplacePathSet::from_descriptor_text("replace_path = \"history/states\"").unwrap();
        let resolver = SourceResolver::new(SourceGeneration::new(8))
            .with_current_project_root(project.path())
            .with_base_game_root(base.path())
            .with_replace_paths(replace_paths);

        let source = resolver
            .resolve_existing("history/states/1-Project.txt")
            .unwrap()
            .expect("current project file should resolve");
        assert_eq!(source.source_kind, SourceKind::CurrentProject);
        assert_eq!(resolver.read_resolved(&source).unwrap(), b"state={id=1}");
    }

    #[test]
    fn map_replace_path_blocks_base_map_descendants() {
        let project = TempRoot::new("project-map-replace");
        let base = TempRoot::new("base-map-replace");
        project.write("map/provinces.bmp", "project-map");
        base.write("map/provinces.bmp", "base-decoy");
        base.write("map/definition.csv", "base-only-decoy");

        let resolver = SourceResolver::new(SourceGeneration::new(10))
            .with_current_project_root(project.path())
            .with_base_game_root(base.path())
            .with_replace_paths(
                ReplacePathSet::from_descriptor_text("replace_path = \"map\"").unwrap(),
            );

        let provinces = resolver
            .resolve_existing("map/provinces.bmp")
            .unwrap()
            .expect("the project map file must win");
        assert_eq!(provinces.source_kind, SourceKind::CurrentProject);
        assert!(
            resolver
                .resolve_existing("map/definition.csv")
                .unwrap()
                .is_none(),
            "replace_path = map must block the base map decoy"
        );
    }

    #[test]
    fn resolver_does_not_fall_back_to_a_previous_project() {
        let first = TempRoot::new("first-project");
        let second = TempRoot::new("second-project");
        first.write("map/definition.csv", "first");

        let first_source = SourceResolver::new(SourceGeneration::new(1))
            .with_current_project_root(first.path())
            .resolve_existing("map/definition.csv")
            .unwrap()
            .expect("first project has the file");
        assert_eq!(first_source.source_kind, SourceKind::CurrentProject);
        assert_eq!(first_source.project_generation, SourceGeneration::new(1));

        let second_source = SourceResolver::new(SourceGeneration::new(2))
            .with_current_project_root(second.path())
            .resolve_existing("map/definition.csv")
            .unwrap();
        assert!(
            second_source.is_none(),
            "a resolver for a new project must not retain previous project paths"
        );
    }

    #[test]
    fn project_b_manifest_cannot_contain_project_a_source_or_replace_paths() {
        let first = TempRoot::new("manifest-first-project");
        let second = TempRoot::new("manifest-second-project");
        for project in [&first, &second] {
            project.write("history/states/1-Test.txt", "state={id=1}");
        }
        first.write("descriptor.mod", "replace_path = \"history/states\"");
        first.write(
            "map/default.map",
            "definitions = \"a-definition.csv\"\nprovinces = \"a-provinces.bmp\"",
        );
        first.write("map/a-definition.csv", "definitions");
        first.write("map/a-provinces.bmp", "provinces");
        second.write(
            "map/default.map",
            "definitions = \"b-definition.csv\"\nprovinces = \"b-provinces.bmp\"",
        );
        second.write("map/b-definition.csv", "definitions");
        second.write("map/b-provinces.bmp", "provinces");

        let first_sources =
            ProjectSources::discover(first.path(), None, SourceGeneration::new(11)).unwrap();
        let second_sources =
            ProjectSources::discover(second.path(), None, SourceGeneration::new(12)).unwrap();
        assert!(
            first_sources
                .manifest()
                .replace_paths
                .blocks("history/states/1-Test.txt")
                .unwrap()
        );
        assert!(second_sources.manifest().replace_paths.is_empty());
        assert_eq!(
            second_sources.manifest().project_generation,
            SourceGeneration::new(12)
        );
        assert_eq!(
            second_sources
                .manifest()
                .map_files
                .provinces_bmp
                .filesystem_path()
                .expect("core project map source is a filesystem path"),
            second.path().join("map/b-provinces.bmp")
        );
        assert!(
            !second_sources
                .manifest()
                .map_files
                .provinces_bmp
                .filesystem_path()
                .expect("core project map source is a filesystem path")
                .starts_with(first.path())
        );
    }

    #[test]
    fn map_dir_only_current_project_resolution_is_limited_to_map_logical_paths() {
        let project = TempRoot::new("map-dir-only");
        project.write("map/custom-definition.csv", "definition");

        let resolver = SourceResolver::new(SourceGeneration::new(3))
            .with_current_project_map_dir(project.path().join("map"));

        assert!(
            resolver
                .resolve_existing("history/states/x.txt")
                .unwrap()
                .is_none()
        );
        let source = resolver
            .resolve_existing("map/custom-definition.csv")
            .unwrap()
            .expect("map logical path should resolve through map_dir");
        assert_eq!(source.source_kind, SourceKind::CurrentProject);
        assert_eq!(resolver.read_resolved(&source).unwrap(), b"definition");
    }

    #[test]
    fn discovers_current_project_descriptor_only() {
        let project = TempRoot::new("descriptor");
        project.write("descriptor.mod", "replace_path = \"history/states\"");
        project.write("other.mod", "replace_path = \"common\"");

        let set = ReplacePathSet::discover_current_project(project.path()).unwrap();
        assert!(set.blocks("history/states/1.txt").unwrap());
        assert!(!set.blocks("common/countries/x.txt").unwrap());
    }

    #[test]
    fn loads_default_map_layout_from_resolved_source() {
        let project = TempRoot::new("load-layout");
        project.write(
            "map/default.map",
            "definitions = \"custom-definition.csv\"\nprovinces = \"custom-provinces.bmp\"",
        );

        let (source, layout) = SourceResolver::new(SourceGeneration::new(9))
            .with_current_project_root(project.path())
            .load_default_map_layout()
            .unwrap()
            .expect("default.map should resolve");

        assert_eq!(source.source_kind, SourceKind::CurrentProject);
        assert_eq!(source.logical_path, PathBuf::from("map/default.map"));
        assert_eq!(
            layout.definition_csv,
            Some(PathBuf::from("map/custom-definition.csv"))
        );
        assert_eq!(
            layout.provinces_bmp,
            Some(PathBuf::from("map/custom-provinces.bmp"))
        );
    }

    #[test]
    fn project_sources_uses_conventional_fallbacks_for_only_supported_map_files() {
        let project = TempRoot::new("project-sources-conventional");
        project.write("map/provinces.bmp", "bmp");
        project.write("map/definition.csv", "definition");
        project.write("map/adjacencies.csv", "adjacency");
        project.write("map/continent.txt", "continent");
        project.write("map/rivers.bmp", "rivers");
        project.write(
            "map/terrain.bmp",
            "terrain exists but has no conventional fallback here",
        );

        let sources =
            ProjectSources::discover(project.path(), None, SourceGeneration::new(11)).unwrap();
        let manifest = sources.manifest();

        assert!(manifest.default_map.is_none());
        assert!(manifest.descriptor.is_none());
        assert_eq!(
            manifest.map_layout.provinces_bmp,
            Some(PathBuf::from("map/provinces.bmp"))
        );
        assert_eq!(
            manifest.map_layout.definition_csv,
            Some(PathBuf::from("map/definition.csv"))
        );
        assert_eq!(
            manifest.map_layout.adjacencies_csv,
            Some(PathBuf::from("map/adjacencies.csv"))
        );
        assert_eq!(
            manifest.map_layout.continent_txt,
            Some(PathBuf::from("map/continent.txt"))
        );
        assert_eq!(
            manifest.map_layout.rivers_bmp,
            Some(PathBuf::from("map/rivers.bmp"))
        );
        assert_eq!(manifest.map_layout.terrain_bmp, None);
        assert!(manifest.map_files.adjacencies_csv.is_some());
        assert!(manifest.map_files.continent_txt.is_some());
        assert!(manifest.map_files.rivers_bmp.is_some());
    }

    #[test]
    fn project_sources_requires_core_map_files_from_current_project() {
        let project = TempRoot::new("project-sources-current-required");
        let base = TempRoot::new("project-sources-base-not-core");
        project.write(
            "map/default.map",
            "definitions = \"custom-definition.csv\"\nprovinces = \"custom-provinces.bmp\"",
        );
        base.write("map/custom-definition.csv", "definition");
        base.write("map/custom-provinces.bmp", "bmp");

        let error = ProjectSources::discover(
            project.path(),
            Some(base.path().to_owned()),
            SourceGeneration::new(12),
        )
        .expect_err("base files must not satisfy required current project map files");
        assert!(matches!(
            error,
            SourceResolutionError::MissingCurrentProjectFile {
                logical_path,
                ..
            } if logical_path == Path::new("map/custom-provinces.bmp")
        ));

        project.write("map/custom-provinces.bmp", "bmp");
        project.write("map/custom-definition.csv", "definition");
        let sources = ProjectSources::discover(
            project.path(),
            Some(base.path().to_owned()),
            SourceGeneration::new(12),
        )
        .unwrap();
        assert_eq!(
            sources.source_files().provinces_bmp.logical_path,
            PathBuf::from("map/custom-provinces.bmp")
        );
        assert_eq!(
            sources.source_files().definition_csv.logical_path,
            PathBuf::from("map/custom-definition.csv")
        );
    }

    #[test]
    fn project_sources_exposes_default_map_descriptor_and_resolver_snapshot() {
        let project = TempRoot::new("project-sources-snapshot");
        let base = TempRoot::new("project-sources-snapshot-base");
        project.write("descriptor.mod", "replace_path = \"history/states\"");
        project.write(
            "map/default.map",
            "definitions = \"definition.csv\"\nprovinces = \"provinces.bmp\"",
        );
        project.write("map/provinces.bmp", "bmp");
        project.write("map/definition.csv", "definition");
        base.write("history/states/1-Base.txt", "state={id=1}");

        let sources = ProjectSources::discover(
            project.path(),
            Some(base.path().to_owned()),
            SourceGeneration::new(13),
        )
        .unwrap();
        let manifest = sources.manifest();

        assert_eq!(manifest.project_generation, SourceGeneration::new(13));
        assert!(manifest.descriptor.as_ref().is_some_and(|source| {
            source.logical_path == Path::new("descriptor.mod")
                && source.source_kind == SourceKind::CurrentProject
        }));
        assert!(manifest.default_map.as_ref().is_some_and(|source| {
            source.logical_path == Path::new("map/default.map")
                && source.source_kind == SourceKind::CurrentProject
        }));
        assert!(
            manifest
                .replace_paths
                .blocks("history/states/1.txt")
                .unwrap()
        );
        assert!(
            sources
                .resolver()
                .resolve_existing("history/states/1-Base.txt")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn resolves_dlc_and_integrated_dlc_folder_sources_with_explicit_precedence() {
        let project = TempRoot::new("lower-current");
        let base = TempRoot::new("lower-base");
        base.write("common/test_domain/a.txt", "base");
        base.write("dlc/dlc010/common/test_domain/a.txt", "dlc");
        base.write(
            "integrated_dlc/dlc020/common/test_domain/a.txt",
            "integrated",
        );
        base.write(
            "integrated_dlc/dlc020/common/test_domain/b.txt",
            "integrated-only",
        );
        project.write("common/test_domain/a.txt", "current");

        let resolver = SourceResolver::new(SourceGeneration::new(21))
            .with_current_project_root(project.path())
            .with_base_game_root(base.path());
        let current = resolver
            .resolve_existing("common/test_domain/a.txt")
            .unwrap()
            .unwrap();
        assert_eq!(current.source_kind, SourceKind::CurrentProject);
        assert_eq!(resolver.read_resolved(&current).unwrap(), b"current");

        let integrated = resolver
            .resolve_existing("common/test_domain/b.txt")
            .unwrap()
            .unwrap();
        assert_eq!(integrated.source_kind, SourceKind::IntegratedDlc);
        assert_eq!(
            resolver.read_resolved(&integrated).unwrap(),
            b"integrated-only"
        );
    }

    #[test]
    fn resolves_zip_entries_reads_them_and_reindexes_when_archive_changes() {
        let project = TempRoot::new("zip-current");
        let base = TempRoot::new("zip-base");
        base.write_zip(
            "dlc/dlc010/content.zip",
            &[("common/test_domain/a.txt", "first")],
        );
        let resolver = SourceResolver::new(SourceGeneration::new(22))
            .with_current_project_root(project.path())
            .with_base_game_root(base.path());
        let first = resolver
            .resolve_existing("common/test_domain/a.txt")
            .unwrap()
            .unwrap();
        assert_eq!(first.source_kind, SourceKind::Dlc);
        assert!(matches!(
            first.location,
            ResolvedLocation::ArchiveEntry { .. }
        ));
        assert_eq!(resolver.read_resolved(&first).unwrap(), b"first");
        assert!(
            resolver
                .resolve_existing("common/test_domain/b.txt")
                .unwrap()
                .is_none()
        );

        base.write_zip(
            "dlc/dlc010/content.zip",
            &[
                ("common/test_domain/a.txt", "second-and-larger"),
                ("common/test_domain/b.txt", "new"),
            ],
        );
        let second = resolver
            .resolve_existing("common/test_domain/b.txt")
            .unwrap()
            .unwrap();
        assert_eq!(resolver.read_resolved(&second).unwrap(), b"new");
        assert_eq!(
            resolver.read_resolved(&first).unwrap(),
            b"second-and-larger"
        );
    }

    #[test]
    fn resolves_integrated_dlc_zip_entries() {
        let project = TempRoot::new("integrated-zip-current");
        let base = TempRoot::new("integrated-zip-base");
        base.write_zip(
            "integrated_dlc/dlc020/content.zip",
            &[("common/test_domain/integrated.txt", "integrated-zip")],
        );
        let resolver = SourceResolver::new(SourceGeneration::new(221))
            .with_current_project_root(project.path())
            .with_base_game_root(base.path());
        let source = resolver
            .resolve_existing("common/test_domain/integrated.txt")
            .unwrap()
            .unwrap();
        assert_eq!(source.source_kind, SourceKind::IntegratedDlc);
        assert!(matches!(
            source.location,
            ResolvedLocation::ArchiveEntry { .. }
        ));
        assert_eq!(resolver.read_resolved(&source).unwrap(), b"integrated-zip");
    }

    #[test]
    fn project_discovery_does_not_open_dlc_archives_before_a_lower_source_lookup() {
        let project = TempRoot::new("lazy-current");
        let base = TempRoot::new("lazy-base");
        project.write("map/provinces.bmp", "bmp");
        project.write("map/definition.csv", "definition");
        base.write("dlc/dlc010/content.zip", "not a zip archive");

        let sources = ProjectSources::discover(
            project.path(),
            Some(base.path().to_owned()),
            SourceGeneration::new(222),
        )
        .expect("core current-project discovery must not index DLC archives");
        assert!(matches!(
            sources.resolve("common/test_domain/a.txt"),
            Err(SourceResolutionError::ArchiveUnavailable { .. })
        ));
    }

    #[test]
    fn profiles_lazy_dlc_fixture_lookup() {
        let project = TempRoot::new("profile-current");
        let base = TempRoot::new("profile-base");
        project.write("map/provinces.bmp", "bmp");
        project.write("map/definition.csv", "definition");
        base.write_zip(
            "dlc/dlc010/content.zip",
            &[("common/test_domain/a.txt", "fixture")],
        );

        let opened = Instant::now();
        let sources = ProjectSources::discover(
            project.path(),
            Some(base.path().to_owned()),
            SourceGeneration::new(223),
        )
        .unwrap();
        let open_in = opened.elapsed();
        let cold = Instant::now();
        assert!(matches!(
            sources.resolve("common/test_domain/a.txt").unwrap(),
            SourceLookup::Found(ResolvedSource {
                source_kind: SourceKind::Dlc,
                ..
            })
        ));
        let cold_in = cold.elapsed();
        let warm = Instant::now();
        assert!(matches!(
            sources.resolve("common/test_domain/a.txt").unwrap(),
            SourceLookup::Found(ResolvedSource {
                source_kind: SourceKind::Dlc,
                ..
            })
        ));
        println!(
            "synthetic source timings: open={}us cold_dlc={}us warm_dlc={}us",
            open_in.as_micros(),
            cold_in.as_micros(),
            warm.elapsed().as_micros()
        );
    }

    #[test]
    fn listing_collapses_lower_duplicates_and_replace_path_blocks_every_lower_layer() {
        let project = TempRoot::new("list-current");
        let base = TempRoot::new("list-base");
        project.write("common/test_domain/current.txt", "current");
        base.write("dlc/dlc010/common/test_domain/a.txt", "dlc");
        base.write(
            "integrated_dlc/dlc020/common/test_domain/a.txt",
            "integrated",
        );
        base.write("common/test_domain/a.txt", "base");
        base.write("common/test_domain/base.txt", "base-only");
        let resolver = SourceResolver::new(SourceGeneration::new(23))
            .with_current_project_root(project.path())
            .with_base_game_root(base.path());
        let listing = resolver.list_files("common/test_domain").unwrap();
        assert_eq!(
            listing
                .files
                .iter()
                .map(|source| source.logical_path.clone())
                .collect::<Vec<_>>(),
            vec![
                PathBuf::from("common/test_domain/a.txt"),
                PathBuf::from("common/test_domain/base.txt"),
                PathBuf::from("common/test_domain/current.txt"),
            ]
        );
        assert_eq!(listing.files[0].source_kind, SourceKind::Dlc);
        assert_eq!(listing.shadowed.len(), 2);

        let blocked = SourceResolver::new(SourceGeneration::new(24))
            .with_current_project_root(project.path())
            .with_base_game_root(base.path())
            .with_replace_paths(
                ReplacePathSet::from_descriptor_text("replace_path = \"common/test_domain\"")
                    .unwrap(),
            );
        assert!(matches!(
            blocked.resolve("common/test_domain/a.txt").unwrap(),
            SourceLookup::BlockedByReplacePath { .. }
        ));
        let blocked_listing = blocked.list_files("common/test_domain").unwrap();
        assert_eq!(blocked_listing.files.len(), 1);
        assert_eq!(
            blocked_listing.files[0].source_kind,
            SourceKind::CurrentProject
        );
    }

    #[test]
    fn ignores_unsafe_zip_entry_paths_and_keeps_project_generations_isolated() {
        let project_a = TempRoot::new("generation-a");
        let project_b = TempRoot::new("generation-b");
        let base_a = TempRoot::new("generation-base-a");
        let base_b = TempRoot::new("generation-base-b");
        base_a.write_zip(
            "dlc/dlc010/content.zip",
            &[
                ("../common/test_domain/unsafe.txt", "unsafe"),
                ("/common/test_domain/absolute.txt", "unsafe"),
                ("common/test_domain/safe.txt", "a"),
            ],
        );
        base_b.write("dlc/dlc010/common/test_domain/safe.txt", "b");
        let resolver_a = SourceResolver::new(SourceGeneration::new(31))
            .with_current_project_root(project_a.path())
            .with_base_game_root(base_a.path());
        assert!(
            resolver_a
                .resolve_existing("common/test_domain/unsafe.txt")
                .unwrap()
                .is_none()
        );
        let source_a = resolver_a
            .resolve_existing("common/test_domain/safe.txt")
            .unwrap()
            .unwrap();
        let resolver_b = SourceResolver::new(SourceGeneration::new(32))
            .with_current_project_root(project_b.path())
            .with_base_game_root(base_b.path());
        let source_b = resolver_b
            .resolve_existing("common/test_domain/safe.txt")
            .unwrap()
            .unwrap();
        assert_eq!(source_a.project_generation, SourceGeneration::new(31));
        assert_eq!(source_b.project_generation, SourceGeneration::new(32));
        assert_eq!(resolver_b.read_resolved(&source_b).unwrap(), b"b");
        assert!(matches!(
            resolver_b.read_resolved(&source_a),
            Err(SourceResolutionError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn project_switch_with_same_base_rebinds_sources_and_applies_new_replace_paths() {
        let project_a = TempRoot::new("same-base-a");
        let project_b = TempRoot::new("same-base-b");
        let base = TempRoot::new("same-base");
        base.write("dlc/dlc010/common/test_domain/a.txt", "dlc");
        let resolver_a = SourceResolver::new(SourceGeneration::new(41))
            .with_current_project_root(project_a.path())
            .with_base_game_root(base.path());
        let source_a = resolver_a
            .resolve_existing("common/test_domain/a.txt")
            .unwrap()
            .unwrap();
        let resolver_b = SourceResolver::new(SourceGeneration::new(42))
            .with_current_project_root(project_b.path())
            .with_base_game_root(base.path())
            .with_replace_paths(
                ReplacePathSet::from_descriptor_text("replace_path = \"common/test_domain\"")
                    .unwrap(),
            );
        assert!(matches!(
            resolver_b.resolve("common/test_domain/a.txt").unwrap(),
            SourceLookup::BlockedByReplacePath { .. }
        ));
        assert!(matches!(
            resolver_b.read_resolved(&source_a),
            Err(SourceResolutionError::StaleGeneration { .. })
        ));
    }
}
