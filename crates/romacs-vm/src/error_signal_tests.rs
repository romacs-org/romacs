//! `error` must signal a typed condition, never one guessed from its message.
//!
//! `route_escape`'s text-matching fallback exists to classify VM-INTERNAL error
//! strings ("wrong primitive arity" and friends). When the `error` primitive
//! routed a user's message through that same channel, the user's prose chose the
//! condition: `(error "x required")` hit the `contains("required")` arm and was
//! caught as `wrong-type-argument`, and `(error "unbound variable")` was caught
//! as `void-variable`. The data was dropped too, because that fallback signals
//! with `Value::NIL`.
//!
//! GNU `src/eval.c` `Ferror` signals `Qerror` with the formatted message in the
//! data list. Oracle, GNU 32.0.50, `(condition-case e (error "boom") (error e))`
//! => `(error "boom")`.
use super::*;

// GNU src/eval.c:2752 preserves original_fun; src/data.c:1657 signals symbol.
macro_rules! void_case {
    ($name:ident, $source:literal, $want:literal) => {
        #[test]
        fn $name() {
            let mut vm = Vm::new();
            assert_eq!(vm.eval_source_raw($source).unwrap_err(), $want);
            assert_eq!(text(&mut vm, &format!("(condition-case e {} (error e))", $source)), $want);
        }
    };
}
void_case!(void_fault_direct, "(missing-census-function)", "(void-function missing-census-function)");
void_case!(void_fault_funcall, "(funcall 'missing-census-function)", "(void-function missing-census-function)");
void_case!(void_fault_apply, "(apply 'missing-census-function nil)", "(void-function missing-census-function)");
void_case!(void_fault_global, "missing-census-variable", "(void-variable missing-census-variable)");
void_case!(void_fault_symbol_value, "(symbol-value 'missing-census-variable)", "(void-variable missing-census-variable)");
void_case!(void_fault_direct_tier, "(+ missing-census-variable 1)", "(void-variable missing-census-variable)");
void_case!(void_fault_alias, "(progn (fset 'census-alias 'missing-census-function) (census-alias))", "(void-function census-alias)");

#[test]
fn void_fault_payload_survives_cleanup_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    for _ in 0..100 { vm.conses.push((Value::NIL, Value::NIL)); }
    let before = vm.heap_stats().collections;
    assert_eq!(text(&mut vm, "(condition-case e (unwind-protect (missing-census-function) (let ((n 0)) (while (< n 1000) (cons n n) (setq n (1+ n))))) (void-function e))"), "(void-function missing-census-function)");
    assert!(vm.heap_stats().collections > before);
}

fn text(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap();
    vm.display(value).unwrap()
}

/// The three inputs recorded in the s121 review reproduction. Each message is
/// chosen to collide with a different arm of the text-matching fallback, so a
/// guessed condition cannot pass by luck.
#[test]
fn error_signals_the_error_condition_whatever_its_message_says() {
    let mut vm = Vm::new();
    for (form, want) in [
        ("(condition-case e (error \"boom\") (error (car e)))", "error"),
        // "required" is the wrong-type-argument arm of the fallback.
        ("(condition-case e (error \"x required\") (error (car e)))", "error"),
        // "unbound variable" is the void-variable arm.
        (
            "(condition-case e (error \"unbound variable\") (error (car e)))",
            "error",
        ),
    ] {
        assert_eq!(
            text(&mut vm, form),
            want,
            "condition was taken from the message text, not from the signal: {form}"
        );
    }
}

/// GNU keeps the formatted message as the error's data. The fallback discarded
/// it by signalling `Value::NIL`, so a handler could not see what went wrong.
#[test]
fn error_carries_its_formatted_message_as_data() {
    let mut vm = Vm::new();
    assert_eq!(
        text(&mut vm, "(condition-case e (error \"boom\") (error (cdr e)))"),
        "(\"boom\")",
        "error data lost; GNU gives (error \"boom\")"
    );
    assert_eq!(
        text(
            &mut vm,
            "(condition-case e (error \"n=%d\" 7) (error (cdr e)))"
        ),
        "(\"n=7\")",
        "format arguments must be applied before signalling"
    );
}

/// A wrong condition is only caught by the wrong handler if a more specific
/// handler can steal it. This is the observable harm, stated as behaviour
/// rather than as representation.
#[test]
fn a_specific_handler_does_not_steal_a_plain_error() {
    let mut vm = Vm::new();
    assert_eq!(
        text(
            &mut vm,
            "(condition-case nil (error \"x required\") \
             (wrong-type-argument 'stolen) (error 'correct))"
        ),
        "correct",
        "wrong-type-argument handler caught a plain `error`"
    );
    assert_eq!(
        text(
            &mut vm,
            "(condition-case nil (error \"unbound variable\") \
             (void-variable 'stolen) (error 'correct))"
        ),
        "correct",
        "void-variable handler caught a plain `error`"
    );
}

/// The internal fallback must keep working: it is what gives VM-raised faults
/// their conditions, and narrowing the `error` primitive must not disturb it.
#[test]
fn internal_faults_keep_their_typed_conditions() {
    let mut vm = Vm::new();
    assert_eq!(
        text(&mut vm, "(condition-case nil (car 1) (wrong-type-argument 'caught))"),
        "caught"
    );
    assert_eq!(
        text(
            &mut vm,
            "(condition-case nil undefined-var-xyz (void-variable 'caught))"
        ),
        "caught"
    );
}

/// The signal's data is heap-allocated immediately before the escape is built.
/// `Value` indices move, so a collection across that window would corrupt the
/// message a handler reads. Collections are counted rather than assumed: a low
/// floor does not by itself prove compaction happened.
#[test]
fn error_data_survives_a_forced_moving_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let before = vm.heap_stats().collections;
    // Dead conses ahead of the signal so the surviving data has somewhere to
    // move from.
    for _ in 0..100 {
        vm.conses.push((Value::NIL, Value::NIL));
    }
    assert_eq!(
        text(
            &mut vm,
            "(condition-case e (error \"survives %s\" \"gc\") (error (cdr e)))"
        ),
        "(\"survives gc\")",
        "error data did not survive a moving collection"
    );
    assert!(
        vm.heap_stats().collections > before,
        "fixture collected nothing, so it proves nothing about moving data"
    );
}

/// A VM-internal message the escape fallback cannot classify still reaches
/// the handler as `error' data. The witness is a printer setting this VM
/// refuses; pick another unsupported one if it becomes supported.
#[test]
fn unclassified_internal_error_keeps_its_message() {
    let mut vm = Vm::new();
    assert_eq!(
        text(&mut vm, "(condition-case e (let ((print-integers-as-characters t)) (prin1-to-string 'a)) (error e))"),
        "(error \"printing print-integers-as-characters is not supported\")"
    );
}
