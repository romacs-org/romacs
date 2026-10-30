//! `mode-line-format' is buffer.c's DEFVAR_PER_BUFFER, local in every
//! buffer once set, unibyte ones included, against pinned GNU in batch:
//! url-http.el sets its connection buffers' own, which must not reach the
//! default every other buffer shows.
use romacs_vm::Vm;

const CASES: &[&str] = &[
    "(local-variable-if-set-p 'mode-line-format)",
    "(let ((d (default-value 'mode-line-format))) (with-temp-buffer (set-buffer-multibyte nil) (setq mode-line-format \"%b [%s]\") (list (local-variable-p 'mode-line-format) mode-line-format (eq (default-value 'mode-line-format) d))))",
    "(let ((d (default-value 'mode-line-format))) (with-temp-buffer (setq mode-line-format '(\"x\")) (list (local-variable-p 'mode-line-format) mode-line-format (eq (default-value 'mode-line-format) d))))",
    "(with-temp-buffer (setq mode-line-format \"x\") (kill-local-variable 'mode-line-format) (list (local-variable-p 'mode-line-format) (eq mode-line-format (default-value 'mode-line-format))))",
    "(let ((d (default-value 'mode-line-format)) (b (get-buffer-create \" ml\"))) (with-current-buffer b (setq mode-line-format \"local\")) (prog1 (list (buffer-local-value 'mode-line-format b) (eq mode-line-format d) (assq 'mode-line-format (buffer-local-variables b))) (kill-buffer b)))",
    "(let ((d (default-value 'mode-line-format))) (with-temp-buffer (set-default (quote mode-line-format) \"new\") (prog1 (list mode-line-format (with-temp-buffer mode-line-format)) (set-default (quote mode-line-format) d))))",
];

#[test]
fn mode_line_format_is_per_buffer_as_in_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES {
        let source = format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
