use romacs_vm::Vm;

/// Fdefine_key (keymap.c): a definition vector whose first element is a
/// list is an XEmacs-style keyboard macro, stored with each modifier list
/// converted by event-convert-list.  bindings.el binds the keypad
/// translations of `function-key-map' this way, [(48)] for <kp-0>, which C-h
/// b names "0 DIGIT ZERO".  A vector starting with an event, a header-line
/// list and a character range are stored as given.
#[test]
fn a_keyboard_macro_definition_converts_its_modifier_lists_as_pinned_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(let ((m (make-sparse-keymap)))
 (define-key m [kp-0] (vector (list ?0)))
 (define-key m [C-kp-0] (vector (list 'control ?0)))
 (define-key m [f5] [(control kp-0) 1 (meta shift a)])
 (define-key m [f6] [48 (control ?a)])
 (define-key m [f7] [(header-line mouse-1)])
 (define-key m [f8] [(97 . 98)])
 (define-key m [f9] "ab")
 (mapcar (lambda (k) (lookup-key m (vector k))) '(kp-0 C-kp-0 f5 f6 f7 f8 f9)))"#;
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mine = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
}
