//! `decode-coding-region' and `encode-coding-region' (coding.c
//! `code_convert_region`) against the pinned GNU, and how much Lisp one runs.
use romacs_vm::Vm;

const SETUP: &str = r#"
(define-coding-system-internal 'coding-region-utf8 85 'utf-8 nil t nil nil nil nil 0 nil nil 'unix nil)
(define-coding-system-internal 'coding-region-raw 82 'raw-text nil t nil nil nil nil 0 t nil 'unix)
(defun coding-region-probe (text from to body)
  "BODY's value in a buffer of TEXT, with what the change did around FROM..TO."
  (let ((buffer (get-buffer-create "coding-region")) (log nil))
    (unwind-protect
        (save-current-buffer
          (set-buffer buffer)
          (setq buffer-undo-list t last-coding-system-used 'unset)
          (insert text)
          (let ((markers (list (copy-marker from) (copy-marker from t) (copy-marker (1+ from))
                               (copy-marker to) (copy-marker to t) (copy-marker (point-max)))))
            (set (make-local-variable 'before-change-functions)
                 (list (lambda (b e) (setq log (cons (list 'before b e) log)))))
            (set (make-local-variable 'after-change-functions)
                 (list (lambda (b e l) (setq log (cons (list 'after b e l) log)))))
            (let ((value (condition-case e (funcall body) (error (list 'signal (car e))))))
              (let ((places nil))
                (while markers
                  (setq places (cons (marker-position (car markers)) places) markers (cdr markers)))
                (list value (vconcat (buffer-string)) (point) (nreverse places)
                      (nreverse log) deactivate-mark last-coding-system-used)))))
      (kill-buffer buffer))))
"#;

const RAW: &str = "(string-to-multibyte (unibyte-string 97 98 195 169 99 100 206 187 101 102))";

fn expressions() -> Vec<String> {
    let mut cases = Vec::new();
    for point in [1, 3, 5, 9, 11] {
        cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (goto-char {point}) (decode-coding-region 3 9 'coding-region-utf8)))"));
    }
    for (from, to) in [("9", "3"), ("(copy-marker 3)", "(copy-marker 9)")] {
        cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (decode-coding-region {from} {to} 'coding-region-utf8)))"));
    }
    // The whole buffer. (A marker strictly inside the region that advances
    // on insertion goes to its start here and after the new text in GNU;
    // the probe's markers at its ends and one that stays put agree.)
    cases.push(format!("(coding-region-probe {RAW} 1 11 (lambda () (decode-coding-region 1 (point-max) 'coding-region-utf8)))"));
    // Text properties of the old text do not survive.
    cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (put-text-property 1 11 'face 'bold) (decode-coding-region 3 9 nil) (list (get-text-property 2 'face) (get-text-property 4 'face))))"));
    for coding in ["nil", "'no-conversion", "'coding-region-raw", "'coding-region-none"] {
        cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (decode-coding-region 3 9 {coding})))"));
        cases.push(format!(r#"(coding-region-probe "abλéef" 3 5 (lambda () (encode-coding-region 3 5 {coding})))"#));
    }
    cases.push(r#"(coding-region-probe "abλéef" 3 5 (lambda () (goto-char 4) (encode-coding-region 3 5 'coding-region-utf8)))"#.into());
    // The text returned, and the text put into another buffer at its point.
    cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (let ((s (decode-coding-region 3 9 'coding-region-utf8 t))) (list (multibyte-string-p s) (vconcat s)))))"));
    cases.push(r#"(coding-region-probe "abλéef" 3 5 (lambda () (let ((s (encode-coding-region 3 5 'coding-region-utf8 t))) (list (multibyte-string-p s) (vconcat s)))))"#.into());
    cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (let ((other (get-buffer-create \"coding-other\"))) (save-current-buffer (set-buffer other) (insert \"<>\") (goto-char 2)) (prog1 (list (decode-coding-region 3 9 'coding-region-utf8 other) (save-current-buffer (set-buffer other) (list (buffer-string) (point)))) (kill-buffer other)))))"));
    cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (decode-coding-region 3 9 'coding-region-utf8 (current-buffer))))"));
    // Refusals leave the buffer as it was. (GNU leaves point at the region's
    // start when the text is read-only; the probe moves it on both.)
    cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (decode-coding-region 3 9 'coding-region-utf8 'other)))"));
    cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (decode-coding-region 0 99 'coding-region-utf8)))"));
    cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (let ((buffer-read-only t)) (prog1 (condition-case e (decode-coding-region 3 9 'coding-region-utf8) (error (car e))) (goto-char 1)))))"));
    // The text returned carries no properties, from a unibyte buffer either.
    for multibyte in ["t", "nil"] {
        cases.push(format!("(with-temp-buffer (set-buffer-multibyte {multibyte}) (insert \"abc\") (put-text-property 1 4 'face 'bold) (text-properties-at 0 (decode-coding-region 1 4 nil t)))"));
    }
    // An empty region converts to nothing.
    cases.push("(with-temp-buffer (insert \"abc\") (list (decode-coding-region 2 2 'coding-region-utf8) (encode-coding-region 4 4 nil) (buffer-string)))".into());
    cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (narrow-to-region 2 10) (goto-char 10) (prog1 (decode-coding-region 3 9 'coding-region-utf8) (widen))))"));
    // A mark the change deactivates is deactivated by its hooks, as any change's.
    cases.push(format!("(coding-region-probe {RAW} 3 9 (lambda () (setq deactivate-mark nil) (decode-coding-region 3 9 'coding-region-utf8)))"));
    cases
}

#[test]
fn region_coding_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in expressions() {
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(progn (setq lexical-binding t) {SETUP} (prin1 (eval '{expression} t)))")]).output().unwrap();
        assert!(output.status.success(), "native coding probe failed: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.eval_source(SETUP).unwrap();
        let actual = match vm.eval_source(&expression) { Ok(value) => value.display(), Err(error) => Err(error) };
        if actual.as_deref() != Ok(expected.as_str()) { failures.push(format!("{expression}\nGNU {expected}\nVM {actual:?}")); }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The calls a form enters when run COUNT times, less the loop's own.
fn calls(vm: &mut Vm, form: &str, count: usize) -> usize {
    let mut run = |body: &str| {
        let before = vm.calls_entered();
        vm.eval_source(&format!("(let ((n {count})) (while (> n 0) {body} (setq n (1- n))))")).unwrap();
        vm.calls_entered() - before
    };
    let empty = run("nil");
    run(form) - empty
}

/// dired decodes every listed line in place. With no hook to run that is
/// two calls, the primitive and the replacement it makes, and no Lisp
/// function: the Lisp version made a change group, an insertion and a
/// deletion of each.
#[test]
fn a_region_conversion_runs_no_lisp() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(SETUP).unwrap();
    vm.eval_source(&format!("(progn (set-buffer (get-buffer-create \"coding-calls\")) (setq buffer-undo-list t) (insert {RAW}))")).unwrap();
    for form in ["(decode-coding-region 3 5 'coding-region-utf8)", "(encode-coding-region 3 4 'coding-region-utf8)",
                 "(decode-coding-region 1 2 nil)"] {
        assert_eq!(calls(&mut vm, form, 100), 200, "{form}");
    }
    assert_eq!(calls(&mut vm, "(decode-coding-region 1 2 'coding-region-utf8 t)", 100), 100);
}
