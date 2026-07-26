use std::collections::{BTreeMap, BTreeSet};

use crate::app::state::StateData;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditableStateProperties {
  pub name: Option<String>,
  pub manpower: Option<u64>,
  pub state_category: Option<String>,
  pub buildings_max_level_factor: Option<f64>,
  pub local_supplies: Option<f64>,
  pub impassable: bool,
  pub owner: Option<String>,
  pub controller: Option<String>,
  pub cores: BTreeSet<String>,
  pub claims: BTreeSet<String>,
  pub resources: BTreeMap<String, i64>,
  pub state_buildings: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditableProvinceData {
  pub victory_point: Option<i64>,
  pub buildings: BTreeMap<String, i64>,
}

impl EditableProvinceData {
  pub fn validate(&self) -> Result<(), String> {
    if self.victory_point.is_some_and(|value| value < 0) {
      return Err("victory point must be non-negative".to_owned());
    }
    for (name, value) in &self.buildings {
      if name.is_empty() || contains_invalid_identifier(name) {
        return Err(format!("province building name is invalid: {name}"));
      }
      if *value < 0 {
        return Err(format!("province building {name} must be non-negative"));
      }
    }
    Ok(())
  }
}

impl EditableStateProperties {
  pub fn validate(&self) -> Result<(), String> {
    if self.name.as_deref().is_some_and(contains_unsafe_token) {
      return Err("state name contains one of the unsupported characters { } = # \"".to_owned());
    }
    for (field, value) in [
      ("state category", self.state_category.as_deref()),
      ("owner", self.owner.as_deref()),
      ("controller", self.controller.as_deref()),
    ] {
      if value.is_some_and(contains_invalid_identifier) {
        return Err(format!("{field} must be one non-empty PDXScript identifier"));
      }
    }
    for (field, values) in [
      ("cores", &self.cores),
      ("claims", &self.claims),
    ] {
      if let Some(value) = values.iter().find(|value| contains_invalid_identifier(value)) {
        return Err(format!("{field} contains an invalid identifier: {value}"));
      }
    }
    for (field, values) in [
      ("resources", &self.resources),
      ("state buildings", &self.state_buildings),
    ] {
      if let Some(value) = values.keys().find(|value| contains_invalid_identifier(value)) {
        return Err(format!("{field} contains an invalid key: {value}"));
      }
    }
    if self.buildings_max_level_factor.is_some_and(|value| !value.is_finite()) {
      return Err("buildings max level factor must be finite".to_owned());
    }
    if self.local_supplies.is_some_and(|value| !value.is_finite()) {
      return Err("local supplies must be finite".to_owned());
    }
    Ok(())
  }

  pub fn from_state(data: &StateData) -> Self {
    Self {
      name: data.name.clone(),
      manpower: data.manpower,
      state_category: data.state_category.clone(),
      buildings_max_level_factor: data.buildings_max_level_factor,
      local_supplies: data.local_supplies,
      impassable: data.impassable.unwrap_or(false),
      owner: data.history.owner.clone(),
      controller: data.history.controller.clone(),
      cores: data.history.cores.clone(),
      claims: data.history.claims.clone(),
      resources: data.resources.clone(),
      state_buildings: data.history.state_buildings.clone(),
    }
  }

  pub fn apply_to(&self, data: &mut StateData) {
    data.name.clone_from(&self.name);
    data.manpower = self.manpower;
    data.state_category.clone_from(&self.state_category);
    data.buildings_max_level_factor = self.buildings_max_level_factor;
    data.local_supplies = self.local_supplies;
    data.impassable = Some(self.impassable);
    data.history.owner.clone_from(&self.owner);
    data.history.controller.clone_from(&self.controller);
    data.history.cores.clone_from(&self.cores);
    data.history.claims.clone_from(&self.claims);
    data.resources.clone_from(&self.resources);
    data.history.state_buildings.clone_from(&self.state_buildings);
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyValidationError {
  pub field: &'static str,
  pub message: String,
}

#[derive(Debug, Clone)]
pub struct StatePropertyDraft {
  pub state_id: u32,
  pub name: String,
  pub manpower: String,
  pub state_category: String,
  pub buildings_max_level_factor: String,
  pub local_supplies: String,
  pub impassable: bool,
  pub owner: String,
  pub controller: String,
  pub cores: String,
  pub claims: String,
  pub resources: String,
  pub state_buildings: String,
  original: DraftValues,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DraftValues {
  fields: [String; StatePropertyDraft::TEXT_FIELD_COUNT],
  impassable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedIntegerValue {
  pub name: String,
  pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvinceDataValidationError {
  pub field: String,
  pub field_index: Option<usize>,
  pub message: String,
}

#[derive(Debug, Clone)]
pub struct ProvinceDataDraft {
  pub province_id: u32,
  pub state_id: u32,
  pub victory_point: Option<String>,
  pub buildings: Vec<NamedIntegerValue>,
  original_victory_point: Option<String>,
  original_buildings: Vec<NamedIntegerValue>,
}

impl StatePropertyDraft {
  pub const TEXT_FIELD_COUNT: usize = 11;

  pub fn new(state_id: u32, properties: &EditableStateProperties) -> Self {
    let mut draft = Self {
      state_id,
      name: option_text(&properties.name),
      manpower: option_number(properties.manpower),
      state_category: option_text(&properties.state_category),
      buildings_max_level_factor: option_number(properties.buildings_max_level_factor),
      local_supplies: option_number(properties.local_supplies),
      impassable: properties.impassable,
      owner: option_text(&properties.owner),
      controller: option_text(&properties.controller),
      cores: properties.cores.iter().cloned().collect::<Vec<_>>().join(", "),
      claims: properties.claims.iter().cloned().collect::<Vec<_>>().join(", "),
      resources: format_map(&properties.resources),
      state_buildings: format_map(&properties.state_buildings),
      original: DraftValues {
        fields: std::array::from_fn(|_| String::new()),
        impassable: properties.impassable,
      },
    };
    draft.original = draft.values();
    draft
  }

  pub fn is_modified(&self) -> bool {
    self.values() != self.original
  }

  pub fn field(&self, index: usize) -> Option<&str> {
    self.fields().get(index).copied()
  }

  pub fn field_mut(&mut self, index: usize) -> Option<&mut String> {
    match index {
      0 => Some(&mut self.name),
      1 => Some(&mut self.manpower),
      2 => Some(&mut self.state_category),
      3 => Some(&mut self.buildings_max_level_factor),
      4 => Some(&mut self.local_supplies),
      5 => Some(&mut self.owner),
      6 => Some(&mut self.controller),
      7 => Some(&mut self.cores),
      8 => Some(&mut self.claims),
      9 => Some(&mut self.resources),
      10 => Some(&mut self.state_buildings),
      _ => None,
    }
  }

  pub fn validate(&self) -> Result<EditableStateProperties, Vec<PropertyValidationError>> {
    let mut errors = Vec::new();
    let name = parse_optional_text("Name", &self.name, &mut errors);
    let manpower = parse_optional_u64("Manpower", &self.manpower, &mut errors);
    let state_category = parse_optional_identifier(
      "State category",
      &self.state_category,
      &mut errors,
    );
    let buildings_max_level_factor = parse_optional_f64(
      "Buildings max level factor",
      &self.buildings_max_level_factor,
      &mut errors,
    );
    let local_supplies = parse_optional_f64("Local supplies", &self.local_supplies, &mut errors);
    let owner = parse_optional_identifier("Owner", &self.owner, &mut errors);
    let controller = parse_optional_identifier("Controller", &self.controller, &mut errors);
    let cores = parse_identifier_set("Cores", &self.cores, &mut errors);
    let claims = parse_identifier_set("Claims", &self.claims, &mut errors);
    let resources = parse_named_integers("Resources", &self.resources, &mut errors);
    let state_buildings = parse_named_integers(
      "State buildings",
      &self.state_buildings,
      &mut errors,
    );

    if errors.is_empty() {
      Ok(EditableStateProperties {
        name,
        manpower,
        state_category,
        buildings_max_level_factor,
        local_supplies,
        impassable: self.impassable,
        owner,
        controller,
        cores,
        claims,
        resources,
        state_buildings,
      })
    } else {
      Err(errors)
    }
  }

  fn fields(&self) -> [&str; Self::TEXT_FIELD_COUNT] {
    [
      &self.name,
      &self.manpower,
      &self.state_category,
      &self.buildings_max_level_factor,
      &self.local_supplies,
      &self.owner,
      &self.controller,
      &self.cores,
      &self.claims,
      &self.resources,
      &self.state_buildings,
    ]
  }

  fn values(&self) -> DraftValues {
    DraftValues {
      fields: self.fields().map(str::to_owned),
      impassable: self.impassable,
    }
  }
}

impl ProvinceDataDraft {
  pub fn new(province_id: u32, state_id: u32, data: &EditableProvinceData) -> Self {
    let victory_point = data.victory_point.map(|value| value.to_string());
    let buildings = data.buildings.iter()
      .map(|(name, value)| NamedIntegerValue {
        name: name.clone(),
        value: value.to_string(),
      })
      .collect::<Vec<_>>();
    Self {
      province_id,
      state_id,
      original_victory_point: victory_point.clone(),
      original_buildings: buildings.clone(),
      victory_point,
      buildings,
    }
  }

  pub fn is_modified(&self) -> bool {
    self.victory_point != self.original_victory_point
      || self.buildings != self.original_buildings
  }

  pub fn text_field_count(&self) -> usize {
    1 + self.buildings.len() * 2
  }

  pub fn field(&self, index: usize) -> Option<&str> {
    match index {
      0 => self.victory_point.as_deref(),
      _ => {
        let building = self.buildings.get((index - 1) / 2)?;
        if index % 2 == 1 {
          Some(&building.name)
        } else {
          Some(&building.value)
        }
      },
    }
  }

  pub fn field_mut(&mut self, index: usize) -> Option<&mut String> {
    match index {
      0 => self.victory_point.as_mut(),
      _ => {
        let building = self.buildings.get_mut((index - 1) / 2)?;
        if index % 2 == 1 {
          Some(&mut building.name)
        } else {
          Some(&mut building.value)
        }
      },
    }
  }

  pub fn toggle_victory_point(&mut self) {
    self.victory_point = self.victory_point.take().is_none().then(String::new);
  }

  pub fn add_building(&mut self) -> usize {
    self.buildings.push(NamedIntegerValue {
      name: String::new(),
      value: String::new(),
    });
    self.buildings.len() - 1
  }

  pub fn remove_building(&mut self, index: usize) -> bool {
    if index < self.buildings.len() {
      self.buildings.remove(index);
      true
    } else {
      false
    }
  }

  pub fn validate(&self) -> Result<EditableProvinceData, Vec<ProvinceDataValidationError>> {
    let mut errors = Vec::new();
    let victory_point = self.victory_point.as_ref().and_then(|raw| {
      match raw.trim().parse::<i64>() {
        Ok(value) if value >= 0 => Some(value),
        _ => {
          errors.push(province_error(
            "Victory point value",
            Some(0),
            "must be a non-negative integer without decimals",
          ));
          None
        },
      }
    });
    let mut buildings = BTreeMap::new();
    for (index, building) in self.buildings.iter().enumerate() {
      let name_field = 1 + index * 2;
      let value_field = name_field + 1;
      let name = building.name.trim();
      if name.is_empty() {
        errors.push(province_error(
          format!("Province building {} name", index + 1),
          Some(name_field),
          "must not be empty",
        ));
        continue;
      }
      if contains_invalid_identifier(name) {
        errors.push(province_error(
          format!("Province building {} name", index + 1),
          Some(name_field),
          "must be one identifier without { } = # \" or whitespace",
        ));
        continue;
      }
      let value = match building.value.trim().parse::<i64>() {
        Ok(value) if value >= 0 => value,
        _ => {
          errors.push(province_error(
            format!("Province building {} level", index + 1),
            Some(value_field),
            "must be a non-negative integer without decimals",
          ));
          continue;
        },
      };
      if buildings.insert(name.to_owned(), value).is_some() {
        errors.push(province_error(
          format!("Province building {} name", index + 1),
          Some(name_field),
          format!("duplicates building {name}"),
        ));
      }
    }
    if errors.is_empty() {
      Ok(EditableProvinceData {
        victory_point,
        buildings,
      })
    } else {
      Err(errors)
    }
  }
}

fn option_text(value: &Option<String>) -> String {
  value.clone().unwrap_or_default()
}

fn option_number(value: Option<impl ToString>) -> String {
  value.map(|value| value.to_string()).unwrap_or_default()
}

fn format_map(values: &BTreeMap<String, i64>) -> String {
  values.iter()
    .map(|(name, value)| format!("{name}={value}"))
    .collect::<Vec<_>>()
    .join(", ")
}

fn parse_optional_text(
  field: &'static str,
  value: &str,
  errors: &mut Vec<PropertyValidationError>,
) -> Option<String> {
  let value = value.trim();
  if value.is_empty() {
    None
  } else if contains_unsafe_token(value) {
    errors.push(error(field, "contains one of the unsupported characters { } = # \""));
    None
  } else {
    Some(value.to_owned())
  }
}

fn parse_optional_identifier(
  field: &'static str,
  value: &str,
  errors: &mut Vec<PropertyValidationError>,
) -> Option<String> {
  let value = value.trim();
  if value.is_empty() {
    None
  } else if contains_invalid_identifier(value) {
    errors.push(error(field, "must be one non-empty PDXScript identifier"));
    None
  } else {
    Some(value.to_owned())
  }
}

fn parse_optional_u64(
  field: &'static str,
  value: &str,
  errors: &mut Vec<PropertyValidationError>,
) -> Option<u64> {
  let value = value.trim();
  if value.is_empty() {
    None
  } else {
    match value.parse() {
      Ok(value) => Some(value),
      Err(_) => {
        errors.push(error(field, "must be a non-negative integer without decimals"));
        None
      },
    }
  }
}

fn parse_optional_f64(
  field: &'static str,
  value: &str,
  errors: &mut Vec<PropertyValidationError>,
) -> Option<f64> {
  let value = value.trim();
  if value.is_empty() {
    return None;
  }
  match value.parse::<f64>() {
    Ok(value) if value.is_finite() => Some(value),
    _ => {
      errors.push(error(field, "must be a finite decimal number"));
      None
    },
  }
}

fn parse_identifier_set(
  field: &'static str,
  value: &str,
  errors: &mut Vec<PropertyValidationError>,
) -> BTreeSet<String> {
  if value.trim().is_empty() {
    return BTreeSet::new();
  }
  let mut result = BTreeSet::new();
  for raw in value.split(',') {
    let value = raw.trim();
    if value.is_empty() {
      errors.push(error(field, "contains an empty entry"));
    } else if contains_invalid_identifier(value) {
      errors.push(error(field, format!("contains an invalid identifier: {value}")));
    } else if !result.insert(value.to_owned()) {
      errors.push(error(field, format!("contains a duplicate entry: {value}")));
    }
  }
  result
}

fn parse_named_integers(
  field: &'static str,
  value: &str,
  errors: &mut Vec<PropertyValidationError>,
) -> BTreeMap<String, i64> {
  if value.trim().is_empty() {
    return BTreeMap::new();
  }
  let mut result = BTreeMap::new();
  for raw in value.split(',') {
    let entry = raw.trim();
    let Some((name, amount)) = entry.split_once('=') else {
      errors.push(error(field, format!("entry must use name=value: {entry}")));
      continue;
    };
    let name = name.trim();
    if name.is_empty() {
      errors.push(error(field, "contains an empty key"));
      continue;
    }
    if contains_invalid_identifier(name) {
      errors.push(error(field, format!("contains an invalid key: {name}")));
      continue;
    }
    let amount = match amount.trim().parse::<i64>() {
      Ok(amount) => amount,
      Err(_) => {
        errors.push(error(field, format!("value for {name} must be an integer")));
        continue;
      },
    };
    if result.insert(name.to_owned(), amount).is_some() {
      errors.push(error(field, format!("contains a duplicate key: {name}")));
    }
  }
  result
}

fn contains_unsafe_token(value: &str) -> bool {
  value.chars().any(|character| matches!(character, '{' | '}' | '=' | '#' | '"'))
}

fn contains_invalid_identifier(value: &str) -> bool {
  contains_unsafe_token(value) || value.chars().any(char::is_whitespace)
}

fn error(field: &'static str, message: impl Into<String>) -> PropertyValidationError {
  PropertyValidationError {
    field,
    message: message.into(),
  }
}

fn province_error(
  field: impl Into<String>,
  field_index: Option<usize>,
  message: impl Into<String>,
) -> ProvinceDataValidationError {
  ProvinceDataValidationError {
    field: field.into(),
    field_index,
    message: message.into(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn draft_validation_is_atomic_and_keeps_custom_zero_values() {
    let properties = EditableStateProperties::default();
    let mut draft = StatePropertyDraft::new(1, &properties);
    draft.manpower = "1.5".into();
    draft.resources = "oil=0, oil=2".into();
    assert!(draft.validate().is_err());

    draft.manpower = "150000".into();
    draft.resources = "custom_resource=0".into();
    let parsed = draft.validate().unwrap();
    assert_eq!(parsed.manpower, Some(150_000));
    assert_eq!(parsed.resources["custom_resource"], 0);

    let mut province = ProvinceDataDraft::new(5144, 1, &EditableProvinceData {
      victory_point: Some(5),
      buildings: BTreeMap::new(),
    });
    province.victory_point = Some("10".into());
    province.buildings.push(NamedIntegerValue {
      name: "custom_test_building".into(),
      value: "2".into(),
    });
    let parsed = province.validate().unwrap();
    assert_eq!(parsed.victory_point, Some(10));
    assert_eq!(parsed.buildings["custom_test_building"], 2);
    province.victory_point = Some("010".into());
    province.buildings[0].value = "02".into();
    assert_eq!(province.validate().unwrap(), parsed);

    province.buildings.push(NamedIntegerValue {
      name: "custom_test_building".into(),
      value: "3".into(),
    });
    assert!(province.validate().is_err());

    for invalid in ["1.5", "-1", "999999999999999999999999999999"] {
      province.buildings.truncate(1);
      province.victory_point = Some(invalid.into());
      assert!(province.validate().is_err());
    }
    province.victory_point = Some("10".into());
    province.buildings = vec![
      NamedIntegerValue { name: String::new(), value: "1".into() },
      NamedIntegerValue { name: "bad{name".into(), value: "1".into() },
      NamedIntegerValue { name: "custom".into(), value: "1.5".into() },
    ];
    assert!(province.validate().is_err());
  }
}
