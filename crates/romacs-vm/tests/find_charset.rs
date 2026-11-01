//! charset.c `find-charset-region' and `find-charset-string' against pinned
//! GNU: ASCII, Unicode, raw bytes and their eight-bit charset, unibyte
//! buffers and strings, regions in either order, empty regions, a
//! translation table, and the argument errors.
use romacs_vm::Vm;
use std::path::Path;

/// The loadup chain (process.rs's), for buffers and coding systems.
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

const CASES: &[&str] = &[
    r#"(find-charset-string "abc")"#,
    r#"(find-charset-string (string-to-unibyte ""))"#,
    r#"(find-charset-string (string-to-multibyte ""))"#,
    r#"(find-charset-string "é中😀")"#,
    r#"(find-charset-string "aé")"#,
    r#"(find-charset-string (string-to-unibyte "a\377"))"#,
    r#"(find-charset-string (string 4194303 97))"#,
    r#"(find-charset-string (string 1838576))"#,
    r#"(find-charset-string (string-to-unibyte "ab") (let ((tb (make-char-table 'translation-table))) (aset tb ?a ?é) tb))"#,
    r#"(find-charset-string "aé" (let ((tb (make-char-table 'translation-table))) (aset tb ?a ?中) tb))"#,
    r#"(romacs-fc "abc é ü Ω 中 " (find-charset-region (point-min) (point-max)))"#,
    r#"(romacs-fc "abc é" (find-charset-region (point-max) 1))"#,
    r#"(romacs-fc "abc" (find-charset-region 2 2))"#,
    r#"(romacs-fc "abc" (find-charset-region 1 4))"#,
    r#"(romacs-fc (string 97 4194303) (find-charset-region 1 3))"#,
    r#"(romacs-fc "aé" (progn (set-buffer-multibyte nil) (find-charset-region 1 (point-max))))"#,
    r#"(romacs-fc "ab" (progn (set-buffer-multibyte nil) (find-charset-region 1 (point-max))))"#,
    r#"(romacs-fc "abc" (condition-case e (find-charset-region 1 9) (error e)))"#,
    r#"(romacs-fc "abc" (condition-case e (find-charset-region "a" 2) (error e)))"#,
    r#"(condition-case e (find-charset-string 7) (error e))"#,
];

/// (romacs-fc TEXT FORM): FORM in a fresh multibyte buffer holding TEXT.
const PROLOGUE: &str = r#"(defmacro romacs-fc (text form)
  (list 'with-temp-buffer (list 'insert text) form))"#;

fn source(case: &str) -> String {
    format!("(prin1-to-string {case})")
}

#[test]
fn find_charset_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    let mut vm = bootstrapped();
    vm.eval_source(PROLOGUE).unwrap();
    for case in CASES {
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", PROLOGUE, "--eval", &format!("(princ (encode-coding-string {} 'utf-8))", source(case))]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&format!("(encode-coding-string {} 'utf-8)", source(case)))
            .and_then(|v| v.bytes().map(|b| String::from_utf8_lossy(b).into_owned()));
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{case}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
