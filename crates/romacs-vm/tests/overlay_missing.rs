use romacs_vm::Vm;

#[test]
fn missing_overlay_primitives_match_pinned_gnu() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/overlay-missing.json")).unwrap();
    assert_eq!(
        evidence["revision"],
        "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
    );
    for case in evidence["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let value = vm
            .eval_source(source)
            .unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(value.display().unwrap(), expected, "{source}");
    }
}
