use refineable::{IsEmpty, Refineable};

#[derive(Clone, Refineable)]
struct NoFields {}

#[derive(Clone, Refineable)]
struct OneField {
    value: u32,
}

#[test]
fn refinement_of_a_struct_without_fields_is_empty() {
    assert!(NoFieldsRefinement::default().is_empty());

    let mut value = NoFields {};
    value.refine(&NoFieldsRefinement::default());
}

#[test]
fn refinement_is_empty_until_a_field_is_set() {
    let mut value = OneField { value: 0 };
    let mut refinement = OneFieldRefinement::default();
    assert!(refinement.is_empty());

    refinement.value = Some(7);
    assert!(!refinement.is_empty());
    value.refine(&refinement);
    assert_eq!(value.value, 7);
}
