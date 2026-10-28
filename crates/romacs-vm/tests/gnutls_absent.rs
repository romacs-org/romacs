use romacs_vm::Vm;

/// gnutls.c in a build without GnuTLS (VM2-TLS1): `gnutls-available-p' is
/// still a zero-argument primitive, answering nil, and `libgnutls-version'
/// is -1. The pinned GNU has GnuTLS, so it supplies the shape only.
#[test]
fn gnutls_queries_have_gnus_shape_and_report_no_tls() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let shape = "(list (func-arity 'gnutls-available-p) (subrp (symbol-function 'gnutls-available-p)) (special-variable-p 'libgnutls-version) (integerp libgnutls-version) (listp (gnutls-available-p)))";
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {shape}))")])
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(vm.eval_source(&format!("(prin1-to-string {shape})")).unwrap().display().unwrap(), format!("{gnu:?}"));
    assert_eq!(vm.eval_source("(list (gnutls-available-p) libgnutls-version)").unwrap().display().unwrap(), "(nil -1)");
}
