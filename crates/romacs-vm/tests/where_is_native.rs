//! The reverse keymap lookup (keymap.c `where-is-internal`,
//! `accessible-keymaps`, `keymap--get-keyelt`) against the pinned GNU, how
//! much Lisp a lookup runs, and what its cache saves.
use romacs_vm::Vm;

/// WN-MAP makes a keymap of KEY DEFINITION pairs; WN-TRY gives a signal as
/// data, so both sides print it.
const SETUP: &str = r#"
(defun wn-map (&rest bindings)
  (let ((map (make-sparse-keymap)))
    (while bindings
      (define-key map (car bindings) (car (cdr bindings)))
      (setq bindings (cdr (cdr bindings))))
    map))
(defun wn-try (thunk) (condition-case e (funcall thunk) (error (list 'signal (prin1-to-string e)))))
(defun wn-keys (maps) (mapcar (lambda (entry) (car entry)) maps))
(autoload 'wn-auto "wn-no-such-file" nil nil 'keymap)
(put 'wn-non-key 'non-key-event t)
(defvar wn-global (current-global-map))
"#;

/// What a bare VM lacks of subr.el.
const BARE: &str = "(defun ignore (&rest _) nil)";

fn expressions() -> Vec<&'static str> {
    [order_and_shadowing(), remapping_and_preference(), menus_and_indirection(), keymaps_searched(), accessible_keymaps(), keyelt_and_metadata()].concat()
}

fn order_and_shadowing() -> Vec<&'static str> {
    vec![
        // The order of the value: each keymap in the order it is reached,
        // a meta character folded onto its ESC prefix.
        r#"(let* ((sub (wn-map "a" 'wn-t "b" 'wn-t))
       (m (wn-map "x" sub "t" 'wn-t [f5] 'wn-t "\e" (wn-map "q" 'wn-t) [f6 f7] 'wn-t [?\M-z] 'wn-t "y" sub)))
  (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m) t) (where-is-internal 'wn-t (list m) 'non-ascii)
        (where-is-internal 'wn-none (list m)) (where-is-internal 'wn-none (list m) t) (where-is-internal nil (list m))))"#,
        // An earlier keymap shadows a key it binds, or binds a prefix of as
        // a command; a prefix it lacks, or has as a keymap without the
        // rest, falls through.
        r#"(let ((hi (wn-map "a" 'wn-o "b" (wn-map "c" 'wn-o) "e" 'wn-o "g" (wn-map "z" 'wn-o) "n" 5))
      (lo (wn-map "a" 'wn-t "b" 'wn-t "d" 'wn-t "ef" 'wn-t "gh" 'wn-t "ij" 'wn-t "no" 'wn-t)))
  (list (where-is-internal 'wn-t (list hi lo)) (where-is-internal 'wn-t (list lo hi)) (where-is-internal 'wn-o (list hi lo))
        (where-is-internal 'wn-t (list hi lo) t) (where-is-internal 'wn-t (list lo lo))))"#,
        // A character range is one event, shadowed through its first character.
        r#"(let ((m (make-keymap)))
  (set-char-table-range (car (cdr m)) '(?a . ?f) 'wn-t)
  (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list (wn-map "a" 'wn-o) m))
        (where-is-internal 'wn-t (list (wn-map "c" 'wn-o) m)) (where-is-internal 'wn-t (list m) t)))"#,
        // Inherited bindings: once each, and not where the child rebinds.
        r#"(let* ((p (wn-map "a" 'wn-t "b" 'wn-t "c" 'wn-t)) (m (wn-map "a" 'wn-t "b" 'wn-o "d" 'wn-t)))
  (set-keymap-parent m p)
  (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m m p)) (where-is-internal 'wn-t (list m) t)
        (where-is-internal 'wn-t (list (list 'keymap m p)))))"#,
        // An event with a `non-key-event' property is no key; one string
        // stands for a menu's strings.
        r#"(let ((m (list 'keymap (cons "one" 'wn-t) (cons "two" 'wn-t) (cons 'wn-non-key 'wn-t) (cons ?a 'wn-t)
               (cons 'wn-prefix (list 'keymap (cons 'wn-non-key 'wn-t) (cons "three" 'wn-t))))))
  (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m) 'non-ascii) (where-is-internal 'wn-t (list m) t)))"#,
    ]
}

