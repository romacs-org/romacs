use romacs_vm::Vm;

#[test]
fn superset_definition_matches_recorded_gnu() {
    let evidence: serde_json::Value = serde_json::from_str(include_str!("../evidence/charset-superset/gnu-observations.json")).unwrap();
    let rows = evidence["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 13);
    for row in rows {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let result = vm.eval_source(row["source"].as_str().unwrap()).unwrap().display().unwrap();
        assert_eq!(result, row["expected"].as_str().unwrap(), "{}", row["id"]);
    }
}
