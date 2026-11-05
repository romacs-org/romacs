use romacs_vm::Vm;

/// `keymapp' is get_keymap without AUTOLOAD (keymap.c): a symbol whose
/// function, through aliases, is a keymap's autoload `(autoload FILE DOC
/// INTERACTIVE keymap)' is a keymap before its file loads, as kmacro.el's
/// `kmacro-keymap' is on C-x C-k; another autoload, or the autoload form
/// itself, is not. describe-map hides such prefix keys from C-h b by it.
#[test]
fn an_autoloaded_keymap_symbol_is_a_keymap_as_in_pinned_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(progn
 (fset 'kmapp-k '(autoload "x" nil t keymap)) (fset 'kmapp-f '(autoload "x" nil t))
 (defalias 'kmapp-a 'kmapp-k) (fset 'kmapp-short '(autoload "x"))
 (list (keymapp 'kmapp-k) (keymapp 'kmapp-f) (keymapp 'kmapp-a) (keymapp '(autoload "x" nil t keymap))
       (keymapp 'kmapp-short) (keymapp nil) (keymapp 'kmapp-unbound) (keymapp (make-sparse-keymap))))"#;
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
