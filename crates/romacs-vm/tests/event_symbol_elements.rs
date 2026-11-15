use romacs_vm::Vm;

/// keyboard.c parse_modifiers caches an event symbol's elements, which
/// subr.el's `event-basic-type' and `event-modifiers' read, whenever a key
/// lookup or definition takes the symbol (keymap.c reorder_modifiers):
/// before that a fresh symbol has none, after it GNU's (BASE MODIFIERS...),
/// with or without modifiers. Checked against the pinned GNU.
#[test]
fn key_lookup_caches_event_symbol_elements_as_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = "(let ((map (make-sparse-keymap)) (out nil))
        (push (get 'down-mouse-9 'event-symbol-elements) out)
        (lookup-key map [down-mouse-9])
        (push (get 'down-mouse-9 'event-symbol-elements) out)
        (push (get 'C-M-S-down-mouse-8 'event-symbol-elements) out)
        (define-key map [C-M-S-down-mouse-8] 'ignore)
        (push (get 'C-M-S-down-mouse-8 'event-symbol-elements) out)
        (lookup-key map [mouse-7])
        (push (get 'mouse-7 'event-symbol-elements) out)
        (lookup-key map [f13])
        (push (get 'f13 'event-symbol-elements) out)
        (push (get 'down-mouse-9 'event-symbol-element-mask) out)
        (push (eq (car (internal-event-symbol-parse-modifiers 'C-nil)) nil) out)
        (push (eq (car (internal-event-symbol-parse-modifiers 'M-t)) t) out)
        (nreverse out))";
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert!(gnu.starts_with("(nil (mouse-9 down)"), "{gnu}");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let ours = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(ours, format!("\"{gnu}\""), "GNU: {gnu}");
}
