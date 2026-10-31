//! `format-mode-line' (xdisp.c Fformat_mode_line) against pinned GNU.
//! Batch formats nothing, so GNU batch checks the argument checks and the
//! empty answer; the formatting itself is compared with GNU's answers on a
//! terminal frame, recorded by `fixtures/format-mode-line-tty.py` into
//! `fixtures/format-mode-line-tty.txt`.
use romacs_vm::Vm;
use std::path::Path;

/// One expression per line.
const BATCH: &str = r###"
(list noninteractive (format-mode-line "%b") (multibyte-string-p (format-mode-line "é")) (format-mode-line nil) (format-mode-line '("" mode-name) 'bold))
(condition-case e (format-mode-line "x" nil nil 'foo) (error e))
(condition-case e (format-mode-line "x" nil 'foo) (error e))
(condition-case e (let ((b (generate-new-buffer "d"))) (kill-buffer b) (format-mode-line "x" nil nil b)) (error e))
(condition-case e (progn (split-window) (format-mode-line "x" nil (frame-root-window))) (error (list (car e) (cadr e) (windowp (nth 2 e)))))
(condition-case e (funcall 'format-mode-line) (error e))
(condition-case e (apply 'format-mode-line '(1 2 3 4 5)) (error e))
(condition-case e (let ((w (split-window))) (delete-window w) (format-mode-line "x" nil w)) (error e))
(condition-case e (let ((w (split-window))) (delete-window w) (format-mode-line "x" nil w (current-buffer))) (error e))
"###;

/// A VM on GNU's loadup chain, in batch as the gnu-ert runner sets it.
fn romacs() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    let lisp = root.join("gnu-lisp");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('=')
            && !name.ends_with('~') && !name.ends_with(".orig") && !name.ends_with(".rej"))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    if names.iter().any(|n| n == "obsolete") { paths.push(lisp.join("obsolete")); }
    paths.push(root.join("rlisp"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

/// EXPRESSION's value as `prin1' prints it, an error as (signal ERR).
fn printed(vm: &mut Vm, expression: &str) -> Result<String, String> {
    let value = vm.eval_source(&format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))"))?.display()?;
    serde_json::from_str(&value).map_err(|e| format!("{e}: {value}"))
}

#[test]
fn batch_checks_its_arguments_and_formats_nothing_as_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = romacs();
    let mut failures = Vec::new();
    for expression in BATCH.lines().filter(|line| !line.is_empty()) {
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {expression})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = printed(&mut vm, expression);
        if actual.as_deref() != Ok(&*expected) {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The fixture's SETUP line, then pairs of an expression and GNU's answer
/// on a terminal frame. Romacs evaluates them interactively, as the GUI
/// session does (`noninteractive' nil). Answers are compared by GNU's
/// `equal-including-properties', not by print: GNU's concatenation keeps
/// equal neighbouring intervals apart, VM2's string intervals merge them.
#[test]
fn formats_as_pinned_gnu_on_a_terminal_frame() {
    let fixture = include_str!("fixtures/format-mode-line-tty.txt");
    let mut lines = fixture.lines().filter(|line| !line.starts_with(";;"));
    let setup = lines.next().unwrap();
    let mut vm = romacs();
    vm.eval_source(&format!("(setq noninteractive nil) {setup}")).unwrap();
    let mut failures = Vec::new();
    while let (Some(expression), Some(expected)) = (lines.next(), lines.next()) {
        let actual = printed(&mut vm, expression);
        let same = printed(&mut vm, &format!("(equal-including-properties {expression} (car (read-from-string {expected:?})))"));
        if same.as_deref() != Ok("t") {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `%[' and `%]' show keyboard.c's command_loop_level, the recursive edits
/// keyboard.el's `recursive-edit' counts, not minibuffers: a bracket per
/// level, or GNU's ellipses past five (xdisp.c decode_mode_spec). This VM
/// does not load keyboard.el, which declares the level in a session.
#[test]
fn recursion_brackets_follow_the_command_loop_level() {
    let mut vm = romacs();
    let actual = printed(&mut vm, r#"(progn (defvar romacs--command-loop-level) (mapcar (lambda (level) (let ((noninteractive nil) (romacs--command-loop-level level)) (substring-no-properties (format-mode-line "%[x%]")))) '(0 1 2 5 6)))"#);
    assert_eq!(actual.as_deref(), Ok(r#"("x" "[x]" "[[x]]" "[[[[[x]]]]]" "[[[... x ...]]]")"#));
}
