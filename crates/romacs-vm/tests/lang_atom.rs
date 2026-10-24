//! `atom` — GNU returns t for every object that is not a cons cell, nil included.
//!
//! Oracle, both forms agreeing:
//!   source    gnu-emacs-ref src/data.c:321 at revision 45b0f7699df
//!             `if (CONSP (object)) return Qnil; return Qt;`
//!   behaviour live GNU Emacs 32.0.50, answers recorded below verbatim
//!
//! GNU declares this `attributes: const`: it is total, takes exactly one
//! argument, and signals nothing. There is no error case to match.
use romacs_vm::Vm;

/// Every answer here was printed by `emacs -Q --batch` at 32.0.50, not inferred.
const GNU_ANSWERS: &[(&str, &str)] = &[
    ("(atom 1)", "t"),
    ("(atom \"s\")", "t"),
    ("(atom 'a)", "t"),
    ("(atom nil)", "t"),
    ("(atom [1])", "t"),
    ("(atom 1.5)", "t"),
    ("(atom (make-hash-table))", "t"),
    // The only non-atoms GNU recognises:
    ("(atom '(1 . 2))", "nil"),
    ("(atom '(1 2))", "nil"),
];

#[test]
fn atom_matches_gnu_on_every_recorded_answer() {
    for &(src, gnu) in GNU_ANSWERS {
        let mut vm = Vm::new();
        let value = vm
            .eval_source(src)
            .unwrap_or_else(|e| panic!("{src} must evaluate, GNU answers {gnu}: {e}"));
        let got = value.display().expect("result must print");
        assert_eq!(got, gnu, "GNU 32.0.50: {src} => {gnu}");
    }
}

/// `atom` and `consp` must stay exact complements. That invariant is the reason
/// `atom` is worth a primitive rather than a Lisp-level alias, and a mutant that
/// makes `atom` agree with `consp` on any input is caught here.
#[test]
fn atom_is_the_exact_complement_of_consp() {
    for &(src, _) in GNU_ANSWERS {
        let inner = src
            .strip_prefix("(atom ")
            .and_then(|s| s.strip_suffix(')'))
            .expect("recorded sources are all (atom X)");
        let mut vm = Vm::new();
        let a = vm
            .eval_source(&format!("(atom {inner})"))
            .and_then(|v| v.display())
            .unwrap_or_else(|e| panic!("atom {inner}: {e}"));
        let c = vm
            .eval_source(&format!("(consp {inner})"))
            .and_then(|v| v.display())
            .unwrap_or_else(|e| panic!("consp {inner}: {e}"));
        assert_ne!(a, c, "atom and consp must disagree for {inner}");
    }
}
