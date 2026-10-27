//! rename-buffer, bury-buffer-internal, the buffer list's order and the
//! random suffix of generate-new-buffer-name against pinned GNU (buffer.c).
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(let ((a (get-buffer-create "ra")) (b (get-buffer-create "rb")) (c (get-buffer-create "rc")) (log nil)) (let ((buffer-list-update-hook (list (lambda () (setq log (cons (buffer-name) log)))))) (list (let ((acc nil) (tail (buffer-list))) (while tail (if (member (buffer-name (car tail)) (quote ("ra" "rb" "rc" "rz" "rb<2>"))) (setq acc (cons (buffer-name (car tail)) acc))) (setq tail (cdr tail))) (nreverse acc)) (with-current-buffer a (rename-buffer "rz")) (buffer-name a) (get-buffer "ra") (eq (get-buffer "rz") a) (let ((acc nil) (tail (buffer-list))) (while tail (if (member (buffer-name (car tail)) (quote ("ra" "rb" "rc" "rz" "rb<2>"))) (setq acc (cons (buffer-name (car tail)) acc))) (setq tail (cdr tail))) (nreverse acc)) (with-current-buffer a (rename-buffer "rz")) (with-current-buffer a (condition-case e (rename-buffer "rb") (error e))) (with-current-buffer a (rename-buffer "rb" t)) (let ((acc nil) (tail (buffer-list))) (while tail (if (member (buffer-name (car tail)) (quote ("ra" "rb" "rc" "rz" "rb<2>"))) (setq acc (cons (buffer-name (car tail)) acc))) (setq tail (cdr tail))) (nreverse acc)) (with-current-buffer a (condition-case e (rename-buffer "") (error e))) (with-current-buffer a (condition-case e (rename-buffer 'x) (error e))) (nreverse log) (progn (setq log nil) (bury-buffer-internal b)) (let ((acc nil) (tail (buffer-list))) (while tail (if (member (buffer-name (car tail)) (quote ("ra" "rb" "rc" "rz" "rb<2>"))) (setq acc (cons (buffer-name (car tail)) acc))) (setq tail (cdr tail))) (nreverse acc)) log (condition-case e (bury-buffer-internal "rb") (error e)) (progn (kill-buffer c) (let ((acc nil) (tail (buffer-list))) (while tail (if (member (buffer-name (car tail)) (quote ("ra" "rb" "rc" "rz" "rb<2>"))) (setq acc (cons (buffer-name (car tail)) acc))) (setq tail (cdr tail))) (nreverse acc))) (progn (get-buffer-create "rc") (let ((acc nil) (tail (buffer-list))) (while tail (if (member (buffer-name (car tail)) (quote ("ra" "rb" "rc" "rz" "rb<2>"))) (setq acc (cons (buffer-name (car tail)) acc))) (setq tail (cdr tail))) (nreverse acc))))))
(let ((q (get-buffer-create "rq" t)) (log nil)) (let ((buffer-list-update-hook (list (lambda () (setq log (cons 'hook log)))))) (list (with-current-buffer q (rename-buffer "rq2")) (bury-buffer-internal q) log (buffer-name q))))
(progn (get-buffer-create " rx") (random "abc") (list (generate-new-buffer-name " rx") (generate-new-buffer-name " rx") (with-current-buffer (get-buffer-create " ry") (rename-buffer " rx" t))))
(let ((a (get-buffer-create "sa")) (b (get-buffer-create "sb")) (called nil)) (fset 'rename-auto-save-file (lambda () (setq called (cons (buffer-name) called)))) (with-current-buffer a (setq buffer-auto-save-file-name "/nonexistent-romacs/sa")) (list (local-variable-p 'buffer-auto-save-file-name b) (with-current-buffer b buffer-auto-save-file-name) (with-current-buffer b (rename-buffer "sb2")) called (with-current-buffer a (rename-buffer "sa2")) called))
(let ((a (get-buffer-create "fa")) (b (get-buffer-create "fb"))) (with-current-buffer a (setq buffer-file-name "/nonexistent-romacs/same")) (with-current-buffer b (setq buffer-file-name "/nonexistent-romacs/same")) (list (buffer-name (get-file-buffer "/nonexistent-romacs/same")) (progn (with-current-buffer a (rename-buffer "fz")) (buffer-name (get-file-buffer "/nonexistent-romacs/same"))) (progn (bury-buffer-internal a) (buffer-name (get-file-buffer "/nonexistent-romacs/same")))))
(progn (with-current-buffer (get-buffer-create "*scratch*") (rename-buffer "scr2")) (list (generate-new-buffer-name "*scratch*") (get-buffer "*scratch*") (buffer-live-p (get-buffer "scr2"))))
"###;

#[test]
fn buffer_names_match_pinned_gnu() {
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
