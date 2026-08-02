use ahash::AHashMap;
use fs_err as fs;
use itertools::Itertools;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use toml_edit::{value, DocumentMut, Item, Table};

use crate::app::map::{Color, ProvinceKind};
use crate::util::files::{atomic_move_new_file, atomic_replace_file, write_complete_file};

use std::env;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

pub const CONFIG_SCHEMA_VERSION: i64 = 1;
pub const GLOBAL_CONFIG_DIRECTORY: &str = "HOI4MapEditor";
pub const PROJECT_CONFIG_DIRECTORY: &str = ".hoi4-map-editor";
pub const PROJECT_CONFIG_FILE: &str = "project.toml";
static NEXT_STAGE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalConfig {
    pub language: String,
    pub open_last_project: bool,
    pub remember_workspace: bool,
    pub remember_map_views: bool,
    pub remember_overlays: bool,
    pub last_project: Option<PathBuf>,
    pub max_undo_states: usize,
    pub change_view_mode_on_undo: bool,
    pub tooltip_delay_ms: u32,
    pub ui_scale: String,
    pub window: WindowPreferences,
    pub workspace: WorkspacePreferences,
    pub overlays: OverlayPreferences,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            language: "en-US".to_owned(),
            open_last_project: true,
            remember_workspace: true,
            remember_map_views: true,
            remember_overlays: true,
            last_project: None,
            max_undo_states: 24,
            change_view_mode_on_undo: true,
            tooltip_delay_ms: 400,
            ui_scale: "system".to_owned(),
            window: WindowPreferences::default(),
            workspace: WorkspacePreferences::default(),
            overlays: OverlayPreferences::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPreferences {
    pub maximized: bool,
    pub width: u32,
    pub height: u32,
    pub x: Option<i32>,
    pub y: Option<i32>,
}

impl Default for WindowPreferences {
    fn default() -> Self {
        Self {
            maximized: false,
            width: 1280,
            height: 800,
            x: None,
            y: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePreferences {
    pub last_workspace: String,
    pub province_map_view: String,
    pub state_map_view: String,
    pub state_inspector_visible: bool,
}

impl Default for WorkspacePreferences {
    fn default() -> Self {
        Self {
            last_workspace: "provinces".to_owned(),
            province_map_view: "province-colors".to_owned(),
            state_map_view: "states".to_owned(),
            state_inspector_visible: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OverlayPreferences {
    pub rivers: bool,
    pub adjacencies: bool,
    pub province_ids: bool,
    pub province_boundaries: bool,
    pub state_boundaries: bool,
    pub image: bool,
}

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub preserve_ids: bool,
    pub generate_coastal_on_save: bool,
    pub terrains: AHashMap<String, Terrain>,
    pub extra_warnings: ExtraWarnings,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            preserve_ids: true,
            generate_coastal_on_save: true,
            terrains: default_terrains(),
            extra_warnings: ExtraWarnings {
                enabled: false,
                lone_pixels: false,
                few_shared_borders: false,
                few_shared_borders_threshold: 3,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub max_undo_states: usize,
    pub preserve_ids: bool,
    pub change_view_mode_on_undo: bool,
    pub generate_coastal_on_save: bool,
    pub terrains: AHashMap<String, Terrain>,
    pub extra_warnings: ExtraWarnings,
}

impl Config {
    pub fn load() -> Result<Self, LoadConfigError> {
        let global = GlobalConfig::load()?.value;
        Ok(Self::from_parts(&global, &ProjectConfig::default()))
    }

    pub fn load_for_project(root: &Path) -> Result<Self, LoadConfigError> {
        let global = GlobalConfig::load()?.value;
        let project = ProjectConfig::load(root)?.value;
        Ok(Self::from_parts(&global, &project))
    }

    pub fn from_parts(global: &GlobalConfig, project: &ProjectConfig) -> Self {
        Self {
            max_undo_states: global.max_undo_states,
            preserve_ids: project.preserve_ids,
            change_view_mode_on_undo: global.change_view_mode_on_undo,
            generate_coastal_on_save: project.generate_coastal_on_save,
            terrains: project.terrains.clone(),
            extra_warnings: project.extra_warnings,
        }
    }

    pub fn terrain_color(&self, terrain: &str) -> Option<Color> {
        self.terrains.get(terrain).map(|terrain| terrain.color)
    }

    pub fn terrain_kind(&self, terrain: &str) -> Option<ProvinceKind> {
        self.terrains.get(terrain).map(|terrain| terrain.kind)
    }

    pub fn cycle_terrains(&self, terrain: Option<&str>, backwards: bool) -> String {
        if let Some(target_terrain) = terrain {
            for tuple in self.terrains.keys().sorted().tuple_windows() {
                if backwards {
                    let (next_terrain, terrain) = tuple;
                    if terrain == target_terrain {
                        return next_terrain.clone();
                    }
                } else {
                    let (terrain, next_terrain) = tuple;
                    if terrain == target_terrain {
                        return next_terrain.clone();
                    }
                }
            }
        }
        self.terrains
            .keys()
            .filter(|name| name.as_str() != "unknown")
            .sorted()
            .next()
            .expect("the built-in terrain catalog is never empty")
            .clone()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_parts(&GlobalConfig::default(), &ProjectConfig::default())
    }
}

#[derive(Debug, Copy, Clone, Deserialize, PartialEq, Eq)]
pub struct Terrain {
    #[serde(alias = "colour")]
    pub color: Color,
    #[serde(rename = "type")]
    pub kind: ProvinceKind,
}

#[derive(Debug, Copy, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "kebab-case")]
pub struct ExtraWarnings {
    #[serde(skip_deserializing)]
    pub enabled: bool,
    pub lone_pixels: bool,
    pub few_shared_borders: bool,
    pub few_shared_borders_threshold: usize,
}

impl Default for ExtraWarnings {
    fn default() -> Self {
        ProjectConfig::default().extra_warnings
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFingerprint {
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct ConfigLoad<T> {
    pub value: T,
    pub path: PathBuf,
    pub fingerprint: Option<FileFingerprint>,
    pub issue: Option<ConfigIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigIssue {
    Invalid(String),
    FutureSchema(i64),
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::FutureSchema(version) => write!(
                formatter,
                "Configuration schema {version} is newer than supported schema {CONFIG_SCHEMA_VERSION}."
            ),
        }
    }
}

#[derive(Error, Debug)]
pub enum LoadConfigError {
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("configuration directory is unavailable")]
    DirectoryUnavailable,
}

#[derive(Error, Debug)]
pub enum SaveConfigError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("The configuration changed outside the editor. Reload it or confirm Save Anyway.")]
    ChangedExternally,
    #[error(
        "Configuration schema {0} is newer than supported schema {CONFIG_SCHEMA_VERSION}; explicit overwrite confirmation is required."
    )]
    FutureSchema(i64),
}

impl GlobalConfig {
    pub fn path() -> Result<PathBuf, LoadConfigError> {
        roaming_app_data()
            .map(|root| root.join(GLOBAL_CONFIG_DIRECTORY).join("config.toml"))
            .ok_or(LoadConfigError::DirectoryUnavailable)
    }

    pub fn load() -> Result<ConfigLoad<Self>, LoadConfigError> {
        let path = Self::path()?;
        load_document(&path, Self::default(), parse_global)
    }

    pub fn validate(&self) -> Result<(), SaveConfigError> {
        if !(1..=500).contains(&self.max_undo_states) {
            return Err(SaveConfigError::Invalid(
                "Maximum undo history must be between 1 and 500.".to_owned(),
            ));
        }
        if !matches!(self.tooltip_delay_ms, 0 | 400 | 800) {
            return Err(SaveConfigError::Invalid(
                "Tooltip delay must be Instant (0), Normal (400), or Slow (800).".to_owned(),
            ));
        }
        if self.ui_scale != "system" {
            return Err(SaveConfigError::Invalid(
                "This renderer currently supports only System UI scale.".to_owned(),
            ));
        }
        if self.window.width < 384 || self.window.height < 256 {
            return Err(SaveConfigError::Invalid(
                "Window size is below the safe minimum of 384x256.".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn save(
        &self,
        expected: Option<&FileFingerprint>,
        overwrite_external: bool,
        overwrite_future_schema: bool,
    ) -> Result<FileFingerprint, SaveConfigError> {
        self.validate()?;
        let path = Self::path().map_err(|error| {
            SaveConfigError::Invalid(format!("Cannot resolve global configuration path: {error}"))
        })?;
        save_document(
            &path,
            expected,
            overwrite_external,
            overwrite_future_schema,
            |document| update_global(document, self),
            parse_global,
        )
    }

    pub fn replace_invalid_file(&self) -> Result<FileFingerprint, SaveConfigError> {
        self.validate()?;
        let path = Self::path().map_err(|error| {
            SaveConfigError::Invalid(format!("Cannot resolve global configuration path: {error}"))
        })?;
        replace_invalid_document(
            &path,
            |document| update_global(document, self),
            parse_global,
        )
    }
}

impl ProjectConfig {
    pub fn path(root: &Path) -> PathBuf {
        root.join(PROJECT_CONFIG_DIRECTORY).join(PROJECT_CONFIG_FILE)
    }

    pub fn load(root: &Path) -> Result<ConfigLoad<Self>, LoadConfigError> {
        let path = Self::path(root);
        load_document(&path, Self::default(), parse_project)
    }

    pub fn validate(&self) -> Result<(), SaveConfigError> {
        if self.extra_warnings.few_shared_borders_threshold == 0 {
            return Err(SaveConfigError::Invalid(
                "Few shared borders threshold must be a positive integer.".to_owned(),
            ));
        }
        for (name, terrain) in &self.terrains {
            validate_terrain_name(name)?;
            if terrain.kind == ProvinceKind::Unknown && name != "unknown" {
                return Err(SaveConfigError::Invalid(format!(
                    "Terrain '{name}': type must be land, sea, or lake."
                )));
            }
        }
        Ok(())
    }

    pub fn save(
        &self,
        root: &Path,
        expected: Option<&FileFingerprint>,
        overwrite_external: bool,
        overwrite_future_schema: bool,
    ) -> Result<FileFingerprint, SaveConfigError> {
        self.validate()?;
        let path = Self::path(root);
        save_document(
            &path,
            expected,
            overwrite_external,
            overwrite_future_schema,
            |document| update_project(document, self),
            parse_project,
        )
    }

    pub fn replace_invalid_file(
        &self,
        root: &Path,
    ) -> Result<FileFingerprint, SaveConfigError> {
        self.validate()?;
        replace_invalid_document(
            &Self::path(root),
            |document| update_project(document, self),
            parse_project,
        )
    }
}

fn load_document<T: Clone>(
    path: &Path,
    defaults: T,
    parser: impl Fn(&DocumentMut) -> Result<T, String>,
) -> Result<ConfigLoad<T>, LoadConfigError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(ConfigLoad {
                value: defaults,
                path: path.to_owned(),
                fingerprint: None,
                issue: None,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let fingerprint = Some(fingerprint_bytes(&bytes));
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) => {
            return Ok(ConfigLoad {
                value: defaults,
                path: path.to_owned(),
                fingerprint,
                issue: Some(ConfigIssue::Invalid(format!(
                    "{} is not valid UTF-8: {error}",
                    path.display()
                ))),
            });
        }
    };
    let document = match text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(error) => {
            return Ok(ConfigLoad {
                value: defaults,
                path: path.to_owned(),
                fingerprint,
                issue: Some(ConfigIssue::Invalid(format!(
                    "{} could not be parsed at {}: {error}",
                    path.display(),
                    error.span()
                        .map(|span| format!("byte {}", span.start))
                        .unwrap_or_else(|| "an unknown position".to_owned())
                ))),
            });
        }
    };
    if let Some(schema) = document
        .get("schema-version")
        .and_then(Item::as_integer)
        .filter(|schema| *schema > CONFIG_SCHEMA_VERSION)
    {
        return Ok(ConfigLoad {
            value: defaults,
            path: path.to_owned(),
            fingerprint,
            issue: Some(ConfigIssue::FutureSchema(schema)),
        });
    }
    match parser(&document) {
        Ok(value) => Ok(ConfigLoad {
            value,
            path: path.to_owned(),
            fingerprint,
            issue: None,
        }),
        Err(error) => Ok(ConfigLoad {
            value: defaults,
            path: path.to_owned(),
            fingerprint,
            issue: Some(ConfigIssue::Invalid(error)),
        }),
    }
}

fn parse_global(document: &DocumentMut) -> Result<GlobalConfig, String> {
    let mut config = GlobalConfig::default();
    let general = table(document, "general")?;
    config.language = string_or(general, "language", &config.language)?;
    config.open_last_project = bool_or(general, "open-last-project", config.open_last_project)?;
    config.remember_workspace =
        bool_or(general, "remember-workspace", config.remember_workspace)?;
    config.remember_map_views =
        bool_or(general, "remember-map-views", config.remember_map_views)?;
    config.remember_overlays = bool_or(general, "remember-overlays", config.remember_overlays)?;
    config.last_project = optional_string(general, "last-project")?.map(PathBuf::from);

    let editing = table(document, "editing")?;
    config.max_undo_states = usize_or(editing, "max-undo-states", config.max_undo_states)?;
    config.change_view_mode_on_undo = bool_or(
        editing,
        "change-view-mode-on-undo",
        config.change_view_mode_on_undo,
    )?;

    let interface = table(document, "interface")?;
    config.tooltip_delay_ms =
        u32_or(interface, "tooltip-delay-ms", config.tooltip_delay_ms)?;
    config.ui_scale = string_or(interface, "ui-scale", &config.ui_scale)?;

    let window = table(document, "window")?;
    config.window.maximized = bool_or(window, "maximized", config.window.maximized)?;
    config.window.width = u32_or(window, "width", config.window.width)?;
    config.window.height = u32_or(window, "height", config.window.height)?;
    config.window.x = optional_i32(window, "x")?;
    config.window.y = optional_i32(window, "y")?;

    let workspace = table(document, "workspace")?;
    config.workspace.last_workspace = string_or(
        workspace,
        "last-workspace",
        &config.workspace.last_workspace,
    )?;
    config.workspace.province_map_view = string_or(
        workspace,
        "province-map-view",
        &config.workspace.province_map_view,
    )?;
    config.workspace.state_map_view = string_or(
        workspace,
        "state-map-view",
        &config.workspace.state_map_view,
    )?;
    config.workspace.state_inspector_visible = bool_or(
        workspace,
        "state-inspector-visible",
        config.workspace.state_inspector_visible,
    )?;

    let overlays = table(document, "overlays")?;
    config.overlays.rivers = bool_or(overlays, "rivers", config.overlays.rivers)?;
    config.overlays.adjacencies =
        bool_or(overlays, "adjacencies", config.overlays.adjacencies)?;
    config.overlays.province_ids =
        bool_or(overlays, "province-ids", config.overlays.province_ids)?;
    config.overlays.province_boundaries = bool_or(
        overlays,
        "province-boundaries",
        config.overlays.province_boundaries,
    )?;
    config.overlays.state_boundaries = bool_or(
        overlays,
        "state-boundaries",
        config.overlays.state_boundaries,
    )?;
    config.overlays.image = bool_or(overlays, "image", config.overlays.image)?;
    config
        .validate()
        .map_err(|error| format!("Invalid global configuration: {error}"))?;
    Ok(config)
}

fn parse_project(document: &DocumentMut) -> Result<ProjectConfig, String> {
    let mut config = ProjectConfig::default();
    let province = table(document, "province-map")?;
    config.preserve_ids = bool_or(province, "preserve-ids", config.preserve_ids)?;
    config.generate_coastal_on_save = bool_or(
        province,
        "generate-coastal-on-save",
        config.generate_coastal_on_save,
    )?;

    let warnings = table(document, "extra-warnings")?;
    config.extra_warnings.lone_pixels =
        bool_or(warnings, "lone-pixels", config.extra_warnings.lone_pixels)?;
    config.extra_warnings.few_shared_borders = bool_or(
        warnings,
        "few-shared-borders",
        config.extra_warnings.few_shared_borders,
    )?;
    config.extra_warnings.few_shared_borders_threshold = usize_or(
        warnings,
        "few-shared-borders-threshold",
        config.extra_warnings.few_shared_borders_threshold,
    )?;
    config.extra_warnings.enabled =
        config.extra_warnings.lone_pixels || config.extra_warnings.few_shared_borders;

    if let Some(terrains) = document.get("terrain") {
        let terrains = terrains
            .as_table()
            .ok_or_else(|| "Field 'terrain' must be a TOML table.".to_owned())?;
        for (name, item) in terrains {
            validate_terrain_name(name).map_err(|error| error.to_string())?;
            let terrain = item
                .as_table()
                .ok_or_else(|| format!("Terrain '{name}' must be a TOML table."))?;
            let color = parse_color(name, terrain)?;
            let kind_name = required_string(terrain, "type", &format!("Terrain '{name}'"))?;
            let kind = ProvinceKind::from_str(&kind_name).map_err(|_| {
                format!("Terrain '{name}': type must be land, sea, or lake.")
            })?;
            if !matches!(kind, ProvinceKind::Land | ProvinceKind::Sea | ProvinceKind::Lake) {
                return Err(format!(
                    "Terrain '{name}': type must be land, sea, or lake."
                ));
            }
            config
                .terrains
                .insert(name.to_owned(), Terrain { color, kind });
        }
    }
    config.terrains.remove("unknown");
    config.terrains.insert(
        "unknown".to_owned(),
        Terrain {
            color: [0, 0, 0],
            kind: ProvinceKind::Unknown,
        },
    );
    config
        .validate()
        .map_err(|error| format!("Invalid project configuration: {error}"))?;
    Ok(config)
}

fn parse_color(name: &str, terrain: &Table) -> Result<Color, String> {
    let array = terrain
        .get("color")
        .and_then(Item::as_array)
        .ok_or_else(|| format!("Terrain '{name}': color must contain exactly 3 integers."))?;
    if array.len() != 3 {
        return Err(format!(
            "Terrain '{name}': color must contain exactly 3 integers."
        ));
    }
    let mut color = [0; 3];
    for (index, component) in array.iter().enumerate() {
        let component = component.as_integer().ok_or_else(|| {
            format!("Terrain '{name}': color component {} must be an integer.", index + 1)
        })?;
        if !(0..=255).contains(&component) {
            return Err(format!(
                "Terrain '{name}': color component {component} must be between 0 and 255."
            ));
        }
        color[index] = component as u8;
    }
    Ok(color)
}

fn validate_terrain_name(name: &str) -> Result<(), SaveConfigError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(SaveConfigError::Invalid(format!(
            "Terrain '{name}': name must use letters, numbers, '_' or '-'."
        )));
    }
    Ok(())
}

fn save_document<T>(
    path: &Path,
    expected: Option<&FileFingerprint>,
    overwrite_external: bool,
    overwrite_future_schema: bool,
    update: impl FnOnce(&mut DocumentMut),
    validate: impl Fn(&DocumentMut) -> Result<T, String>,
) -> Result<FileFingerprint, SaveConfigError> {
    let current = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let current_fingerprint = current.as_deref().map(fingerprint_bytes);
    if expected != current_fingerprint.as_ref() && !overwrite_external {
        return Err(SaveConfigError::ChangedExternally);
    }
    let mut document = match current.as_deref() {
        Some(bytes) => {
            let text = std::str::from_utf8(bytes).map_err(|error| {
                SaveConfigError::Invalid(format!(
                    "{} is not valid UTF-8 and was not overwritten: {error}",
                    path.display()
                ))
            })?;
            text.parse::<DocumentMut>().map_err(|error| {
                SaveConfigError::Invalid(format!(
                    "{} contains invalid TOML and was not overwritten: {error}",
                    path.display()
                ))
            })?
        }
        None => DocumentMut::new(),
    };
    if let Some(schema) = document
        .get("schema-version")
        .and_then(Item::as_integer)
        .filter(|schema| *schema > CONFIG_SCHEMA_VERSION)
        && !overwrite_future_schema
    {
        return Err(SaveConfigError::FutureSchema(schema));
    }
    document["schema-version"] = value(CONFIG_SCHEMA_VERSION);
    update(&mut document);
    validate(&document).map_err(SaveConfigError::Invalid)?;
    let bytes = document.to_string().into_bytes();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(current) = current.as_deref() {
        let backup = backup_path(path);
        write_complete_file(&backup, current)?;
        if fs::read(&backup)? != current {
            return Err(SaveConfigError::Invalid(format!(
                "Backup verification failed for {}.",
                backup.display()
            )));
        }
    }
    atomic_write(path, &bytes)?;
    let reloaded = fs::read(path)?;
    let reloaded_text = std::str::from_utf8(&reloaded)
        .map_err(|error| SaveConfigError::Invalid(format!("Saved configuration is not UTF-8: {error}")))?;
    let reloaded_document = reloaded_text
        .parse::<DocumentMut>()
        .map_err(|error| SaveConfigError::Invalid(format!("Saved configuration is invalid: {error}")))?;
    validate(&reloaded_document).map_err(SaveConfigError::Invalid)?;
    Ok(fingerprint_bytes(&reloaded))
}

fn replace_invalid_document<T>(
    path: &Path,
    update: impl FnOnce(&mut DocumentMut),
    validate: impl Fn(&DocumentMut) -> Result<T, String>,
) -> Result<FileFingerprint, SaveConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Ok(current) = fs::read(path) {
        let backup = backup_path(path);
        write_complete_file(&backup, &current)?;
        if fs::read(&backup)? != current {
            return Err(SaveConfigError::Invalid(format!(
                "Backup verification failed for {}.",
                backup.display()
            )));
        }
    }
    let mut document = DocumentMut::new();
    document["schema-version"] = value(CONFIG_SCHEMA_VERSION);
    update(&mut document);
    validate(&document).map_err(SaveConfigError::Invalid)?;
    atomic_write(path, document.to_string().as_bytes())?;
    let bytes = fs::read(path)?;
    Ok(fingerprint_bytes(&bytes))
}

fn update_global(document: &mut DocumentMut, config: &GlobalConfig) {
    set_string(document, "general", "language", &config.language);
    set_bool(
        document,
        "general",
        "open-last-project",
        config.open_last_project,
    );
    set_bool(
        document,
        "general",
        "remember-workspace",
        config.remember_workspace,
    );
    set_bool(
        document,
        "general",
        "remember-map-views",
        config.remember_map_views,
    );
    set_bool(
        document,
        "general",
        "remember-overlays",
        config.remember_overlays,
    );
    if let Some(path) = &config.last_project {
        set_string(
            document,
            "general",
            "last-project",
            &path.to_string_lossy(),
        );
    } else if let Some(table) = document["general"].as_table_mut() {
        table.remove("last-project");
    }
    set_integer(
        document,
        "editing",
        "max-undo-states",
        config.max_undo_states as i64,
    );
    set_bool(
        document,
        "editing",
        "change-view-mode-on-undo",
        config.change_view_mode_on_undo,
    );
    set_integer(
        document,
        "interface",
        "tooltip-delay-ms",
        config.tooltip_delay_ms as i64,
    );
    set_string(document, "interface", "ui-scale", &config.ui_scale);
    set_bool(document, "window", "maximized", config.window.maximized);
    set_integer(document, "window", "width", config.window.width as i64);
    set_integer(document, "window", "height", config.window.height as i64);
    set_optional_integer(document, "window", "x", config.window.x.map(i64::from));
    set_optional_integer(document, "window", "y", config.window.y.map(i64::from));
    set_string(
        document,
        "workspace",
        "last-workspace",
        &config.workspace.last_workspace,
    );
    set_string(
        document,
        "workspace",
        "province-map-view",
        &config.workspace.province_map_view,
    );
    set_string(
        document,
        "workspace",
        "state-map-view",
        &config.workspace.state_map_view,
    );
    set_bool(
        document,
        "workspace",
        "state-inspector-visible",
        config.workspace.state_inspector_visible,
    );
    set_bool(document, "overlays", "rivers", config.overlays.rivers);
    set_bool(
        document,
        "overlays",
        "adjacencies",
        config.overlays.adjacencies,
    );
    set_bool(
        document,
        "overlays",
        "province-ids",
        config.overlays.province_ids,
    );
    set_bool(
        document,
        "overlays",
        "province-boundaries",
        config.overlays.province_boundaries,
    );
    set_bool(
        document,
        "overlays",
        "state-boundaries",
        config.overlays.state_boundaries,
    );
    set_bool(document, "overlays", "image", config.overlays.image);
}

fn update_project(document: &mut DocumentMut, config: &ProjectConfig) {
    set_bool(
        document,
        "province-map",
        "preserve-ids",
        config.preserve_ids,
    );
    set_bool(
        document,
        "province-map",
        "generate-coastal-on-save",
        config.generate_coastal_on_save,
    );
    set_bool(
        document,
        "extra-warnings",
        "lone-pixels",
        config.extra_warnings.lone_pixels,
    );
    set_bool(
        document,
        "extra-warnings",
        "few-shared-borders",
        config.extra_warnings.few_shared_borders,
    );
    set_integer(
        document,
        "extra-warnings",
        "few-shared-borders-threshold",
        config.extra_warnings.few_shared_borders_threshold as i64,
    );
}

fn table<'a>(document: &'a DocumentMut, name: &str) -> Result<Option<&'a Table>, String> {
    document
        .get(name)
        .map(|item| {
            item.as_table()
                .ok_or_else(|| format!("Field '{name}' must be a TOML table."))
        })
        .transpose()
}

fn bool_or(table: Option<&Table>, key: &str, default: bool) -> Result<bool, String> {
    match table.and_then(|table| table.get(key)) {
        Some(item) => item
            .as_bool()
            .ok_or_else(|| format!("Field '{key}' must be true or false.")),
        None => Ok(default),
    }
}

fn usize_or(table: Option<&Table>, key: &str, default: usize) -> Result<usize, String> {
    match table.and_then(|table| table.get(key)) {
        Some(item) => item
            .as_integer()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| format!("Field '{key}' must be a non-negative integer.")),
        None => Ok(default),
    }
}

