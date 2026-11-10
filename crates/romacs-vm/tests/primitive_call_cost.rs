//! What a call from compiled Lisp into a primitive costs, by the evaluating
//! thread's instruction count (`romacs-instructions`): a loop making the
//! call, less the same loop without it, at two lengths. A machine without
//! the counter fails these tests: they never pass unmeasured.
use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(romacs_vm::Local::display).unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// Instructions one evaluation of BODY costs over one of `i`, in a loop
/// binding `i` (the counter) and `s` (the last value).
fn per_call(vm: &mut Vm, body: &str) -> f64 {
    per_call_in(vm, "progn", body)
}

/// `per_call` with the loop inside the form WRAP opens, as
/// `with-temp-buffer (insert ...)`.
fn per_call_in(vm: &mut Vm, wrap: &str, body: &str) -> f64 {
    let mut run = |body: &str, n: usize| -> f64 {
        let form = format!(
            "({wrap} (let ((f (lambda (n) (let ((i 0) (s 0)) (while (< i n) (setq s {body}) (setq i (1+ i))) s)))) \
               (funcall f 10) (let ((a (romacs-instructions))) (funcall f {n}) (and a (- (romacs-instructions) a)))))"
        );
        let text = eval(vm, &form);
        text.parse().unwrap_or_else(|_| panic!("no instruction counter (romacs-instructions gave {text})"))
    };
    let with = (run(body, 3000) - run(body, 1000)) / 2000.0;
    let without = (run("i", 3000) - run("i", 1000)) / 2000.0;
    with - without
}

/// The bitwise primitives fold fixnums in machine words, as GNU's
/// `arith_driver` does; a bignum per operand cost several hundred
/// instructions a call.
#[test]
fn bitwise_on_fixnums_stays_in_machine_words() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for body in ["(logand i 1023)", "(logior i 1 2)", "(logxor i 5)", "(lognot i)", "(ash i 2)", "(ash i -1)", "(logcount i)"] {
        let cost = per_call(&mut vm, body);
        // 290-350 with the fold; 770-990 through bignums.
        assert!(cost < 450.0, "{body} costs {cost:.0} instructions a call");
    }
    assert_eq!(
        eval(&mut vm, "(list (logand 12 10) (logand) (logior 12 3) (logxor 12 10) (lognot 5) (ash 5 2) (ash -5 -1) (ash 1 70) (logcount -1) (logand most-negative-fixnum -1))"),
        "(8 -1 15 6 -6 20 -3 1180591620717411303424 0 -2305843009213693952)"
    );
    // The edges of the fold: the first bignum, a shift past the word, the
    // widest left shift of a fixnum, a marker, and the complement's end.
    assert_eq!(
        eval(&mut vm, "(list (ash 1 61) (ash -1 -64) (ash most-negative-fixnum 1) (with-temp-buffer (logand (point-marker))) (lognot most-negative-fixnum))"),
        "(2305843009213693952 -1 -4611686018427387904 1 2305843009213693951)"
    );
}
