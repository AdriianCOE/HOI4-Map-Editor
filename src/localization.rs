use once_cell::sync::Lazy;
use toml::Value;

use std::cell::Cell;
use std::collections::BTreeMap;

const EN_US_SOURCE: &str = include_str!("../locales/en-US.toml");
const PT_BR_SOURCE: &str = include_str!("../locales/pt-BR.toml");
const ES_ES_SOURCE: &str = include_str!("../locales/es-ES.toml");
const FR_FR_SOURCE: &str = include_str!("../locales/fr-FR.toml");
const RU_RU_SOURCE: &str = include_str!("../locales/ru-RU.toml");
const ZH_CN_SOURCE: &str = include_str!("../locales/zh-CN.toml");

pub const SUPPORTED_LANGUAGES: &[(&str, &str)] = &[
    ("en-US", "English"),
    ("pt-BR", "Português do Brasil"),
    ("es-ES", "Español"),
    ("fr-FR", "Français"),
    ("ru-RU", "Русский"),
    ("zh-CN", "简体中文"),
];

thread_local! {
    static LANGUAGE: Cell<u8> = const { Cell::new(0) };
}
static EN_US: Lazy<BTreeMap<String, &'static str>> = Lazy::new(|| catalog(EN_US_SOURCE));
static PT_BR: Lazy<BTreeMap<String, &'static str>> = Lazy::new(|| catalog(PT_BR_SOURCE));
static ES_ES: Lazy<BTreeMap<String, &'static str>> = Lazy::new(|| catalog(ES_ES_SOURCE));
static FR_FR: Lazy<BTreeMap<String, &'static str>> = Lazy::new(|| catalog(FR_FR_SOURCE));
static RU_RU: Lazy<BTreeMap<String, &'static str>> = Lazy::new(|| catalog(RU_RU_SOURCE));
static ZH_CN: Lazy<BTreeMap<String, &'static str>> = Lazy::new(|| catalog(ZH_CN_SOURCE));

pub fn set_language(language: &str) -> bool {
    if let Some(index) = SUPPORTED_LANGUAGES
        .iter()
        .position(|(code, _)| *code == language)
    {
        LANGUAGE.set(index as u8);
        true
    } else {
        LANGUAGE.set(0);
        eprintln!("Unsupported UI language '{language}'; using en-US for this session");
        false
    }
}

pub fn language() -> &'static str {
    SUPPORTED_LANGUAGES
        .get(LANGUAGE.get() as usize)
        .map(|(code, _)| *code)
        .unwrap_or("en-US")
}

pub fn native_name(language: &str) -> &str {
    SUPPORTED_LANGUAGES
        .iter()
        .find_map(|(code, name)| (*code == language).then_some(*name))
        .unwrap_or(language)
}

pub fn next_language(language: &str) -> &'static str {
    let next = SUPPORTED_LANGUAGES
        .iter()
        .position(|(code, _)| *code == language)
        .map(|index| (index + 1) % SUPPORTED_LANGUAGES.len())
        .unwrap_or_default();
    SUPPORTED_LANGUAGES[next].0
}

pub fn tr(key: &str) -> &'static str {
    let selected = match LANGUAGE.get() {
        1 => &*PT_BR,
        2 => &*ES_ES,
        3 => &*FR_FR,
        4 => &*RU_RU,
        5 => &*ZH_CN,
        _ => &*EN_US,
    };
    if !std::ptr::eq(selected, &*EN_US) {
        if let Some(value) = selected.get(key) {
            return value;
        }
        eprintln!("Missing {} localization key: {key}", language());
    }
    EN_US
        .get(key)
        .copied()
        .unwrap_or_else(|| Box::leak(readable_key(key).into_boxed_str()))
}

pub fn tr_args(key: &str, arguments: &[(&str, &str)]) -> String {
    arguments
        .iter()
        .fold(tr(key).to_owned(), |text, (name, value)| {
            text.replace(&format!("{{{name}}}"), value)
        })
}

