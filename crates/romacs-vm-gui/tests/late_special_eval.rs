//! A `let' in a form `--eval' (so `eval', `eval-buffer', `eval-expression')
//! evaluates binds dynamically when its variable is special as the binding
//! is made, as GNU's interpreter decides at each `let' (eval.c Flet), even
//! when an earlier part of the same form made it special: a `require', an
//! `eval-buffer' or an `eval' of a `defvar' (VM2-LATE-SPECIAL1).  Each case
//! runs in pinned GNU and in Romacs's batch session; the printed values
//! must be GNU's.
use std::process::Command;

/// (NAME, FORM): FORM's printed value, or its signal.
const CASES: &[(&str, &str)] = &[
    ("require-then-let", "(progn (require 'late-special-lib) (defun late-get () late-lib-var) (let ((late-lib-var 2)) (late-get)))"),
    ("require-org-then-let", "(progn (require 'org) (let ((org-yank-dnd-method 'attach)) (funcall (lambda () (symbol-value 'org-yank-dnd-method)))))"),
    ("eval-buffer-defvar", "(progn (with-temp-buffer (insert \"(defvar late-zz 1) (defun late-zz-get () late-zz)\") (eval-buffer)) (let ((late-zz 2)) (late-zz-get)))"),
    ("nested-let-in-lambda", "(progn (eval '(defvar late-n 1) t) (defun late-n-get () late-n)
        (funcall (lambda () (let ((late-n 2)) (list (late-n-get) (funcall (lambda () late-n)) (let ((f (lambda () late-n))) (funcall f)))))))"),
    ("closure-escaping-dynamic-let", "(progn (eval '(defvar late-e 1) t) (let ((g (let ((late-e 9)) (lambda () late-e)))) (funcall g)))"),
    ("closure-over-lexical-let", "(let ((x 1)) (list (let ((y 2)) (funcall (lambda () (+ x y)))) (funcall (let ((z 10)) (lambda () (setq z (1+ z)) z)))))"),
    ("special-variable-p-timing", "(list (special-variable-p 'late-t) (progn (eval '(defvar late-t 1) t) (special-variable-p 'late-t)) (let ((late-t 3)) (symbol-value 'late-t)) late-t)"),
    ("let*-setq-and-restore", "(progn (eval '(defvar late-a 1) t) (list (let* ((late-b 5) (late-a (+ late-b 1))) (setq late-a 9) (list late-a (symbol-value 'late-a) late-b)) late-a))"),
    ("throw-restores", "(progn (eval '(defvar late-c 1) t) (catch 'out (let ((late-c 2)) (throw 'out nil))) late-c)"),
    ("eval-expression", "(progn (eval '(defvar late-x 1) t) (defun late-x-get () late-x) (eval '(let ((late-x 4)) (late-x-get)) t))"),
    // A closure's late `let' that turns dynamic hides the lexical binding it
    // captured: references and `setq' in its body still reach that one.
    ("closure-rebinding-a-captured-name", "(let ((f (let ((late-lib-var 1)) (lambda () (list late-lib-var (let ((late-lib-var 2)) (setq late-lib-var 3) (list late-lib-var (symbol-value 'late-lib-var)))))))) (require 'late-special-lib) (list (funcall f) late-lib-var))"),
    // A later initializer declares the variable: the binding is made
    // dynamically and unbound as it was made.
    ("initializer-defvar", "(list (let ((late-m 1) (late-k (defvar late-m 5))) (list late-m (symbol-value 'late-m) late-k)) late-m)"),
    ("initializer-eval-defvar", "(list (let ((late-v 1) (late-k (eval '(defvar late-v 5) t))) (list late-v (symbol-value 'late-v) late-k)) late-v)"),
];

