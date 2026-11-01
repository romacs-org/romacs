//! Keyboard macros and the command loop (prims/keyboard.rs, rlisp/vm/keyboard.el)
//! against pinned GNU 32.0.50 in batch: the same cases, evaluated in order in
//! one GNU process and in one VM on GNU's loadup chain, must print the same.
//! Every case binds its keys in a private global map and empties *scratch*.
use romacs_vm::Vm;
use std::path::Path;

fn bootstrapped(root: &Path) -> Vm {
    let mut vm = Vm::new();
    // GNU batch, as the gnu-ert runner sets it before the chain loads.
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

/// Run BODY with a private global map over GNU's, then empty *scratch*.
const PRELUDE: &str = "(defmacro kt-case (&rest body)
  `(let ((kt-map (make-sparse-keymap)))
     (set-keymap-parent kt-map global-map)
     (use-global-map kt-map)
     (unwind-protect (progn ,@body)
       (use-global-map global-map)
       (with-current-buffer \"*scratch*\" (erase-buffer)))))";

const CASES: &[&str] = &[
    // The command loop runs in the selected window's buffer and leaves it current.
    r#"(with-temp-buffer (execute-kbd-macro "abc") (list (buffer-string) (buffer-name)))"#,
    // Hooks, the command variables, the key record and the macro's own state.
    r#"(let (log)
  (fset 'kt-a (lambda () (interactive)
    (push (list 'a (this-command-keys) (this-command-keys-vector) (this-single-command-keys)
                executing-kbd-macro executing-kbd-macro-index this-command real-this-command
                last-command last-command-event) log)))
  (global-set-key "a" 'kt-a)
  (let ((pre-command-hook (list (lambda () (push (list 'pre this-command) log))))
        (post-command-hook (list (lambda () (push (list 'post this-command) log))))
        (kbd-macro-termination-hook (list (lambda () (push (list 'term executing-kbd-macro real-this-command) log)))))
    (setq real-this-command 'outer this-command nil)
    (execute-kbd-macro "aa"))
  (list (nreverse log) executing-kbd-macro executing-kbd-macro-index real-this-command last-command this-command))"#,
    // record_char counts every recorded event outside a macro, an unread
    // one too (auto-save-interval reads the count); `no-record' events,
    // inhibit--record-char and macro input are not counted.
    r#"(let ((before num-nonmacro-input-events))
  (list (let ((unread-command-events (list ?a ?b (cons 'no-record ?c)))) (list (read-event) (read-event) (read-event)))
        (- num-nonmacro-input-events before)
        (let ((inhibit--record-char t) (unread-command-events (list ?d))) (read-event))
        (- num-nonmacro-input-events before)
        (progn (global-set-key "x" 'ignore) (execute-kbd-macro "xx") (- num-nonmacro-input-events before))))"#,
    // A command's error leaves the macro and reaches the caller; state is restored.
    r#"(progn (global-set-key "e" (lambda () (interactive) (error "Boom %d" executing-kbd-macro-index)))
  (list (condition-case e (execute-kbd-macro "aeaa") (error e)) executing-kbd-macro executing-kbd-macro-index
        (with-current-buffer "*scratch*" (buffer-string))))"#,
    r#"(progn (global-set-key "u" (lambda () (interactive) (user-error "Nope")))
  (condition-case e (execute-kbd-macro "au") (user-error (list 'user-error e))))"#,
    // read-event and read-char take the macro's next events; the rest run as commands.
    r#"(let (got) (global-set-key "r" (lambda () (interactive)
    (setq got (list (read-event) executing-kbd-macro-index (read-char) (this-command-keys) last-input-event))))
  (execute-kbd-macro "rxyz") (list got (with-current-buffer "*scratch*" (buffer-string))))"#,
    // A read with SECONDS takes the macro's event but leaves it out of the key record.
    r#"(let (got) (global-set-key "r" (lambda () (interactive)
    (setq got (list (read-event nil nil 0) (this-command-keys) (read-char nil nil 0) (this-command-keys)
                    (read-char-exclusive nil nil 1) (this-command-keys-vector) last-input-event))))
  (execute-kbd-macro "rxyz") (list got (with-current-buffer "*scratch*" (buffer-string))))"#,
    r#"(let (got) (global-set-key "c" (lambda () (interactive)
    (setq got (condition-case e (read-char) (error (list e unread-command-events))))))
  (list (condition-case e (execute-kbd-macro [?c f1]) (error e)) (prog1 got (setq unread-command-events nil))))"#,
    r#"(let (got) (global-set-key "c" (lambda () (interactive) (setq got (list (read-char-exclusive) (read-char)))))
  (execute-kbd-macro [?c f1 ?q ?\C-\M-a]) got)"#,
    r#"(let (got) (global-set-key "k" (lambda () (interactive)
    (setq got (list (read-key-sequence "") (this-command-keys-vector) (this-single-command-keys)
                    (read-key-sequence-vector "")))))
  (execute-kbd-macro (kbd "k C-x C-f M-x")) got)"#,
    // COUNT, prefix values for COUNT, and LOOPFUNC.
    r#"(let ((n 0)) (global-set-key "n" (lambda () (interactive) (setq n (1+ n))))
  (list (progn (execute-kbd-macro "n" 3) n)
        (progn (setq n 0) (execute-kbd-macro "nn" 0 (lambda () (< n 5))) n)
        (progn (setq n 0) (execute-kbd-macro "n" '(4)) n)
        (progn (setq n 0) (execute-kbd-macro "n" -1 (lambda () (< n 2))) n)))"#,
    r#"(progn (fset 'kt-macro "ab")
  (list (condition-case e (execute-kbd-macro 'kt-macro) (error e)) (with-current-buffer "*scratch*" (buffer-string))
        (condition-case e (execute-kbd-macro 12) (error e)) (condition-case e (execute-kbd-macro '(1 2)) (error e))))"#,
    // Prefix arguments through universal-argument's transient map.
    r#"(let (got) (global-set-key "p" (lambda (arg) (interactive "P")
    (push (list arg current-prefix-arg last-prefix-arg prefix-arg) got)))
  (execute-kbd-macro (kbd "C-u 3 p C-u p C-- 1 p M-5 p p")) (nreverse got))"#,
    // An undefined key runs `undefined' and the macro goes on; macro input is
    // not recorded in recent-keys.
    r#"(let ((n 0)) (global-set-key "n" (lambda () (interactive) (setq n (1+ n))))
  (list (execute-kbd-macro (kbd "n C-c z n")) n (key-description (recent-keys))))"#,
    // An unbound upper-case key falls back to its lower case.
    r#"(let ((m (make-sparse-keymap)) log)
  (define-key m "s" (lambda () (interactive) (push (list 's this-command-keys-shift-translated (this-command-keys)) log)))
  (with-temp-buffer (switch-to-buffer (current-buffer)) (use-local-map m)
    (execute-kbd-macro "sS")
    (define-key m "S" (lambda () (interactive) (push (list 'S this-command-keys-shift-translated (this-command-keys)) log)))
    (execute-kbd-macro "S")
    (switch-to-buffer "*scratch*"))
  (nreverse log))"#,
    // Command remapping.
    r#"(let (got) (fset 'kt-orig (lambda () (interactive) (setq got (list 'orig this-command))))
  (fset 'kt-new (lambda () (interactive) (setq got (list 'new this-command this-original-command real-this-command))))
  (global-set-key "o" 'kt-orig) (global-set-key [remap kt-orig] 'kt-new)
  (execute-kbd-macro "o") got)"#,
    // set-transient-map: one key, then with KEEP-PRED t and ON-EXIT.
    r#"(let ((m (make-sparse-keymap)) log)
  (define-key m "w" (lambda () (interactive) (push 'tw log)))
  (global-set-key "w" (lambda () (interactive) (push 'gw log)))
  (global-set-key "t" (lambda () (interactive) (set-transient-map m)))
  (global-set-key "T" (lambda () (interactive) (set-transient-map m t (lambda () (push 'exit log)))))
  (execute-kbd-macro "twwtw") (push '| log) (execute-kbd-macro "Twwaw")
  (list (nreverse log) overriding-terminal-local-map (with-current-buffer "*scratch*" (buffer-string))))"#,
    // C-g quits the macro and the caller.
    r#"(list (condition-case e (execute-kbd-macro (kbd "a C-g a")) (quit (list 'quit e executing-kbd-macro)))
  (with-current-buffer "*scratch*" (buffer-string)))"#,
    // A macro run by a command of a macro.
    r#"(let (got) (global-set-key "i" (lambda () (interactive)
    (setq got (list executing-kbd-macro (progn (execute-kbd-macro "j") (car got)) executing-kbd-macro executing-kbd-macro-index))))
  (global-set-key "j" (lambda () (interactive) (setq got (list (list executing-kbd-macro executing-kbd-macro-index (recursion-depth))))))
  (execute-kbd-macro "ii") got)"#,
    // unread-command-events come before the macro's own events.
    r#"(let (got) (global-set-key "q" (lambda () (interactive) (setq unread-command-events (list ?z))))
  (global-set-key "z" (lambda () (interactive) (push (list 'z executing-kbd-macro-index) got)))
  (execute-kbd-macro "qza") (list (nreverse got) (with-current-buffer "*scratch*" (buffer-string))))"#,
    // Translation: function-key-map for an undefined sequence (return is
    // RET at the prompt), key-translation-map always (C-x 8 ' e), and the
    // shift fallback on a function key.
    r#"(let (got) (global-set-key "m" (lambda () (interactive) (setq got (read-string "P: "))))
  (execute-kbd-macro [?m ?h ?i return]) got)"#,
    r#"(progn (execute-kbd-macro (kbd "C-x 8 ' e")) (with-current-buffer "*scratch*" (buffer-string)))"#,
    r#"(let (got) (global-set-key [return] (lambda () (interactive)
    (setq got (list this-command-keys-shift-translated (this-command-keys-vector)))))
  (execute-kbd-macro [S-return]) got)"#,
    // A pending translation prefix in any of the three maps keeps the key
    // sequence open: C-x @ c is function-key-map's control prefix, whose
    // function reads the next event. A prefix the macro ends inside is
    // dropped; an unbound prefix with no translation left is discarded and
    // reading goes on from the next key (C-x @ then C-x y).
    r#"(let (got) (global-set-key "\C-a" (lambda () (interactive) (push (list 'ca (this-command-keys-vector) (this-single-command-keys)) got)))
  (global-set-key (kbd "C-x y") (lambda () (interactive) (push (list 'xy (this-command-keys-vector)) got)))
  (list (execute-kbd-macro (kbd "C-x @ c a b")) (execute-kbd-macro (kbd "C-x @ C-x y"))
        (condition-case e (execute-kbd-macro (kbd "a C-x @")) (error e))
        (nreverse got) (with-current-buffer "*scratch*" (buffer-string))))"#,
    r#"(let ((key-translation-map (make-sparse-keymap)))
  (define-key key-translation-map (kbd "C-c q z") "Z")
  (list (execute-kbd-macro (kbd "a C-c q z b")) (execute-kbd-macro (kbd "C-c q y c"))
        (condition-case e (execute-kbd-macro (kbd "d C-c q")) (error e)) (with-current-buffer "*scratch*" (buffer-string))))"#,
    r#"(let ((input-decode-map (make-sparse-keymap)) got)
  (define-key input-decode-map (kbd "C-c q z") [f7]) (define-key input-decode-map (kbd "q z") [f7])
  (global-set-key [f7] (lambda () (interactive) (push (list 'f7 (this-command-keys-vector)) got)))
  (list (execute-kbd-macro (kbd "a C-c q z b q z")) (condition-case e (execute-kbd-macro (kbd "c C-c q")) (error e))
        got (with-current-buffer "*scratch*" (buffer-string))))"#,
    // When a pending prefix fails, each map's scan moves on one event and
    // a later map translates a shorter sequence inside it; the translated
    // keys are reread through the maps after it. C-c q y: key-translation-map
    // turns C-c into A behind input-decode-map's C-c q z; behind
    // function-key-map's, it turns q into Q, which is unbound and looked up
    // as q, then restored.
    r#"(let ((input-decode-map (make-sparse-keymap)) (key-translation-map (make-sparse-keymap)) got)
  (define-key input-decode-map [3 113 122] [f7]) (define-key key-translation-map [3] "A")
  (global-set-key "k" (lambda () (interactive) (push (list (read-key-sequence-vector "") (this-command-keys-vector)) got)))
  (global-set-key [f7] (lambda () (interactive) (push (list 'f7 (this-command-keys-vector)) got)))
  (list (execute-kbd-macro [?k 3 113 121 ?k 3 113 122]) (execute-kbd-macro [3 113 121 ?b 3 113 122])
        (nreverse got) (with-current-buffer "*scratch*" (buffer-string))))"#,
    r#"(let ((local-function-key-map (make-sparse-keymap)) (key-translation-map (make-sparse-keymap)) got)
  (define-key local-function-key-map [3 113 122] [f7]) (define-key key-translation-map [113] "Q")
  (global-set-key "k" (lambda () (interactive) (push (list (read-key-sequence-vector "") (this-command-keys-vector)) got)))
  (global-set-key [f7] (lambda () (interactive) (push (list 'f7 (this-command-keys-vector)) got)))
  (list (execute-kbd-macro [?k 3 113 121 ?k 3 113 122]) (execute-kbd-macro [3 113 121 ?b 3 113 122])
        (nreverse got) (with-current-buffer "*scratch*" (buffer-string))))"#,
    r#"(list (progn (set--this-command-keys "\370abc") (this-command-keys-vector))
  (progn (set--this-command-keys "") (this-command-keys-vector)) (condition-case e (set--this-command-keys 1) (error e)))"#,
    // The minibuffer reads its input from the macro.
    r#"(let (got) (global-set-key "m" (lambda () (interactive)
    (setq got (list (read-string "P: ") (recursion-depth) (minibuffer-depth) (this-command-keys)))))
  (execute-kbd-macro (kbd "m h i RET")) got)"#,
    r#"(let (got) (global-set-key "m" (lambda () (interactive) (setq got (read-from-minibuffer "P: " "init"))))
  (execute-kbd-macro (kbd "m x DEL DEL y RET")) got)"#,
    r#"(let (got) (global-set-key "m" (lambda () (interactive)
    (setq got (list (minibuffer-prompt) (minibuffer-prompt-end) (minibuffer-contents) (completing-read "C: " '("alpha" "beta") nil t)))))
  (add-hook 'minibuffer-setup-hook (lambda () (setq kt-in (list (minibuffer-prompt) (minibuffer-prompt-end) (innermost-minibuffer-p) (minibuffer-innermost-command-loop-p)))))
  (execute-kbd-macro (kbd "m b e t a RET")) (setq minibuffer-setup-hook (cdr minibuffer-setup-hook)) (list got kt-in))"#,
    r#"(let (got) (global-set-key "m" (lambda () (interactive) (setq got (list (y-or-n-p "Q? ") (y-or-n-p "R? ")))))
  (execute-kbd-macro (kbd "m y RET n RET")) got)"#,
    r#"(let (got) (global-set-key "m" (lambda () (interactive) (setq got (list (yes-or-no-p "Q?") (yes-or-no-p "R? ")))))
  (execute-kbd-macro (kbd "m y e s RET n o RET")) got)"#,
    // The macro ending inside the minibuffer ends the prompt.
    r#"(let (got) (global-set-key "m" (lambda () (interactive) (setq got (read-string "P: "))))
  (execute-kbd-macro "mab") got)"#,
    // C-g at the prompt quits that command only; the macro goes on.
    r#"(let ((got 'unset) errors) (global-set-key "m" (lambda () (interactive) (setq got (read-string "P: "))))
  (let ((command-error-function (lambda (data context _signal) (push (list data context) errors))))
    (list (condition-case e (execute-kbd-macro (kbd "m a C-g a")) (quit (list 'quit e)))
          got errors (with-current-buffer "*scratch*" (buffer-string)))))"#,
    // Recording: keys read as new input (t . EVENT) are stored while a
    // macro is defined, kmacro.el's C-x ( C-x ) C-x e and F3 F4 drive it,
    // and the keys that end it are left out.
    r#"(progn (global-set-key [f9] 'exit-recursive-edit) (set-buffer "*scratch*")
  (setq unread-command-events (mapcar (lambda (e) (cons t e)) (list ?\C-x ?\( ?a ?b ?\C-x ?\) ?\C-x ?e 'f9)))
  (recursive-edit) (list last-kbd-macro defining-kbd-macro (buffer-string)))"#,
    r#"(progn (global-set-key [f9] 'exit-recursive-edit) (set-buffer "*scratch*")
  (setq unread-command-events (mapcar (lambda (e) (cons t e)) (list 'f3 ?x ?y 'f4 'f4 'f4 'f9)))
  (recursive-edit) (list last-kbd-macro defining-kbd-macro (buffer-string)))"#,
    // A prefix argument's keys stay in the macro with the command they prefix.
    r#"(progn (global-set-key [f9] 'exit-recursive-edit) (set-buffer "*scratch*")
  (setq unread-command-events (mapcar (lambda (e) (cons t e)) (list ?\C-x ?\( ?\C-u ?3 ?z ?\C-x ?\) 'f9)))
  (recursive-edit) (list last-kbd-macro (buffer-string)))"#,
    // The primitives: their refusals, appending, a repeat count, and
    // call-last-kbd-macro keeping the commands' view of the last command.
    r#"(list (condition-case e (end-kbd-macro) (error e))
  (condition-case e (let ((last-kbd-macro nil)) (call-last-kbd-macro)) (error e))
  (progn (start-kbd-macro nil) (prog1 (list defining-kbd-macro (condition-case e (start-kbd-macro nil) (error e))
                                           (condition-case e (call-last-kbd-macro) (error e)))
                                 (end-kbd-macro)))
  last-kbd-macro
  (let ((last-kbd-macro "ab")) (start-kbd-macro t t) (store-kbd-macro-event ?c) (end-kbd-macro) last-kbd-macro)
  (with-temp-buffer (let ((last-kbd-macro "q") (last-command 'previous)) (call-last-kbd-macro 3) (list (buffer-string) this-command))))"#,
];

fn gnu_results(root: &Path) -> Vec<String> {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let home = std::env::temp_dir().join(format!("kbd-macro-gnu-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let program = CASES.iter().map(|case| format!("(kt-case (princ (prin1-to-string {case}))) (princ \"\\n\\x1e\\n\")"))
        .collect::<String>();
    let output = std::process::Command::new(binary).env("HOME", &home).current_dir(root)
        .args(["-Q", "--batch", "--eval", &format!("(progn {PRELUDE} {program})")]).output().unwrap();
    std::fs::remove_dir_all(&home).ok();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    // Batch `ding' writes BEL, and a `minibuffer-quit' its message, to stdout.
    let stdout = String::from_utf8(output.stdout).unwrap().replace('\x07', "");
    stdout.split("\n\x1e\n").map(|s| s.trim_start_matches("Quit").to_owned()).take(CASES.len()).collect()
}

#[test]
fn keyboard_macros_match_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let expected = gnu_results(&root);
    assert_eq!(expected.len(), CASES.len());
    let mut vm = bootstrapped(&root);
    vm.eval_source(PRELUDE).unwrap();
    for (case, gnu) in CASES.iter().zip(&expected) {
        let printed = match vm.eval_source(&format!("(kt-case (prin1-to-string {case}))")) {
            Ok(value) => value.display().unwrap(),
            Err(e) => format!("VM-ERROR {e}"),
        };
        let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
        assert_eq!(&actual, gnu, "{case}");
    }
}

/// DIVERGENCES.md VM2-KBD1: where batch GNU would block for terminal input
/// (no unread event, no macro input left, no host keyboard), VM2 signals
/// `end-of-file`, from the innermost recursive edit when there is one.
#[test]
fn reading_past_the_input_signals_end_of_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped(&root);
    vm.eval_source(PRELUDE).unwrap();
    for (source, expected) in [
        ("(condition-case e (read-event) (error e))", "(end-of-file \"Error reading from stdin\")"),
        ("(list (read-event nil nil 0.01) (read-char nil nil 0) (read-char-exclusive nil nil 0))", "(nil nil nil)"),
        ("(let (got) (global-set-key \"r\" (lambda () (interactive) (setq got (list (read-event) (read-event)))))
            (list (condition-case e (execute-kbd-macro \"rx\") (error e)) got))",
         "((end-of-file \"Error reading from stdin\") nil)"),
        // A command's error at the prompt ends the macro; the prompt then has
        // no input left and leaves with end-of-file.
        ("(let (got) (global-set-key \"m\" (lambda () (interactive) (setq got (read-string \"P: \"))))
            (global-set-key \"e\" (lambda () (interactive) (error \"Boom\")))
            (let ((minibuffer-local-map (let ((m (make-sparse-keymap))) (set-keymap-parent m minibuffer-local-map) (define-key m \"e\" (key-binding \"e\")) m)))
              (list (condition-case e (execute-kbd-macro \"mxe\") (error e)) got)))",
         "((end-of-file \"Error reading from stdin\") nil)"),
    ] {
        let printed = vm.eval_source(&format!("(kt-case (prin1-to-string {source}))")).unwrap().display().unwrap();
        let actual: String = serde_json::from_str(&printed).unwrap();
        assert_eq!(actual, expected, "{source}");
    }
}

/// A bare VM runs macros through the command loop's stand-ins for
/// `command-execute` and `undefined` (VM2-KBD1), on its native commands.
/// GNU 32.0.50 -Q --batch, same form: ("aXaXbb" "yo" 0).
#[test]
fn bare_vm_runs_a_macro() {
    let mut vm = Vm::new();
    let printed = vm.eval_source("(list (with-temp-buffer (execute-kbd-macro \"ab\\C-bX\" 2) (buffer-string)) \
        (let ((executing-kbd-macro t) (unread-command-events (list ?y ?o ?\\r))) (read-string \"P: \")) \
        (recursion-depth))").unwrap().display().unwrap();
    assert_eq!(printed, "(\"aXaXbb\" \"yo\" 0)");
}
