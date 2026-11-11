//! What a collection does to undo lists: what a marker nothing holds loses,
//! here against the pinned GNU, and where the collector's own modes and paths
//! matter. `test/src/undo-collection-tests.el` holds the rest of what GNU's
//! does. The forms here each run as a request of their own, since within one a
//! register a frame has finished with still holds what it held: as firmly as a
//! variable, until the roots stop at the frames in use.
use romacs_vm::{HostEvent, HostFrame, HostInput, Vm, WindowEnd};

/// A buffer changed since the last collection, the list's only marker held by
/// nothing else, and limits that cut it after its second record.
const SETUP: &str = "(progn \
    (set-buffer (get-buffer-create \"undo\")) \
    (setq undo-limit 100 undo-strong-limit 1000 undo-outer-limit 100000) \
    (fset 'undo-test-touch (lambda () (let ((buffer-undo-list t)) (insert \"x\")))) \
    (fset 'undo-test-fill (lambda (entries) \
       (let ((marker (point-marker)) (list nil)) \
         (while entries \
           (setq list (cons (if (eq (car entries) 'm) (cons marker 1) (car entries)) list) \
                 entries (cdr entries))) \
         (setq buffer-undo-list (nreverse list)) nil))) \
    (fset 'undo-test-circle (lambda (list) \
       (setcdr (nthcdr (1- (length list)) list) list) \
       (setq buffer-undo-list list) nil)) \
    (undo-test-touch) t)";

/// Each of the collector's modes: threads, moving or not.
fn modes() -> impl Iterator<Item = (usize, bool)> {
    [1, 4].into_iter().flat_map(|threads| [false, true].map(|moving| (threads, moving)))
}

fn vm_in(threads: usize, moving: bool) -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_gc_threads(threads);
    vm.set_gc_moving(moving);
    vm.set_gc_verify(true);
    vm.eval_source(SETUP).unwrap();
    vm
}

fn shown(vm: &mut Vm, form: &str) -> String {
    vm.eval_source(form).unwrap().display().unwrap()
}

/// A buffer with undo on and a list that names a marker nothing else holds, by
/// `m` in ENTRIES; the same form for the pinned GNU.
const ORACLE_SETUP: &str = "(progn \
    (set-buffer (get-buffer-create \"undo\")) \
    (buffer-enable-undo) (insert \"hello world\") (setq buffer-undo-list nil) \
    (fset 'undo-test-touch (lambda () (let ((buffer-undo-list t)) (insert \"x\")))) \
    (fset 'undo-test-fill (lambda (entries) \
       (let ((marker (point-marker)) (list nil)) \
         (while entries \
           (setq list (cons (if (eq (car entries) 'm) (cons marker 1) (car entries)) list) \
                 entries (cdr entries))) \
         (setq buffer-undo-list (nreverse list)) nil))) t)";

/// What the pinned GNU prints for CHECK after FORMS, each at top level of its
/// own, and that the candidate prints the same, EXPECTED.
fn agree(forms: &[&str], check: &str, expected: &str) {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let printed = format!("(prin1 {check})");
    let mut args = vec!["-Q", "--batch", "--eval", ORACLE_SETUP];
    for form in forms.iter().chain([&printed.as_str()]) {
        args.extend(["--eval", form]);
    }
    let output = std::process::Command::new(oracle).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "pinned GNU");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(ORACLE_SETUP).unwrap();
    for form in forms {
        vm.eval_source(form).unwrap();
    }
    assert_eq!(shown(&mut vm, check), expected, "candidate");
}

#[test]
fn drops_an_entry_whose_marker_nothing_holds() {
    agree(&["(let ((m (copy-marker 5))) (delete-region 3 8) nil)", "(garbage-collect)"],
        "buffer-undo-list", "((\"llo w\" . 3))");
}

#[test]
fn keeps_an_entry_whose_marker_is_held() {
    agree(&["(setq held (copy-marker 5))", "(progn (delete-region 3 8) nil)", "(garbage-collect)"],
        "(list (length buffer-undo-list) (eq (car (nth 1 buffer-undo-list)) held))", "(2 t)");
}

#[test]
fn drops_marker_entries_wherever_they_are() {
    agree(&["(undo-test-fill '(m m (1 . 2) m nil m (3 . 4) m m))", "(garbage-collect)"],
        "buffer-undo-list", "((1 . 2) nil (3 . 4))");
}

#[test]
fn all_entries_dropped_leaves_nil() {
    agree(&["(undo-test-fill '(m m))", "(garbage-collect)"], "buffer-undo-list", "nil");
}

