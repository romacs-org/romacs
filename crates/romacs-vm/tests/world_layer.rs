//! An init layer's build (`world::layer`): the plan's libraries load
//! after a fake chain, a definition that reads a per-process fact is kept
//! for restore and its variable tainted, any other form's such read, a
//! timer or a signal leaves its entry live, by name, and the startup function
//! runs again after them, reproducing what it computed before.
use romacs_vm::Vm;
use romacs_vm::host::{self, Pinned};
use romacs_vm::world::layer::{Entry, Kind, Plan};
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

const PINNED: Pinned = Pinned { clock: UNIX_EPOCH, pid: 4242, seed: [1, 2, 3, 4] };

/// A fake loadup: its startup function records what it computed from the
/// state it found, and a `defcustom' that defines as `defvar' does.
const BOOTSTRAP: &str = "\
(defvar romacs--world-reseed-forms nil)
(defvar timer-list nil)
(defvar tracking-out nil)
(defvar tracking-mode nil)
(put 'tracking-mode 'custom-set 'tracking-set-mode)
(defun tracking-mode-value () 'on)
(defvar custom-delayed-init-variables '(tracking-mode))
(defmacro defcustom (symbol value &rest _) (list 'defvar symbol value))
(defun fake-startup ()
  (unwind-protect
      (progn (romacs--startup-phase t)
             (setq tracking-out (if (boundp 'lib-g-flag) 'g 'none))
             (setq tracking-mode (list (get 'tracking-mode 'custom-set) (tracking-mode-value)))
             (setq custom-delayed-init-variables t))
    (romacs--startup-phase nil)))
(defvar romacs-startup-function #'fake-startup)
(fake-startup)
";

const LIBRARIES: [(&str, &str); 13] = [
    // A definition reading a tracked fact, and one reading its variable.
    ("lib-a", "(defcustom lib-a-program (car exec-path) \"doc\")\n(defvar lib-a-derived (concat lib-a-program \"!\"))\n(provide 'lib-a)\n"),
    // A plain form reading a tracked fact.
    ("lib-b", "(defvar lib-b-ok 1)\n(setq lib-b-dir (expand-file-name \"x\"))\n(provide 'lib-b)\n"),
    ("lib-c", "(setq timer-list (cons 'fake timer-list))\n(provide 'lib-c)\n"),
    // A plain form reading a tainted variable.
    ("lib-d", "(setq lib-d-x lib-a-program)\n(provide 'lib-d)\n"),
    ("lib-e", "(error \"boom\")\n"),
    ("lib-f", "(defvar lib-f 1)\n(provide 'lib-f)\n"),
    // A library that changes what startup computes.
    ("lib-g", "(defvar lib-g-flag t)\n(provide 'lib-g)\n"),
    // A buffer-local definition reading a tracked fact, one reading it, and a plain form reading it.
    ("lib-h", "(defvar-local lib-h-local (car exec-path))\n(defvar lib-h-from-local (list lib-h-local))\n(defvar lib-h-in-buffer (buffer-local-value 'lib-h-local (current-buffer)))\n(provide 'lib-h)\n"),
    ("lib-i", "(setq lib-i-x (buffer-local-value 'lib-h-local (current-buffer)))\n(provide 'lib-i)\n"),
    // A definition an effect cuts short.
    ("lib-j", "(defvar-local lib-j-dir (let ((d \"/nonexistent-romacs-layer/x\")) (make-directory-internal d) d))\n(defvar lib-j-after lib-j-dir)\n(provide 'lib-j)\n"),
    // As cus-start: whether a delayed variable is bound, and the property
    // startup read, written with what it held.
    ("lib-k", "(when (boundp 'tracking-mode) (put 'tracking-mode 'custom-set 'tracking-set-mode))\n(provide 'lib-k)\n"),
    // The property startup reads, written with another value.
    ("lib-l", "(put 'tracking-mode 'custom-set 'tracking-other)\n(provide 'lib-l)\n"),
    // A plain form reading a delayed variable.
    ("lib-m", "(setq lib-m-x tracking-mode)\n(provide 'lib-m)\n"),
];

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("romacs-world-layer-{}-{unique}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lisp/session")).unwrap();
    std::fs::create_dir_all(dir.join("lisp")).unwrap();
    for (name, text) in LIBRARIES {
        std::fs::write(dir.join(format!("lisp/{name}.el")), text).unwrap();
    }
    std::fs::write(dir.join("init.el"), ";; the init\n").unwrap();
    dir
}

fn plan(dir: &Path, names: &[&str]) -> Plan {
    let entries = names.iter().map(|name| Entry { kind: Kind::Require, name: (*name).to_owned(), file: None, noerror: false, load_path: 0, declared: false, forms: Vec::new() }).collect();
    Plan::new(&dir.join("init.el"), entries, vec![vec![dir.join("lisp").display().to_string()]]).unwrap()
}

/// Build with NAMES' libraries as the layer: the VM, the live entries
/// and the trace.
fn build(name: &str, names: &[&str]) -> (Vm, Vec<(usize, String)>, Json) {
    let dir = scratch(name);
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"), BOOTSTRAP).unwrap();
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan(&dir, names), &[]);
    let trace = host::leave_build().unwrap();
    (vm, live, trace)
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap()
}

fn uncaptured<'a>(trace: &'a Json, kind: &str) -> Vec<&'a Json> {
    trace["uncaptured"].as_array().unwrap().iter().filter(|r| r["kind"] == kind).collect()
}

#[test]
fn definitions_that_read_the_process_are_kept_for_restore_and_other_forms_leave_their_entry_live() {
    let (mut vm, live, trace) = build("entries", &["lib-a", "lib-b", "lib-c", "lib-d", "lib-e", "lib-f"]);
    let live: Vec<(usize, &str)> = live.iter().map(|(at, why)| (*at, why.split(':').next().unwrap())).collect();
    assert_eq!(live, [(1, "per-process"), (2, "timer"), (3, "per-process"), (4, "error")], "{}", trace["uncaptured"]);
    // The build names each live entry, and lib-d's read names the tainted root.
    let named: Vec<u64> = uncaptured(&trace, "init-layer").iter().map(|r| r["entry"].as_u64().unwrap()).collect();
    assert_eq!(named, [1, 2, 3, 4]);
    let tracks: Vec<(u64, String)> = uncaptured(&trace, "per-process").iter()
        .filter_map(|r| Some((r["root"].as_u64()?, r["symbol"].as_str()?.to_owned()))).collect();
    assert!(tracks.contains(&(2, "default-directory".into())), "{tracks:?}");
    assert!(tracks.contains(&(7, "lib-a-program".into())), "{tracks:?}");
    // lib-a's definitions are kept, in load order once reversed, with their forms.
    assert_eq!(eval(&mut vm, "(mapcar (lambda (e) (list (car e) (car (cdr e)) (car (nth 2 e)))) (reverse romacs--world-reseed-forms))"),
               "((defcustom lib-a-program defcustom) (defvar lib-a-derived defvar))");
    assert_eq!(eval(&mut vm, "(nth 2 (nth 2 (car romacs--world-reseed-forms)))"), "(concat lib-a-program \"!\")");
    let kept: Vec<&str> = trace["records"].as_array().unwrap().iter().filter(|r| r["kind"] == "layer-reseed")
        .map(|r| r["symbol"].as_str().unwrap()).collect();
    assert_eq!(kept, ["lib-a-program", "lib-a-derived"]);
    // The libraries loaded, lib-f too, and the startup function ran again after them.
    assert_eq!(eval(&mut vm, "(list (featurep 'lib-a) (featurep 'lib-f) lib-f tracking-out)"), "(t t 1 none)");
    let runs = trace["records"].as_array().unwrap().iter().filter(|r| r["kind"] == "startup" && r["on"] == true).count();
    assert_eq!(runs, 2, "{trace}");
    assert!(!uncaptured(&trace, "per-process").iter().any(|r| r["access"] == "startup-output"), "{trace}");
}

#[test]
fn a_tainted_buffer_local_definition_is_tracked_and_a_cut_definition_is_kept_whole() {
    let (mut vm, live, trace) = build("locals", &["lib-h", "lib-i", "lib-j"]);
    let live: Vec<(usize, &str)> = live.iter().map(|(at, why)| (*at, why.split(':').next().unwrap())).collect();
    assert_eq!(live, [(1, "per-process")], "{}", trace["uncaptured"]);
    // lib-h's buffer-local definition and its reader are kept; lib-i's plain read of it is noted.
    let kept = eval(&mut vm, "(mapcar (lambda (e) (list (car e) (car (cdr e)) (nth 3 e))) (reverse romacs--world-reseed-forms))");
    assert_eq!(kept, "((defvar-local lib-h-local nil) (defvar lib-h-from-local nil) (defvar lib-h-in-buffer nil) (defvar-local lib-j-dir t) (defvar lib-j-after nil))");
    assert_eq!(eval(&mut vm, "(list (local-variable-if-set-p 'lib-h-local) lib-j-dir lib-j-after)"), "(t nil nil)");
    let effects: Vec<&str> = trace["records"].as_array().unwrap().iter().filter(|r| r["kind"] == "layer-effect")
        .map(|r| r["what"].as_str().unwrap()).collect();
    assert_eq!(effects.len(), 1, "{effects:?}");
    assert!(effects[0].starts_with("make directory"), "{effects:?}");
    assert!(uncaptured(&trace, "effect").is_empty(), "{trace}");
}

#[test]
fn a_library_that_changes_what_startup_computes_is_not_captured() {
    let (mut vm, live, trace) = build("outputs", &["lib-f", "lib-g"]);
    assert!(live.is_empty(), "{live:?}");
    assert_eq!(eval(&mut vm, "tracking-out"), "g");
    let changed: Vec<&str> = uncaptured(&trace, "per-process").iter().filter(|r| r["access"] == "startup-output")
        .map(|r| r["symbol"].as_str().unwrap()).collect();
    assert_eq!(changed, ["tracking-out"]);
    assert!(uncaptured(&trace, "init-layer").is_empty(), "no entry is named: {trace}");
}

/// Build NAMES' libraries, written from LIBRARIES into a fresh scratch
/// NAME, over BOOTSTRAP: the VM, the live entries and the trace.
fn build_libraries(name: &str, libraries: &[(&str, &str)], bootstrap: &str) -> (Vm, Vec<(usize, String)>, Json) {
    let dir = scratch(name);
    for (library, source) in libraries {
        std::fs::write(dir.join(format!("lisp/{library}.el")), format!("{source}(provide '{library})\n")).unwrap();
    }
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"), bootstrap).unwrap();
    let names: Vec<&str> = libraries.iter().map(|(library, _)| *library).collect();
    let plan = plan(&dir, &names);
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan, &[]);
    let trace = host::leave_build().unwrap();
    (vm, live, trace)
}

/// A form that meets what a build cannot hold, other than a secret's ask,
/// leaves its entry to the session, however it writes: a `let' around an
/// assignment, an assignment in a condition, a property, a list changed in
/// place by its own syntax or by a function it calls. The layer is built
/// again without the entry, so the image holds nothing it did.
#[test]
fn a_form_that_reads_the_process_leaves_its_entry_to_the_session() {
    let (mut vm, live, trace) = build_libraries("live-entries", &[
        ("lib-w1", "(let ((x (car exec-path))) (setq lib-w1-a x))\n(defvar lib-w1-b lib-w1-a)\n"),
        ("lib-w2", "(when (setq lib-w2-a (car exec-path)) (setq lib-w2-b t))\n(defvar lib-w2-c lib-w2-a)\n"),
        ("lib-w3", "(put 'lib-w3 'directory (car exec-path))\n(defvar lib-w3-directory (get 'lib-w3 'directory))\n"),
        ("lib-w4", "(defvar lib-w4-list (list 'kept))\n(setq lib-w4-list (nconc lib-w4-list (list (car exec-path))))\n"),
        ("lib-w5", "(defvar lib-w5-list (list 'kept))\n(defun lib-w5-grow (l) (nconc l (list (car exec-path))))\n(setq lib-w5-list (lib-w5-grow lib-w5-list))\n"),
    ], BOOTSTRAP);
    let live: Vec<(usize, &str)> = live.iter().map(|(at, why)| (*at, why.as_str())).collect();
    assert_eq!(live, [(0, "per-process"), (1, "per-process"), (2, "per-process"), (3, "per-process"), (4, "per-process")], "{}", trace["uncaptured"]);
    assert_eq!(eval(&mut vm, "romacs--world-reseed-forms"), "nil", "no form is kept as code");
}

/// A timer an entry starts is the image's only when restore makes it
/// again: org-persist's lock refresh, which its re-seed form keeps in its
/// variable. Any other, beside a re-seed form that read the clock for its
/// own variable (cl-extra's random state), leaves its entry to the session.
#[test]
fn a_timer_restore_does_not_make_again_leaves_its_entry_to_the_session() {
    let dir = scratch("timers");
    let (extra, org) = (dir.join("lisp/emacs-lisp"), dir.join("lisp/org"));
    std::fs::create_dir_all(&extra).unwrap();
    std::fs::create_dir_all(&org).unwrap();
    std::fs::write(extra.join("cl-extra.el"), "(defvar cl--random-state (list 'state (current-time)))\n\
        (setq timer-list (cons (list 'unrelated) timer-list))\n(provide 'cl-extra)\n").unwrap();
    std::fs::write(org.join("org-persist.el"), "(defvar org-persist--disable-when-emacs-Q nil)\n\
        (defvar org-persist--refresh-gc-lock-timer nil)\n\
        (unless (and org-persist--disable-when-emacs-Q nil)\n  \
        (setq org-persist--refresh-gc-lock-timer (car (setq timer-list (cons (list 'lock (current-time)) timer-list)))))\n\
        (provide 'org-persist)\n").unwrap();
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"), BOOTSTRAP).unwrap();
    let require = |name: &str| Entry { kind: Kind::Require, name: name.into(), file: None, noerror: false, load_path: 0, declared: false, forms: Vec::new() };
    let paths = vec![vec![extra.display().to_string(), org.display().to_string()]];
    let plan = Plan::new(&dir.join("init.el"), vec![require("cl-extra"), require("org-persist")], paths).unwrap();
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan, &[]);
    let trace = host::leave_build().unwrap();
    let live: Vec<(usize, &str)> = live.iter().map(|(at, why)| (*at, why.as_str())).collect();
    assert_eq!(live, [(0, "timer")], "{}", trace["uncaptured"]);
    assert!(trace["uncaptured"].as_array().unwrap().iter().all(|r| r["kind"] == "init-layer"), "{}", trace["uncaptured"]);
    assert_eq!(eval(&mut vm, "timer-list"), "nil", "no image holds a timer of the builder's");
}

/// A live library the init only declared names the features it
/// provided, in the order they loaded: the session loads them ahead of its
/// first use. One the init loads at startup names none.
#[test]
fn a_declared_library_left_live_names_its_features_in_load_order() {
    let dir = scratch("preload");
    std::fs::write(dir.join("lisp/lib-r0.el"), "(provide 'lib-r0)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-r1.el"), "(require 'lib-r0)\n(setq lib-r1-seen (car exec-path))\n(provide 'lib-r1)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-r2.el"), "(setq lib-r2-seen (car exec-path))\n(provide 'lib-r2)\n").unwrap();
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"), BOOTSTRAP).unwrap();
    let entry = |name: &str, declared| Entry { kind: Kind::Require, name: name.into(), file: None, noerror: false, load_path: 0, declared, forms: Vec::new() };
    let plan = Plan::new(&dir.join("init.el"), vec![entry("lib-r1", true), entry("lib-r2", false)], vec![vec![dir.join("lisp").display().to_string()]]).unwrap();
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan, &[]);
    let trace = host::leave_build().unwrap();
    assert_eq!(live.iter().map(|(at, _)| *at).collect::<Vec<_>>(), [0, 1], "{}", trace["uncaptured"]);
    let features: Vec<&Json> = uncaptured(&trace, "init-layer").iter().map(|r| &r["features"]).collect();
    assert_eq!(features, [&serde_json::json!(["lib-r0", "lib-r1"]), &serde_json::json!([])]);
}

