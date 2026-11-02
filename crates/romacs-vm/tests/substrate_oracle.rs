//! Runtime substrate GNU's own test suite needs, against pinned GNU.
//! Each line is one expression, evaluated after GNU's loadup chain; both
//! sides run in this process's environment, so collation compares under
//! the same locale.
use romacs_vm::Vm;
use std::path::Path;

/// The loadup chain (find_charset.rs's), for buffers and coding systems.
fn bootstrapped() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    let lisp = root.join("gnu-lisp");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('='))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    paths.push(root.join("rlisp"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

fn matches_pinned_gnu(cases: &str) {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = bootstrapped();
    let mut failures = Vec::new();
    for expression in cases.lines().filter(|line| !line.is_empty()) {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn obarray_clear_matches_pinned_gnu() {
    matches_pinned_gnu(r###"
(let ((ob (obarray-make))) (intern "a" ob) (intern "b" ob) (list (obarray-clear ob) (intern-soft "a" ob) (let (n) (mapatoms (lambda (s) (push s n)) ob) n) (progn (intern "c" ob) (intern-soft "c" ob)) (unintern "c" ob) (intern-soft "c" ob)))
(let ((ob (obarray-make 1000))) (dotimes (i 100) (intern (format "s%d" i) ob)) (obarray-clear ob) (dotimes (i 100) (intern (format "t%d" i) ob)) (let ((n 0)) (mapatoms (lambda (_) (setq n (1+ n))) ob) (list n (intern-soft "s5" ob) (symbol-name (intern-soft "t5" ob)))))
(condition-case e (obarray-clear [0]) (error e))
(condition-case e (obarray-clear nil) (error e))
"###);
}
