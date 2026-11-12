use romacs_vm::Vm;
#[path = "support/template.rs"]
mod template;

#[test]
fn unrelated_handlers_allow_local_operations() {
    let cases: serde_json::Value = serde_json::from_str(include_str!("fixtures/handler-local/cases.json")).unwrap();
    let root = std::env::temp_dir().join(format!("romacs-handler-local-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let directory = serde_json::to_string(&format!("{}/", root.display())).unwrap();
    let file = serde_json::to_string(root.join("input").to_str().unwrap()).unwrap();
    let mut failures = Vec::new();
    for case in cases.as_array().unwrap() {
        std::fs::write(root.join("input"), "hello\n").unwrap();
        let mut vm = Vm::new();
        let home = serde_json::to_string(&std::env::var("HOME").unwrap()).unwrap();
        let source = template::fill(case["source"].as_str().unwrap(), &[("@DIR@", &directory), ("@FILE@", &file), ("@HOME@", &home)]);
        let actual = vm.eval_source(&source).map(|v| v.display().unwrap()).unwrap_or_else(|e| format!("ERROR {e}"));
        let expected = case.get("candidate_expected").unwrap_or(&case["expected"]).as_str().unwrap();
        // The file as GNU leaves it: a handler that signals stops the
        // operation where GNU calls it, before or after the write.
        let file_after = case.get("file").map_or("hello\n", |f| f.as_str().unwrap());
        assert_eq!(std::fs::read_to_string(root.join("input")).unwrap(), file_after, "FILE_AFTER: {}", case["id"]);
        if actual != expected {
            failures.push(format!("{}: {actual}", case["id"]));
        }
    }
    std::fs::remove_dir_all(root).unwrap();
    assert!(failures.is_empty(), "LOCAL_HANDLER_PARITY: {}", failures.join("\n"));
}
