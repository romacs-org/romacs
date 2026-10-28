use std::collections::BTreeSet;

#[test]
fn named_void_function_diagnostics_do_not_inflate_the_live_registry() {
    let universe = BTreeSet::from([
        "car".to_owned(),
        "boot-registry-missing-symbol".to_owned(),
        "and".to_owned(),
    ]);
    let (bound, panics) = romacs_introspect_live::vm2_registry(&universe);
    assert!(panics.is_empty(), "{panics:?}");
    // `and` is a special form, fbound as in GNU.
    assert_eq!(bound, vec!["and", "car"]);
}
