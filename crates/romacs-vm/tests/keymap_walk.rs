//! The keymap walk calls a menu item's `:filter` at each occurrence it
//! meets, on the live maps and the live key sequence (keymap.c
//! `access_keymap_1`, `get_keyelt`, `lookup_key_1`).  Each form and its
//! answer are the pinned GNU's (45b0f7699df, `-Q --batch --eval`): the
//! shared item, the mutating filter and the mutating sequence, through each
//! primitive that looks keys up, and a filter met in each state the walk
//! can stop in.  The reader's and `key-binding`'s rows for the shared item
//! and the mutating filter are `test/romacs/lisp/command-loop-tests.el`'s.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
use gnu_lisp::gnu_lisp;

/// FORM's value as `prin1` prints it, in GNU batch's VM.
fn printed(form: &str) -> String {
    let mut vm = gnu_lisp(&[]);
    let printed = match vm.eval_source(&format!("(prin1-to-string {form})")) {
        Ok(value) => value.display().unwrap(),
        Err(e) => format!("VM-ERROR {e}"),
    };
    serde_json::from_str(&printed).unwrap_or(printed)
}

#[test]
fn a_filter_that_changes_an_unread_event_changes_the_lookup() {
    let form = r#"(let ((key (vector 'f5 'f6))
      (map (make-sparse-keymap))
      (prefix (make-sparse-keymap)))
  (define-key prefix [f6] 'audit-before)
  (define-key prefix [f7] 'audit-after)
  (define-key map [f5]
              (list 'menu-item "x" prefix :filter
                    (lambda (definition)
                      (aset key 1 'f7)
                      definition)))
  (prin1 (list (lookup-key map key) key)))"#;
    assert_eq!(printed(form), "(audit-after [f5 f7])");
}

#[test]
fn where_is_filters_at_each_occurrence_in_its_forward_lookup() {
    let form = r#"(let* ((hits 0)
       (item (list 'menu-item "x" 'wi-cmd :filter
                   (lambda (d) (setq hits (1+ hits)) (and (= hits 2) d))))
       (high (list 'keymap (cons 'f5 item)))
       (low (list 'keymap (cons 'f5 item)))
       (map (make-composed-keymap (list high low))))
  (prin1 (list (where-is-internal 'wi-cmd (list map)) hits)))"#;
    assert_eq!(printed(form), "(([f5]) 4)");
}

#[test]
fn where_is_sees_a_filter_replace_its_binding() {
    let form = r#"(let* ((log nil)
       (binding (cons 'f5 nil))
       (high (list 'keymap binding))
       (low (list 'keymap (cons 'f5 'wi-cmd)))
       (map (make-composed-keymap (list high low))))
  (setcdr binding (list 'menu-item "x" 'ignore :filter
                        (lambda (_) (push 'filter log) (setcdr binding 'wi-other) nil)))
  (prin1 (list (where-is-internal 'wi-cmd (list map)) (where-is-internal 'wi-cmd (list map)) (reverse log))))"#;
    assert_eq!(printed(form), "(([f5]) nil (filter))");
}

#[test]
fn accessible_keymaps_filters_at_each_occurrence_in_its_prefix_lookup() {
    let form = r#"(let* ((hits 0)
       (sub (list 'keymap (cons 'f6 'acc-cmd)))
       (item (list 'menu-item "x" sub :filter
                   (lambda (d) (setq hits (1+ hits)) (and (= hits 2) d))))
       (map (make-composed-keymap (list (list 'keymap (cons 'f5 item))
                                        (list 'keymap (cons 'f5 item))))))
  (prin1 (list (accessible-keymaps map [f5]) hits)))"#;
    assert_eq!(printed(form), "((([f5] keymap (f6 . acc-cmd))) 2)");
}

#[test]
fn accessible_keymaps_reads_the_prefix_a_filter_changed() {
    let form = r#"(let* ((prefix (vector 'f5 'f6))
       (inner6 (list 'keymap (cons 'f9 'acc-six)))
       (inner7 (list 'keymap (cons 'f9 'acc-seven)))
       (sub (list 'keymap (cons 'f6 inner6) (cons 'f7 inner7)))
       (map (list 'keymap (cons 'f5 (list 'menu-item "x" sub :filter
                                         (lambda (d) (aset prefix 1 'f7) d))))))
  (prin1 (list (accessible-keymaps map prefix) prefix)))"#;
    assert_eq!(printed(form), "((([f5 f7] keymap (f9 . acc-seven))) [f5 f7])");
}

/// The walk reads each event once, when it reaches it: a change to an
/// event already read does not restart the lookup.
#[test]
fn a_filter_that_changes_an_event_already_read_does_not_restart_the_lookup() {
    let form = r#"(let ((key (vector 'f5 'f6))
      (map (make-sparse-keymap))
      (prefix (make-sparse-keymap)))
  (define-key prefix [f6] 'audit-before)
  (define-key map [f9] 'audit-restarted)
  (define-key map [f5]
              (list 'menu-item "x" prefix :filter
                    (lambda (definition)
                      (aset key 0 'f9)
                      definition)))
  (prin1 (list (lookup-key map key) key)))"#;
    assert_eq!(printed(form), "(audit-before [f9 f6])");
}

/// Both occurrences of one menu item call its filter; the second's result binds.
#[test]
fn lookup_key_filters_a_shared_item_at_each_occurrence() {
    let form = r#"(let* ((hits 0)
       (item (list 'menu-item "x" 'ignore :filter
                   (lambda (_)
                     (setq hits (1+ hits))
                     (and (= hits 2) 'ignore))))
       (high (list 'keymap (cons 'f5 item)))
       (low (list 'keymap (cons 'f5 item))))
  (prin1 (list (lookup-key (make-composed-keymap (list high low)) [f5]) hits)))"#;
    assert_eq!(printed(form), "(ignore 2)");
}

/// The walk goes on over the live map: the replacement binds from the next lookup.
#[test]
fn lookup_key_falls_through_a_filter_that_replaces_its_binding() {
    let form = r#"(let* ((log nil)
       (binding (cons 'f5 nil))
       (high (list 'keymap binding))
       (low (list 'keymap (cons 'f5 'audit-fallback)))
       (composed (make-composed-keymap (list high low))))
  (setcdr binding (list 'menu-item "x" 'ignore :filter
                        (lambda (_) (push 'filter log) (setcdr binding 'audit-replacement) nil)))
  (prin1 (list (lookup-key composed [f5]) (lookup-key composed [f5]) (reverse log))))"#;
    assert_eq!(printed(form), "(audit-fallback audit-replacement (filter))");
}

/// A `[remap COMMAND]` too long for the maps once a filter ran is no remapping.
#[test]
fn where_is_maps_a_too_long_remap_found_after_a_filter_to_nil() {
    let form = r#"(let* ((hits 0) (m (make-sparse-keymap)))
  (define-key m "a" 'wi-cmd)
  (define-key m [remap] (list 'menu-item "r" 'ignore :filter (lambda (d) (setq hits (1+ hits)) d)))
  (prin1 (list (where-is-internal 'wi-cmd (list m)) (> hits 0))))"#;
    assert_eq!(printed(form), "(([97]) t)");
}

/// A filter met on a default binding, at the end of the map's list.
#[test]
fn a_filter_on_a_default_binding_resumes_the_default() {
    let form = r#"(let* ((hits 0) (m (make-sparse-keymap)))
  (define-key m [t] (list 'menu-item "d" 'kw-default :filter (lambda (d) (setq hits (1+ hits)) d)))
  (prin1 (list (lookup-key m [f5] t) (lookup-key m [f5]) hits
               (let ((overriding-local-map m)) (key-binding [f5] t)) hits)))"#;
    assert_eq!(printed(form), "(kw-default nil 1 kw-default 2)");
}

/// A filter met in a parent map while the child's prefix waits to compose.
#[test]
fn a_filter_in_a_parent_map_composes_with_the_childs_prefix() {
    let form = r#"(let* ((hits 0) (parent (make-sparse-keymap)) (child (make-sparse-keymap))
       (p1 (make-sparse-keymap)) (p2 (make-sparse-keymap)))
  (define-key p1 [f6] 'kw-child)
  (define-key p2 [f7] 'kw-parent)
  (define-key parent [f5] (list 'menu-item "p" p2 :filter (lambda (d) (setq hits (1+ hits)) d)))
  (define-key child [f5] p1)
  (set-keymap-parent child parent)
  (prin1 (list (lookup-key child [f5 f6]) (lookup-key child [f5 f7]) hits)))"#;
    assert_eq!(printed(form), "(kw-child kw-parent 2)");
}

/// A filter met on ESC, with the meta character's base still to look up.
#[test]
fn a_filter_on_the_meta_prefix_keeps_the_meta_characters_base() {
    let form = r#"(let* ((hits 0) (m (make-sparse-keymap)) (esc (make-sparse-keymap)))
  (define-key esc "x" 'kw-meta-x)
  (define-key m [27] (list 'menu-item "e" esc :filter (lambda (d) (setq hits (1+ hits)) d)))
  (prin1 (list (lookup-key m [?\M-x]) (lookup-key m "\M-x") (lookup-key m [?\M-x ?y]) hits)))"#;
    assert_eq!(printed(form), "(kw-meta-x kw-meta-x 1 3)");
}

/// A default taken ends the frame's defaults for its embedded maps: the later one's filter does not run.
#[test]
fn an_embedded_default_does_not_override_an_earlier_one() {
    let form = r#"(let* ((trace nil)
       (inner (list 'keymap (cons t (list 'menu-item "second" 'later-default :filter
                                          (lambda (d) (push 'second trace) d)))))
       (m (list 'keymap (cons t 'first-default) inner)))
  (prin1 (list (lookup-key m [f8] t) trace)))"#;
    assert_eq!(printed(form), "(first-default nil)");
}

/// A parent map is searched with what the child left of its defaults: a child that took its default keeps the parent's from running.
#[test]
fn a_parent_scan_inherits_the_childs_spent_default() {
    let form = r#"(let* ((trace nil)
       (p1 (make-sparse-keymap)) (p2 (make-sparse-keymap))
       (parent (list 'keymap (cons t (list 'menu-item "p" p2 :filter
                                           (lambda (d) (push 'parent trace) d)))))
       (child (make-sparse-keymap)))
  (define-key p1 "y" 'kw-child-y)
  (define-key p2 "x" 'kw-parent-x)
  (define-key child [t] 'child-default)
  (define-key child [f8] p1)
  (set-keymap-parent child parent)
  (prin1 (list (lookup-key child [f8 ?x] t) (lookup-key child [f8 ?y] t) (lookup-key child [t]) trace)))"#;
    assert_eq!(printed(form), "(nil kw-child-y child-default nil)");
}

/// A meta character whose ESC binds no keymap takes its map's default, and is too long only once it is resolved whole.
#[test]
fn a_meta_character_falls_back_to_its_maps_default() {
    let form = r#"(let* ((hits 0) (m (make-sparse-keymap)))
  (define-key m [t] (list 'menu-item "d" 'kw-default :filter (lambda (d) (setq hits (1+ hits)) d)))
  (prin1 (list (lookup-key m [?\M-x] t) hits (lookup-key m [?\M-x ?y] t) (lookup-key m [?\M-x]) (lookup-key m "\M-xy" t) hits)))"#;
    assert_eq!(printed(form), "(kw-default 2 1 nil 1 6)");
}

/// An advertised key with a range event is looked up as itself: the walk reads its later events live, once.
#[test]
fn where_is_looks_the_advertised_key_up_live() {
    let form = r#"(progn
(defvar kw-key nil)
(defvar kw-count 0)
(let* ((m (make-sparse-keymap))
       (p (make-sparse-keymap))
       (kw-key (vector 'f5 (cons ?x ?z) 'f6))
       (kw-count 0))
  (define-key p [120 f7] 'kw-dest)
  (define-key m [f5]
    (list 'menu-item "P" p :filter
          (lambda (d)
            (setq kw-count (1+ kw-count))
            (aset kw-key 2 'f7)
            d)))
  (put 'kw-dest :advertised-binding kw-key)
  (let ((answer (where-is-internal 'kw-dest (list m) t)))
    (prin1 (list answer (eq answer kw-key) kw-key kw-count)))))"#;
    assert_eq!(printed(form), "([f5 (120 . 122) f7] t [f5 (120 . 122) f7] 1)");
}

/// A filter changes a multibyte string key ahead of the walk, past a
/// two-byte character: the walk reads the next event by its character
/// index in the string as it is now.
#[test]
fn a_filter_that_changes_a_multibyte_key_ahead_of_the_walk_changes_the_lookup() {
    let form = r#"(let ((key (string ?\xe9 ?a ?b))
      (m (make-sparse-keymap))
      (p (make-sparse-keymap)))
  (define-key p "ac" 'kw-changed)
  (define-key p "ab" 'kw-old)
  (define-key m (string ?\xe9) (list 'menu-item "x" p :filter (lambda (d) (aset key 2 ?c) d)))
  (prin1 (list (lookup-key m key) (append key nil) (multibyte-string-p key))))"#;
    assert_eq!(printed(form), "(kw-changed (233 97 99) t)");
}
