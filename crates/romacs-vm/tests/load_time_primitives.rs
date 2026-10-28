//! Display, terminal, face and file-name primitives the GNU tree reaches at
//! load time: force-window-update (window.c:4492), redraw-display
//! (dispnew.c:3252), the terminal parameter alist (terminal.c:536-585),
//! tty-display-color-p/-cells (term.c:2284-2311; VM2's terminals are not
//! text terminals), file-name-concat (fileio.c:846) and
//! internal-copy-lisp-face (xfaces.c:3145). Expected answers are pinned GNU
//! 32.0.50's (`emacs -Q --batch`), in one VM as GNU evaluated them.
use romacs_vm::Vm;

#[test]
fn load_time_display_terminal_face_and_file_name_primitives_match_gnu() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for (form, expected) in [
        (r#"(force-window-update)"#, r#"t"#),
        (r#"(force-window-update (selected-window))"#, r#"t"#),
        (r#"(force-window-update "no such buffer")"#, r#"nil"#),
        (r#"(force-window-update 5)"#, r#"nil"#),
        (r#"(force-window-update (window-buffer (selected-window)))"#, r#"t"#),
        (r#"(force-window-update (get-buffer-create " fwu-hidden"))"#, r#"nil"#),
        (r#"(redraw-display)"#, r#"nil"#),
        (r#"(commandp 'redraw-display)"#, r#"t"#),
        (r#"(terminal-parameter nil 'tp-x)"#, r#"nil"#),
        (r#"(set-terminal-parameter nil 'tp-x 1)"#, r#"nil"#),
        (r#"(set-terminal-parameter nil 'tp-x 2)"#, r#"1"#),
        (r#"(terminal-parameter nil 'tp-x)"#, r#"2"#),
        (r#"(assq 'tp-x (terminal-parameters))"#, r#"(tp-x . 2)"#),
        (r#"(let ((p (terminal-parameters))) (setcdr (assq 'tp-x p) 9) (terminal-parameter nil 'tp-x))"#, r#"2"#),
        (r#"(condition-case e (terminal-parameter nil 5) (error e))"#, r#"(wrong-type-argument symbolp 5)"#),
        (r#"(condition-case e (terminal-parameter 5 'tp-x) (error e))"#, r#"(wrong-type-argument terminal-live-p 5)"#),
        (r#"(tty-display-color-p)"#, r#"nil"#),
        (r#"(tty-display-color-cells)"#, r#"0"#),
        (r#"(condition-case e (tty-display-color-p 5) (error e))"#, r#"(wrong-type-argument terminal-live-p 5)"#),
        (r#"(file-name-concat "foo" "bar")"#, r#""foo/bar""#),
        (r#"(file-name-concat "foo/" "bar")"#, r#""foo/bar""#),
        (r#"(file-name-concat "foo" nil "" "bar/")"#, r#""foo/bar/""#),
        (r#"(file-name-concat "" "a")"#, r#""a""#),
        (r#"(file-name-concat "a")"#, r#""a""#),
        (r#"(multibyte-string-p (file-name-concat "a" "λ"))"#, r#"t"#),
        (r#"(condition-case e (file-name-concat "a" 5) (error e))"#, r#"(wrong-type-argument stringp 5)"#),
        (r#"(progn (internal-make-lisp-face 'cf-1) (internal-set-lisp-face-attribute 'cf-1 :foreground "red" t) (internal-copy-lisp-face 'cf-1 'cf-2 t nil))"#, r#"cf-2"#),
        (r#"(internal-get-lisp-face-attribute 'cf-2 :foreground t)"#, r#""red""#),
        (r#"(condition-case e (internal-copy-lisp-face 'cf-nope 'cf-3 t nil) (error e))"#, r#"(error "Invalid face" cf-nope)"#),
        (r#"(condition-case e (internal-copy-lisp-face 'cf-1 'cf-3 nil nil) (error e))"#, r#"(wrong-type-argument frame-live-p nil)"#),
    ] {
        let source = format!("(prin1-to-string {form})");
        let got = vm.eval_source(&source).and_then(|v| v.display()).map(|s| serde_json::from_str::<String>(&s).unwrap_or(s));
        assert_eq!(got.as_deref(), Ok(expected), "{form}");
    }
}

/// syntax.c:984 syntax-table-p: a char-table whose purpose is syntax-table.
#[test]
fn syntax_table_p_matches_gnu() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let source = "(list (syntax-table-p (standard-syntax-table)) (syntax-table-p (make-char-table 'syntax-table)) (syntax-table-p (make-char-table 'foo)) (syntax-table-p 5))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(t t nil nil)");
}

/// data.c:2500 variable-binding-locus: the current buffer for a local
/// binding (through an alias, and for a per-buffer C variable once set),
/// the terminal for a KBOARD variable, else nil.
#[test]
fn variable_binding_locus_matches_gnu() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let source = "(progn (defvar vbl-x 1) (defvaralias 'vbl-y 'vbl-x) (set-buffer (get-buffer-create \"vbl\"))
      (list (variable-binding-locus 'vbl-x) (progn (set (make-local-variable 'vbl-x) 2) (bufferp (variable-binding-locus 'vbl-x)))
            (bufferp (variable-binding-locus 'vbl-y)) (variable-binding-locus 'fill-column)
            (bufferp (progn (setq fill-column 3) (variable-binding-locus 'fill-column)))
            (terminal-live-p (variable-binding-locus 'last-command)) (condition-case e (variable-binding-locus 5) (error e))))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(nil t t nil t t (wrong-type-argument symbolp 5))");
}
