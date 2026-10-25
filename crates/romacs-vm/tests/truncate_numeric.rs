use romacs_vm::Vm;

#[test]
fn truncate_uses_exact_signed_quotients_like_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut cases = Vec::new();
    for number in ["0", "7", "-7", "1.9", "-1.9", "0.3", "-0.3", "5e-324", "1.0e+INF", "0.0e+NaN"] {
        for divisor in ["", " nil", " 2", " -2", " 0.1", " -0.1", " 0", " 0.0", " 1.0e+INF"] {
            cases.push(format!("(truncate {number}{divisor})"));
        }
    }
    cases.extend(["(truncate 'symbol)", "(truncate 1 'symbol)"].map(str::to_owned));
    let expression = format!("(mapcar (lambda (form) (condition-case error (eval form t) (error (car error)))) '({}))", cases.join(" "));
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(prin1 {expression})")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(vm.eval_source(&expression).unwrap().display().unwrap(), String::from_utf8(output.stdout).unwrap());
}
