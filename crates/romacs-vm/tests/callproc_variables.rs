use romacs_vm::Vm;

/// callproc.c's program-name DEFVARs and emacs.c's path-separator are
/// specials holding GNU's build_string constants (syms_of_callproc,
/// syms_of_emacs); exec-directory is a directory name, VM2's own
/// invocation directory, and configure-info-directory the runtime's info
/// (VM2-EXECDIR1).
#[test]
fn callproc_program_names_match_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let names = ["ctags-program-name", "etags-program-name", "hexl-program-name", "emacsclient-program-name",
                 "movemail-program-name", "ebrowse-program-name", "rcs2log-program-name", "path-separator"];
    let expression = format!(
        "(mapcar (lambda (s) (list s (symbol-value s) (special-variable-p s) (multibyte-string-p (symbol-value s)))) '({}))",
        names.join(" "));
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert!(gnu.contains("(etags-program-name \"etags\" t nil)"), "{gnu}");
    let same = format!("(equal (prin1-to-string {expression}) {gnu:?})");
    assert_eq!(vm.eval_source(&same).unwrap().display().unwrap(), "t", "GNU: {gnu}");
    let directory = "(list (special-variable-p 'exec-directory) (equal exec-directory invocation-directory) (directory-name-p exec-directory) (file-name-absolute-p exec-directory) (special-variable-p 'configure-info-directory) (equal (expand-file-name configure-info-directory) (expand-file-name \"../info\" data-directory)))";
    assert_eq!(vm.eval_source(directory).unwrap().display().unwrap(), "(t t t t t t)");
}
