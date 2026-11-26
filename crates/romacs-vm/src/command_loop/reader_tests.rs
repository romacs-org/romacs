//! The reader's pure parts, and its frames under a moving collector.
use super::keyremap::{Scan, unshifted_char, unshifted_name};
use crate::{Value, Vm};

/// The shift fallback's decision: a shift bit cleared, an upper-case
/// letter's lower case with its modifiers kept, and nothing else.
#[test]
fn unshifted_events() {
    let down = |c: i64| if (65..=90).contains(&c) { c + 32 } else { c };
    assert_eq!(unshifted_char(0x200_0061, down), Some(0x61));
    assert_eq!(unshifted_char(0x41, down), Some(0x61));
    assert_eq!(unshifted_char(0x800_0041, down), Some(0x800_0061));
    assert_eq!(unshifted_char(0x61, down), None);
    assert_eq!(unshifted_name("S-backspace").as_deref(), Some("backspace"));
    assert_eq!(unshifted_name("C-M-S-f5").as_deref(), Some("C-M-f5"));
    assert_eq!(unshifted_name("C-f5"), None);
    assert_eq!(unshifted_name("s-S-x"), None);
}

/// A scan restarts from its whole map and moves with a translation before it.
#[test]
fn scan_reset_and_shift() {
    let mut scan = Scan::new(Value::TRUE);
    scan.map = Value::NIL;
    scan.end = 3;
    scan.reset(2);
    assert_eq!((scan.map, scan.start, scan.end), (Value::TRUE, 2, 2));
    scan.shift(-1);
    assert_eq!((scan.start, scan.end), (1, 1));
    scan.shift(2);
    assert_eq!((scan.start, scan.end), (3, 3));
}

fn shown(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap();
    vm.display(value).unwrap()
}

/// Reads nested in a filter and in a translation function, every Lisp
/// call moving every live object: the outer read's frame, keys, raw keys
/// and walk are traced and rewritten, and each started read ends once.
#[test]
fn nested_reads_survive_moving_collections() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    vm.set_gc_moving(true);
    shown(&mut vm, "(progn (setq ends 0 log nil romacs--sequence-end-functions (list (lambda () (setq ends (1+ ends)))))
                     (use-local-map (make-sparse-keymap))
                     (define-key (current-local-map) [f5] (list 'menu-item \"x\" 'ignore :filter
                                               (lambda (b) (push (read-key-sequence-vector nil) log) b)))
                     (setq key-translation-map (make-sparse-keymap))
                     (define-key key-translation-map [f7] (lambda (_) (push (read-key-sequence-vector nil) log) [f9]))
                     nil)");
    vm.set_gc_stress(1);
    // Events with parameters, heap conses the collector moves.
    let got = shown(&mut vm, "(let ((unread-command-events (list (list 'f5 1) (list 'f6 2) (list 'f7 3) (list 'f8 4))))
                                (list (read-key-sequence-vector nil) (this-single-command-raw-keys)
                                      (read-key-sequence-vector nil) (this-command-keys-vector)
                                      (reverse log) ends))");
    vm.set_gc_stress(0);
    assert_eq!(got, "([(f5 1)] [(f5 1)] [f9] [f9] ([(f6 2)] [(f8 4)]) 4)");
    assert_eq!(vm.reads.depth(super::MAIN), 0, "every read's frame popped");
}

