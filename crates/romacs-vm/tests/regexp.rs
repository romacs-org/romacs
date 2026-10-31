use romacs_vm::{Form, Vm};

fn replay(family: &str) {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/regexp.json")).unwrap();
    let mut checked = 0;
    for row in evidence["cases"].as_array().unwrap() {
        if row["family"].as_str().unwrap() != family {
            continue;
        }
        let mut vm = Vm::new();
        // Only the rows calling this Lisp-level API need the GNU bootstrap.
        // Other rows intentionally exercise bare-VM regexp primitives: subr.el
        // also replaces match-string, which is a different integration surface.
        if row["source"].as_str().unwrap().contains("generate-new-buffer") {
            vm.install_lisp_prelude().unwrap();
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")]).unwrap();
            for file in ["emacs-lisp/debug-early.el", "emacs-lisp/byte-run.el", "emacs-lisp/backquote.el", "subr.el"] {
                vm.load_file(&root.join("gnu-lisp").join(file)).unwrap();
            }
        }
        vm.set_collection_floor(1);
        let form = Form::from_json(row["transport"].clone()).unwrap();
        let f = vm.compile(&form).unwrap();
        let actual = match vm.execute(f) {
            Ok(v) => v.display().unwrap(),
            Err(e) => format!("VM error: {e}"),
        };
        assert_eq!(
            actual,
            row["expected"].as_str().unwrap(),
            "regexp oracle: {}",
            row["source"]
        );
        checked += 1;
    }
    assert!(checked > 0);
}

macro_rules! families {
    ($($family:ident),*) => {$(
        #[test]
        fn $family() { replay(stringify!($family)); }
    )*};
}
families!(
    string, groups, gc, fold, syntax, engine, unicode, inhibit, edge, looking, narrow, forward,
    backward, count, bound, noerror
);

#[test]
fn binary_match_data_uses_integer_registers() {
    let mut vm = Vm::new();
    let actual = match vm.eval_source("(with-temp-buffer (set-buffer-multibyte nil) (insert \"ab\") (goto-char 1) (looking-at \"ab\") (list (match-data) (match-beginning 0) (match-end 0)))") {
        Ok(v) => v.display().unwrap(),
        Err(e) => format!("VM error: {e}"),
    };
    assert_eq!(actual, "((1 3) 1 3)", "binary match-data integers");
}

#[test]
fn cyclic_match_data_reuse_is_bounded() {
    let mut vm = Vm::new();
    let result = vm.eval_source("(progn (string-match \"a\" \"a\") (let ((r (list 0))) (setcdr r r) (condition-case e (match-data nil r) (error e))))").unwrap();
    assert_eq!(
        result.display().unwrap(),
        "(error \"Circular match-data reuse list\")",
        "bounded match-data reuse"
    );
}

#[test]
fn match_string_after_actual_compaction() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX);
    vm.eval_source("(dotimes (i 80) (copy-sequence \"dead\")) (setq subject (copy-sequence \"αabcβ\")) (string-match \"\\\\(ab\\\\)c\" subject)").unwrap();
    let old = format!("{:?}", vm.eval_source("subject").unwrap());
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    let new = format!("{:?}", vm.eval_source("subject").unwrap());
    assert_ne!(old, new, "fixture must move the source string");
    assert!(vm.heap_stats().collections > before, "fixture must collect");
    vm.set_collection_floor(1);
    let actual = match vm.eval_source("(list (match-string 0 subject) (match-string 1 subject) (match-beginning 0) (match-end 0))") {
        Ok(v) => v.display().unwrap(),
        Err(e) => format!("VM error: {e}"),
    };
    assert_eq!(actual, "(\"abc\" \"ab\" 1 4)", "moving-GC match-string");
}

/// Match positions over characters of every internal width (ASCII, two-
/// to four-byte UTF-8, raw bytes), in strings and buffers, against pinned GNU.
#[test]
fn match_positions_over_mixed_widths_match_pinned_gnu() {
    const CASES: &str = r###"
(let ((s (concat "aé😀b" (string (unibyte-char-to-multibyte #xe9)) "λc😀d"))) (list (string-match "\\(😀\\)\\(.\\)" s) (match-data) (string-match "😀\\(.\\)" s 3) (match-data) (string-match "x" s) (string-match "d\\'" s)))
(with-temp-buffer (insert "ab😀λ\n" (string (unibyte-char-to-multibyte #xff)) "😀c\nλd") (goto-char 2) (list (re-search-forward "\\(λ\\)\n\\(.\\)" nil t) (match-data t) (point) (re-search-backward "😀" nil t) (point) (looking-at "😀\\(.\\)") (match-data t) (progn (goto-char (point-max)) (re-search-backward "^\\(.\\)" nil t)) (match-data t) (re-search-forward "zz" nil t)))
(with-temp-buffer (insert "aé😀λb😀\nλ😀c" (string (unibyte-char-to-multibyte #xe9)) "d😀") (goto-char (point-max)) (list (re-search-backward "😀" 8 t) (match-data t) (re-search-backward "😀\\(.\\)" 2 t) (match-data t) (progn (goto-char 12) (re-search-forward "d" (1- (point-max)) t)) (point) (progn (goto-char 13) (re-search-backward "λ" 9 t)) (point)))
(let ((s "aé😀λb😀λc")) (list (string-match "λ" s -3) (match-data) (string-match "😀\\(.\\)" s -4) (match-data) (string-match "a" s -8) (condition-case e (string-match "a" s -9) (error e))))
(with-temp-buffer (insert "é😀éé😀") (narrow-to-region 3 6) (goto-char (point-min)) (list (re-search-forward "é+" nil t) (match-data t) (re-search-forward "😀" nil t)))
"###;
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        let source = format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
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
