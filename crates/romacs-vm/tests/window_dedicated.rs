//! window-dedicated-p and set-window-dedicated-p against pinned GNU
//! (window.c): weak and strong dedication, set-window-buffer's rules for
//! each. (Batch GNU cannot restore a window configuration inside the
//! oracle's condition-case, so configurations are not compared here.)
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(let ((w (selected-window)) (a (get-buffer-create "da")) (b (get-buffer-create "db"))) (set-window-buffer w a) (list (window-dedicated-p) (set-window-dedicated-p nil 'soft) (window-dedicated-p w) (progn (set-window-buffer w b) (window-dedicated-p w)) (set-window-dedicated-p w t) (condition-case e (set-window-buffer w a) (error e)) (eq (window-buffer w) b) (set-window-buffer w b) (window-dedicated-p w) (progn (set-window-dedicated-p w nil) (set-window-buffer w a) (window-dedicated-p w)) (condition-case e (set-window-dedicated-p 1 t) (error e)) (condition-case e (window-dedicated-p 'x) (error e))))
"###;

#[test]
fn window_dedication_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
