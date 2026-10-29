//! `current-bidi-paragraph-direction' (xdisp.c, bidi.c bidi_paragraph_init)
//! against pinned GNU in batch: one buffer per case, TEXT inserted, point
//! at each position in turn.
use romacs_vm::Vm;

/// (TEXT SETUP): SETUP runs in the buffer after the insertion.
const CASES: &[(&str, &str)] = &[
    (r#""""#, ""),
    (r#""plain text\nmore""#, ""),
    (r#""שלום עולם\nabc\n""#, ""),
    (r#""abc\n\nשלום\n\n\n123\n""#, ""),
    (r#""123 !?\n\nمرحبا\n""#, ""),
    (r#""  \t\n\fשלום\n\f\nabc""#, ""),
    (r#""\u2067abc\u2069 שלום\n\u2066שלום\nx""#, ""),
    (r#""(*) 42 ١٢٣ abc""#, ""),
    (r#""abc\nשלום""#, "(setq bidi-paragraph-direction 'right-to-left)"),
    (r#""שלום""#, "(setq bidi-display-reordering nil)"),
    (r#""abc\n--\nשלום\n--\n""#, "(setq bidi-paragraph-start-re \"^--$\" bidi-paragraph-separate-re \"^--$\")"),
    (r#""abc\nשלום\nxyz""#, "(narrow-to-region 5 10)"),
    (r#""שלום""#, "(set-buffer-multibyte nil)"),
];

fn expression(text: &str, setup: &str) -> String {
    format!("(progn (set-buffer (get-buffer-create \"bidi\")) (kill-all-local-variables) (widen) (erase-buffer) (insert {text}) {setup}
       (let ((answers nil) (p (point-min))) (while (<= p (point-max)) (goto-char p) (setq answers (cons (current-bidi-paragraph-direction) answers)) (setq p (1+ p)))
         (list (nreverse answers) (current-bidi-paragraph-direction (current-buffer)) (condition-case e (current-bidi-paragraph-direction 'x) (error e)))))")
}

#[test]
fn paragraph_direction_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mut failures = Vec::new();
    for (text, setup) in CASES {
        let source = expression(text, setup);
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{text} {setup}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
