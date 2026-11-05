//! The width rule (width_rule.rs) on every path that counts columns,
//! against pinned GNU 32.0.50 on the loadup chain: `string-width' of every
//! code point to #x10FFFF as runs (FROM TO WIDTH; char_width_sweep.rs
//! covers the rest by `char-width'); `truncate-string-to-width', with
//! and without padding, an ellipsis and a start column, of strings mixing
//! ASCII, a tab and one character of each run; `format''s field width and
//! precision over wide, combining and control characters; and
//! `format-mode-line''s field width and precision over them.  Automatic
//! composition is off on both sides (`composition-function-table' empty):
//! the rule counts characters, not shaped clusters.
use romacs_vm::Vm;
use std::path::Path;

fn bootstrapped(root: &Path) -> Vm {
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

/// Runs of `string-width' over 0..#x10FFFF, and the first character of
/// each that is a Unicode scalar, which the other cases mix into their
/// strings (GNU prints the others as its internal bytes).
const SETUP: &str = r#"(progn
 ;; No automatic composition: GNU measures a composed cluster as its
 ;; shaping draws it, which the width rule does not (DIVERGENCES.md
 ;; VM2-STRING-WIDTH-COMPOSITION).
 (setq composition-function-table (make-char-table nil))
 (defun wr-runs (from to width)
   (let ((out nil) (start from) (last (funcall width from)) (c (1+ from)))
     (while (<= c to)
       (let ((w (funcall width c)))
         (unless (equal w last) (push (list start (1- c) last) out) (setq start c last w)))
       (setq c (1+ c)))
     (push (list start to last) out)
     (nreverse out)))
 (defvar wr-string-runs (wr-runs 0 #x10FFFF (lambda (c) (string-width (string c)))))
 (defvar wr-samples (seq-filter (lambda (c) (and (< c #x110000) (not (<= #xD800 c #xDFFF)))) (mapcar #'car wr-string-runs)))
 t)"#;

const CASES: &[&str] = &[
    "wr-string-runs",
    "(mapcar (lambda (c) (let ((s (string ?a c ?b c ?\\t c ?z)))
       (list (string-width s) (truncate-string-to-width s 3) (truncate-string-to-width s 4 nil ?.)
             (truncate-string-to-width s 6 nil nil \"...\") (truncate-string-to-width s 7 2 ?x \"+\")
             (truncate-string-to-width s 9 1 nil t))))
     wr-samples)",
    "(mapcar (lambda (s) (list (format \"[%6s]\" s) (format \"[%-6s]\" s) (format \"[%.3s]\" s) (format \"[%5.2s]\" s) (format \"[%4c]\" (aref s 0))))
     (list \"\u{754C}\u{754C}\u{754C}\" \"a\u{0301}b\u{0301}c\" \"x\\ty\" \"\\C-a\\C-b\" \"\u{1F600}\u{1F600}\" \"\u{321F}z\" \"\u{AD}\u{AD}q\"))",
    "(mapcar (lambda (s) (list (format-mode-line (list -4 s)) (format-mode-line (list 7 s)) (format-mode-line (list -7 s))))
     (list \"\u{754C}\u{754C}\u{754C}\" \"a\u{0301}b\u{0301}c\" \"\u{1F600}\u{1F600}x\" \"\u{321F}z\"))",
    "(let ((ctl-arrow nil)) (list (string-width \"\\C-a\") (format \"[%5s]\" \"\\C-a\") (with-temp-buffer (insert \"\\C-a\") (current-column))))",
    "(let ((tab-width 4)) (list (string-width \"\\t\") (format \"[%6s]\" \"a\\tb\") (truncate-string-to-width \"a\\tbcdef\" 6)))",
];

fn gnu(oracle: &str, form: &str) -> String {
    let program = format!("(progn {SETUP} (princ (prin1-to-string {form})))");
    let output = std::process::Command::new(oracle).args(["-Q", "--batch", "--eval", &program])
        .stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn widths_match_pinned_gnu_on_every_path() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expected: Vec<String> = CASES.iter().map(|form| gnu(&oracle, form)).collect();
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root);
        vm.eval_source(SETUP).unwrap();
        CASES.iter().map(|form| vm.eval_source(&format!("(prin1-to-string {form})")).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))).collect::<Vec<_>>()
    }).unwrap().join().unwrap();
    for ((form, gnu), mine) in CASES.iter().zip(&expected).zip(&actual) {
        let mine = mine.as_deref().unwrap_or("<error>");
        if mine != gnu {
            let (g, m): (Vec<&str>, Vec<&str>) = (gnu.split(") (").collect(), mine.split(") (").collect());
            let first = g.iter().zip(&m).position(|(a, b)| a != b);
            panic!("{form}\nfirst difference: gnu {:?}\n                  romacs {:?}\n({} vs {} parts; romacs result {:?})",
                first.map(|i| g[i]), first.map(|i| m[i]), g.len(), m.len(), &mine[..mine.len().min(300)]);
        }
    }
}
