//! The byte-op kernels answer as pinned GNU's byte codes do: every case of
//! `byte_op_kernels.el', as a compiled call and through `funcall', over fixnum
//! limits, bignums, floats, markers and wrong types, with condition data.
use romacs_vm::Vm;
use std::path::{Path, PathBuf};

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

fn cases() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/byte_op_kernels.el")
}

fn gnu() -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval",
            "(setq byte-compile-warnings nil native-comp-enable-subr-trampolines nil)",
            "--eval", "(defalias 'bok-compile #'byte-compile)",
            "-l", cases().to_str().unwrap(),
            "--eval", "(princ (bok-run))"])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn romacs() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root);
        vm.eval_source("(defalias 'bok-compile #'identity)").unwrap();
        vm.load_file(&cases()).unwrap();
        let text = vm.eval_source("(bok-run)").and_then(|v| v.display()).unwrap();
        serde_json::from_str::<String>(&text).unwrap_or(text)
    }).unwrap().join().unwrap()
}

/// Cases that differ on purpose, each recorded in DIVERGENCES.md. A case
/// that comes to agree must leave this list.
const KNOWN: &[&str] = &[
    "equal closures:",       // VM2-EQUAL1
    "equal char-tables:",    // VM2-EQUAL1
    "aset string-wide:",     // VM2-ASET-STRING1
    "aset string-raw:",      // VM2-ASET-STRING1
];

#[test]
fn byte_op_kernels_match_compiled_gnu() {
    let (expected, actual) = (gnu(), romacs());
    let known = |line: &str| KNOWN.iter().any(|k| line.starts_with(k));
    for k in KNOWN {
        let same = expected.lines().zip(actual.lines()).any(|(e, a)| e.starts_with(k) && e == a);
        assert!(!same, "{k} now agrees: take it out of KNOWN and DIVERGENCES.md");
    }
    let wrong: Vec<String> = expected.lines().zip(actual.lines())
        .filter(|(e, a)| e != a && !known(e))
        .map(|(e, a)| format!("GNU    {e}\nRomacs {a}"))
        .collect();
    assert_eq!(expected.lines().count(), actual.lines().count(), "case counts differ");
    assert!(expected.lines().count() > 20_000, "the matrix shrank");
    if let Ok(dir) = std::env::var("ROMACS_BOK_OUT") {
        std::fs::write(Path::new(&dir).join("gnu.txt"), &expected).unwrap();
        std::fs::write(Path::new(&dir).join("romacs.txt"), &actual).unwrap();
    }
    assert!(wrong.is_empty(), "{} of {} cases differ:\n{}", wrong.len(), expected.lines().count(),
        wrong.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

/// The float and list kernels allocate between collections that move: every
/// value they leave in a register or a cell survives, with a moving collector
/// running throughout.
#[test]
fn kernels_keep_their_results_across_moving_collections() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let answer = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root);
        vm.set_gc_moving(true);
        vm.eval_source("(let ((sum 0.0) (kept nil) (i 0))
                           (while (< i 200000)
                             (setq sum (+ sum (/ (* 3.0 i) 3.0)))
                             (when (= (% i 1000) 0)
                               (setq kept (cons (list i (- sum 0.5)) kept))
                               (garbage-collect))
                             (setq i (1+ i)))
                           (list sum (length kept) (equal (car (last kept)) (list 0 -0.5))
                                 (max 3 4) (min 3 4) (nthcdr 1 (list 1 2)) (member (list 1) (list 0 (list 1)))))")
            .and_then(|v| v.display()).unwrap()
    }).unwrap().join().unwrap();
    assert_eq!(answer, "(19999900000.0 200 t 4 3 (2) ((1)))");
}
