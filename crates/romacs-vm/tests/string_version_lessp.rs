use romacs_vm::Vm;

/// Fstring_version_lessp (fns.c:577): gnulib filenvercmp over the names'
/// bytes, symbols by their names, other arguments (wrong-type-argument
/// stringp X).
#[test]
fn string_version_lessp_matches_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(mapcar (lambda (p) (condition-case e (string-version-lessp (car p) (cdr p)) (error e)))
 '(("foo2.png" . "foo12.png") ("foo12.png" . "foo2.png") ("1.10" . "1.9") ("1.9" . "1.10") ("" . "a") ("a" . "") ("" . "")
   ("." . "..") (".." . ".") (".." . ".a") (".a" . "a") ("a" . ".a") (".b" . ".a") ("1.0~rc1" . "1.0") ("1.0" . "1.0~rc1")
   ("a.tar.gz" . "a.tar.bz2") ("a.tar.bz2" . "a.tar.gz") ("file1.txt" . "file01.txt") ("file01.txt" . "file1.txt")
   ("29.1" . "32.0.50") ("32.0.50" . "29.1") (abc . "abd") ("é1" . "é2") ("x-1" . "x_1") ("x_1" . "x-1")
   ("x" . 1) (1 . "x") ("a1b" . "a1b") ("emacs-29.1" . "emacs-29.1.50") ("a~" . "a") ("a" . "a~") ("a.~1~" . "a")
   ("v1.2.3a" . "v1.2.3b") ("0010" . "9") ("9" . "0010") ("x.1" . "x.a") ("x.a" . "x.1")))"#;
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