/// A source session runs startup before its libraries and keeps what they
/// set; a restored one runs it after them. A library whose value the
/// startup run after the libraries sets again is live, and the session
/// runs it after its own startup, as from source. The coding system a read
/// reports (`last-coding-system-used') is no startup output: every read
/// sets it.
#[test]
fn a_library_whose_value_startup_sets_again_runs_in_the_session() {
    let bootstrap = BOOTSTRAP.replace("(setq tracking-out", "(setq last-coding-system-used 'utf-8)\n             (setq tracking-out");
    assert_ne!(bootstrap, BOOTSTRAP);
    let (_, live, trace) = build_libraries("reset", &[
        ("lib-q", "(setq last-coding-system-used 'iso-latin-1)\n"),
        ("lib-o", "(setq tracking-out 'lib-o)\n"),
    ], &bootstrap);
    assert_eq!(live, [(1, "startup sets tracking-out again".to_owned())], "{}", trace["uncaptured"]);
    assert!(!uncaptured(&trace, "per-process").iter().any(|r| r["access"] == "startup-output"), "{}", trace["uncaptured"]);
}

#[test]
fn a_library_may_ask_whether_a_delayed_variable_is_bound_and_rewrite_what_startup_reads() {
    let (mut vm, live, trace) = build("cus-start", &["lib-k"]);
    assert!(live.is_empty(), "{}", trace["uncaptured"]);
    assert!(uncaptured(&trace, "per-process").is_empty(), "{trace}");
    assert_eq!(eval(&mut vm, "(list (featurep 'lib-k) tracking-mode)"), "(t (tracking-set-mode on))");
    // Its value is still the process's: reading it leaves the entry live.
    let (_, live, trace) = build("delayed-read", &["lib-k", "lib-m"]);
    assert_eq!(live, [(1, "per-process".to_owned())], "{}", trace["uncaptured"]);
    let tracks: Vec<(u64, &str)> = uncaptured(&trace, "per-process").iter()
        .filter_map(|r| Some((r["root"].as_u64()?, r["symbol"].as_str()?))).collect();
    assert_eq!(tracks, [(3, "tracking-mode")]);
    // Another value leaves its entry live, whatever startup computes from it.
    let (_, live, trace) = build("rewritten", &["lib-l"]);
    assert_eq!(live, [(0, "per-process".to_owned())], "{}", trace["uncaptured"]);
    let written: Vec<(u64, &str, &str)> = uncaptured(&trace, "per-process").iter().filter(|r| r["access"] == "write")
        .map(|r| (r["root"].as_u64().unwrap(), r["symbol"].as_str().unwrap(), r["property"].as_str().unwrap())).collect();
    assert_eq!(written, [(6, "tracking-mode", "custom-set")], "{trace}");
}

