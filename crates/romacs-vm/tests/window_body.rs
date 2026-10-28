//! window-body-width and window-body-height against pinned GNU (window.c),
//! as relations to the window's total size: VM2's batch frame keeps its
//! pixel backend geometry (DIVERGENCES.md VM2-WIN5), not GNU's 80x25
//! terminal, so absolute sizes differ.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(list (- (window-total-width) (window-body-width)) (progn (set-window-margins nil 2 3) (prog1 (- (window-total-width) (window-body-width)) (set-window-margins nil nil nil))) (= (window-body-width nil t) (* (window-body-width) (frame-char-width))) (= (window-body-width nil 'remap) (window-body-width)) (- (window-pixel-height) (window-body-height nil t) (window-mode-line-height)) (= (window-body-height nil t) (* (window-body-height) (frame-char-height))) (window-body-height (minibuffer-window)) (- (window-total-width (minibuffer-window)) (window-body-width (minibuffer-window))) (condition-case e (window-body-width 'x) (error e)) (condition-case e (window-body-height 1) (error e)))
"###;

#[test]
fn window_body_sizes_match_pinned_gnu() {
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
