use romacs_vm::Vm;

#[test]
fn logarithms_match_gnu_numeric_coercion_and_base_rules() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut cases = Vec::new();
    for number in ["1", "2", "3", "10", "0", "-1", "0.5", "-0.0", "1e300", "1.0e+INF", "0.0e+NaN", "#x123456789abcdef0123456789"] {
        for base in ["", " nil", " 2", " 10", " 3", " 0", " 1", " -2", " 1.0e+INF"] {
            cases.push(format!("(log {number}{base})"));
        }
    }
    cases.extend(["(log 'symbol)", "(log 1 'symbol)"].map(str::to_owned));
    let expression = format!("(mapcar (lambda (form) (condition-case error (let ((value (eval form t))) (if (= value value) value 'nan)) (error (car error)))) '({}))", cases.join(" "));
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(prin1 {expression})")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(vm.eval_source(&expression).unwrap().display().unwrap(), String::from_utf8(output.stdout).unwrap());
}