/// Libraries the init declared (`Kind::After`, a declared `Kind::Autoload`)
/// are noted at the init's own depth, each once, with their forms in order;
/// `romacs-init-layer-exclude` leaves one out.
#[test]
fn the_libraries_an_init_declares_are_noted_once_with_their_forms() {
    let dir = scratch("declares");
    std::fs::write(dir.join("lisp/lib-x.el"), "(romacs--note-init-after-load 'lib-nested \"(t . 1)\")\n(autoload 'lib-nested-cmd \"lib-nested\")\n(provide 'lib-x)\n").unwrap();
    let init = dir.join("init.el");
    std::fs::write(&init, format!("(setq load-path (cons {:?} load-path))
(romacs--note-init-after-load 'lib-x \"(t setq lib-x-a 1)\")
(autoload 'lib-z-cmd \"lib-z\" nil t)
(romacs--note-init-after-load \"lib-y\" \"(nil . 2)\")
(autoload 'lib-z-other \"lib-z\")
(setq ordinals (list (romacs--init-after-load-ordinal 'lib-x) (romacs--init-after-load-ordinal \"lib-y\") (romacs--init-after-load-ordinal 'lib-new)))
(romacs--note-init-after-load 'lib-x \"(t setq lib-x-b 2)\")
(require 'lib-x)
(romacs--note-init-after-load 'lib-q \"(t . 3)\")
(setq romacs-init-layer-exclude '(lib-q))
(romacs--init-recording-hold t)
(romacs--note-init-after-load 'lib-held \"(t . 4)\")
(require 'lib-x)
(romacs--init-recording-hold nil)
", dir.join("lisp").display().to_string())).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().expect("prelude");
    vm.record_init_loads();
    vm.load_file(&init).unwrap();
    let plan = vm.take_init_loads(&init).expect("a plan");
    let seen: Vec<(Kind, &str, Option<&str>, bool, Vec<&str>)> = plan.entries.iter()
        .map(|e| (e.kind, e.name.as_str(), e.file.as_deref(), e.declared, e.forms.iter().map(String::as_str).collect())).collect();
    assert_eq!(seen, [
        (Kind::After, "lib-x", None, true, vec!["(t setq lib-x-a 1)", "(t setq lib-x-b 2)"]),
        (Kind::Autoload, "lib-z", None, true, vec![]),
        (Kind::After, "lib-y", Some("lib-y"), true, vec!["(nil . 2)"]),
        (Kind::Require, "lib-x", None, false, vec![]),
    ], "the nested library's calls are lib-x's, not the init's; lib-q is excluded; held calls are not the init's");
    assert_eq!(Plan::from_json(&plan.json()).as_ref(), Some(&plan));
    assert_eq!(eval(&mut vm, "(list ordinals (romacs--init-after-load-ordinal 'lib-x))"), "((1 1 0) nil)", "no recording, no ordinal");
}

/// A library the init loads (not only declares) whose load calls a secret
/// reader the build could not make defer (no romacs-world-layer.el here, so
/// the reader runs) is left out by name: the observed call is the net.
#[test]
fn a_loaded_library_runs_live_for_reading_a_secret() {
    let dir = scratch("loaded-secret");
    std::fs::write(dir.join("lisp/lib-r.el"), "(defun auth-source-netrc-parse (&rest _) (list (list :secret \"romacs-fixture-netrc-4c1d\")))\n\
(defvar lib-r-secret (auth-source-netrc-parse :file \"~/.authinfo\"))\n(provide 'lib-r)\n").unwrap();
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"), BOOTSTRAP).unwrap();
    let plan = plan(&dir, &["lib-a", "lib-r", "lib-v"]);
    std::fs::write(dir.join("lisp/lib-v.el"), "(defvar lib-v 1)\n(provide 'lib-v)\n").unwrap();
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan, &[]);
    let trace = host::leave_build().unwrap();
    let live: Vec<(usize, &str)> = live.iter().map(|(at, why)| (*at, why.as_str())).collect();
    assert_eq!(live, [(1, "secret: auth-source-netrc-parse")], "{}", trace["uncaptured"]);
}

/// A library the init loads that reads a credential file itself, not
/// through a reader: the read fails in the build, and the form that made it
/// is kept, as code, for restore to evaluate, a definition or not; the
/// library's other forms are captured, and the entry is not left out.
#[test]
fn a_library_s_credential_read_is_kept_as_code_for_restore() {
    let dir = scratch("credential-file");
    std::fs::write(dir.join(".authinfo"), "machine example login me password romacs-fixture-authinfo-5e2b\n").unwrap();
    let read = format!("(condition-case nil (with-temp-buffer (insert-file-contents {:?}) (buffer-string)) (error nil))",
                       dir.join(".authinfo").display().to_string());
    std::fs::write(dir.join("lisp/lib-c.el"), format!(
        "(defvar lib-c-text {read})\n(setq lib-c-plain {read})\n(defvar lib-c-after 1)\n(provide 'lib-c)\n")).unwrap();
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"), BOOTSTRAP).unwrap();
    let plan = plan(&dir, &["lib-c"]);
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan, &[]);
    let trace = host::leave_build().unwrap();
    assert!(live.is_empty(), "{live:?} {}", trace["uncaptured"]);
    assert_eq!(eval(&mut vm, "(list lib-c-text lib-c-plain lib-c-after)"), "(nil nil 1)",
               "the builder read no credential and went on with the library");
    assert_eq!(eval(&mut vm, "(mapcar (lambda (e) (list (car e) (nth 1 e) (car (nth 2 e)) (nth 3 e))) (reverse romacs--world-reseed-forms))"),
               "((defvar lib-c-text defvar t) (form nil setq t))");
}

/// A definition that loads another file before it reads a per-process
/// fact is still a definition once that file is loaded: it is re-seeded at
/// restore, not live (the file's own forms are judged as theirs).
#[test]
fn a_definition_stays_one_across_a_load_it_makes() {
    let dir = scratch("nested");
    std::fs::write(dir.join("lisp/lib-inner.el"), "(defvar lib-inner-x 1)\n(setq lib-inner-y 2)\n(provide 'lib-inner)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-outer.el"), "(defvar lib-outer-path (progn (require 'lib-inner) (car exec-path)))\n(provide 'lib-outer)\n").unwrap();
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"), BOOTSTRAP).unwrap();
    let plan = plan(&dir, &["lib-outer"]);
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan, &[]);
    let trace = host::leave_build().unwrap();
    assert!(live.is_empty(), "{live:?} {}", trace["uncaptured"]);
    assert_eq!(eval(&mut vm, "(mapcar (lambda (e) (nth 1 e)) romacs--world-reseed-forms)"), "(lib-outer-path)");
}

/// The form that asked for a secret is the one kept, though a library it
/// loads afterwards has forms of its own: their ends do not take its ask.
#[test]
fn a_form_keeps_its_ask_across_a_load_it_makes() {
    let dir = scratch("nested-ask");
    std::fs::write(dir.join(".netrc"), "machine example login me password romacs-fixture-netrc-9a1f\n").unwrap();
    std::fs::write(dir.join("lisp/lib-inner2.el"), "(defvar lib-inner2-x 1)\n(setq lib-inner2-y 2)\n(provide 'lib-inner2)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-asks.el"), format!(
        "(progn (condition-case nil (with-temp-buffer (insert-file-contents {:?})) (error nil)) (require 'lib-inner2))\n(provide 'lib-asks)\n",
        dir.join(".netrc").display().to_string())).unwrap();
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"), BOOTSTRAP).unwrap();
    let plan = plan(&dir, &["lib-asks"]);
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan, &[]);
    let trace = host::leave_build().unwrap();
    assert!(live.is_empty(), "{live:?} {}", trace["uncaptured"]);
    assert_eq!(eval(&mut vm, "(mapcar (lambda (e) (list (car e) (car (nth 2 e)))) romacs--world-reseed-forms)"), "((form progn))");
}

/// A declared library's definitions that read a per-process fact are kept
/// as declared, so restore may leave them pending; one whose load or
/// configuration reads a secret, or that connects to the network, is
/// live by name.
#[test]
fn a_declared_library_runs_live_for_a_secret_or_a_connection() {
    let dir = scratch("declared");
    std::fs::write(dir.join("lisp/lib-p.el"), "(defcustom lib-p-program (car exec-path) \"doc\")\n(provide 'lib-p)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-s.el"), "(defun auth-source-pass-get (&rest _) \"hunter2\")\n(defvar lib-s-secret (auth-source-pass-get 'secret \"x\"))\n(provide 'lib-s)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-n.el"), "(defvar lib-n-connection (condition-case nil (make-network-process :name \"n\" :host \"127.0.0.1\" :service 9) (error nil)))\n(provide 'lib-n)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-v.el"), "(defvar lib-v 1)\n(provide 'lib-v)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-u.el"), "(require 'lib-v)\n(provide 'lib-u)\n").unwrap();
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"),
                   format!("{BOOTSTRAP}(defun romacs--layer-load-declared (file) (load file nil t))
(defun romacs--layer-load-configured (library forms)
  (require library) (mapc (lambda (f) (eval (cdr (car (read-from-string f))) t)) forms))\n")).unwrap();
    let declared = |name: &str| Entry { kind: Kind::Autoload, name: name.into(), file: None, noerror: true, load_path: 0, declared: true, forms: Vec::new() };
    let entries = vec![
        Entry { kind: Kind::Require, name: "lib-a".into(), file: None, noerror: false, load_path: 0, declared: false, forms: Vec::new() },
        declared("lib-p"), declared("lib-s"), declared("lib-n"),
        // lib-u loads lib-v, whose configuration runs live.
        declared("lib-u"),
        Entry { kind: Kind::After, name: "lib-v".into(), file: None, noerror: true, load_path: 0, declared: true, forms: vec!["(t error \"live\")".into()] },
    ];
    let plan = Plan::new(&dir.join("init.el"), entries, vec![vec![dir.join("lisp").display().to_string()]]).unwrap();
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan, &[]);
    let trace = host::leave_build().unwrap();
    let live: Vec<(usize, &str)> = live.iter().map(|(at, why)| (*at, why.as_str())).collect();
    assert_eq!(live.len(), 4, "{}", trace["uncaptured"]);
    assert_eq!(live[0], (2, "secret: auth-source-pass-get"));
    assert_eq!(live[1].0, 3, "a connection is never deferred to restore: {}", trace["uncaptured"]);
    assert_eq!(live[2].0, 5);
    assert_eq!(live[3], (4, "loads lib-v, whose configuration runs live"), "the library stays deferred whole");
    assert!(uncaptured(&trace, "effect").iter().any(|r| r["what"].as_str().is_some_and(|w| w.starts_with("connect ") || w.starts_with("resolve "))), "{trace}");
    // lib-a's definitions are the init's own load's; lib-p's are a declared library's.
    assert_eq!(eval(&mut vm, "(mapcar (lambda (e) (list (car (cdr e)) (nth 4 e))) (reverse romacs--world-reseed-forms))"),
               "((lib-a-program nil) (lib-a-derived nil) (lib-p-program t))");
}

/// A pending variable's first access evaluates its definition, once, then
/// takes the plain paths; a write, a `let' and `boundp' force it too; it
/// can be made pending again; one bound by a `let' cannot be.
#[test]
fn a_pending_variable_is_defined_at_its_first_access() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().expect("prelude");
    eval(&mut vm, "(progn (setq forced nil)
  (defun romacs--world-force (i) (setq forced (cons i forced)) (set-default (aref '[pa pb pc pd] i) (* 10 (1+ i))))
  (defvar pa 'stale) (defvar pb 'stale) (defvar pc 'stale) (defvar pd 'stale)
  (defun read-pa () pa))");
    for (name, index) in [("pa", 0), ("pb", 1), ("pc", 2), ("pd", 3)] {
        assert_eq!(eval(&mut vm, &format!("(romacs--world-pend '{name} {index})")), "t");
    }
    assert_eq!(eval(&mut vm, "forced"), "nil", "pending costs nothing until read");
    // Read, compiled and interpreted, many times: one evaluation.
    assert_eq!(eval(&mut vm, "(let ((n 0)) (while (< n 200) (read-pa) (setq n (1+ n))) (list (read-pa) pa forced))"), "(10 10 (0))");
    assert_eq!(eval(&mut vm, "(progn (setq pb 'mine) (list pb forced))"), "(mine (1 0))", "a write forces, then stores");
    assert_eq!(eval(&mut vm, "(list (let ((pc 'bound)) pc) pc forced)"), "(bound 30 (2 1 0))");
    assert_eq!(eval(&mut vm, "(list (boundp 'pd) forced)"), "(t (3 2 1 0))");
    // Pending again, as every restore makes it: read again, evaluated again.
    assert_eq!(eval(&mut vm, "(progn (setq pa 'stale) (romacs--world-pend 'pa 0))"), "t");
    assert_eq!(eval(&mut vm, "(list (read-pa) (car forced))"), "(10 0)");
    assert_eq!(eval(&mut vm, "(let ((pa 1)) (romacs--world-pend 'pa 0))"), "nil", "a let-bound variable cannot wait");
    assert_eq!(eval(&mut vm, "(romacs--world-pend 'tab-width 0)"), "nil", "a built-in cannot wait");
}

/// `buffer-local-value' is a leaf that runs with no activation, but Lisp it
/// reaches sees its frame: here a pending variable's definition, through a
/// compiled call.
#[test]
fn lisp_a_leaf_reaches_sees_the_leafs_frame() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().expect("prelude");
    eval(&mut vm, "(progn (setq seen nil)
  (defun romacs--world-force (_i)
    (mapbacktrace (lambda (_evaluated function arguments _flags)
      (if (eq function 'buffer-local-value) (setq seen (cons (car arguments) seen)))))
    (set-default 'pe 1))
  (defvar pe 'stale)
  (defun read-pe () (buffer-local-value 'pe (current-buffer))))");
    assert_eq!(eval(&mut vm, "(romacs--world-pend 'pe 0)"), "t");
    assert_eq!(eval(&mut vm, "(list (read-pe) seen)"), "(1 (pe))");
    // Nothing of the leaf stays behind for a later call.
    assert_eq!(eval(&mut vm, "(progn (setq seen nil) (mapbacktrace (lambda (_e function _a _f) (if (eq function 'buffer-local-value) (setq seen t)))) seen)"), "nil");
}

/// A pending variable is the world's, not the process's: renewing what a
/// restore remakes (`Schema::renew`) leaves it pending, and its first
/// access still forces it.
#[test]
fn a_pending_variable_outlives_a_restores_renewal() {
    use romacs_vm::world::safe::Schema;
    let mut vm = Vm::new();
    vm.install_lisp_prelude().expect("prelude");
    eval(&mut vm, "(progn (defun romacs--world-force (_i) (set-default 'pr 'forced)) (defvar pr 'stale))");
    assert_eq!(eval(&mut vm, "(romacs--world-pend 'pr 0)"), "t");
    vm.renew();
    assert_eq!(eval(&mut vm, "pr"), "forced");
}

/// A debug-on-exit flag on the leaf's frame leaves with the frame, as it does
/// from any primitive's: the caller's return does not stop for it.
#[test]
fn a_flag_on_a_leafs_frame_leaves_with_the_frame() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().expect("prelude");
    eval(&mut vm, "(progn (setq stopped nil debugger (lambda (&rest _) (setq stopped t) 'replaced))
  (defun romacs--world-force (_i)
    (backtrace-debug 0 t 'buffer-local-value)
    (set-default 'pf 1))
  (defvar pf 'stale)
  (defun read-pf () (buffer-local-value 'pf (current-buffer))))");
    assert_eq!(eval(&mut vm, "(romacs--world-pend 'pf 0)"), "t");
    assert_eq!(eval(&mut vm, "(list (read-pf) stopped)"), "(1 nil)");
}

/// A form the init gives after a secret reader ran is noted withheld, never
/// as its text, which may hold what the reader returned.
#[test]
fn a_form_given_after_a_secret_reader_is_withheld_from_the_plan() {
    let dir = scratch("secret-recording");
    let init = dir.join("init.el");
    std::fs::write(&init, "(romacs--note-init-after-load 'lib-early \"(t . 1)\")
(defun auth-source-pass-get (&rest _) \"hunter2\")
(romacs--note-init-after-load 'lib-pw (concat \"(t setq pw \" (prin1-to-string (auth-source-pass-get 'secret \"x\")) \")\"))
").unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().expect("prelude");
    vm.record_init_loads();
    vm.load_file(&init).unwrap();
    let plan = vm.take_init_loads(&init).expect("a plan");
    let forms: Vec<(&str, &[String])> = plan.entries.iter().map(|e| (e.name.as_str(), e.forms.as_slice())).collect();
    assert_eq!(forms, [("lib-early", &["(t . 1)".to_owned()][..]), ("lib-pw", &["#<after a secret reader>".to_owned()][..])]);
    assert!(!plan.json().to_string().contains("hunter2"));
}

/// A configuration an earlier build of the plan live (DEFERRED) stays
/// deferred: a declared entry that loads its library, by feature or by
/// file, is live.
#[test]
fn a_configuration_left_live_carries_into_the_next_build() {
    let dir = scratch("deferred");
    std::fs::write(dir.join("lisp/lib-v.el"), "(provide 'lib-v)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-u.el"), "(require 'lib-v)\n(provide 'lib-u)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-w.el"), "(defvar lib-w 1)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-y.el"), "(load \"lib-w\" nil t)\n(provide 'lib-y)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-q.el"), "(defvar lib-q 1)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-z.el"), "(load \"lib-q\" nil t)\n(provide 'lib-z)\n").unwrap();
    // An absolute name through a symbolic link names the file's true name.
    std::os::unix::fs::symlink(dir.join("lisp"), dir.join("alias")).unwrap();
    let alias = dir.join("alias/lib-w.el").display().to_string();
    // A name without its suffix, which exists only with one.
    std::fs::write(dir.join("lisp/lib-s2.el"), "(defvar lib-s2 1)\n").unwrap();
    std::fs::write(dir.join("lisp/lib-t.el"), "(load \"lib-s2\" nil t)\n(provide 'lib-t)\n").unwrap();
    let bare_alias = dir.join("alias/lib-s2").display().to_string();
    std::fs::write(dir.join("lisp/session/romacs-loadup-bootstrap.el"),
                   format!("{BOOTSTRAP}(defun romacs--layer-load-declared (file) (load file nil t))\n")).unwrap();
    let declared = |name: &str| Entry { kind: Kind::Autoload, name: name.into(), file: None, noerror: true, load_path: 0, declared: true, forms: Vec::new() };
    let after = |name: &str, file: Option<&str>| Entry { kind: Kind::After, name: name.into(), file: file.map(str::to_owned), noerror: true, load_path: 0, declared: true, forms: vec!["(t . 1)".into()] };
    let plan = Plan::new(&dir.join("init.el"), vec![declared("lib-u"), declared("lib-y"), declared("lib-f"), declared("lib-z"), declared("lib-t")],
                         vec![vec![dir.join("lisp").display().to_string()]]).unwrap();
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("lisp/session/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let live = vm.world_layer_load(&plan, &[after("lib-v", None), after(&alias, Some(&alias)), after("lib-q", Some("lib-q")),
                                              after(&bare_alias, Some(&bare_alias))]);
    let _ = host::leave_build();
    assert_eq!(live, [(0, "loads lib-v, whose configuration runs live".to_owned()), (1, format!("loads {alias}, whose configuration runs live")),
                         (3, "loads lib-q, whose configuration runs live".to_owned()), (4, format!("loads {bare_alias}, whose configuration runs live"))]);
}
