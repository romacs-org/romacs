use romacs_vm::Vm;

#[test]
fn provide_subfeatures_are_stored_and_queried() {
    let mut vm = Vm::new();
    let source = "(progn (provide 'ps '(display syntax-table field)) (list (get 'ps 'subfeatures) (featurep 'ps) (featurep 'ps 'display) (featurep 'ps 'absent)))";
    assert_eq!(vm.eval_source(source).and_then(|v| v.display()).as_deref(), Ok("((display syntax-table field) t t nil)"), "PROVIDE_SUBFEATURES");
}

#[test]
fn provide_featurep_replay_live_gnu() {
    let evidence: serde_json::Value = serde_json::from_str(include_str!("../tools/primh/evidence/provide.json")).unwrap();
    for case in evidence["cases"].as_array().unwrap() {
        let mut vm = Vm::new();
        let source = case["source"].as_str().unwrap();
        let source = source.strip_prefix("(prin1-to-string ").unwrap().strip_suffix(')').unwrap();
        let actual = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(actual.as_deref(), case["expected"].as_str().map(Ok).unwrap(), "PROVIDE_GNU {}: {source}", case["id"]);
    }
}
