use romacs_vm::Vm;

#[test]
fn concat_preserves_properties_at_ascii_and_unicode_offsets() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    for source in [
        "(let* ((s (propertize \"ab\" 'face 'keyword)) (r (concat [4194303] s \"z\"))) (list (get-text-property 0 'face r) (get-text-property 1 'face r) (get-text-property 2 'face r) (get-text-property 3 'face r) (get-text-property 0 'face s)))",
        "(let* ((s (propertize \"ab\" 'face 'keyword)) (r (concat '(4194303) s \"z\"))) (list (get-text-property 0 'face r) (get-text-property 1 'face r) (get-text-property 2 'face r) (get-text-property 3 'face r) (get-text-property 0 'face s)))",
        "(let* ((s (propertize \"ab\" 'face 'keyword)) (r (concat \"x\" s \"z\"))) (list (get-text-property 0 'face r) (get-text-property 1 'face r) (get-text-property 2 'face r) (get-text-property 3 'face r) (get-text-property 0 'face s)))",
        "(let* ((s (propertize \"λ雪\" 'face 'keyword)) (r (concat \"😀\" s \"z\"))) (list (get-text-property 0 'face r) (get-text-property 1 'face r) (get-text-property 2 'face r) (get-text-property 3 'face r) (get-text-property 0 'face s)))",
        "(let* ((s (propertize \"ab\" 'face 'keyword)) (r (concat '(120) nil s [122]))) (list (get-text-property 0 'face r) (get-text-property 1 'face r) (get-text-property 2 'face r) (get-text-property 3 'face r) (get-text-property 0 'face s)))",
    ] {
        let result = vm.eval_source(source).unwrap().display().unwrap();
        assert_eq!(result, "(nil keyword keyword nil keyword)", "CONCAT_RETAINS_PROPERTY_RANGES: {source}");
    }
    for prefix in ["[4194303]", "'(4194303)"] {
        vm.eval_source(&format!("(setq raw-source (propertize \"ab\" 'payload (list 42)) raw-result (concat {prefix} raw-source)) (garbage-collect)")).unwrap();
        assert_eq!(vm.eval_source("(list (get-text-property 0 'payload raw-result) (get-text-property 1 'payload raw-result) (get-text-property 2 'payload raw-result) (get-text-property 0 'payload raw-source))").unwrap().display().unwrap(), "(nil (42) (42) (42))", "CONCAT_RAW_PREFIX_PROPERTY_GC: {prefix}");
    }
    vm.eval_source("(setq kept (concat \"x\" (propertize \"λ\" 'payload (list 42)) \"z\")) (garbage-collect)").unwrap();
    assert_eq!(vm.eval_source("(get-text-property 1 'payload kept)").unwrap().display().unwrap(), "(42)", "CONCAT_PROPERTY_VALUE_IS_ROOTED");
}
