//! The active keymaps and the lookups over them (keymap.c
//! `current-active-maps`, `key-binding`, `minor-mode-key-binding`,
//! `current-minor-mode-maps`, `command-remapping`) against the pinned GNU,
//! and how much Lisp a key's lookup runs.
use romacs_vm::Vm;

/// Keymaps are named by their prompts, so a list of them prints the same
/// on both sides; AK-IN runs BODY in a buffer of TEXT.
const SETUP: &str = r#"
(defvar ak-a nil) (defvar ak-b nil) (defvar ak-c nil) (defvar ak-em nil) (defvar ak-emulation nil)
(defvaralias 'ak-a-alias 'ak-a)
(defun ak-map (name &rest bindings)
  (let ((map (make-sparse-keymap name)))
    (while bindings
      (define-key map (car bindings) (car (cdr bindings)))
      (setq bindings (cdr (cdr bindings))))
    map))
(defun ak-names (maps)
  (if (consp maps)
      (mapcar (lambda (map) (cond ((eq map (current-global-map)) 'G) ((keymapp map) (keymap-prompt map)) (t map))) maps)
    maps))
(defalias 'ak-symbol-map (ak-map "SYM"))
(defun ak-in (text body)
  (let ((buffer (get-buffer-create "ak")))
    (unwind-protect
        (save-current-buffer
          (set-buffer buffer)
          (insert text)
          (goto-char 1)
          (ak-try body))
      (kill-buffer buffer))))
(defun ak-collect () nil)
(defun ak-try (thunk) (condition-case e (funcall thunk) (error (list 'signal (prin1-to-string e)))))
(defun ak-modes (body)
  "BODY with three minor modes on, an overriding entry and two emulation alists."
  (setq ak-a t ak-b t ak-c nil ak-em t)
  (let ((minor-mode-map-alist
         (list (cons 'ak-a (ak-map "A" "x" (ak-map "AX" "y" 'ak-a-xy) "r" 'ak-a-r))
               (cons 'ak-b (ak-map "B" "x" (ak-map "BX") "r" 'ak-b-r "n" 'ak-b-n))
               (cons 'ak-c (ak-map "C" "x" 'ak-c-x))
               (cons 'ak-void (ak-map "V" "x" 'ak-void-x))
               (cons 'ak-a 'ak-symbol-map) (cons 'ak-a 'ak-no-function) (cons 'ak-a nil)
               'junk (cons "string" (ak-map "S")) (cons t (ak-map "T" "x" 'ak-t-x)) (cons nil (ak-map "N"))))
        (minor-mode-overriding-map-alist (list (cons 'ak-b (ak-map "OB" "n" 'ak-ob-n)) (cons 'ak-c (ak-map "OC"))))
        (emulation-mode-map-alists (list 'ak-emulation (list (cons 'ak-em (ak-map "E2" "x" 'ak-e2-x))) 'ak-void-alist nil 5))
        (ak-emulation (list (cons 'ak-em (ak-map "E1" [t] 'ak-e1-default)))))
    (funcall body)))
"#;

fn expressions() -> Vec<String> {
    [sources_and_positions(), another_windows_buffer(), key_bindings(), filters_minor_modes_and_remapping()].concat()
}

fn sources_and_positions() -> Vec<String> {
    let mut cases: Vec<String> = Vec::new();
    // Every source at once, with and without OLP: the terminal map, the
    // `keymap' property, emulation maps, the overriding alist, the minor
    // modes it does not shadow, the `local-map' property, the global map.
    cases.push(r#"(ak-in "abcdef" (lambda ()
  (use-local-map (ak-map "L"))
  (put-text-property 3 4 'keymap (ak-map "K")) (put-text-property 3 4 'local-map (ak-map "LM"))
  (ak-modes (lambda ()
    (list (ak-names (current-active-maps)) (ak-names (current-active-maps t))
          (progn (goto-char 3) (list (ak-names (current-active-maps)) (ak-names (current-active-maps t))))
          (let ((overriding-local-map (ak-map "OL")))
            (list (ak-names (current-active-maps)) (ak-names (current-active-maps t))
                  (let ((overriding-terminal-local-map (ak-map "OT")))
                    (list (ak-names (current-active-maps)) (ak-names (current-active-maps t))))))
          (let ((overriding-terminal-local-map (ak-map "OT"))) (ak-names (current-active-maps t)))
          (ak-names (current-minor-mode-maps))
          (progn (setq ak-a nil ak-b nil ak-em nil) (list (ak-names (current-active-maps)) (current-minor-mode-maps))))))))"#.into());
    // A minor mode's keymap that is no keymap is still listed.
    cases.push(r#"(ak-in "abc" (lambda ()
  (let ((minor-mode-map-alist (list (cons 'ak-a 5) (cons 'ak-a "map"))) (emulation-mode-map-alists 'ak-emulation))
    (setq ak-a t) (list (ak-names (current-active-maps)) (current-minor-mode-maps)))))"#.into());
    // The properties: at point, at an integer and at a marker, on text and
    // on overlays, what an insertion would inherit, at the end of the text.
    for probe in ["(progn (goto-char p) (current-active-maps))", "(current-active-maps nil p)", "(current-active-maps t (copy-marker p))"] {
        cases.push(format!(r#"(ak-in "abcdefgh" (lambda ()
  (use-local-map (ak-map "L"))
  (put-text-property 2 3 'keymap (ak-map "K")) (put-text-property 2 3 'local-map (ak-map "LM"))
  (put-text-property 3 4 'keymap 'ak-symbol-map) (put-text-property 4 5 'local-map 'junk) (put-text-property 4 5 'keymap 7)
  (overlay-put (make-overlay 5 6) 'keymap (ak-map "OK"))
  (overlay-put (make-overlay 6 7 nil nil t) 'local-map (ak-map "OLM"))
  (overlay-put (make-overlay 8 9 nil t nil) 'keymap (ak-map "FRONT"))
  (put-text-property 8 9 'keymap (ak-map "END"))
  (mapcar (lambda (p) (ak-names {probe})) '(1 2 3 4 5 6 7 8 9))))"#));
    }
    // Narrowing hides neither the properties nor what they inherit from,
    // and a position outside it is out of range.
    cases.push(r#"(ak-in "abcdef" (lambda ()
  (put-text-property 3 4 'keymap (ak-map "K")) (put-text-property 5 6 'local-map (ak-map "LM"))
  (narrow-to-region 4 5)
  (list (ak-names (current-active-maps)) (progn (goto-char 5) (ak-names (current-active-maps)))
        (ak-names (current-active-maps nil 4)) (ak-names (current-active-maps nil (copy-marker 5)))
        (ak-try (lambda () (current-active-maps nil 3))) (ak-try (lambda () (current-active-maps nil 6)))
        (ak-try (lambda () (current-active-maps nil (copy-marker 3))))
        (ak-try (lambda () (key-binding "a" nil nil 6)))
        (list (point) (point-min) (point-max)))))"#.into());
    cases.push(r#"(ak-in "abc" (lambda ()
  (list (ak-try (lambda () (current-active-maps nil 0))) (ak-try (lambda () (current-active-maps nil 5)))
        (ak-try (lambda () (current-active-maps nil (make-marker))))
        (let ((overriding-local-map (ak-map "OL")))
          (list (ak-names (current-active-maps t 99)) (ak-try (lambda () (current-active-maps nil 99)))))
        (ak-names (current-active-maps nil 2.0)) (ak-names (current-active-maps nil 'symbol)) (ak-names (current-active-maps nil "string")))))"#.into());
    cases
}

fn another_windows_buffer() -> Vec<String> {
    vec![
    // A window, or a click in one, means its buffer's variables and maps.
    r#"(let ((other (get-buffer-create "ak-other")) (window (selected-window)) (old (window-buffer)))
  (unwind-protect
      (ak-in "abcdef" (lambda ()
        (use-local-map (ak-map "L"))
        (set-window-buffer window other)
        (save-current-buffer
          (set-buffer other)
          (insert "0123456789") (goto-char 4)
          (use-local-map (ak-map "OTHER"))
          (put-text-property 4 5 'keymap (ak-map "K4")) (put-text-property 7 8 'keymap (ak-map "K7"))
          (put-text-property 7 8 'local-map (ak-map "LM7"))
          (set (make-local-variable 'ak-a) t)
          (set (make-local-variable 'text-property-default-nonsticky) '((keymap . t)))
          (set (make-local-variable 'minor-mode-overriding-map-alist) (list (cons 'ak-b (ak-map "OB")))))
        (setq ak-a nil ak-b t)
        (let ((minor-mode-map-alist (list (cons 'ak-a (ak-map "A")) (cons 'ak-b (ak-map "B")) (cons 'ak-a-alias (ak-map "ALIAS"))))
              (string (propertize "xyz" 'keymap (ak-map "SK") 'local-map (ak-map "SLM"))))
          (put-text-property 0 1 'keymap nil string) (put-text-property 0 1 'local-map nil string)
          (list (ak-names (current-active-maps)) (ak-names (current-active-maps nil window))
                (ak-names (current-active-maps nil (list window 8 '(0 . 0) 0 nil 8)))
                (ak-names (current-active-maps nil (list window 5 '(0 . 0) 0 nil 5)))
                (progn (goto-char 5) (ak-names (current-active-maps)))
                (ak-names (current-active-maps nil (list window 7 '(0 . 0) 0 nil 7)))
                (ak-names (current-active-maps nil (list window 7)))
                (ak-names (current-active-maps nil (list window '(7 . 8))))
                (ak-names (current-active-maps nil (list window 'mode-line '(0 . 0) 0)))
                (ak-names (current-active-maps nil (list window 'mode-line '(0 . 0) 0 (cons string 1))))
                (ak-names (current-active-maps nil (list window 7 '(0 . 0) 0 (cons string 1) 7)))
                (ak-names (current-active-maps nil (list window 7 '(0 . 0) 0 (cons string 0) 7)))
                (ak-names (current-active-maps nil (list window 7 '(0 . 0) 0 (cons string 3) 7)))
                (ak-names (current-active-maps nil (list window 7 '(0 . 0) 0 (cons string -1) 7)))
                (ak-names (current-active-maps nil (list window 7 '(0 . 0) 0 (cons 'symbol 1) 7)))
                (ak-names (current-active-maps nil (list window 7 '(0 . 0) 0 'area 0)))
                (ak-names (current-active-maps nil (list window 7 '(0 . 0) 0 nil 99)))
                (ak-names (current-active-maps nil (list window 7 '(0 . 0) 0 nil 11)))
                (ak-names (current-active-maps nil (list nil 3 '(0 . 0) 0 nil 3)))
                (ak-names (current-active-maps nil (list 'window 3)))
                (ak-try (lambda () (current-active-maps nil (cons window 7))))
                (ak-try (lambda () (current-active-maps nil (cons window (cons 7 8)))))
                (ak-names (current-active-maps t (list window 7)))
                (let ((overriding-local-map (ak-map "OL"))) (ak-names (current-active-maps t (list window 7))))
                (eq (current-buffer) (get-buffer "ak"))))))
    (set-window-buffer window old)
    (kill-buffer other)))"#.into(),
    ]
}

fn key_bindings() -> Vec<String> {
    vec![
    // key-binding: prefixes, a key too long, default bindings, remapping,
    // an explicit position, things that are not commands.
    r#"(ak-in "abcdef" (lambda ()
  (use-global-map (ak-map "G" "g" 'ak-g "p" (ak-map "GP" "q" 'ak-g-pq) [t] 'ak-g-default "s" "macro" "v" [1 2] "l" '(lambda () (interactive)) "r" 'ak-a-r
                          [remap ak-g] 'ak-g-remapped [remap ak-a-xy] 'ak-xy-remapped [remap ak-e1-default] 'ak-default-remapped
                          [remap ak-b-n] 5 [remap ak-g-remapped] 'ak-twice))
  (use-local-map (ak-map "L" "g" nil "k" 'ak-l-k [remap ak-l-k] (ak-map "REMAP-PREFIX")))
  (put-text-property 3 4 'keymap (ak-map "K" "g" 'ak-k-g "k" 'ak-k-k))
  (ak-modes (lambda ()
    (list (key-binding "g") (key-binding "g" nil t) (key-binding [?g]) (key-binding "g" nil nil 3) (key-binding "k") (key-binding "k" nil nil 3)
          (ak-names (key-binding "p")) (key-binding "pq") (key-binding "pqr") (key-binding "gq") (key-binding "x")
          (ak-names (key-binding "x")) (key-binding "xy") (key-binding "xy" nil t) (key-binding "xyz")
          (key-binding "z") (key-binding "z" t) (key-binding "z" t t) (key-binding "zz" t)
          (key-binding "s") (key-binding "v") (key-binding "l") (key-binding "r") (key-binding "n") (key-binding "n" nil t)
          (key-binding []) (ak-names (key-binding [] nil nil 3)) (ak-names (key-binding "")) (ak-names (key-binding "" nil nil (copy-marker 3)))
          (key-binding [remap ak-g]) (key-binding [ak-unbound-event]) (key-binding [ak-unbound-event] t)
          (let ((overriding-local-map (ak-map "OL" "k" 'ak-ol-k))) (list (key-binding "k") (key-binding "g") (key-binding "x")))
          (let ((overriding-terminal-local-map (ak-map "OT" "k" 'ak-ot-k))) (list (key-binding "k") (key-binding "g") (key-binding "xy")))
          (ak-try (lambda () (key-binding 5))) (ak-try (lambda () (key-binding "g" nil nil 99)))
          (ak-try (lambda () (key-binding 'symbol))))))))"#.into(),
    // A key that starts with a mouse click is looked up where it clicked,
    // in the clicked string first; POSITION overrides the event's own.
    r#"(let ((other (get-buffer-create "ak-other")) (window (selected-window)) (old (window-buffer)))
  (unwind-protect
      (ak-in "abcdef" (lambda ()
        (use-global-map (ak-map "G" [mouse-1] 'ak-global-click [mode-line mouse-1] 'ak-global-line))
        (set-window-buffer window other)
        (save-current-buffer
          (set-buffer other)
          (insert "0123456789")
          (put-text-property 2 3 'keymap (ak-map "K2" [mouse-1] 'ak-click-2 [mode-line mouse-1] 'ak-line-2 [remap ak-click-3] 'ak-remapped-at-2))
          (put-text-property 3 4 'keymap (ak-map "K3" [mouse-1] 'ak-click-3 [mouse-2] 'ak-other-button)))
        (let* ((string (propertize "xyz" 'keymap (ak-map "SK" [mouse-1] 'ak-click-string)))
               (at2 (list window 2 '(0 . 0) 0 nil 2)) (at3 (list window 3 '(0 . 0) 0 nil 3))
               (on-string (list window 'mode-line '(0 . 0) 0 (cons string 1) nil))
               (click (lambda (button posn) (vector (list button posn))))
               (probe (lambda ()
                        (list (key-binding (funcall click 'mouse-1 at2)) (key-binding (funcall click 'mouse-1 at3))
                              (key-binding (funcall click 'mouse-1 at3) nil t)
                              (key-binding (funcall click 'mouse-1 on-string))
                              (key-binding (vector 'mode-line (list 'mouse-1 at2)))
                              (key-binding (vector 'mode-line (list 'mouse-1 at2)) nil nil at3)
                              (key-binding (funcall click 'mouse-1 at2) nil nil at3) (key-binding [mouse-1] nil nil at2)
                              (key-binding [mouse-1] nil nil window) (key-binding (funcall click 'mouse-2 at3))
                              (key-binding (vector '(mouse-1))) (key-binding (vector '(mouse-1 . 5))) (ak-try (lambda () (key-binding (vector (list 5 at2)))))
                              (ak-names (key-binding (vector 'mode-line))) (key-binding (vector nil (list 'mouse-1 at2)))))))
          (list (funcall probe)
                (progn (put 'mouse-1 'event-kind 'mouse-click) (funcall probe))))))
    (put 'mouse-1 'event-kind nil)
    (set-window-buffer window old)
    (kill-buffer other)))"#.into(),
    ]
}

fn filters_minor_modes_and_remapping() -> Vec<String> {
    vec![
    // A menu item's filter runs during the lookup, and its result is
    // remapped like any command.
    r#"(ak-in "abc" (lambda ()
  (use-global-map (ak-map "G" "f" '(menu-item "F" ak-raw :filter (lambda (command) (if (eq command 'ak-raw) 'ak-filtered 'ak-wrong)))
                          [remap ak-filtered] '(menu-item "R" ak-raw :filter (lambda (_) 'ak-filtered-remap))))
  (list (key-binding "f") (key-binding "f" nil t) (command-remapping 'ak-filtered)
        (lookup-key (current-active-maps) "f") (minor-mode-key-binding "f"))))"#.into(),
    // Filters that allocate, and in Romacs collect, in the middle of a
    // lookup: the prefixes found before them survive, and a filter that
    // changes the active maps changes where the command is remapped.
    r#"(ak-in "abc" (lambda ()
  (setq ak-a t ak-b t ak-c t)
  (let* ((churn (lambda (map) (ak-collect) (make-list 200 (make-string 20 ?x)) (ak-collect) map))
         (minor-mode-map-alist
          (list (cons 'ak-a (ak-map "A" "x" (ak-map "AX")))
                (cons 'ak-b (ak-map "B" "x" (list 'menu-item "B" (ak-map "BX") :filter churn)))
                (cons 'ak-c (ak-map "C" "x" (list 'menu-item "C" (ak-map "CX") :filter churn) "y" (list 'menu-item "Y" 'ak-c-y :filter churn)))))
         (minor-mode-overriding-map-alist nil) (emulation-mode-map-alists nil)
         (names (lambda (found) (mapcar (lambda (entry) (cons (car entry) (if (keymapp (cdr entry)) (keymap-prompt (cdr entry)) (cdr entry)))) found))))
    (use-global-map (ak-map "G" "f" (list 'menu-item "F" 'ak-raw :filter (lambda (command) (ak-collect) (use-local-map (ak-map "L2" [remap ak-raw] 'ak-late)) command))
                            [remap ak-c-y] (list 'menu-item "R" 'ak-y-remapped :filter churn)))
    (use-local-map (ak-map "L1" [remap ak-raw] 'ak-early))
    (list (funcall names (minor-mode-key-binding "x")) (funcall names (minor-mode-key-binding "y"))
          (ak-names (key-binding "x")) (key-binding "y") (key-binding "y" nil t)
          (command-remapping 'ak-raw) (key-binding "f") (keymap-prompt (current-local-map)) (command-remapping 'ak-raw)))))"#.into(),
    // A unibyte buffer's properties, and what an insertion inherits there.
    r#"(ak-in "abcd" (lambda ()
  (set-buffer-multibyte nil)
  (use-local-map (ak-map "L"))
  (put-text-property 1 2 'keymap (ak-map "K")) (put-text-property 3 4 'local-map (ak-map "LM"))
  (list (mapcar (lambda (p) (ak-names (current-active-maps nil p))) '(1 2 3 4 5))
        (mapcar (lambda (p) (goto-char p) (ak-names (current-active-maps))) '(1 2 3 4 5))
        (mapcar (lambda (p) (and (get-pos-property p 'keymap) t)) '(1 2 3 4 5))
        (progn (narrow-to-region 2 3) (goto-char 2) (list (ak-names (current-active-maps)) (point-min) (point-max))))))"#.into(),
    // minor-mode-key-binding: the prefixes, or the first binding alone.
    r#"(ak-modes (lambda ()
  (let ((names (lambda (found) (mapcar (lambda (entry) (cons (car entry) (if (keymapp (cdr entry)) (keymap-prompt (cdr entry)) (cdr entry)))) found))))
    (list (funcall names (minor-mode-key-binding "x")) (funcall names (minor-mode-key-binding "r")) (funcall names (minor-mode-key-binding "n"))
          (funcall names (minor-mode-key-binding "xy")) (minor-mode-key-binding "xyz") (minor-mode-key-binding "q")
          (funcall names (minor-mode-key-binding "q" t)) (funcall names (minor-mode-key-binding [ak-event] t))
          (funcall names (minor-mode-key-binding ""))
          (progn (setq ak-em nil) (funcall names (minor-mode-key-binding "x")))
          (progn (setq ak-a nil) (funcall names (minor-mode-key-binding "x")))
          (ak-try (lambda () (minor-mode-key-binding 5)))
          (let ((minor-mode-map-alist nil) (minor-mode-overriding-map-alist nil) (emulation-mode-map-alists nil))
            (list (minor-mode-key-binding "x") (ak-try (lambda () (minor-mode-key-binding 5)))))
          (let ((minor-mode-map-alist (list (cons 'ak-b 5))) (minor-mode-overriding-map-alist nil) (emulation-mode-map-alists nil))
            (ak-try (lambda () (minor-mode-key-binding "x"))))))))"#.into(),
    // command-remapping: in the active maps, at a position, in KEYMAPS.
    r#"(ak-in "abcdef" (lambda ()
  (use-global-map (ak-map "G" [remap ak-one] 'ak-global-one [remap ak-two] 'ak-global-two [remap ak-prefix] (ak-map "P") [remap ak-number] 5 [remap nil] 'ak-nil [remap t] 'ak-t))
  (use-local-map (ak-map "L" [remap ak-one] 'ak-local-one))
  (put-text-property 3 4 'keymap (ak-map "K" [remap ak-one] 'ak-text-one))
  (let ((one (ak-map "ONE" [remap ak-one] 'ak-keymaps-one [remap ak-number] 7 [remap ak-long] (ak-map "LONG")))
        (two (ak-map "TWO" [remap ak-one] 'ak-keymaps-two [remap ak-two] 'ak-keymaps-two)))
    (list (command-remapping 'ak-one) (command-remapping 'ak-two) (command-remapping 'ak-three) (command-remapping 'ak-one 3)
          (command-remapping 'ak-one (copy-marker 3)) (ak-names (command-remapping 'ak-prefix)) (command-remapping 'ak-number)
          (command-remapping nil) (command-remapping t) (command-remapping "ak-one") (command-remapping 5) (command-remapping '(lambda ()))
          (command-remapping 'ak-one nil one) (command-remapping 'ak-two nil one) (command-remapping 'ak-two nil (list one two))
          (command-remapping 'ak-one nil (list two one)) (command-remapping 'ak-number nil one) (command-remapping 'ak-one 3 two)
          (ak-names (command-remapping 'ak-long nil one))
          (let ((overriding-local-map (ak-map "OL" [remap ak-one] 'ak-overriding-one))) (list (command-remapping 'ak-one) (command-remapping 'ak-two)))
          (ak-try (lambda () (command-remapping 'ak-one 99))) (ak-try (lambda () (command-remapping 'ak-one nil 5)))
          (ak-try (lambda () (command-remapping 5 99)))))))"#.into(),
    ]
}

#[test]
fn active_keymaps_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in expressions() {
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(progn (setq lexical-binding t) {SETUP} (prin1 (prin1-to-string (eval '{expression} t))))")]).output().unwrap();
        assert!(output.status.success(), "GNU failed on {expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.eval_source(SETUP).unwrap();
        // GNU's `minor-mode-key-binding` does not survive a collection in a
        // filter (its array of maps is not protected), so only Romacs
        // collects there; the answers are compared all the same.
        vm.eval_source("(defun ak-collect () (garbage-collect))").unwrap();
        let actual = match vm.eval_source(&format!("(prin1-to-string {expression})")) { Ok(value) => value.display(), Err(error) => Err(error) };
        if actual.as_deref() != Ok(expected.as_str()) { failures.push(format!("{expression}\nGNU {expected}\nVM {actual:?}")); }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The calls a form enters when run COUNT times, less the loop's own.
fn calls(vm: &mut Vm, form: &str, count: usize) -> usize {
    let mut run = |body: &str| {
        let before = vm.calls_entered();
        vm.eval_source(&format!("(let ((n {count})) (while (> n 0) {body} (setq n (1- n))))")).unwrap();
        vm.calls_entered() - before
    };
    let empty = run("nil");
    run(form) - empty
}

/// The command loop asks for a typed key's binding three times. With three
/// minor modes on, each `key-binding` is one call, the primitive itself:
/// the active maps, the lookup in them and the remapping of the command
/// found enter no function. A call
/// here is a Lisp function or a primitive that may call Lisp, so a Lisp
/// function around `key-binding` makes two and `current-active-maps`,
/// which cannot call Lisp, none.
#[test]
fn a_typed_keys_binding_enters_no_lisp() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(SETUP).unwrap();
    vm.eval_source(r#"
        (defvar ak-one t) (defvar ak-two t) (defvar ak-three t)
        (use-global-map (ak-map "G" "a" 'self-insert-command "b" 'ak-plain [remap ak-plain] 'ak-remapped))
        (use-local-map (ak-map "L"))
        (setq minor-mode-map-alist (list (cons 'ak-one (ak-map "1" "q" 'ak-q)) (cons 'ak-two (ak-map "2")) (cons 'ak-three (ak-map "3"))))
        (setq emulation-mode-map-alists (list (list (cons 'ak-one (ak-map "E")))))"#).unwrap();
    assert_eq!(vm.eval_source("(list (key-binding \"a\") (key-binding \"b\") (length (current-minor-mode-maps)))").unwrap().display().unwrap(),
        "(self-insert-command ak-remapped 4)");
    vm.eval_source("(defun ak-wrapped () (key-binding \"a\"))").unwrap();
    let per_run = |vm: &mut Vm, form: &str| calls(vm, form, 100) / 100;
    assert_eq!(per_run(&mut vm, "(ak-wrapped)"), 2);
    for form in ["(key-binding \"a\")", "(key-binding \"b\")", "(key-binding \"a\" t t)", "(command-remapping 'ak-plain)", "(minor-mode-key-binding \"q\")"] {
        assert_eq!(calls(&mut vm, form, 100), 100, "{form}");
    }
    assert_eq!(calls(&mut vm, "(current-active-maps t)", 100), 0);
}
