//! `apply' spreads any number of arguments, as GNU's Fapply does: gnosis
//! flattens a 39k-row query with (apply #'append rows).
use romacs_vm::Vm;

#[test]
fn apply_spreads_beyond_the_initial_reserve() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(length (apply 'list (make-list 100000 1)))", "100000"),
        ("(let ((rows nil) (i 0)) (while (< i 40000) (setq rows (cons (list i) rows) i (1+ i)))
            (length (apply 'append rows)))", "40000"),
        ("(progn (defun spread-rest (&rest xs) (length xs)) (apply 'spread-rest 1 2 (make-list 9000 0)))", "9002"),
        // A spread inside a callee of a wide spread, then a narrow one again.
        ("(progn (defun spread-inner (&rest xs) (+ (length xs) (apply '+ (make-list 5000 1))))
                 (list (apply 'spread-inner (make-list 20000 0)) (apply '+ 1 '(2 3))))", "(25000 6)"),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}