fn u32_or(table: Option<&Table>, key: &str, default: u32) -> Result<u32, String> {
    match table.and_then(|table| table.get(key)) {
        Some(item) => item
            .as_integer()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| format!("Field '{key}' must be a non-negative integer.")),
        None => Ok(default),
    }
}

fn string_or(table: Option<&Table>, key: &str, default: &str) -> Result<String, String> {
    optional_string(table, key).map(|value| value.unwrap_or_else(|| default.to_owned()))
}

fn optional_string(table: Option<&Table>, key: &str) -> Result<Option<String>, String> {
    match table.and_then(|table| table.get(key)) {
        Some(item) => item
            .as_str()
            .map(|value| Some(value.to_owned()))
            .ok_or_else(|| format!("Field '{key}' must be a string.")),
        None => Ok(None),
    }
}

fn optional_i32(table: Option<&Table>, key: &str) -> Result<Option<i32>, String> {
    match table.and_then(|table| table.get(key)) {
        Some(item) => item
            .as_integer()
            .and_then(|value| i32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| format!("Field '{key}' must be a signed 32-bit integer.")),
        None => Ok(None),
    }
}

fn required_string(table: &Table, key: &str, context: &str) -> Result<String, String> {
    table
        .get(key)
        .and_then(Item::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{context}: field '{key}' must be a string."))
}

fn ensure_table<'a>(document: &'a mut DocumentMut, name: &str) -> &'a mut Table {
    if !document.contains_key(name) || !document[name].is_table() {
        document[name] = Item::Table(Table::new());
    }
    document[name]
        .as_table_mut()
        .expect("table was created immediately above")
}

