//! An advised command stays a command: `interactive-form' and `commandp' ask
//! `oclosure-interactive-form' for an OClosure such as nadvice's advice
//! (data.c:1186, eval.c:2301), as pinned GNU 32.0.50 answers on its loadup
//! chain.  org-fold advises org-self-insert-command, every Org buffer's
//! self-insertion.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
use gnu_lisp::gnu_lisp;

const CASE: &str = "(progn
  (defun adv-cmd (n) (interactive \"p\") n)
  (defun adv-plain (n) n)
  (advice-add 'adv-cmd :before #'ignore)
  (advice-add 'adv-plain :before #'ignore)
  (list (commandp 'adv-cmd) (interactive-form 'adv-cmd)
        (commandp 'adv-plain) (interactive-form 'adv-plain)))";

#[test]
fn advised_commands_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {CASE}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = gnu_lisp(&[]);
        vm.eval_source(&format!("(prin1-to-string {CASE})")).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
    }).unwrap().join().unwrap();
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
}