/// `compact_undo_list` looks before it marks: a marker only another entry names
/// is not marked yet, so its own entry goes, and it lives on through the other.
#[test]
fn an_entry_whose_marker_another_entry_holds_goes() {
    agree(&["(let ((marker (point-marker))) \
               (setq buffer-undo-list (list (cons marker 1) (list nil 'face marker 1 2))) nil)",
            "(garbage-collect)"],
        "(list (length buffer-undo-list) (markerp (nth 2 (car buffer-undo-list))))", "(1 t)");
}

/// `compact_buffer` passes over a buffer unchanged since the last; `compact_undo_list` does not.
#[test]
fn strips_the_markers_of_a_buffer_unchanged_since() {
    agree(&["(setq undo-limit 40 undo-strong-limit 80 undo-outer-limit 100000)",
            "(progn (undo-test-touch) (garbage-collect) nil)",
            "(undo-test-fill '(m (1 . 2) nil (3 . 4) nil (5 . 6)))",
            "(garbage-collect)"],
        "buffer-undo-list", "((1 . 2) nil (3 . 4) nil (5 . 6))");
}

/// The cut comes first, and the kept record's last entry then goes as well.
#[test]
fn a_cut_record_ending_in_a_dropped_marker_ends_clean() {
    agree(&["(setq undo-limit 60 undo-strong-limit 1000 undo-outer-limit 100000)",
            "(undo-test-touch)",
            "(undo-test-fill '((1 . 2) nil (3 . 4) m nil (5 . 6) nil (7 . 8)))",
            "(garbage-collect)"],
        "buffer-undo-list", "((1 . 2) nil (3 . 4))");
}

#[test]
fn a_unibyte_buffer_is_treated_alike() {
    agree(&["(setq undo-limit 100 undo-strong-limit 1000 undo-outer-limit 100000)",
            "(progn (set-buffer-multibyte nil) (undo-test-touch))",
            "(undo-test-fill '(m (1 . 2) nil (3 . 4) m nil (5 . 6) nil (7 . 8)))",
            "(garbage-collect)"],
        "buffer-undo-list", "((1 . 2) nil (3 . 4))");
}

/// One list, the base's: an entry made through the indirect buffer goes too,
/// and both buffers see it gone.
#[test]
fn an_indirect_buffer_shares_its_bases_list() {
    agree(&["(setq indirect (make-indirect-buffer (current-buffer) \"undo-indirect\"))",
            "(with-current-buffer indirect (let ((m (copy-marker 5))) (delete-region 3 8) nil))",
            "(garbage-collect)"],
        "(list buffer-undo-list (buffer-local-value 'buffer-undo-list indirect))",
        "(((\"llo w\" . 3)) ((\"llo w\" . 3)))");
}

/// A clone's copy of the variables is no second holder of the list.
#[test]
fn a_clone_does_not_hold_the_list_it_was_cloned_with() {
    agree(&["(let ((m (copy-marker 5))) (delete-region 3 8) nil)",
            "(setq clone (make-indirect-buffer (current-buffer) \"undo-clone\" t))",
            "(garbage-collect)"],
        "(list buffer-undo-list (eq buffer-undo-list (buffer-local-value 'buffer-undo-list clone)))",
        "(((\"llo w\" . 3)) t)");
}

#[test]
fn every_mode_cuts_and_strips_and_keeps_what_is_held() {
    for (threads, moving) in modes() {
        let mode = format!("{threads} threads, moving {moving}");
        let mut vm = vm_in(threads, moving);
        // What the moving collector has to move the list past.
        vm.eval_source("(let ((i 0)) (while (< i 10000) (cons i nil) (setq i (1+ i))))").unwrap();
        vm.eval_source("(undo-test-fill '(m (1 . 2) nil (3 . 4) m nil (5 . 6) nil (7 . 8)))").unwrap();
        vm.collect_garbage();
        vm.collect_garbage();
        assert_eq!(shown(&mut vm, "buffer-undo-list"), "((1 . 2) nil (3 . 4))", "{mode}");
        // Nothing is cut until the buffer changes, and a held marker stays.
        vm.eval_source("(setq undo-test-held (point-marker) buffer-undo-list (list (cons undo-test-held 1) '(1 . 2)))").unwrap();
        vm.collect_garbage();
        assert_eq!(shown(&mut vm, "(list (eq (car (car buffer-undo-list)) undo-test-held) (cdr buffer-undo-list))"), "(t ((1 . 2)))", "{mode}");
    }
}

/// A list something else holds is all reached before the cut, and the cut still
/// comes: what it leaves behind is marked, reachable from nothing, and dies next
/// time. A verifying mark has to agree about that with the threads'.
#[test]
fn a_cut_of_a_list_something_else_holds() {
    for (threads, moving) in modes() {
        let mode = format!("{threads} threads, moving {moving}");
        let mut vm = vm_in(threads, moving);
        vm.eval_source("(undo-test-fill '((1 . 2) nil (3 . 4) nil (5 . 6) nil (7 . 8)))").unwrap();
        vm.eval_source("(setq undo-test-saved buffer-undo-list)").unwrap();
        vm.collect_garbage();
        vm.collect_garbage();
        assert_eq!(shown(&mut vm, "(list (eq undo-test-saved buffer-undo-list) buffer-undo-list)"), "(t ((1 . 2) nil (3 . 4) nil (5 . 6)))", "{mode}");
    }
}

