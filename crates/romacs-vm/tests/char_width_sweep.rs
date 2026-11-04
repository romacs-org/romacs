//! Every code point's width against pinned GNU 32.0.50 on the loadup
//! chain, as runs (FROM TO WIDTH): `char-width' over 0..#x3FFFFF, which
//! reads `char-width-table' (international/characters.el builds it), and
//! `current-column' after the character alone on a line of a multibyte
//! buffer, over the Unicode scalars and the raw bytes.  GNU's
//! scan_for_column counts a character by that same table
//! (CHARACTER_WIDTH), so an unassigned code point in a wide block, U+321F,
//! is two columns in both.  Surrogates, which a Romacs buffer cannot hold
//! (DIVERGENCES.md VM2-RAWTEXT1), are left out of the column sweep.
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

const SWEEP: &str = r#"(let ((runs (lambda (from to width)
               (let ((out nil) (start from) (last (funcall width from)) (c (1+ from)))
                 (while (<= c to)
                   (let ((w (funcall width c)))
                     (unless (equal w last) (push (list start (1- c) last) out) (setq start c last w)))
                   (setq c (1+ c)))
                 (push (list start to last) out)
                 (nreverse out)))))
  (list (funcall runs 0 #x3FFFFF (lambda (c) (char-width c)))
        (with-temp-buffer
          (set-buffer-multibyte t)
          (let ((column (lambda (c) (if (<= #xD800 c #xDFFF) 'surrogate (erase-buffer) (insert c) (current-column)))))
            (append (funcall runs 0 #x10FFFF column) (funcall runs #x3FFF80 #x3FFFFF column))))))"#;

#[test]
fn every_code_points_width_and_column_match_pinned_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {SWEEP}))")])
        .stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mine = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root);
        vm.eval_source(&format!("(prin1-to-string {SWEEP})")).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
    }).unwrap().join().unwrap().unwrap();
    // The first differing run, to name what differs.
    let (g, m): (Vec<&str>, Vec<&str>) = (gnu.split(") (").collect(), mine.split(") (").collect());
    let first = g.iter().zip(&m).position(|(a, b)| a != b);
    assert!(gnu == mine, "first differing run: gnu {:?} romacs {:?} ({} vs {} runs)",
        first.map(|i| g[i]), first.map(|i| m[i]), g.len(), m.len());
}
