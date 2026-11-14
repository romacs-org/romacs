//! What reading and writing one character of a char table costs, by the
//! evaluating thread's instruction count (`romacs-instructions`). In a
//! process of its own: a char-table write renews what is derived from
//! every table (`char_table::writes`), so these writes would cost the
//! cached syntax and regexp lookups other tests measure.
use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(romacs_vm::Local::display).unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// Instructions one evaluation of BODY costs over one of `i`, in a loop
/// binding `i` (the counter) and `s` (the last value).
fn per_call(vm: &mut Vm, body: &str) -> f64 {
    let mut run = |body: &str, n: usize| -> f64 {
        let form = format!(
            "(let ((f (lambda (n) (let ((i 0) (s 0)) (while (< i n) (setq s {body}) (setq i (1+ i))) s)))) \
               (funcall f 10) (let ((a (romacs-instructions))) (funcall f {n}) (and a (- (romacs-instructions) a))))"
        );
        let text = eval(vm, &form);
        text.parse().unwrap_or_else(|_| panic!("no instruction counter (romacs-instructions gave {text})"))
    };
    let with = (run(body, 3000) - run(body, 1000)) / 2000.0;
    let without = (run("i", 3000) - run("i", 1000)) / 2000.0;
    with - without
}

/// A character read from or written into a char table walks the nodes on
/// its path once, and allocates nothing: `ucs-normalize` builds its
/// translation tables with tens of thousands of `aset`s at load.
#[test]
fn a_char_table_access_walks_only_its_path() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    eval(&mut vm, "(defvar ct (make-char-table 'test))");
    // 1,353 when written; 8,642 when a write cloned every node it checked.
    // Pinned GNU's is about 300; an ASCII write here costs 967, which is the
    // primitive call's own share.
    let write = per_call(&mut vm, "(aset ct (+ #x4e00 (* 3 i)) i)");
    assert!(write < 1600.0, "a char-table write costs {write:.0} instructions");
    // 1,474 when written; 2,696 when a read allocated a set of the nodes seen.
    let read = per_call(&mut vm, "(aref ct (+ #x4e00 (* 3 i)))");
    assert!(read < 1700.0, "a char-table read costs {read:.0} instructions");
    // Ranges and single characters land where they did, across blocks.
    assert_eq!(eval(&mut vm, "(let ((c (make-char-table 'test))) (set-char-table-range c '(#x4dfe . #x4e81) 'r) \
        (aset c #x4e00 'a) (aset c #x10ffff 'z) (aset c 65 'ascii) \
        (list (aref c #x4dfd) (aref c #x4dfe) (aref c #x4e00) (aref c #x4e01) (aref c #x4e81) (aref c #x4e82) (aref c #x10ffff) (aref c 65)))"),
        "(nil r a r r nil z ascii)");
}