fn remapping_and_preference() -> Vec<&'static str> {
    vec![
        // A remapped command has the keys of what it is remapped to; the
        // keys of the commands remapped to a command come after its own.
        r#"(let ((m (wn-map "a" 'wn-old [remap wn-old] 'wn-t "b" 'wn-t [remap wn-t2] 'wn-new "c" 'wn-t2 [remap wn-chain] 'wn-old "d" 'wn-chain
                 [remap 5] 'wn-t [remap wn-self] 'wn-self "s" 'wn-self)))
  (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m) nil nil t)
        (where-is-internal 'wn-old (list m)) (where-is-internal 'wn-old (list m) nil nil t)
        (where-is-internal 'wn-t2 (list m)) (where-is-internal 'wn-new (list m)) (where-is-internal 'wn-new (list m) nil nil t)
        (where-is-internal 'wn-chain (list m)) (where-is-internal 'wn-self (list m))
        (where-is-internal 'wn-t (list m) t) (where-is-internal 'wn-old (list m) t) (where-is-internal 'wn-t (list m) t nil t)
        (where-is-internal 'wn-t (list m) 'non-ascii)))"#,
        // The remapping may live in another keymap than the binding.
        r#"(let ((hi (wn-map [remap wn-old] 'wn-t "x" 'wn-o)) (lo (wn-map "a" 'wn-old "x" 'wn-old "b" 'wn-t)))
  (list (where-is-internal 'wn-t (list hi lo)) (where-is-internal 'wn-t (list lo hi)) (where-is-internal 'wn-old (list hi lo))
        (where-is-internal 'wn-t (list hi lo) t) (where-is-internal 'wn-t (list lo))))"#,
        // `:advertised-binding': one key or a list, taken when it runs the
        // command and a first key is asked for.
        r#"(progn
  (put 'wn-adv :advertised-binding [?z]) (put 'wn-adv2 :advertised-binding '([?q] [?y])) (put 'wn-adv3 :advertised-binding "w")
  (put 'wn-adv4 :advertised-binding [?u]) (put 'wn-adv5 :advertised-binding '([?q] . [?k]))
  (let ((m (wn-map "a" 'wn-adv "z" 'wn-adv "b" 'wn-adv2 "y" 'wn-adv2 "c" 'wn-adv3 "w" 'wn-adv3 "d" 'wn-adv4 "j" 'wn-adv5 "k" 'wn-adv5
                   [remap wn-from] 'wn-adv)))
    (list (where-is-internal 'wn-adv (list m) t) (where-is-internal 'wn-adv (list m)) (where-is-internal 'wn-adv (list m) 'non-ascii)
          (where-is-internal 'wn-adv2 (list m) t) (where-is-internal 'wn-adv3 (list m) t) (where-is-internal 'wn-adv4 (list m) t)
          (where-is-internal 'wn-adv5 (list m) t) (where-is-internal 'wn-from (list m) t)
          (where-is-internal 'wn-adv (list (wn-map "z" 'wn-o) m) t))))"#,
        r#"(progn (put 'wn-adv6 :advertised-binding 'junk)
  (wn-try (lambda () (where-is-internal 'wn-adv6 (list (wn-map "a" 'wn-adv6)) t))))"#,
        // `where-is-preferred-modifier': a first key with that modifier,
        // else one of plain characters, else the first found.
        r#"(let ((m (wn-map [f1] 'wn-t [?\s-a] 'wn-t [?\H-a] 'wn-t "a" 'wn-t [?\M-a] 'wn-t [?\M-\s-b] 'wn-t [?\C-c ?\s-c] 'wn-t))
      (odd (wn-map [f1] 'wn-t [?\s-a] 'wn-t [?\A-a] 'wn-t)) (try (lambda (modifier map) (let ((where-is-preferred-modifier modifier)) (where-is-internal 'wn-t (list map) t)))))
  (list (where-is-internal 'wn-t (list m) t)
        (mapcar (lambda (modifier) (funcall try modifier m)) '(nil super s hyper H meta M control alt shift junk 5 down))
        (mapcar (lambda (modifier) (funcall try modifier odd)) '(nil super alt A hyper))
        (let ((where-is-preferred-modifier 'super)) (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m) 'non-ascii)))))"#,
    ]
}

