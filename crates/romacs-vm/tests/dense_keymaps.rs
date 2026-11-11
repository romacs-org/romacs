//! Dense keymaps, as keymap.c reads and writes them: a vector element of
//! a keymap, (keymap [DEF0 DEF1 ...] ...), binds the characters below its
//! length (access_keymap_1, store_in_keymap, map_keymap_internal,
//! copy_keymap_1), beside char-table keymaps (make-keymap's).  Each
//! primitive is compared with pinned GNU 32.0.50 on the loadup chain:
//! lookup-key (an index past the vector, nil and t slots, a parent behind
//! them, a dense prefix map), define-key (into the vector, past it, a
//! range partly inside it, a removal), key-binding, map-keymap,
//! keymap-parent, where-is-internal, copy-keymap, accessible-keymaps and
//! help--describe-vector with the vector's keymap as ENTIRE-MAP.
use romacs_vm::Vm;
use std::path::Path;

fn bootstrapped(root: &Path) -> Vm {
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    let lisp = root.join("lisp/gnu");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('=')
            && !name.ends_with('~') && !name.ends_with(".orig") && !name.ends_with(".rej"))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    if names.iter().any(|n| n == "obsolete") { paths.push(lisp.join("obsolete")); }
    paths.push(root.join("lisp/romacs"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("lisp/romacs/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

/// Both sides define DK-MAP: a dense keymap of ten slots, 3 and 4 bound,
/// 6 bound to t, with a sparse parent binding 5, 6 and 7.
const SETUP: &str = r#"(progn
 (defun dk-map ()
   (let ((v (make-vector 10 nil)) (parent (make-sparse-keymap)))
     (aset v 3 'three) (aset v 4 'four) (aset v 6 t)
     (define-key parent [5] 'parent-five) (define-key parent [6] 'parent-six) (define-key parent [7] 'parent-seven)
     (let ((m (list 'keymap v (cons 'f1 'eff-one))))
       (set-keymap-parent m parent)
       m)))
 (defmacro dk-try (&rest body) `(condition-case err (progn ,@body) (error (list 'ERROR err))))
 t)"#;

const CASES: &[&str] = &[
    "(let ((m (dk-map))) (mapcar (lambda (k) (dk-try (lookup-key m k))) (list [3] [4] [5] [6] [7] [9] [10] [12] [f1] \"\\C-c\" [?\\M-\\C-c] [3 1])))",
    "(let ((m (dk-map))) (mapcar (lambda (k) (dk-try (lookup-key m k t))) (list [3] [9] [100])))",
    "(let ((m (dk-map))) (dk-try (define-key m [4] 'new-four)) (define-key m [8] 'eight) (define-key m [20] 'twenty) (list (lookup-key m [4]) (lookup-key m [8]) (lookup-key m [20]) (aref (cadr m) 8) (length m)))",
    "(let ((m (dk-map))) (define-key m [(2 . 15)] 'rng) (list (mapcar (lambda (c) (lookup-key m (vector c))) '(1 2 9 10 15 16)) (cadr m) (char-table-p (nth 2 m))))",
    "(let ((m (dk-map))) (define-key m [(1 . 2)] 'small) (list (cadr m) (length m)))",
    "(let ((m (dk-map))) (define-key m [3] nil t) (list (aref (cadr m) 3) (lookup-key m [3])))",
    "(let ((m (dk-map))) (define-key m [3] nil) (list (aref (cadr m) 3) (lookup-key m [3])))",
    "(let ((m (list 'keymap (make-vector 4 nil)))) (define-key m [?\\C-x] (list 'keymap (make-vector 5 nil))) (define-key m [?\\C-x 2] 'cx2) (define-key m [?\\C-x 3 1] 'deep) (list (lookup-key m [?\\C-x 2]) (aref (cadr (lookup-key m [?\\C-x])) 2) (lookup-key m [?\\C-x 3 1]) m))",
    "(with-temp-buffer (use-local-map (dk-map)) (list (key-binding [3]) (key-binding [4]) (key-binding [5]) (key-binding [6]) (key-binding [9])))",
    "(let ((m (dk-map)) (out nil)) (map-keymap (lambda (k v) (push (cons k v) out)) m) (nreverse out))",
    "(let ((m (dk-map)) (out nil)) (map-keymap-internal (lambda (k v) (push (cons k v) out)) m) (nreverse out))",
    "(let ((m (dk-map))) (list (keymap-parent m) (keymap-parent (keymap-parent m))))",
    "(let ((m (dk-map))) (list (where-is-internal 'three m) (where-is-internal 'parent-five m) (where-is-internal 'eff-one m) (where-is-internal 'three (list m) t)))",
    "(let* ((m (dk-map)) (c (copy-keymap m))) (list (equal m c) (eq (cadr m) (cadr c)) (progn (aset (cadr c) 3 'changed) (lookup-key m [3])) (lookup-key c [3])))",
    "(let ((m (list 'keymap (make-vector 4 nil)))) (aset (cadr m) 2 (list 'keymap (make-vector 3 nil))) (aset (cadr (aref (cadr m) 2)) 1 'inner) (accessible-keymaps m))",
    "(let ((m (dk-map))) (with-temp-buffer (help--describe-vector (cadr m) [?\\C-c] #'help--describe-command nil nil m nil) (buffer-string)))",
    "(let ((m (make-keymap))) (define-key m \"a\" 'ay) (define-key m [(?b . ?d)] 'bd) (define-key m \"\\M-a\" 'meta-a) (list (lookup-key m \"a\") (lookup-key m \"c\") (lookup-key m \"\\M-a\") (let (out) (map-keymap (lambda (k v) (push (cons k v) out)) m) (nreverse out)) (where-is-internal 'bd m)))",
];

fn gnu(oracle: &str, form: &str) -> String {
    let program = format!("(progn {SETUP} (princ (prin1-to-string {form})))");
    let output = std::process::Command::new(oracle).args(["-Q", "--batch", "--eval", &program])
        .stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn dense_keymaps_match_pinned_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expected: Vec<String> = CASES.iter().map(|form| gnu(&oracle, form)).collect();
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root);
        vm.eval_source(SETUP).unwrap();
        CASES.iter().map(|form| vm.eval_source(&format!("(prin1-to-string {form})")).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))).collect::<Vec<_>>()
    }).unwrap().join().unwrap();
    let failures: Vec<String> = CASES.iter().zip(&expected).zip(&actual)
        .filter(|((_, gnu), mine)| mine.as_deref() != Ok(gnu.as_str()))
        .map(|((form, gnu), mine)| format!("{form}\n  gnu:    {gnu}\n  romacs: {mine:?}"))
        .collect();
    assert!(failures.is_empty(), "{} of {} differ:\n{}", failures.len(), CASES.len(), failures.join("\n"));
}
