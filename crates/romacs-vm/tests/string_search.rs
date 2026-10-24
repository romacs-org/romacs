use romacs_vm::Vm;

#[test]
fn string_search_live_gnu_values() {
    let evidence: serde_json::Value = serde_json::from_str(include_str!("../tools/primh/evidence/string-search.json")).unwrap();
    assert_eq!(evidence["gnu_binary_sha256"], "a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8");
    for case in evidence["cases"].as_array().unwrap() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let source = case["source"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let expression = source.strip_prefix("(prin1-to-string ").unwrap().strip_suffix(')').unwrap();
        let actual = vm.eval_source(expression).and_then(|v| v.display());
        assert_eq!(actual.as_deref(), Ok(expected), "STRING_SEARCH_GNU {}: {source}", case["id"]);
    }
}