fn menus_and_indirection() -> Vec<&'static str> {
    vec![
        // A first key that is not `non-ascii' leaves out the keymaps under
        // a menu or mouse prefix, with any modifiers; a lone mouse event is
        // a key.
        r"(let ((m (wn-map [menu-bar wn] 'wn-t [mouse-1] 'wn-t [C-down-mouse-2 wn] 'wn-t [tool-bar wn] 'wn-t [f5 ?a] 'wn-t
                 [mode-line mouse-1] 'wn-t [M-S-double-mouse-3 wn] 'wn-t [mouse-9 wn] 'wn-t [menu wn] 'wn-t [tab-bar ?x ?y] 'wn-t))
      (menus (wn-map [menu-bar wn] 'wn-t [header-line mouse-2] 'wn-t [tab-line wn] 'wn-t)))
  (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m) t) (where-is-internal 'wn-t (list m) 'non-ascii)
        (where-is-internal 'wn-t (list menus) t) (where-is-internal 'wn-t (list menus) 'non-ascii) (where-is-internal 'wn-t (list menus))))",
        // Menu items and menu strings are looked through, unless NOINDIRECT.
        r#"(let* ((item (list 'menu-item "A" 'wn-t)) (m (wn-map "a" item "b" (cons "Label" 'wn-t) "c" (cons "L" (cons "help" 'wn-t)) "d" 'wn-t
                                                         "e" (list 'menu-item "E") "f" (list 'menu-item "F" 'wn-t :enable nil))))
  (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m) nil t) (where-is-internal 'wn-t (list m) t t)
        (where-is-internal item (list m) nil t) (where-is-internal (list 'menu-item "A" 'wn-t) (list m) nil t)
        (where-is-internal (cons "Label" 'wn-t) (list m) nil t) (where-is-internal (cons "Label" 'wn-t) (list m))
        (where-is-internal nil (list m)) (where-is-internal item (list m) t t)))"#,
        // A definition that is a list matches by `equal' in a walk, by
        // identity in the cache a first-key lookup reads.
        r#"(let* ((macro (list 'lambda nil '(interactive))) (m (wn-map "a" macro "b" (list 'lambda nil '(interactive)) "c" [?x ?y] "d" "xy")))
  (list (where-is-internal macro (list m)) (where-is-internal (list 'lambda nil '(interactive)) (list m))
        (where-is-internal macro (list m) t) (where-is-internal (list 'lambda nil '(interactive)) (list m) t)
        (where-is-internal [?x ?y] (list m)) (where-is-internal "xy" (list m)) (where-is-internal "xy" (list m) t)))"#,
        // A `:filter' does not decide what is found, but does shadow.
        r#"(let ((m (wn-map "a" '(menu-item "x" wn-t :filter identity) "b" '(menu-item "x" ignore :filter (lambda (_) 'wn-t))
                 "c" '(menu-item "x" wn-t :filter (lambda (_) 'ignore)) "d" '(menu-item "x" wn-t :filter (lambda (_) (error "no")))
                 "e" (list 'menu-item "x" (wn-map "f" 'wn-t) :filter 'identity)
                 "g" (list 'menu-item "x" (wn-map "h" 'wn-t) :filter (lambda (_) nil)))))
  (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m) t) (where-is-internal 'wn-t (list m) nil t)
        (wn-keys (accessible-keymaps m))))"#,
        // A filter runs once for each lookup that meets its item, and what
        // it does to the keymaps does not change the sequences being checked.
        r#"(let* ((n 0) (sub (wn-map "a" 'wn-t)) (item (list 'menu-item "S" sub :filter (lambda (d) (setq n (1+ n)) (if (= n 1) d nil))))
       (m (wn-map "x" item "y" item)))
  (list (where-is-internal 'wn-t (list m)) n (where-is-internal 'wn-t (list m)) n))"#,
        r#"(let* ((m (wn-map "a" 'wn-t)) (n 0)
       (item (list 'menu-item "S" 'wn-t :filter (lambda (d) (setq n (1+ n)) (define-key m "z" 'wn-t) (define-key m "a" 'wn-o) d))))
  (define-key m "b" item)
  (list (where-is-internal 'wn-t (list m)) n (where-is-internal 'wn-t (list m)) n (where-is-internal 'wn-t (list m) t) n))"#,
        r#"(let* ((n 0) (m (wn-map "a" 'wn-old [remap wn-old] (list 'menu-item "R" 'wn-t :filter (lambda (d) (setq n (1+ n)) d)) "b" 'wn-t)))
  (list (where-is-internal 'wn-t (list m)) n (where-is-internal 'wn-old (list m)) n (where-is-internal 'wn-t (list m) t) n))"#,
        // A keymap's filters run once for a sequence: where the sequence
        // is too long for the keymap, whether its prefix is defined there
        // comes from the same results.
        r#"(let ((n 0) (hi (make-sparse-keymap)) (lo (wn-map "xa" 'wn-t)))
  (define-key hi "x" (list 'menu-item "X" 'ignore :filter (lambda (d) (setq n (1+ n)) (if (= n 1) d nil))))
  (list (where-is-internal 'wn-t (list hi lo)) n))"#,
        // A change made behind the keymap primitives is seen by a walk,
        // which drops the cache; numbers are bindings by value in the cache.
        r#"(let ((m (wn-map "a" 'wn-t)))
  (list (where-is-internal 'wn-t (list m) t)
        (progn (setcdr m (list (cons ?b 'wn-t))) (where-is-internal 'wn-t (list m) t))
        (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m) t)))"#,
        r#"(let ((m (wn-map "a" 1.5 "b" (+ 1.0 0.5) "c" (expt 2 80) "d" (expt 2 80))))
  (list (where-is-internal 1.5 (list m) t) (where-is-internal 1.5 (list m)) (where-is-internal (expt 2 80) (list m) t)
        (where-is-internal (expt 2 80) (list m))))"#,
        // The cache follows a keymap change, a new parent and other keymaps,
        // and shares the sequences it holds.
        r#"(let ((m (wn-map "a" 'wn-t)) (other (wn-map "o" 'wn-t)) (parent (wn-map "p" 'wn-u)))
  (list (where-is-internal 'wn-t (list m) t)
        (progn (define-key m "b" 'wn-u) (where-is-internal 'wn-u (list m) t))
        (progn (define-key m "a" nil) (where-is-internal 'wn-t (list m) t))
        (where-is-internal 'wn-t (list other) t) (where-is-internal 'wn-t (list m other) t)
        (progn (define-key m "b" nil) (set-keymap-parent m parent) (where-is-internal 'wn-u (list m) t))
        (let ((found (where-is-internal 'wn-u (list m) t))) (eq found (where-is-internal 'wn-u (list m) t)))
        (let ((found (where-is-internal 'wn-u (list m) t))) (define-key m "z" 'wn-z) (eq found (where-is-internal 'wn-u (list m) t)))))"#,
    ]
}

fn keymaps_searched() -> Vec<&'static str> {
    vec![
        // KEYMAP: a keymap with the global map, a list of keymaps alone,
        // nil for the active keymaps without `overriding-local-map'.
        r#"(progn
  (use-global-map (wn-map "g" 'wn-t "a" 'wn-g "r" 'wn-old))
  (use-local-map (wn-map "l" 'wn-t "r" 'wn-o))
  (fset 'wn-symbol-map (wn-map "s" 'wn-t))
  (let ((m (wn-map "a" 'wn-t [remap wn-old] 'wn-t)) (minor-mode-map-alist nil) (minor-mode-overriding-map-alist nil) (emulation-mode-map-alists nil))
    (list (where-is-internal 'wn-t m) (where-is-internal 'wn-g m) (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t)
          (where-is-internal 'wn-t nil t) (where-is-internal 'wn-old) (where-is-internal 'wn-t 'wn-symbol-map)
          (where-is-internal 'wn-t '(wn-symbol-map)) (where-is-internal 'wn-t (list 'wn-symbol-map m))
          (let ((overriding-local-map (wn-map "o" 'wn-t))) (where-is-internal 'wn-t))
          (let ((minor-mode-map-alist (list (cons 'wn-on (wn-map "m" 'wn-t "l" 'wn-o))))) (defvar wn-on t) (where-is-internal 'wn-t))
          (wn-try (lambda () (where-is-internal 'wn-t 5))) (wn-try (lambda () (where-is-internal 'wn-t (list m 5))))
          (wn-try (lambda () (where-is-internal 'wn-t '(junk)))))))"#,
        // The walk neither loads nor enters an autoloaded keymap: as a
        // prefix, as a parent; embedded in a keymap, it is no keymap to
        // walk.  (`lookup-key' is what loads one in GNU, so a reverse
        // lookup that has to look past the keymap's own bindings is not
        // compared here.)
        r#"(let ((m (wn-map "x" 'wn-auto "a" 'wn-t)) (child (list 'keymap (cons ?a 'wn-t) (cons ?b 'wn-t))) (embedded (list 'keymap 'wn-auto (cons ?a 'wn-t))))
  (setcdr (cdr (cdr child)) 'wn-auto)
  (list (where-is-internal 'wn-t (list m)) (where-is-internal 'wn-t (list m) t) (wn-keys (accessible-keymaps m)) (cdr (car (cdr (accessible-keymaps m))))
        (accessible-keymaps m "x") (wn-keys (accessible-keymaps child))
        (wn-try (lambda () (accessible-keymaps embedded)))
        (wn-try (lambda () (accessible-keymaps 'wn-auto))) (car (symbol-function 'wn-auto))))"#,
    ]
}

fn accessible_keymaps() -> Vec<&'static str> {
    vec![
        // Breadth first; a meta character right after its ESC prefix; a
        // keymap reached again under a longer sequence that starts with
        // an earlier one is a cycle.
        r#"(let* ((subsub (wn-map "y" 'ignore)) (sub (wn-map "x" subsub)) (m (wn-map "\C-c" sub [f5] sub "\e" subsub "a" 'ignore)))
  (define-key m "\M-g" (wn-map "g" 'ignore)) (define-key m "\M-h" (wn-map "h" 'ignore)) (define-key sub "m" m) (define-key subsub "s" sub)
  (define-key m [f6] (cons "Menu" sub)) (define-key m [f7] (list 'menu-item "Item" subsub)) (define-key m [27 f8] (wn-map "i" 'ignore))
  (fset 'wn-sub sub) (define-key m [f9] 'wn-sub)
  (list (wn-keys (accessible-keymaps m)) (wn-keys (accessible-keymaps m "\C-c")) (wn-keys (accessible-keymaps m [f5]))
        (wn-keys (accessible-keymaps m "\e")) (wn-keys (accessible-keymaps m [27])) (wn-keys (accessible-keymaps m "\eg"))
        (wn-keys (accessible-keymaps m "\M-g")) (wn-keys (accessible-keymaps m [?\M-g])) (wn-keys (accessible-keymaps m [f5 ?x]))
        (accessible-keymaps m "q") (accessible-keymaps m "a") (accessible-keymaps m [f5 ?x ?y ?z]) (accessible-keymaps m [])
        (wn-keys (accessible-keymaps m "")) (wn-keys (accessible-keymaps sub)) (wn-keys (accessible-keymaps 'wn-sub))
        (eq (cdr (car (accessible-keymaps m))) m) (eq (cdr (car (accessible-keymaps 'wn-sub))) sub)
        (eq (cdr (assoc [f9] (accessible-keymaps m))) sub) (let ((prefix [f5])) (eq (car (car (accessible-keymaps m prefix))) prefix))
        (eq (cdr (car (accessible-keymaps m [f5]))) sub)))"#,
        // A string prefix's 8-bit characters are meta characters; the last
        // event of the prefix is not folded.
        r#"(let ((m (make-sparse-keymap)) (sub (wn-map "b" (wn-map "c" 'ignore))))
  (define-key m [134217825] sub)
  (list (wn-keys (accessible-keymaps m (string 225))) (wn-keys (accessible-keymaps m (unibyte-string 225)))
        (wn-keys (accessible-keymaps m [134217825])) (wn-keys (accessible-keymaps m [27])) (wn-keys (accessible-keymaps m [27 ?a]))
        (wn-keys (accessible-keymaps m))))"#,
        // Parents, embedded keymaps, dense keymaps and vectors are walked;
        // a prefix reached through a menu filter is that filter's keymap.
        r#"(let* ((sub (wn-map "s" 'ignore)) (p (wn-map "z" sub)) (m (wn-map "a" sub)) (dense (make-keymap))
       (filtered (wn-map "f" (list 'menu-item "F" sub :filter 'identity) "n" (list 'menu-item "N" sub :filter 'ignore))))
  (set-keymap-parent m p) (define-key dense "d" sub) (define-key dense [f1] sub)
  (list (wn-keys (accessible-keymaps m)) (wn-keys (accessible-keymaps (list 'keymap m (wn-map "e" sub))))
        (wn-keys (accessible-keymaps dense)) (wn-keys (accessible-keymaps (list 'keymap (vector nil sub nil sub))))
        (wn-keys (accessible-keymaps filtered)) (wn-keys (accessible-keymaps filtered "f")) (accessible-keymaps filtered "n")
        (wn-keys (accessible-keymaps (wn-map "t" t "k" (wn-map "t" t))))))"#,
        r#"(let ((m (wn-map "a" (wn-map "b" 'ignore))))
  (list (wn-try (lambda () (accessible-keymaps 5))) (wn-try (lambda () (accessible-keymaps nil))) (wn-try (lambda () (accessible-keymaps 'wn-no-map)))
        (wn-try (lambda () (accessible-keymaps m 5))) (wn-try (lambda () (accessible-keymaps m 'symbol))) (wn-try (lambda () (accessible-keymaps m '(1 2))))
        (wn-try (lambda () (accessible-keymaps 5 "a"))) (wn-try (lambda () (accessible-keymaps m '(1 . 2))))))"#,
    ]
}

fn keyelt_and_metadata() -> Vec<&'static str> {
    vec![
        // get_keyelt: menu strings and menu items come off; AUTOLOAD applies
        // a `:filter', whose error is nil and whose value is unwrapped too.
        r#"(list (keymap--get-keyelt '("S" . ignore) nil) (keymap--get-keyelt '("S" "help" . ignore) nil) (keymap--get-keyelt '("S" "help" "more" ignore) t)
      (keymap--get-keyelt '(menu-item "I" ignore :filter (lambda (_) 'undo)) nil) (keymap--get-keyelt '(menu-item "I" ignore :filter (lambda (_) 'undo)) t)
      (keymap--get-keyelt '(menu-item "I" ignore :filter (lambda (_) 'undo)) 'yes)
      (keymap--get-keyelt '(menu-item) nil) (keymap--get-keyelt '(menu-item) t) (keymap--get-keyelt '(menu-item "I") t) (keymap--get-keyelt '(menu-item "I" . junk) t)
      (keymap--get-keyelt '(menu-item "I" ("S" . inner) :enable t :filter (lambda (d) (list 'menu-item "J" d))) t)
      (keymap--get-keyelt '(menu-item "I" ("S" . inner) :enable t :filter (lambda (d) (list 'menu-item "J" d))) nil)
      (keymap--get-keyelt '(menu-item "I" x :filter (lambda (_) (error "boom"))) t) (keymap--get-keyelt '(menu-item "I" x :filter) t)
      (keymap--get-keyelt '(menu-item "I" x :filter (lambda (d) (list d d)) :filter ignore) t)
      (keymap--get-keyelt '(menu-item "I" :filter (lambda (d) (list 'got d))) t)
      (keymap--get-keyelt '(menu-item "I" (menu-item "J" (menu-item "K" deep :filter (lambda (d) (cons "S" d))))) t)
      (keymap--get-keyelt 'symbol t) (keymap--get-keyelt 5 nil) (keymap--get-keyelt nil t) (keymap--get-keyelt '(keymap (97 . x)) t)
      (keymap--get-keyelt '(lambda () (interactive)) nil) (keymap--get-keyelt [menu-item "I" x] t))"#,
        // They are subrs, with a subr's arity.
        r"(mapcar (lambda (f) (list (subrp (symbol-function f)) (func-arity f) (commandp f))) '(where-is-internal accessible-keymaps keymap--get-keyelt))",
    ]
}

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(BARE).unwrap();
    vm.eval_source(SETUP).unwrap();
    vm
}

#[test]
fn reverse_lookups_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in expressions() {
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(progn (setq lexical-binding t) {SETUP} (prin1 (prin1-to-string (eval '{expression} t))))")]).output().unwrap();
        assert!(output.status.success(), "GNU failed on {expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = match vm().eval_source(&format!("(prin1-to-string {expression})")) { Ok(value) => value.display(), Err(error) => Err(error) };
        if actual.as_deref() != Ok(expected.as_str()) { failures.push(format!("{expression}\nGNU {expected}\nVM {actual:?}")); }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// GNU follows a circular menu wrapper forever; here it signals, with or
/// without AUTOLOAD, however long the cycle, also when a filter closes it
/// and when a lookup meets it in a keymap.
#[test]
fn a_circular_menu_wrapper_signals() {
    let mut vm = vm();
    let circular = "(signal \"(error \\\"Circular menu item\\\")\")";
    for form in [
        "(let ((x (list \"a\"))) (setcdr x x) (keymap--get-keyelt x nil))",
        "(let ((x (list \"a\" \"b\" \"c\" \"d\" \"e\"))) (setcdr (nthcdr 4 x) x) (keymap--get-keyelt x t))",
        "(let ((x (list 'menu-item \"I\" nil))) (setcar (nthcdr 2 x) x) (keymap--get-keyelt x nil))",
        "(let ((x (list 'menu-item \"I\" 'd :filter nil))) (setcar (nthcdr 4 x) (list 'lambda '(_) (list 'quote x))) (keymap--get-keyelt x t))",
        "(let ((x (list \"a\"))) (setcdr x x) (where-is-internal 'wn-t (list (list 'keymap (cons ?a x)))))",
        "(let ((x (list \"a\"))) (setcdr x x) (accessible-keymaps (list 'keymap (cons ?a x))))",
    ] {
        assert_eq!(vm.eval_source(&format!("(wn-try (lambda () {form}))")).unwrap().display().unwrap(), circular, "{form}");
    }
    // A long wrapper that ends is no cycle.
    assert_eq!(vm.eval_source("(let ((x 'end) (i 0)) (while (< i 500) (setq x (cons \"s\" x) i (1+ i))) (keymap--get-keyelt x nil))").unwrap().display().unwrap(), "end");
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

/// A command's keys in the active keymaps, the global map among them, are
/// found without entering a Lisp function: each lookup is one call, the
/// primitive itself.  A call here is a Lisp function or a primitive that
/// may call Lisp, as these three may for a menu item's `:filter`; walking
/// the keymaps, shadowing, remapping and the cache are none.  A Lisp
/// function around the lookup makes two.
#[test]
fn a_reverse_lookup_enters_no_lisp() {
    let mut vm = vm();
    vm.eval_source(r#"
        (let ((global (current-global-map)))
          (define-key global [f9] 'wn-command) (define-key global "\C-cw" 'wn-command) (define-key global "\M-_" 'wn-command)
          (define-key global [menu-bar wn] (cons "Menu" (wn-map [item] '(menu-item "Item" wn-command))))
          (define-key global [remap wn-other] 'wn-command) (define-key global [f8] 'wn-other))
        (use-local-map (wn-map "\C-cw" 'wn-shadow))
        (defun wn-wrapped () (where-is-internal 'wn-command))"#).unwrap();
    assert_eq!(vm.eval_source("(list (where-is-internal 'wn-command) (where-is-internal 'wn-command nil t) (length (accessible-keymaps (current-global-map))))").unwrap().display().unwrap(),
        "(([f9] [134217823] [menu-bar wn item] [f8]) [134217823] 7)");
    assert_eq!(calls(&mut vm, "(wn-wrapped)", 100), 200);
    for form in ["(where-is-internal 'wn-command)", "(where-is-internal 'wn-command nil t)", "(where-is-internal 'wn-command nil 'non-ascii)",
        "(where-is-internal 'wn-command nil nil t)", "(where-is-internal 'wn-command wn-global t t t)",
        "(accessible-keymaps wn-global)", "(accessible-keymaps wn-global [menu-bar])",
        "(keymap--get-keyelt '(menu-item \"I\" (\"S\" . wn-command)) nil)"] {
        assert_eq!(calls(&mut vm, form, 100), 100, "{form}");
    }
}

/// After a keymap change, ten definitions' first keys in one keymap come
/// from one walk of it, and later ones from none, until a keymap changes
/// again or other keymaps are searched.  A lookup of all keys walks, and
/// drops the cache as GNU's does.  What the cache holds is shared: the
/// sequence a second lookup gives is the object the first gave.
#[test]
fn ten_lookups_after_a_change_walk_the_keymap_once() {
    let mut vm = vm();
    vm.eval_source(r"
        (defvar wn-m (make-sparse-keymap))
        (defvar wn-commands '(wn-0 wn-1 wn-2 wn-3 wn-4 wn-5 wn-6 wn-7 wn-8 wn-9))
        (let ((commands wn-commands) (key ?a))
          (while commands
            (define-key wn-m (vector ?\C-c key) (car commands))
            (setq commands (cdr commands) key (1+ key))))
        (defun wn-round (&optional first) (mapcar (lambda (command) (where-is-internal command (list wn-m) first)) wn-commands))").unwrap();
    let walks = |vm: &mut Vm, form: &str| {
        let before = vm.where_is_walks();
        vm.eval_source(form).unwrap();
        vm.where_is_walks() - before
    };
    assert_eq!(walks(&mut vm, "(wn-round t)"), 1);
    assert_eq!(walks(&mut vm, "(wn-round t)"), 0);
    assert_eq!(walks(&mut vm, "(wn-round 'yes)"), 0);
    assert_eq!(vm.eval_source("(list (car (wn-round t)) (eq (car (wn-round t)) (car (wn-round t))))").unwrap().display().unwrap(), "([3 97] t)");
    assert_eq!(walks(&mut vm, "(progn (define-key wn-m [f5] 'wn-new) (wn-round t))"), 1);
    // A collection moves what the cache holds, and it still serves.
    assert_eq!(walks(&mut vm, "(progn (garbage-collect) (wn-round t))"), 0);
    assert_eq!(vm.eval_source("(where-is-internal 'wn-new (list wn-m) t)").unwrap().display().unwrap(), "[f5]");
    assert_eq!(walks(&mut vm, "(wn-round nil)"), 10);
    assert_eq!(walks(&mut vm, "(wn-round t)"), 1);
    assert_eq!(walks(&mut vm, "(where-is-internal 'wn-0 (list wn-m wn-m) t)"), 1);
    assert_eq!(walks(&mut vm, "(set-keymap-parent wn-m (make-sparse-keymap))"), 0);
    assert_eq!(walks(&mut vm, "(where-is-internal 'wn-0 (list wn-m wn-m) t)"), 1);
}