/// A throw and a signal out of a nested read leave no frame behind, and
/// the outer read's end hook runs once with the escape kept.
#[test]
fn escapes_pop_every_frame() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    shown(&mut vm, "(progn (setq ends 0 romacs--sequence-end-functions (list (lambda () (setq ends (1+ ends)))))
                     (setq key-translation-map (make-sparse-keymap))
                     (define-key key-translation-map [f5] (lambda (_) (throw 'out 'thrown)))
                     (define-key key-translation-map [f6] (lambda (_) (error \"bad\")))
                     (use-local-map (make-sparse-keymap))
                     ;; A filter whose nested read throws: caught inside it,
                     ;; the outer read goes on; uncaught, both reads leave.
                     (define-key (current-local-map) [f8] (list 'menu-item \"x\" 'ignore :filter
                       (lambda (b) (catch 'inner (let ((key-translation-map (make-sparse-keymap)))
                                                   (define-key key-translation-map [f9] (lambda (_) (throw 'inner nil)))
                                                   (read-key-sequence-vector nil)))
                               b)))
                     (define-key (current-local-map) [f10] (list 'menu-item \"x\" 'ignore :filter
                       (lambda (b) (read-key-sequence-vector nil) b)))
                     nil)");
    let got = shown(&mut vm, "(let ((unread-command-events (list 'f5 'f6 'f7 'f8 'f9 'f10 'f5)))
                                (list (catch 'out (read-key-sequence-vector nil))
                                      (condition-case e (read-key-sequence-vector nil) (error (car (cdr e))))
                                      (read-key-sequence-vector nil)
                                      (read-key-sequence-vector nil)
                                      (catch 'out (read-key-sequence-vector nil))
                                      ends))");
    assert_eq!(got, "(thrown \"bad\" [f7] [f8] thrown 7)");
    assert_eq!(vm.reads.depth(super::MAIN), 0, "every read's frame popped");
    assert!(vm.reads.walks.is_empty(), "no walk left at a filter");
}

/// GNU's third point: when the current buffer changes while `read_char`
/// waits (here a watcher of `last-input-event`, which the reader sets as the
/// Lisp port did), the sequence is replayed with the new buffer's maps.
#[test]
fn a_buffer_change_in_the_wait_replays_the_sequence() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let got = shown(&mut vm, "(let ((a (get-buffer-create \"a\")) (b (get-buffer-create \"b\")))
       (set-buffer b) (use-local-map (make-sparse-keymap)) (define-key (current-local-map) [f5 f6] 'in-b)
       (set-buffer a) (use-local-map (make-sparse-keymap)) (define-key (current-local-map) [f5 f6] 'in-a)
       (add-variable-watcher 'last-input-event (lambda (_ value _ _) (if (eq value 'f6) (set-buffer b))))
       (let ((unread-command-events (list 'f5 'f6)))
         (list (romacs--read-key-sequence nil t nil) romacs--key-sequence-command (buffer-name)
               (this-command-keys-vector))))");
    assert_eq!(got, "([f5 f6] in-b \"b\" [f5 f6])");
}

/// A prefix bound to a keymap still to autoload: the next event is unbound,
/// as the Lisp reader found it (VM2-AUTOLOAD1), not an error.
#[test]
fn an_autoloaded_prefix_map_reads_as_unbound() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    let got = shown(&mut vm, "(progn (autoload 'c2-map \"c2-nowhere\" nil nil 'keymap)
       (use-local-map (make-sparse-keymap)) (define-key (current-local-map) [f5] 'c2-map)
       (let ((unread-command-events (list 'f5 ?a)))
         (list (romacs--read-key-sequence nil t nil) romacs--key-sequence-command)))");
    assert_eq!(got, "([f5 97] nil)");
}

/// A VM with the prelude and the command loop's Lisp half, collecting and
/// moving every live object at each Lisp entry from then on.
fn moving_vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude_raw().unwrap();
    // Any read installs the Lisp half; then the collector moves at every entry.
    shown(&mut vm, "(let ((unread-command-events (list 'f1))) (read-key-sequence-vector nil))");
    vm.set_gc_moving(true);
    vm.set_gc_stress(1);
    vm
}

/// A prefix help key taken under a moving collector: the checkpoint's
/// binding and maps are traced and moved with it.
#[test]
fn a_prefix_help_checkpoint_survives_moving_collections() {
    let mut vm = moving_vm();
    let got = shown(&mut vm, "(progn (setq queue (list 24 8 97) romacs--host-input t)
       (use-local-map (make-sparse-keymap)) (define-key (current-local-map) [24 97] 'done)
       (setq romacs--prefix-help-functions (list (lambda (_p _e) (garbage-collect) t)))
       (fset 'romacs--read-host-char (lambda (&rest _) (prog1 (car queue) (setq queue (cdr queue)))))
       (prog1 (list (romacs--read-key-sequence nil t nil) romacs--key-sequence-command (this-command-keys-vector))
         (setq romacs--host-input nil romacs--prefix-help-functions nil)))");
    vm.set_gc_stress(0);
    assert_eq!(got, "([24 97] done [24 97])");
}

/// A `switch-frame' read after a prefix waits, traced, for the read after
/// the sequence.
#[test]
fn a_delayed_switch_frame_survives_moving_collections() {
    let mut vm = moving_vm();
    let got = shown(&mut vm, "(progn (use-local-map (make-sparse-keymap)) (define-key (current-local-map) [24 97] 'done)
       (let ((unread-command-events (list 24 (list 'switch-frame (selected-frame)) 97)))
         (list (read-key-sequence-vector nil) (car (read-event)) (eq (car (cdr (read-event nil nil 0))) nil))))");
    vm.set_gc_stress(0);
    assert_eq!(got, "([24 97] switch-frame t)");
}

/// An input method whose function reads a key sequence of its own, under a
/// moving collector.
#[test]
fn an_input_method_reading_keys_survives_moving_collections() {
    let mut vm = moving_vm();
    let got = shown(&mut vm, "(let ((input-method-function (lambda (c) (list c (aref (read-key-sequence-vector nil) 0))))
                                    (unread-command-events (list ?a (list 'f9 1))))
                                (list (read-event nil t) (read-event nil t)))");
    vm.set_gc_stress(0);
    assert_eq!(got, "(97 (f9 1))");
}
