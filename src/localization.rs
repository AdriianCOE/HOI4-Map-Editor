use once_cell::sync::Lazy;
use toml::Value;

use std::cell::Cell;
use std::collections::BTreeMap;

const EN_US_SOURCE: &str = include_str!("../locales/en-US.toml");
const PT_BR_SOURCE: &str = include_str!("../locales/pt-BR.toml");

thread_local! {
    static LANGUAGE: Cell<u8> = const { Cell::new(0) };
}
static EN_US: Lazy<BTreeMap<String, &'static str>> = Lazy::new(|| catalog(EN_US_SOURCE));
static PT_BR: Lazy<BTreeMap<String, &'static str>> = Lazy::new(|| catalog(PT_BR_SOURCE));

pub fn set_language(language: &str) -> bool {
    let value = match language {
        "en-US" => 0,
        "pt-BR" => 1,
        _ => return false,
    };
    LANGUAGE.set(value);
    true
}

pub fn language() -> &'static str {
    if LANGUAGE.get() == 1 {
        "pt-BR"
    } else {
        "en-US"
    }
}

pub fn tr(key: &str) -> &'static str {
    if LANGUAGE.get() == 1 {
        if let Some(value) = PT_BR.get(key) {
            return value;
        }
        eprintln!("Missing pt-BR localization key: {key}");
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
    fn english_is_complete_and_portuguese_placeholders_match() {
        assert!(!EN_US.is_empty());
        for (key, english) in EN_US.iter() {
            let portuguese = PT_BR
                .get(key)
                .unwrap_or_else(|| panic!("pt-BR is missing required key {key}"));
            assert_eq!(placeholders(english), placeholders(portuguese), "{key}");
        }
    }

    #[test]
    fn language_switch_fallback_plural_and_utf8_work() {
        assert!(set_language("pt-BR"));
        assert_eq!(tr("menu.edit"), "Editar");
        assert_eq!(tr_count("status.pending_changes", 0), "Nenhuma alteração pendente");
        assert_eq!(tr_count("status.pending_changes", 1), "1 alteração pendente");
        assert_eq!(tr_count("status.pending_changes", 4), "4 alterações pendentes");
        assert_eq!(tr("missing-key"), "missing key");
        assert!(set_language("en-US"));
        assert_eq!(tr_count("status.pending_changes", 2), "2 pending changes");
    }

    #[test]
    fn catalogs_do_not_contain_mojibake() {
        for value in EN_US.values().chain(PT_BR.values()) {
            assert!(!value.contains("Ã"));
            assert!(!value.contains("â€"));
        }
    }

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .filter_map(|tail| tail.split('}').next())
            .collect()
    }
}