/// A host with a key press queued: the first look finds it.
struct KeyQueued;
impl HostInput for KeyQueued {
    fn input_pending(&mut self) -> bool { false }
    fn key_pending(&mut self) -> bool { true }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        Err("no events".into())
    }
}

/// A host whose key press arrives at its AT-th look, counting the looks.
struct KeyAt {
    at: usize,
    looks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl HostInput for KeyAt {
    fn input_pending(&mut self) -> bool { false }
    fn key_pending(&mut self) -> bool {
        self.looks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 >= self.at
    }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        Err("no events".into())
    }
}

/// A key press that arrives while the undo round is at work ends the mark:
/// the round looks for one between buffers, as it decides and again as it
/// walks, and a scan of a list this short makes no look of its own. With
/// 1,500 buffers that is 1,500 looks and then 1,500 more; the rest of this
/// small heap's mark makes a few dozen, so a press at the 2,500th look is
/// seen only by a round that looks in both.
#[test]
fn a_key_press_during_the_undo_round_ends_the_mark() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_gc_threads(1);
    vm.eval_source("(let ((i 0)) \
        (while (< i 1500) \
          (with-current-buffer (get-buffer-create (format \"undo-%d\" i)) \
            (buffer-enable-undo) (insert \"x\") \
            (setq buffer-undo-list (list '(1 . 2) nil '(3 . 4)))) \
          (setq i (1+ i))))").unwrap();
    let looks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    vm.set_host_input(Some(Box::new(KeyAt { at: 2500, looks: looks.clone() })));
    assert!(!vm.collect_garbage_until_key(), "the press was seen, and the mark given up");
    vm.set_host_input(None);
    let seen = looks.load(std::sync::atomic::Ordering::Relaxed);
    assert!((2500..3200).contains(&seen), "{seen} looks: in the round, not after it");
}

#[test]
fn an_abandoned_mark_leaves_the_lists_as_they_were() {
    for (threads, moving) in modes() {
        let mode = format!("{threads} threads, moving {moving}");
        let mut vm = vm_in(threads, moving);
        vm.eval_source("(undo-test-fill '(m (1 . 2) nil (3 . 4) m nil (5 . 6) nil (7 . 8)))").unwrap();
        vm.set_host_input(Some(Box::new(KeyQueued)));
        assert!(!vm.collect_garbage_until_key(), "{mode}");
        vm.set_host_input(None);
        assert_eq!(shown(&mut vm, "(length buffer-undo-list)"), "9", "{mode}: cut");
        assert_eq!(shown(&mut vm, "(markerp (car (car buffer-undo-list)))"), "t", "{mode}: stripped");
        // The cut still comes: an abandoned mark is no compaction of the buffer.
        vm.collect_garbage();
        assert_eq!(shown(&mut vm, "buffer-undo-list"), "((1 . 2) nil (3 . 4))", "{mode}");
    }
}

/// GNU's walks would not end. Here a circular list is no reason to hang.
#[test]
fn a_circular_list_ends_the_walks() {
    for (threads, moving) in modes() {
        let mode = format!("{threads} threads, moving {moving}");
        let mut vm = vm_in(threads, moving);
        // No boundary, so no cut: the walk has to see it has come round.
        vm.eval_source("(undo-test-circle (list '(1 . 2) '(3 . 4) '(5 . 6)))").unwrap();
        vm.collect_garbage();
        assert_eq!(shown(&mut vm, "(nth 3 buffer-undo-list)"), "(1 . 2)", "{mode}");
        // A circle all of whose entries would go.
        vm.eval_source("(undo-test-touch) (undo-test-fill '(m m)) (undo-test-circle buffer-undo-list)").unwrap();
        vm.collect_garbage();
        assert_eq!(shown(&mut vm, "(markerp (car (nth 2 buffer-undo-list)))"), "t", "{mode}");
    }
}

