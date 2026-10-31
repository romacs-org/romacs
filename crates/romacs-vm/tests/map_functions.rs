//! The prelude's `mapc', `mapcar', `mapcan' and `mapconcat' (rlisp/vm/
//! prelude.el, DIVERGENCES.md VM2-MAPCAR1) called from compiled code,
//! against pinned byte-compiled GNU: the frames a callback sees, nonlocal
//! exits through and around the walk, redefinition and advice of the
//! mapping function, and every sequence type.
use romacs_vm::Vm;
use std::path::Path;

// The loadup bootstrap (subr.el, nadvice.el, cl-lib.el), as tests/sealed_primitives.rs.
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

fn compare(name: &str, source: &str) {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let oracle = format!(
        "(progn (setq lexical-binding t byte-compile-warnings nil native-comp-enable-subr-trampolines nil) \
         (defalias 'bt-compile #'byte-compile) (prin1 {source}))"
    );
    let output = std::process::Command::new(binary)
        .args(["-Q", "--batch", "--eval", &oracle])
        .output()
        .unwrap();
    assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let source = format!("(prin1-to-string (progn (defun bt-compile (function) function) {source}))");
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root);
        vm.eval_source(&source).and_then(|v| v.display()).map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
    }).unwrap().join().unwrap();
    assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{name}");
}

// Frames named in bt-frames-of, innermost first, as (EVALUATED FUNCTION ARGS).
const FRAMES: &str = r#"(fset 'bt-frames (bt-compile (lambda ()
    (let (seen)
      (mapbacktrace (lambda (e f a _flags)
        (if (memq f '(bt-cb mapc mapcar mapcan mapconcat)) (setq seen (cons (list e f a) seen)))))
      (nreverse seen)))))"#;

#[test]
fn callbacks_see_the_mapping_functions_frame() {
    compare(
        "frames",
        &format!(r#"(progn {FRAMES}
      (defvar bt-out nil)
      (fset 'bt-cb (bt-compile (lambda (x) (setq bt-out (cons (bt-frames) bt-out)) (list x))))
      (fset 'bt-run (bt-compile (lambda (l)
        (list (mapcar 'bt-cb l) (mapc 'bt-cb l) (mapcan 'bt-cb l)
              (mapconcat 'bt-cb3 l) (mapconcat 'bt-cb2 l "-")
              (nreverse bt-out)))))
      (fset 'bt-cb2 (bt-compile (lambda (x) (bt-cb x) "t")))
      (fset 'bt-cb3 (bt-compile (lambda (x) (bt-cb x) "s")))
      (bt-run '(7)))"#),
    );
}

#[test]
fn nonlocal_exits_leave_no_mapping_frame() {
    compare(
        "exits",
        &format!(r#"(progn {FRAMES}
      (fset 'bt-run (bt-compile (lambda (l)
        (let (cleanup inner)
          (list
           (catch 'out (mapcar (lambda (x) (throw 'out x)) l))
           (bt-frames)
           (catch 'out (unwind-protect (mapc (lambda (x) (throw 'out x)) l)
                         (setq cleanup (bt-frames))))
           cleanup
           (condition-case e (mapcan (lambda (x) (car x)) l) (error e))
           (bt-frames)
           (mapcar (lambda (x)
                     (catch 'in (unwind-protect (throw 'in x) (setq inner (length (bt-frames)))))
                     (condition-case nil (car x) (error (length (bt-frames)))))
                   l)
           inner)))))
      (bt-run '(7 8)))"#),
    );
}

#[test]
fn redefinition_and_advice_reach_compiled_calls() {
    compare(
        "advice",
        r#"(progn
      (require 'nadvice) (require 'cl-lib)
      (fset 'bt-inc (lambda (x) (1+ x))) (fset 'bt-list (lambda (x) (list x)))
      (fset 'bt-num (lambda (x) (number-to-string x)))
      (fset 'bt-run (bt-compile (lambda (l)
        (list (mapcar 'bt-inc l) (mapc 'bt-inc l) (mapcan 'bt-list l) (mapconcat 'bt-num l ",")))))
      (fset 'bt-tag (lambda (orig f &rest args)
        (if (memq f '(bt-inc bt-list bt-num)) (list 'advised (apply orig f args)) (apply orig f args))))
      (let (out)
        (setq out (cons (bt-run '(1 2)) out))
        (dolist (f '(mapcar mapc mapcan mapconcat)) (advice-add f :around 'bt-tag))
        (setq out (cons (bt-run '(1 2)) out))
        (dolist (f '(mapcar mapc mapcan mapconcat)) (advice-remove f 'bt-tag))
        (setq out (cons (bt-run '(1 2)) out))
        (let ((old (symbol-function 'mapcar)))
          (setq out (cons (cl-letf (((symbol-function 'mapcar)
                                     (lambda (f l) (if (eq f 'bt-inc) 'let-bound (funcall old f l)))))
                            (bt-run '(1 2)))
                          out)))
        (setq out (cons (bt-run '(1 2)) out))
        (nreverse out)))"#,
    );
}

#[test]
fn every_sequence_type_and_signal_matches_gnu() {
    compare(
        "sequences",
        r#"(progn
      (defvar bt-dyn 1)
      (fset 'bt-run (bt-compile (lambda ()
        (let ((record (record 'bt-rec 1)) (shorten (list 1 2 3)) (calls nil))
          (list (mapcar #'1+ [1 2]) (mapcar #'identity "aλb") (mapcar #'identity (bool-vector t nil))
                (mapc #'ignore [1]) (mapcan #'list "ab") (mapconcat #'string "ab" "-")
                (mapconcat #'identity ["a" "b"] '(?,)) (mapconcat #'identity '("a" "b") nil)
                (mapconcat #'identity '("a" "b") "") (mapconcat #'identity '("a") 'bad)
                (mapconcat #'identity nil 'bad) (mapcar #'1+ nil) (mapc #'1+ nil) (mapcan #'list nil)
                (condition-case e (mapcar #'1+ '(1 . 2)) (error e))
                (condition-case e (mapc #'1+ 7) (error e))
                (condition-case e (mapcar #'1+ (make-char-table 'bt)) (error e))
                (condition-case e (mapcar #'1+ record) (error e))
                (condition-case e (mapcan #'identity (list (list 1) 2 (list 3))) (error e))
                (condition-case e (mapconcat #'identity '(a b) "-") (error e))
                (condition-case e (mapconcat #'identity '("a" "b") 5) (error e))
                (mapcan #'identity (list (list 1) nil (list 2) nil nil 3))
                (mapcar (lambda (x) (setcdr shorten nil) (setq calls (cons x calls))) shorten) calls
                (let ((bt-dyn 2)) (mapcar (lambda (_) bt-dyn) '(1)))
                (mapcar (lambda (x) (let ((bt-dyn x)) (funcall (lambda () bt-dyn)))) '(5))
                (condition-case e (mapcar 5 '(1)) (error e))
                (condition-case e (mapcar (lambda () 1) '(1)) (error e)))))))
      (bt-run))"#,
    );
}