pub fn tr_count(key: &str, count: usize) -> String {
    let form = if count == 0 {
        "zero"
    } else if language() == "ru-RU" && count % 10 == 1 && count % 100 != 11 {
        "one"
    } else if language() == "ru-RU"
        && matches!(count % 10, 2..=4)
        && !matches!(count % 100, 12..=14)
    {
        "few"
    } else if count == 1 {
        "one"
    } else {
        "other"
    };
    tr_args(
        &format!("{key}.{form}"),
        &[("count", &count.to_string())],
    )
}

pub fn ui_literal(english: &'static str) -> &'static str {
    let key = match english {
        "File" => "menu.file",
        "Edit" => "menu.edit",
        "View" => "menu.view",
        "Tools" => "menu.tools",
        "Help" => "menu.help",
        "Open File or Archive..." => "file.open_archive",
        "Open HOI4 Mod..." => "file.open_mod",
        "Review State Changes" => "workspace.review",
        "Save Current Workspace" => "file.save_workspace",
        "Export Province Map Archive..." => "file.export_archive",
        "Export Province Map As..." => "file.export_map",
        "Reveal in File Browser" => "file.reveal",
        "Export Land Map..." => "file.export_land",
        "Export Terrain Map..." => "file.export_terrain",
        "Undo" => "edit.undo",
        "Redo" => "edit.redo",
        "Find on Map" => "edit.find",
        "New State" => "edit.new_state",
        "Settings..." => "menu.settings",
        "Project Settings..." => "menu.project_settings",
        "Edit province data" => "edit.province_data",
        "Clear Selection" => "edit.clear_selection",
        "Remove state from session" => "edit.remove_state",
        "Edit state properties" => "edit.state_properties",
        "Select All Provinces in Target State" => "edit.select_target",
        "Move Selected Provinces to Target State" => "edit.move_target",
        "Unassign Selected Provinces" => "edit.unassign",
        "Discard all in-memory state edits" => "edit.discard_states",
        "Re-calculate Coastal Provinces" => "edit.recalculate_coastal",
        "Re-color Provinces" => "edit.recolor",
        "Calculate Map Errors/Warnings" => "edit.problems",
        "Edit Adjacencies" => "edit.edit_adjacencies",
        "Lasso Options: Pixel Snap" => "tools.lasso_snap",
        "Lasso Options: Replace Selection" => "tools.lasso_replace",
        "Lasso Options: Add to Selection" => "tools.lasso_add",
        "Lasso Options: Remove from Selection" => "tools.lasso_remove",
        "Lasso Options: Include Centroid" => "tools.lasso_centroid",
        "Lasso Options: Include Any Intersection" => "tools.lasso_intersection",
        "Lasso Options: Include Majority" => "tools.lasso_majority",
        "Lasso: Confirm Selection" => "tools.lasso_confirm",
        "Lasso: Cancel" => "tools.lasso_cancel",
        "Brush Options: Next Mask Mode" => "tools.brush_mask",
        "Brush Mode: Assign to Target" => "tools.brush_assign",
        "Brush Mode: Unassign" => "tools.brush_unassign",
        "Brush: Cancel" => "tools.brush_cancel",
        "Fill Mode: Hovered Province" => "tools.fill_hover",
        "Fill Mode: Connected Same State" => "tools.fill_state",
        "Fill Mode: Connected Unassigned" => "tools.fill_unassigned",
        "Fill Mode: Whole Source State" => "tools.fill_whole",
        "Fill: Apply Preview" => "tools.fill_apply",
        "Fill: Cancel" => "tools.fill_cancel",
        "Preview / Generate" => "tools.preview_generate",
        "Preview / Regenerate" => "tools.preview_regenerate",
        "Preview / Previous File" => "tools.preview_previous",
        "Preview / Next File" => "tools.preview_next",
        "Validation / Temporary Copy" => "tools.validation_copy",
        "Validation / Review-Required" => "tools.validation_review",
        "Validation / Cancel" => "tools.validation_cancel",
        "Validation / View Report" => "tools.validation_report",
        "Validation / Clear Result" => "tools.validation_clear",
        "Save / Cancel" => "tools.save_cancel",
        "Save / View Report" => "tools.save_report",
        "Save / Recover Interrupted" => "tools.save_recover",
        "Preview / Clear" => "tools.preview_clear",
        "Map View" => "view.map",
        "Overlays" => "view.overlays",
        "Province Colors" => "view.province_colors",
        "Province Types" => "view.province_types",
        "Terrain / Biome" => "view.terrain",
        "Continents" => "view.continents",
        "Coastal Provinces" => "view.coastal",
        "States" => "view.states",
        "Political" => "view.political",
        "Rivers" => "view.rivers",
        "Adjacencies" => "view.adjacencies",
        "Province IDs" => "view.province_ids",
        "Province Borders" => "view.province_borders",
        "State Borders" => "view.state_borders",
        "Image Overlay" => "view.image",
        "Map View: Province Colors" => "view.map_province_colors",
        "Map View: Province Types" => "view.map_province_types",
        "Map View: Terrain / Biome" => "view.map_terrain",
        "Map View: Continents" => "view.map_continents",
        "Map View: Coastal Provinces" => "view.map_coastal",
        "Map View: States" => "view.map_states",
        "Map View: Political" => "view.map_political",
        "Overlays: Rivers" => "view.overlay_rivers",
        "Overlays: Adjacencies" => "view.overlay_adjacencies",
        "Overlays: Province IDs" => "view.overlay_ids",
        "Overlays: Province Borders" => "view.overlay_province_borders",
        "Overlays: State Borders" => "view.overlay_state_borders",
        "Overlays: Image Overlay..." => "view.overlay_image",
        "Overlays: Configure Image..." => "view.configure_image",
        "Overlays: Use Project Heightmap" => "view.heightmap",
        "Overlays: Opacity -10%" => "view.opacity_down",
        "Overlays: Opacity +10%" => "view.opacity_up",
        "Overlays: Clear Image" => "view.clear_image",
        "Panels: State Inspector" => "view.state_inspector",
        "Panels: Developer Diagnostics" => "view.developer",
        "Definitions: Choose Base Game..." => "view.choose_definitions",
        "Definitions: Clear Base Game" => "view.clear_definitions",
        "Reset Zoom" => "view.reset_zoom",
        "About HOI4 Map Editor" => "about.title",
        "Copy Version Information" => "help.copy_version",
        "Open Logs Folder" => "about.open_logs",
        "Font Licenses" => "help.font_licenses",
        _ => return english,
    };
    tr(key)
}

fn catalog(source: &'static str) -> BTreeMap<String, &'static str> {
    let value = source
        .parse::<Value>()
        .expect("embedded localization catalog must be valid TOML");
    let mut result = BTreeMap::new();
    flatten("", &value, &mut result);
    result
}

fn flatten(prefix: &str, value: &Value, output: &mut BTreeMap<String, &'static str>) {
    match value {
        Value::Table(table) => {
            for (key, value) in table {
                let key = if prefix.is_empty() {
                    key.to_owned()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&key, value, output);
            }
        }
        Value::String(text) => {
            output.insert(
                prefix.to_owned(),
                Box::leak(text.clone().into_boxed_str()),
            );
        }
        _ => panic!("localization catalog values must be strings"),
    }
}

fn readable_key(key: &str) -> String {
    key.rsplit('.')
        .next()
        .unwrap_or(key)
        .replace(['_', '-'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn all_supported_catalogs_match_english_keys_placeholders_and_content_rules() {
        assert!(!EN_US.is_empty());
        assert_eq!(SUPPORTED_LANGUAGES.len(), 6);
        for (code, catalog) in catalogs() {
            assert_eq!(catalog.len(), EN_US.len(), "{code} key count");
            for (key, english) in EN_US.iter() {
                let translated = catalog
                    .get(key)
                    .unwrap_or_else(|| panic!("{code} is missing required key {key}"));
                assert!(!translated.trim().is_empty(), "{code}.{key} is empty");
                assert_eq!(
                    placeholders(english),
                    placeholders(translated),
                    "{code}.{key}"
                );
            }
            for key in catalog.keys() {
                assert!(EN_US.contains_key(key), "{code} has unknown key {key}");
            }
        }
    }

    #[test]
    fn language_switch_fallback_plural_and_utf8_work() {
        for (code, _) in SUPPORTED_LANGUAGES {
            assert!(set_language(code));
            for count in [0, 1, 2, 4] {
                assert!(
                    tr_count("status.pending_changes", count).contains(&count.to_string())
                        || count == 0
                );
            }
        }
        assert!(set_language("pt-BR"));
        assert_eq!(tr("menu.edit"), "Editar");
        assert_eq!(tr_count("status.pending_changes", 0), "Nenhuma alteração pendente");
        assert_eq!(tr_count("status.pending_changes", 1), "1 alteração pendente");
        assert_eq!(tr_count("status.pending_changes", 4), "4 alterações pendentes");
        assert_eq!(tr("missing-key"), "missing key");
        assert!(!set_language("unknown-language"));
        assert_eq!(language(), "en-US");
        assert_eq!(tr("menu.file"), "File");
        assert!(set_language("en-US"));
        assert_eq!(tr_count("status.pending_changes", 2), "2 pending changes");
    }

    #[test]
    fn catalogs_do_not_contain_mojibake() {
        const BROKEN_SEQUENCES: &[&str] = &[
            "Ãƒ", "Ã¢â", "Â ", "â€”", "â€“", "Ð°", "Ðµ", "Ñ€", "锟斤拷",
        ];
        for (code, catalog) in catalogs() {
            for (key, value) in catalog {
                for broken in BROKEN_SEQUENCES {
                    assert!(
                        !value.contains(broken),
                        "{code}.{key} contains mojibake sequence {broken}"
                    );
                }
            }
        }
    }

    #[test]
    fn native_language_names_and_long_text_are_available() {
        assert_eq!(native_name("pt-BR"), "Português do Brasil");
        assert_eq!(native_name("ru-RU"), "Русский");
        assert_eq!(native_name("zh-CN"), "简体中文");
        assert_eq!(next_language("zh-CN"), "en-US");
        assert_eq!(next_language("unknown"), "en-US");
        assert!(FR_FR.values().any(|value| value.chars().count() > 60));
    }

    fn catalogs() -> [(&'static str, &'static BTreeMap<String, &'static str>); 6] {
        [
            ("en-US", &EN_US),
            ("pt-BR", &PT_BR),
            ("es-ES", &ES_ES),
            ("fr-FR", &FR_FR),
            ("ru-RU", &RU_RU),
            ("zh-CN", &ZH_CN),
        ]
    }

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .filter_map(|tail| tail.split('}').next())
            .collect()
    }
}

#[cfg(test)]
mod dialog_layout_regressions {
    use super::*;

    /// Settings/Project Settings rows draw flat, unwrapped text starting
    /// 16px from the dialog's left edge, with a selection highlight ending
    /// 8px from the right edge (see `draw_dialog_text` and the row highlight
    /// rect in `app.rs`). This asserts every row's translated text, at the
    /// dialog's narrowest allowed width (`preferences_rect`'s 520px floor),
    /// still fits before that edge in every supported language. `"[x] "` and
    /// a `": 999"` suffix are added uniformly as a conservative stand-in for
    /// the widest real row shape (checkbox rows and "label: value" rows).
    #[test]
    fn settings_dialog_rows_fit_the_narrowest_dialog_width_in_every_language() {
        const DIALOG_MIN_WIDTH: f64 = 520.0;
        const DIALOG_MARGIN: f64 = 24.0;
        let budget = DIALOG_MIN_WIDTH - DIALOG_MARGIN;
        let keys = [
            "settings.title", "settings.language", "settings.open_last",
            "settings.remember_workspace", "settings.remember_overlays",
            "settings.tooltip_delay", "settings.max_undo", "settings.change_view_undo",
            "settings.reset_layout", "settings.restore", "settings.cancel", "settings.save",
            "project_settings.title", "project_settings.preserve_ids",
            "project_settings.generate_coastal", "project_settings.lone_pixels",
            "project_settings.few_borders", "project_settings.threshold",
            "project_settings.open", "project_settings.validate",
        ];
        let mut overflows = Vec::new();
        for (code, _) in SUPPORTED_LANGUAGES {
            set_language(code);
            for key in keys {
                let approx = format!("[x] {}: 999", tr(key));
                let width = crate::font::get_width_metric_str(&approx);
                if width > budget {
                    overflows.push(format!("{code}.{key} width={width:.1} budget={budget:.1}"));
                }
            }
        }
        assert!(
            overflows.is_empty(),
            "settings dialog rows overflow the narrowest dialog width:\n{}",
            overflows.join("\n")
        );
    }
}
