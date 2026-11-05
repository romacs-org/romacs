//! sysdep.c `get-internal-run-time' against pinned GNU: the processor time
//! used, in `current-time''s form for both values of `current-time-list',
//! to the microsecond, growing while the process computes. The readings
//! themselves are each process's own, so their shape and order are compared.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(let ((current-time-list t)) (let ((a (get-internal-run-time))) (list (length a) (integerp (nth 0 a)) (integerp (nth 1 a)) (nth 3 a) (< -1 (nth 2 a) 1000000))))
(let ((current-time-list nil)) (let ((a (get-internal-run-time))) (list (integerp (car a)) (cdr a) (> (car a) 0))))
(let ((a (float-time (get-internal-run-time))) (i 0)) (while (< i 2000000) (setq i (1+ i))) (let ((b (float-time (get-internal-run-time)))) (list (> b a) (< (- b a) 60))))
"###;

#[test]
fn get_internal_run_time_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        let source = format!("(condition-case e {expression} (error (list 'signal e)))");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = Vm::new().eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
