//! set-time-zone-rule (timefns.c), recursion-depth (keyboard.c) and
//! redisplay's value (dispnew.c) against pinned GNU.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(list (recursion-depth) (redisplay) (redisplay t) (let ((executing-kbd-macro "x")) (redisplay)))
(list (progn (set-time-zone-rule "UTC0") (list (current-time-zone 0) (format-time-string "%H %Z" 0))) (progn (set-time-zone-rule t) (current-time-zone 0)) (progn (set-time-zone-rule 0) (current-time-zone 0)) (progn (set-time-zone-rule 3600) (current-time-zone 0)) (progn (set-time-zone-rule 19800) (current-time-zone 0)) (progn (set-time-zone-rule -7200) (current-time-zone 0)) (progn (set-time-zone-rule 3661) (current-time-zone 0)) (progn (set-time-zone-rule '(3600 "ABC")) (list (current-time-zone 0) (format-time-string "%H:%M %Z" 0))) (progn (set-time-zone-rule "EST5EDT") (list (current-time-zone 0) (current-time-zone 1720000000))) (progn (set-time-zone-rule "Europe/Paris") (current-time-zone 1720000000)) (progn (set-time-zone-rule 'wall) (equal (current-time-zone 0) (current-time-zone 0 'wall))) (progn (set-time-zone-rule nil) (equal (current-time-zone 0) (current-time-zone 0 'wall))) (condition-case e (set-time-zone-rule 1.5) (error e)) (condition-case e (set-time-zone-rule '(a b)) (error e)))
(list (progn (set-time-zone-rule 0) (list (encode-time '(0 0 0 1 1 2024 nil t nil)) (encode-time '(0 0 0 1 1 2024 nil nil nil)) (encode-time '(0 0 0 1 1 2024 nil -1 nil)) (decode-time 0) (format-time-string "%Z %z" 0))) (progn (set-time-zone-rule t) (encode-time '(0 0 12 1 7 2024 nil t nil))) (progn (set-time-zone-rule "UTC0") (list (encode-time '(0 0 0 1 1 2024 nil t nil)) (encode-time '(0 0 0 1 1 2024 nil nil nil)))) (progn (set-time-zone-rule "GMT0") (encode-time '(0 0 0 1 1 2024 nil t nil))))
(list (progn (set-time-zone-rule "Europe/Paris") (list (encode-time '(0 30 2 27 10 2024 nil -1 nil)) (encode-time '(0 30 2 27 10 2024 nil t nil)) (encode-time '(0 30 2 27 10 2024 nil nil nil)))) (progn (set-time-zone-rule 0) (encode-time '(0 0 0 1 1 2024 nil t nil))) (progn (set-time-zone-rule "Europe/Paris") (encode-time '(0 30 2 27 10 2024 nil -1 nil))) (progn (encode-time '(0 30 2 27 10 2024 nil t nil)) (set-time-zone-rule t) (encode-time '(0 0 0 1 7 2024 nil nil nil)) (set-time-zone-rule "Europe/Paris") (encode-time '(0 30 2 27 10 2024 nil -1 nil))))
"###;

#[test]
fn time_zone_rule_and_depth_match_pinned_gnu() {
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
