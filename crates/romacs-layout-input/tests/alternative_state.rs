use romacs_layout_input::FontAlternativeState;

#[test]
fn family_alternatives_publish_owned_rows_and_every_setter_revision() {
    let mut state = FontAlternativeState::default();
    let families = vec![vec!["Family".into(), "Alternative".into()], vec![]];
    state.set_family_alist(families.clone());
    let published = state.clone();
    assert_eq!(state.family_alist(), families);
    assert_eq!(published.revision(), 1);
    state.set_family_alist(families.clone());
    assert_eq!(state.revision(), 2, "equal setter inputs still invalidate");
    state.set_family_alist(vec![]);
    assert_eq!(
        published.family_alist(),
        families,
        "previous publication is immutable"
    );
    state.set_family_alist(families);
    assert_eq!(state.family_alist(), published.family_alist());
    assert_eq!(
        state.revision(),
        4,
        "A -> B -> A preserves producer history"
    );
}