/// An error from the function is logged and leaves the list to it
/// (VM2-UNDO-OUTER); GNU's leaves `garbage-collect` with it.
#[test]
fn an_error_from_the_function_leaves_the_list() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(ORACLE_SETUP).unwrap();
    vm.eval_source("(setq undo-limit 10 undo-strong-limit 20 undo-outer-limit 40 undo-test-asked 0 \
        undo-outer-limit-function (lambda (_) (setq undo-test-asked (1+ undo-test-asked)) (error \"no\")))").unwrap();
    vm.eval_source("(undo-test-touch)").unwrap();
    vm.eval_source("(setq buffer-undo-list (list (cons \"abcdefghij\" 1) nil '(1 . 2) nil '(3 . 4)))").unwrap();
    vm.eval_source("(garbage-collect)").unwrap();
    vm.eval_source("(garbage-collect)").unwrap();
    assert_eq!(shown(&mut vm, "(list undo-test-asked (length buffer-undo-list))"), "(1 5)");
}

/// Each buffer is asked about as it stands when its turn comes: one the
/// function has already dealt with while answering for another is not.
#[test]
fn a_list_the_function_emptied_while_asked_about_another_is_not_offered() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(ORACLE_SETUP).unwrap();
    vm.eval_source("(setq undo-limit 10 undo-strong-limit 20 undo-outer-limit 40 undo-test-asked nil \
        undo-outer-limit-function (lambda (_) \
          (setq undo-test-asked (cons (buffer-name) undo-test-asked)) \
          (let ((here (buffer-name)) (names '(\"undo\" \"other\"))) \
            (while names \
              (if (equal (car names) here) nil \
                (with-current-buffer (car names) (setq buffer-undo-list nil))) \
              (setq names (cdr names)))) \
          t))").unwrap();
    for name in ["undo", "other"] {
        vm.eval_source(&format!("(progn (set-buffer (get-buffer-create \"{name}\")) (buffer-enable-undo) \
            (let ((buffer-undo-list t)) (insert \"x\")) \
            (setq buffer-undo-list (list (cons \"abcdefghij\" 1) nil '(1 . 2) nil '(3 . 4))) nil)")).unwrap();
    }
    vm.eval_source("(garbage-collect)").unwrap();
    vm.eval_source("(garbage-collect)").unwrap();
    assert_eq!(shown(&mut vm, "(length undo-test-asked)"), "1");
}

/// A run asks about each buffer once, and about every one: a function that,
/// asked about one buffer, puts the others past the limit again, those it
/// has already answered for among them, neither keeps the run going nor
/// hides a buffer not yet asked about behind one that was.
#[test]
fn a_run_asks_about_each_buffer_once() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(ORACLE_SETUP).unwrap();
    vm.eval_source("(fset 'undo-test-over (lambda (name) \
          (with-current-buffer (get-buffer-create name) \
            (buffer-enable-undo) \
            (let ((buffer-undo-list t)) (insert \"x\")) \
            (setq buffer-undo-list (list (cons \"abcdefghij\" 1) nil '(1 . 2) nil '(3 . 4))) nil)))").unwrap();
    vm.eval_source("(setq undo-limit 10 undo-strong-limit 20 undo-outer-limit 40 undo-test-asked 0 \
        undo-outer-limit-function (lambda (_) \
          (setq undo-test-asked (1+ undo-test-asked)) \
          (if (< undo-test-asked 10) \
              (let ((here (buffer-name)) (names '(\"undo\" \"other\" \"third\"))) \
                (while names \
                  (if (equal (car names) here) nil (undo-test-over (car names))) \
                  (setq names (cdr names))))) \
          t))").unwrap();
    vm.eval_source("(progn (undo-test-over \"undo\") (undo-test-over \"other\") (undo-test-over \"third\"))").unwrap();
    vm.eval_source("(garbage-collect)").unwrap();
    assert_eq!(shown(&mut vm, "undo-test-asked"), "3");
}

/// A collection nobody asked for runs `undo-outer-limit-function` at a later
/// safe point, with the list's buffer current for it and the running code's
/// put back; the same forms on the pinned GNU give the same answers.
#[test]
fn an_automatic_collection_offers_the_list_at_a_safe_point() {
    let form = "(with-temp-buffer \
        (let ((here (current-buffer)) (asked nil) (undo (get-buffer-create \"undo\"))) \
          (with-current-buffer undo \
            (buffer-enable-undo) \
            (let ((buffer-undo-list t)) (insert \"x\")) \
            (setq buffer-undo-list (list (cons \"abcdefghij\" 1) nil '(1 . 2) nil '(3 . 4)))) \
          (setq undo-limit 10 undo-strong-limit 20 undo-outer-limit 40 \
                undo-outer-limit-function \
                (lambda (size) (setq asked (list size (buffer-name))) nil)) \
          (let ((i 0)) (while (and (not asked) (< i 20000000)) (cons i nil) (setq i (1+ i)))) \
          (setq undo-outer-limit-function nil) \
          (list asked (eq (current-buffer) here) \
                (length (buffer-local-value 'buffer-undo-list undo)))))";
    let expected = "((73 \"undo\") t 1)";
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {form})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "pinned GNU");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(shown(&mut vm, form), expected, "candidate");
}
