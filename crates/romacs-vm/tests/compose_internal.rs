//! composite.c:2009-2046 compose-region-internal and
//! compose-string-internal: compose_text's `composition' property
//! ((LENGTH . COMPONENTS) . MODIFICATION-FUNC). Pinned GNU 32.0.50 answers.
use romacs_vm::Vm;

#[test]
fn compose_internals_put_gnus_composition_property() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for (form, expected) in [
        ("(compose-string-internal (copy-sequence \"abc\") 0 2 \"x\" 'f)", "#(\"abc\" 0 2 (composition ((2 . \"x\") . f)))"),
        ("(compose-string-internal (copy-sequence \"abc\") -2 nil)", "#(\"abc\" 1 3 (composition ((2))))"),
        ("(condition-case e (compose-string-internal \"abc\" 2 1) (error e))", "(args-out-of-range \"abc\" 2 1)"),
        ("(progn (set-buffer (get-buffer-create \"ci\")) (insert \"abcd\") (list (compose-region-internal 2 4 [?a ?b]) (get-text-property 2 'composition)))", "(nil ((2 . [97 98])))"),
        ("(condition-case e (compose-region-internal 1 2 'sym) (error e))", "(wrong-type-argument vectorp sym)"),
    ] {
        let got = vm.eval_source(&format!("(prin1-to-string {form})")).and_then(|v| v.display()).map(|s| serde_json::from_str::<String>(&s).unwrap_or(s));
        assert_eq!(got.as_deref(), Ok(expected), "{form}");
    }
}