fn run(binary: &str, dir: &std::path::Path, form: &str, romacs: bool) -> String {
    let wrapped = format!("(progn (add-to-list 'load-path {:?}) (prin1 (condition-case e {form} (error (list 'signal e)))))", dir.display().to_string());
    let mut command = Command::new(binary);
    command.args(["-Q", "--batch", "--eval", &wrapped]).current_dir(dir).env("HOME", dir).env("TMPDIR", dir);
    if romacs { command.env("ROMACS_WORLD", "off"); }
    let output = command.output().unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn late_specials_in_evaluated_forms_bind_as_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("romacs-late-special-eval-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("late-special-lib.el"), ";;; late-special-lib.el -*- lexical-binding: t -*-\n(defvar late-lib-var 1)\n(provide 'late-special-lib)\n").unwrap();
    let mut failures = vec![];
    for (name, form) in CASES {
        let (expected, actual) = (run(&gnu, &dir, form, false), run(env!("CARGO_BIN_EXE_romacs-vm-gui"), &dir, form, true));
        if expected.is_empty() || actual != expected { failures.push(format!("{name}\n  GNU    {expected}\n  Romacs {actual}")); }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// fixtures/late-special/late-file-test.el loaded: each `progn' makes a
/// variable special (a `require', a `defvar') and then let-binds it, and a
/// function it defines binds it later.  GNU binds dynamically loading the
/// file as source and as bytecomp compiles it (`byte-compile-file-form-progn'
/// takes each child in order, evaluating `require' and declaring `defvar' as
/// it goes); so does Romacs, loading source and through the compiled-Lisp
/// store's compile and its artifact.  late-file-early.el's function binds a
/// variable its file declares only later: dynamically where GNU interprets
/// the file, lexically where GNU loads it compiled, and Romacs alike.
#[test]
fn late_specials_in_loaded_files_bind_as_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let root = std::env::temp_dir().join(format!("romacs-late-special-files-{}", std::process::id()));
    let fixtures = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/late-special"));
    let (source, compiled, home) = (root.join("source"), root.join("compiled"), root.join("home"));
    for dir in [&source, &compiled, &home.join("config"), &home.join("tmp")] { std::fs::create_dir_all(dir).unwrap(); }
    for name in ["late-file-lib.el", "late-file-test.el", "late-file-early.el"] {
        for dir in [&source, &compiled] { std::fs::copy(fixtures.join(name), dir.join(name)).unwrap(); }
    }
    let probe = "(progn (require 'late-file-test) (require 'late-file-early) (prin1 (list (reverse late-file-results) late-early-result (late-early-f))))";
    let run_probe = |binary: &str, dir: &std::path::Path, store: bool, probe: &str| -> String {
        let mut command = Command::new(binary);
        command.args(["-Q", "--batch", "-L"]).arg(dir).args(["--eval", probe])
            .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_CACHE_HOME", home.join("cache")).env("TMPDIR", home.join("tmp")).env("LANG", "C.UTF-8")
            .envs(["ROMACS_RUNTIME_ROOT", "ROMACS_LISP_DIR", "LD_LIBRARY_PATH"].iter().filter_map(|name| Some((name, std::env::var_os(name)?))));
        if store { command.env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", root.join("store")); }
        let output = command.output().unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let run = |binary: &str, dir: &std::path::Path, store: bool| run_probe(binary, dir, store, probe);
    // A `load' naming the `.el' reads the source even beside an `.elc'.
    let explicit = |dir: &std::path::Path| format!("(progn (load {:?} nil t t) (prin1 (list late-early-result (late-early-f))))",
        dir.join("late-file-early.el").display().to_string());
    // A function late-file-early.el defines before its variable's `defvar'
    // binds dynamically interpreted (2) and lexically compiled (1).
    let (interpreted, compiled_answer) = ("(((require 2) (defvar 2) (later-call 3)) 2 2)", "(((require 2) (defvar 2) (later-call 3)) 1 1)");
    // GNU from source, then as byte-compiled.
    assert_eq!(run(&gnu, &source, false), interpreted, "GNU loading .el");
    let compile = format!("(progn (add-to-list 'load-path {:?}) (dolist (f '(\"late-file-lib.el\" \"late-file-test.el\" \"late-file-early.el\")) (byte-compile-file (expand-file-name f {:?}))))",
        compiled.display().to_string(), compiled.display().to_string());
    let output = Command::new(&gnu).args(["-Q", "--batch", "--eval", &compile]).env("HOME", &home).output().unwrap();
    assert!(output.status.success() && compiled.join("late-file-test.elc").exists(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(run(&gnu, &compiled, false), compiled_answer, "GNU loading .elc");
    assert_eq!(run_probe(&gnu, &compiled, false, &explicit(&compiled)), "(2 2)", "GNU loading the .el beside its .elc");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    // Source GNU would interpret: without a store, and with one but no `.elc'.
    assert_eq!(run(romacs, &source, false), interpreted, "Romacs loading source");
    assert_eq!(run(romacs, &source, true), interpreted, "Romacs loading source beside a store");
    // A sibling `.elc': GNU would load compiled code.  Without a store
    // Romacs reads the source as GNU's compiled load would have it; with
    // one, the store's compile process makes the artifact, then a load runs it.
    for name in ["late-file-lib.elc", "late-file-test.elc", "late-file-early.elc"] { std::fs::write(source.join(name), "").unwrap(); }
    assert_eq!(run(romacs, &source, false), compiled_answer, "Romacs loading source GNU would load compiled");
    assert_eq!(run_probe(romacs, &source, false, &explicit(&source)), "(2 2)", "Romacs loading the .el beside an .elc");
    assert_eq!(run_probe(romacs, &source, true, &explicit(&source)), "(2 2)", "Romacs loading the .el beside an .elc and a store");
    assert_eq!(run(romacs, &source, true), compiled_answer, "Romacs compiling through the store");
    assert!(std::fs::read_dir(root.join("store")).unwrap().count() > 0, "the store holds the artifacts");
    assert_eq!(run(romacs, &source, true), compiled_answer, "Romacs loading the store's artifact");
    let _ = std::fs::remove_dir_all(&root);
}
