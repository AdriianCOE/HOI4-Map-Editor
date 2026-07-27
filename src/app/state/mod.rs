mod extractor;
mod loader;
mod model;
mod syntax;

pub use extractor::{ExtractStateResult, extract_state};
pub use loader::{StateLoadBatch, load_state_documents};
pub use model::{DatedHistoryBlock, StateData, StateDocument, StateHistory, VictoryPoint};
pub use syntax::{
    NewlineStyle, ParseOptions, PdxBlock, PdxDocument, PdxEntry, PdxScalar, PdxScalarKind,
    PdxValue, SourceText, SyntaxDiagnostic, SyntaxDiagnosticKind, TextSpan, Token, TokenKind, lex,
    lex_text, parse, parse_text, parse_with_options,
};

#[cfg(test)]
mod synthetic_fixture_tests {
    use super::load_state_documents;
    use crate::app::project::{
        EditableProvinceData, EditableStateProperties, ProvinceDataDraft, StatePropertyDraft,
    };
    use std::collections::BTreeMap;
    use std::path::Path;

    #[test]
    fn synthetic_state_exercises_inspector_reference_values() {
        let directory =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic/history/states");
        let batch = load_state_documents(&directory);
        let data = batch.documents[0].data.as_ref().unwrap();

        assert_eq!(data.id, Some(900));
        assert_eq!(data.name.as_deref(), Some("TEST_STATE"));
        assert_eq!(data.provinces, [101, 102, 103, 104].into());
        assert_eq!(data.manpower, Some(12_000));
        assert_eq!(data.state_category.as_deref(), Some("synthetic_category"));
        assert_eq!(data.history.owner.as_deref(), Some("TST"));
        assert_eq!(data.history.controller, None);
        assert!(data.history.cores.contains("TST"));
        assert_eq!(data.resources.get("test_metal"), Some(&3));
        assert_eq!(data.resources.get("test_fuel"), Some(&2));
        assert_eq!(
            data.history
                .victory_points
                .iter()
                .map(|vp| (vp.province_id, vp.value))
                .collect::<Vec<_>>(),
            [(101, 5), (103, 10)]
        );
        assert_eq!(data.history.state_buildings.get("infrastructure"), Some(&2));
        assert_eq!(
            data.history.state_buildings.get("industrial_complex"),
            Some(&1)
        );
        assert_eq!(
            data.history
                .province_buildings
                .get(&103)
                .and_then(|buildings| buildings.get("naval_base")),
            Some(&1)
        );

        let mut state_draft =
            StatePropertyDraft::new(900, &EditableStateProperties::from_state(data));
        state_draft.manpower = "12500".to_owned();
        state_draft.resources = "test_metal=4, test_fuel=2".to_owned();
        state_draft.state_buildings = "infrastructure=3, industrial_complex=1".to_owned();
        let properties = state_draft.validate().unwrap();
        assert_eq!(properties.manpower, Some(12_500));
        assert_eq!(properties.resources.get("test_metal"), Some(&4));
        assert_eq!(properties.state_buildings.get("infrastructure"), Some(&3));

        let mut province_draft = ProvinceDataDraft::new(
            103,
            900,
            &EditableProvinceData {
                victory_point: Some(10),
                buildings: BTreeMap::new(),
            },
        );
        province_draft.victory_point = Some("15".to_owned());
        assert_eq!(province_draft.validate().unwrap().victory_point, Some(15));
    }
}
