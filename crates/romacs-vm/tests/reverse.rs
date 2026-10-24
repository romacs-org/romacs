use romacs_vm::Vm;

#[test]
fn reverse_matches_pinned_gnu() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/reverse.json")).unwrap();
    assert_eq!(
        evidence["gnu"]["revision"],
        "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
    );
    let rows = evidence["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 23, "REVERSE_ORACLE_NONVACUOUS");
    let ids: std::collections::BTreeSet<_> =
        rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), rows.len(), "REVERSE_ORACLE_UNIQUE");
    let mut failures = Vec::new();
    for row in rows {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_collection_floor(1);
        let actual = vm
            .eval_source(row["source"].as_str().unwrap())
            .and_then(|v| v.display());
        if actual.as_deref() != Ok(row["expected"].as_str().unwrap()) {
            failures.push(format!(
                "REVERSE_ORACLE_{}: expected {}; actual {actual:?}",
                row["id"].as_str().unwrap(),
                row["expected"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn reverse_limitations_remain_explicit() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/reverse.json")).unwrap();
    let rows = evidence["limitations"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["expected"], "#&3\"\u{7}\"");
    let mut vm = Vm::new();
    let actual = vm
        .eval_source(rows[0]["source"].as_str().unwrap())
        .unwrap().display().unwrap();
    assert_eq!(
        actual, rows[0]["expected"].as_str().unwrap(),
        "REVERSE_BOOL_VECTOR_SUBSTRATE"
    );
    assert_eq!(rows[1]["expected"], "t");
    assert_eq!(
        vm.eval_source(rows[1]["source"].as_str().unwrap())
            .unwrap()
            .display()
            .unwrap(),
        "nil",
        "REVERSE_EMPTY_GLOBAL_IDENTITY_GAP"
    );
}

#[test]
fn reverse_results_survive_moving_collection() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_collection_floor(1_000_000);
    vm.eval_source("(let ((i 0)) (while (< i 100) (cons i nil) (setq i (1+ i))))")
        .unwrap();
    vm.eval_source("(setq reverse-list (reverse (list (list 11) (list 22) (list 33)))) (setq reverse-vector (reverse (vector (list 11) (list 22) (list 33)))) (setq reverse-string (reverse (string 955 128578 65)))").unwrap();
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(vm.heap_stats().collections > before, "REVERSE_GC_EXECUTED");
    assert!(vm.heap_stats().reclaimed >= 100, "REVERSE_GC_RECLAIMED");
    assert_eq!(
        vm.eval_source("(list reverse-list reverse-vector (append reverse-string nil))")
            .unwrap()
            .display()
            .unwrap(),
        "(((33) (22) (11)) [(33) (22) (11)] (65 128578 955))",
        "REVERSE_GC_CONTENTS"
    );
}
