//! `current-idle-time' (keyboard.c:4949) is nil while Emacs is not idle,
//! which is every moment Lisp runs in batch; org's element cache reads it
//! from `after-change-functions' to schedule its idle sync.
use romacs_vm::Vm;

#[test]
fn current_idle_time_is_nil_while_lisp_runs_as_in_gnu_batch() {
    let source = "(list (current-idle-time) (subr-arity (symbol-function 'current-idle-time)))";
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let actual = Vm::new().eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
}
