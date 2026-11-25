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
    let got = shown(&mut vm, "(let ((unread-command-events (list 'f5 'f6 'f7 'f8)))
                                (list (read-key-sequence-vector nil) (this-single-command-raw-keys)
                                      (read-key-sequence-vector nil) (this-command-keys-vector)
                                      (reverse log) ends))");
    vm.set_gc_stress(0);
    assert_eq!(got, "([f5] [f5] [f9] [f9] ([f6] [f8]) 4)");
    assert_eq!(vm.reads.depth(super::MAIN), 0, "every read's frame popped");
}

/// A throw and a signal out of a nested read leave no frame behind, and
/// the outer read's end hook runs once with the escape kept.
#[test]
fn escapes_pop_every_frame() {
    let mut vm = Vm::new();
    shown(&mut vm, "(progn (setq ends 0 romacs--sequence-end-functions (list (lambda () (setq ends (1+ ends)))))
                     (setq key-translation-map (make-sparse-keymap))
                     (define-key key-translation-map [f5] (lambda (_) (throw 'out 'thrown)))
                     (define-key key-translation-map [f6] (lambda (_) (error \"bad\")))
                     nil)");
    let got = shown(&mut vm, "(let ((unread-command-events (list 'f5 'f6 'f7)))
                                (list (catch 'out (read-key-sequence-vector nil))
                                      (condition-case e (read-key-sequence-vector nil) (error (car (cdr e))))
                                      (read-key-sequence-vector nil) ends))");
    assert_eq!(got, "(thrown \"bad\" [f7] 3)");
    assert_eq!(vm.reads.depth(super::MAIN), 0);
}