fn set_bool(document: &mut DocumentMut, table: &str, key: &str, setting: bool) {
    ensure_table(document, table)[key] = value(setting);
}

fn set_integer(document: &mut DocumentMut, table: &str, key: &str, setting: i64) {
    ensure_table(document, table)[key] = value(setting);
}

fn set_string(document: &mut DocumentMut, table: &str, key: &str, setting: &str) {
    ensure_table(document, table)[key] = value(setting);
}

fn set_optional_integer(
    document: &mut DocumentMut,
    table: &str,
    key: &str,
    setting: Option<i64>,
) {
    if let Some(setting) = setting {
        set_integer(document, table, key, setting);
    } else if let Some(table) = document[table].as_table_mut() {
        table.remove(key);
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .unwrap_or_default()
        .to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

fn fingerprint_bytes(bytes: &[u8]) -> FileFingerprint {
    let sha256: [u8; 32] = Sha256::digest(bytes).into();
    FileFingerprint {
        size: bytes.len() as u64,
        sha256,
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let stage_id = NEXT_STAGE_ID.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{}.hoi4me-stage-{}-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        stage_id,
    ));
    write_complete_file(&temporary, bytes)?;
    let staged = fs::read(&temporary)?;
    if staged != bytes {
        let _ = fs::remove_file(&temporary);
        return Err(std::io::Error::other(
            "staged configuration did not match the serialized bytes",
        ));
    }
    let result = if path.exists() {
        atomic_replace_file(&temporary, path, None)
    } else {
        atomic_move_new_file(&temporary, path)
    };
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn roaming_app_data() -> Option<PathBuf> {
    env::var_os("APPDATA").map(PathBuf::from)
}

fn default_terrains() -> AHashMap<String, Terrain> {
    DEFAULT_TERRAINS
        .iter()
        .map(|&(color, name, kind)| (name.to_owned(), Terrain { color, kind }))
        .collect()
}

const DEFAULT_TERRAINS: &[(Color, &str, ProvinceKind)] = &[
    ([0, 0, 0], "unknown", ProvinceKind::Unknown),
    ([255, 129, 66], "plains", ProvinceKind::Land),
    ([255, 63, 0], "desert", ProvinceKind::Land),
    ([89, 199, 85], "forest", ProvinceKind::Land),
    ([248, 255, 153], "hills", ProvinceKind::Land),
    ([127, 191, 0], "jungle", ProvinceKind::Land),
    ([76, 96, 35], "marsh", ProvinceKind::Land),
    ([124, 135, 125], "mountain", ProvinceKind::Land),
    ([0, 255, 255], "lakes", ProvinceKind::Lake),
    ([0, 0, 255], "ocean", ProvinceKind::Sea),
    ([155, 0, 255], "urban", ProvinceKind::Land),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn root(name: &str) -> PathBuf {
        let root = env::temp_dir().join(format!(
            "hoi4-map-editor-config-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn project_defaults_match_safe_editor_behavior() {
        let config = ProjectConfig::default();
        assert!(config.preserve_ids);
        assert!(config.generate_coastal_on_save);
        assert_eq!(config.terrains["plains"].color, [255, 129, 66]);
        assert_eq!(config.terrains["ocean"].kind, ProvinceKind::Sea);
        assert_eq!(config.extra_warnings.few_shared_borders_threshold, 3);
        assert!(!config.extra_warnings.enabled);
    }

    #[test]
    fn missing_project_config_uses_defaults_without_creating_a_file() {
        let root = root("missing");
        let loaded = ProjectConfig::load(&root).unwrap();
        assert!(loaded.issue.is_none());
        assert!(loaded.fingerprint.is_none());
        assert!(!loaded.path.exists());
    }

    #[test]
    fn project_config_merges_override_and_custom_terrain() {
        let root = root("terrain");
        let path = ProjectConfig::path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"schema-version = 1
[terrain.plains]
color = [1, 2, 3]
type = "land"
[terrain.volcanic]
color = [55, 44, 33]
type = "land"
"#,
        )
        .unwrap();
        let loaded = ProjectConfig::load(&root).unwrap();
        assert!(loaded.issue.is_none(), "{:?}", loaded.issue);
        assert_eq!(loaded.value.terrains["plains"].color, [1, 2, 3]);
        assert_eq!(loaded.value.terrains["volcanic"].color, [55, 44, 33]);
        assert!(loaded.value.terrains.contains_key("forest"));
    }

    #[test]
    fn invalid_terrain_reports_exact_component_and_keeps_original() {
        let root = root("invalid-terrain");
        let path = ProjectConfig::path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = br#"[terrain.volcanic]
color = [1, 301, 3]
type = "land"
"#;
        fs::write(&path, bytes).unwrap();
        let loaded = ProjectConfig::load(&root).unwrap();
        assert!(loaded.issue.unwrap().to_string().contains(
            "Terrain 'volcanic': color component 301 must be between 0 and 255"
        ));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn project_save_preserves_comments_unknown_keys_and_creates_backup() {
        let root = root("preserve");
        let path = ProjectConfig::path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            b"# keep me\nschema-version = 1\nfuture-key = \"kept\"\n",
        )
        .unwrap();
        let loaded = ProjectConfig::load(&root).unwrap();
        let mut changed = loaded.value;
        changed.generate_coastal_on_save = false;
        changed
            .save(&root, loaded.fingerprint.as_ref(), false, false)
            .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me"));
        assert!(text.contains("future-key = \"kept\""));
        assert!(backup_path(&path).is_file());
    }

    #[test]
    fn external_change_and_future_schema_require_explicit_overwrite() {
        let root = root("conflict");
        let path = ProjectConfig::path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"schema-version = 1\n").unwrap();
        let loaded = ProjectConfig::load(&root).unwrap();
        fs::write(&path, b"schema-version = 2\n").unwrap();
        assert!(matches!(
            loaded
                .value
                .save(&root, loaded.fingerprint.as_ref(), false, false),
            Err(SaveConfigError::ChangedExternally)
        ));
        assert!(matches!(
            loaded.value.save(&root, None, true, false),
            Err(SaveConfigError::FutureSchema(2))
        ));
    }

    #[test]
    fn invalid_global_settings_are_rejected() {
        let mut config = GlobalConfig {
            max_undo_states: 0,
            ..GlobalConfig::default()
        };
        assert!(config.validate().unwrap_err().to_string().contains("1 and 500"));
        config.max_undo_states = 501;
        assert!(config.validate().is_err());
        config.max_undo_states = 24;
        config.ui_scale = "150%".to_owned();
        assert!(config.validate().unwrap_err().to_string().contains("System"));
    }

    #[test]
    fn global_document_roundtrips_utf8_preferences_and_preserves_unknown_content() {
        let root = root("global-roundtrip");
        let path = root.join("config.toml");
        fs::write(
            &path,
            "# comentário preservado\nschema-version = 1\nfuture-key = \"kept\"\n",
        )
        .unwrap();
        let bytes = fs::read(&path).unwrap();
        let expected = fingerprint_bytes(&bytes);
        let mut config = GlobalConfig {
            language: "pt-BR".to_owned(),
            ..GlobalConfig::default()
        };
        config.workspace.last_workspace = "states".to_owned();
        config.window.x = Some(-120);
        save_document(
            &path,
            Some(&expected),
            false,
            false,
            |document| update_global(document, &config),
            parse_global,
        )
        .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# comentário preservado"));
        assert!(text.contains("future-key = \"kept\""));
        assert!(text.contains("language = \"pt-BR\""));
        assert!(text.contains("x = -120"));
    }

    #[test]
    fn explicit_invalid_restore_creates_verified_backup() {
        let root = root("invalid-restore");
        let path = ProjectConfig::path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let invalid = b"[terrain.bad\n";
        fs::write(&path, invalid).unwrap();
        let config = ProjectConfig::default();
        replace_invalid_document(
            &path,
            |document| update_project(document, &config),
            parse_project,
        )
        .unwrap();
        assert_eq!(fs::read(backup_path(&path)).unwrap(), invalid);
        assert!(ProjectConfig::load(&root).unwrap().issue.is_none());
    }

    #[test]
    fn project_config_save_is_isolated_from_mod_files() {
        let root = root("isolation");
        fs::create_dir_all(root.join("map")).unwrap();
        fs::create_dir_all(root.join("history/states")).unwrap();
        fs::write(root.join("map/provinces.bmp"), b"bmp").unwrap();
        fs::write(root.join("map/definition.csv"), b"csv").unwrap();
        fs::write(root.join("history/states/1.txt"), b"state").unwrap();
        ProjectConfig::default()
            .save(&root, None, false, false)
            .unwrap();
        assert_eq!(fs::read(root.join("map/provinces.bmp")).unwrap(), b"bmp");
        assert_eq!(fs::read(root.join("map/definition.csv")).unwrap(), b"csv");
        assert_eq!(fs::read(root.join("history/states/1.txt")).unwrap(), b"state");
        assert!(ProjectConfig::path(&root).is_file());
    }
}
