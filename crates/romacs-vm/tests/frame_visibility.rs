//! Frame visibility (frame.c) and FRAME-OR-WINDOW arguments (window.c)
//! against pinned GNU: GNU's batch terminal frame, like VM2's one frame,
//! is always visible.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(list (frame-visible-p (selected-frame)) (eq (make-frame-visible) (selected-frame)) (eq (make-frame-visible (selected-frame)) (selected-frame)) (raise-frame) (lower-frame) (raise-frame (selected-frame)) (iconify-frame) (frame-visible-p (selected-frame)) (make-frame-invisible nil t) (frame-visible-p (selected-frame)) (equal (visible-frame-list) (list (selected-frame))))
(list (condition-case e (frame-visible-p nil) (error e)) (condition-case e (frame-visible-p 'x) (error e)) (condition-case e (make-frame-visible 3) (error e)) (condition-case e (make-frame-invisible) (error e)) (condition-case e (make-frame-invisible 3) (error e)) (condition-case e (iconify-frame 3) (error e)) (condition-case e (raise-frame 3) (error e)) (condition-case e (lower-frame 'x) (error e)))
(list (eq (frame-root-window (selected-window)) (frame-root-window)) (eq (frame-selected-window (selected-window)) (selected-window)) (eq (frame-root-window (minibuffer-window)) (frame-root-window)) (eq (frame-selected-window (minibuffer-window)) (selected-window)) (eq (frame-root-window (selected-frame)) (frame-root-window)))
"###;

#[test]
fn frame_visibility_matches_pinned_gnu() {
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
