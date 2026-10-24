use romacs_vm::Vm;

#[test]
fn production_registry_excludes_t2_fixture() {
    let mut vm = Vm::new();
    let value = vm.eval_source("(fboundp 'romacs--t2-assoc)").unwrap();
    assert_eq!(value.display().unwrap(), "nil");
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/primitives.json"))).unwrap();
    let rows = manifest.as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        let name = row["name"].as_str().unwrap();
        assert_ne!(name, "romacs--t2-assoc");
        let (file, line) = row["rust"].as_str().unwrap().rsplit_once(':').unwrap();
        let source =
            std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file))
                .unwrap();
        let declaration = source
            .lines()
            .nth(line.parse::<usize>().unwrap() - 1)
            .unwrap();
        assert!(
            declaration.contains("defsubr!") && declaration.contains(name),
            "{name}: {file}:{line}: {declaration}"
        );
    }
}
