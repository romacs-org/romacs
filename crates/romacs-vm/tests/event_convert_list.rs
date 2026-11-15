use romacs_vm::Vm;

/// keyboard.c Fevent_convert_list against the pinned GNU: characters with
/// shift and control folded in as make_ctrl_char does, symbols prefixed in
/// apply_modifiers' order with the click bit printing nothing, and both
/// refusals. mouse.el's `mouse--click-1-maybe-follows-link' calls it on
/// every mouse-1 that `key-translation-map' sees.
#[test]
fn event_convert_list_matches_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = "(mapcar (lambda (desc) (condition-case err (event-convert-list desc) (error (list 'error (car (cdr err))))))
        '((a) (control a) (shift a) (control shift a) (meta control x) (control ?\\s) (control ??) (control ?1)
          (super hyper ?x) (control ?é) (down click mouse-1) (double mouse-1) (control meta shift mouse-3)
          (triple down mouse-2) (drag mouse-1) (up mouse-1) (alt wheel-up) (control f1) (control) (a b)))";
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert!(gnu.contains("down-mouse-1"), "{gnu}");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let ours = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    let same = format!("(equal (prin1-to-string {expression}) {})", romacs_vm_lisp_string(&gnu));
    assert_eq!(vm.eval_source(&same).unwrap().display().unwrap(), "t", "GNU: {gnu}\nRomacs: {ours}");
}

/// GNU's output as a Lisp string literal.
fn romacs_vm_lisp_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        if c == '"' || c == '\\' { out.push('\\'); }
        out.push(c);
    }
    out.push('"');
    out
}
