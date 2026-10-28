//! GNU's built-in (C) variables: symbols forwarded to runtime storage
//! (ruling R216). GNU defines 948 variables in C with DEFVAR_LISP, DEFVAR_BOOL,
//! DEFVAR_INT and DEFVAR_PER_BUFFER (lisp.h `Lisp_Fwd`); every read, write,
//! `let` binding, `makunbound`, `default-value' and `set-default' of such a
//! symbol goes through its forwarding (data.c `find_symbol_value`,
//! `set_internal`, `default_value`, `set_default_internal`; eval.c `specbind`,
//! `do_one_unbind`).
//!
//! The four transcript tests replay, in a fresh VM each, a section recorded
//! from live GNU Emacs 32.0.50 (`/usr/bin/emacs -Q --batch`), also in a fresh
//! process each: evidence/c-variables/c-variables-oracle.el and the
//! c-variables-oracle-<section>.txt it wrote. No expected value is written by
//! hand here; every one is read from those transcripts.
use romacs_vm::Vm;

/// (form, GNU's printed outcome) pairs, in transcript order.
fn transcript(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with(";;")).collect();
    assert!(lines.len() >= 2 && lines.len() % 2 == 0, "malformed transcript");
    lines
        .chunks(2)
        .map(|pair| {
            let result = pair[1].strip_prefix("  => ").expect("transcript result line");
            (pair[0].to_owned(), result.to_owned())
        })
        .collect()
}

/// VM2's outcome printed exactly as the oracle printed GNU's.
fn outcome(vm: &mut Vm, form: &str) -> String {
    let source = format!("(prin1-to-string (condition-case err (list 'ok {form}) (error (list 'signal err))))");
    match vm.eval_source(&source) {
        Ok(value) => {
            let shown = value.display().unwrap_or_else(|e| format!("VM-ERROR {e}"));
            serde_json::from_str::<String>(&shown).unwrap_or(shown)
        }
        Err(e) => format!("VM-ERROR {e}"),
    }
}

fn replay(section: &str, text: &str) {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let mut bad = Vec::new();
    for (form, expected) in transcript(text) {
        let actual = outcome(&mut vm, &form);
        if actual != expected {
            bad.push(format!("{form}\n  GNU {expected}\n  VM2 {actual}"));
        }
    }
    assert!(bad.is_empty(), "{section}: {} divergences from GNU:\n{}", bad.len(), bad.join("\n"));
}

/// DEFVAR_LISP: minibuffer-local-map (keymap.c:3362), the chain's subr.el
/// form 303 frontier, plus system-type (emacs.c:3500).
#[test]
fn object_variables_forward_as_in_gnu() {
    replay("object", include_str!("../evidence/c-variables/c-variables-oracle-object.txt"));
}

/// DEFVAR_BOOL stores `!NILP (newval)`: any value is accepted and reads
/// back as t or nil (data.c store_symval_forwarding, Lisp_Fwd_Bool).
#[test]
fn bool_variables_coerce_as_in_gnu() {
    replay("bool", include_str!("../evidence/c-variables/c-variables-oracle-bool.txt"));
}

/// DEFVAR_INT rejects a non-integer with (wrong-type-argument integerp V)
/// through set, setq, let and set-default, leaving the old value.
#[test]
fn int_variables_reject_non_integers_as_in_gnu() {
    replay("int", include_str!("../evidence/c-variables/c-variables-oracle-int.txt"));
}

/// DEFVAR_PER_BUFFER: fill-column (buffer.c:5259). Default plus a per-buffer
/// value, automatically local when set, `let` binding the default when the
/// buffer has no local value, the integerp predicate on set but not on
/// set-default or a default `let`.
#[test]
fn per_buffer_variables_behave_as_in_gnu() {
    replay("per-buffer", include_str!("../evidence/c-variables/c-variables-oracle-per-buffer.txt"));
}

fn settled(vm: &mut Vm) -> (usize, usize) {
    vm.collect_garbage();
    let stats = vm.heap_stats();
    (stats.live_objects, stats.live_bytes)
}

fn shown(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    value.display().unwrap()
}

const GARBAGE: &str = "(let ((i 0) (g nil)) (while (< i 2000) (setq g (cons i g)) (setq i (1+ i))) nil)";
const PAYLOAD: &str = "(list 'cv-payload (make-string 65536 ?z) (let ((l nil) (i 300)) (while (> i 0) (setq l (cons i l) i (1- i))) l))";
const READBACK: &str = "(list (car minibuffer-local-map) (length (nth 1 minibuffer-local-map)) (aref (nth 1 minibuffer-local-map) 65535) (length (nth 2 minibuffer-local-map)) (apply #'+ (nth 2 minibuffer-local-map)))";
const EXPECTED: &str = "(cv-payload 65536 122 300 45150)";

/// A forwarded object value is collector-visible state held outside every
/// symbol value cell. With 2000 garbage conses allocated first, a forced
/// collection compacts them away and MOVES the payload, so the value must be
/// both marked (it stays live: its 300 conses and 64 KiB string are counted)
/// and forwarded (it still reads back intact). Measured against a twin VM
/// whose forwarded variable holds nil; dropping the payload returns live
/// objects AND bytes exactly to the twin's figures, so nothing leaks.
#[test]
fn forwarded_object_value_survives_moving_collection_and_is_reclaimed() {
    let mut twin = Vm::new();
    let mut held = Vm::new();
    for vm in [&mut twin, &mut held] {
        vm.eval_source(GARBAGE).unwrap();
    }
    twin.eval_source(&format!("(progn {PAYLOAD} (setq minibuffer-local-map nil))")).unwrap();
    held.eval_source(&format!("(setq minibuffer-local-map {PAYLOAD})")).unwrap();
    let (twin_objects, twin_bytes) = settled(&mut twin);
    let (held_objects, held_bytes) = settled(&mut held);
    eprintln!("twin {twin_objects} objects / {twin_bytes} bytes; held {held_objects} / {held_bytes}");
    assert!(
        held_objects >= twin_objects + 300 && held_bytes >= twin_bytes + 65_536,
        "a forwarded value must stay live across collection: twin {twin_objects}/{twin_bytes}, held {held_objects}/{held_bytes}"
    );
    assert_eq!(shown(&mut held, READBACK), EXPECTED, "a forwarded value must survive the moving collection");
    // Collect twice more: a stale (unforwarded) index would now point into
    // reused storage.
    held.eval_source(GARBAGE).unwrap();
    settled(&mut held);
    assert_eq!(shown(&mut held, READBACK), EXPECTED);
    held.eval_source("(setq minibuffer-local-map nil)").unwrap();
    twin.eval_source("(setq minibuffer-local-map nil)").unwrap();
    let (twin_after, twin_after_bytes) = settled(&mut twin);
    let (held_after, held_after_bytes) = settled(&mut held);
    eprintln!("dropped: twin {twin_after}/{twin_after_bytes} held {held_after}/{held_after_bytes}");
    assert_eq!((held_after, held_after_bytes), (twin_after, twin_after_bytes), "a dropped forwarded value must be reclaimed");
}

/// The same contract for the load-machinery DEFVAR_LISPs (lread.c:5637,
/// lread.c:5693): a held value must be marked (live objects AND bytes
/// counted) and rewritten (it reads back intact after the payload moved,
/// twice), and dropping it must reclaim exactly what it held. A `let` of
/// the variable saved across collections must be restored intact too.
#[test]
fn load_variables_hold_values_across_moving_collection() {
    for name in ["after-load-alist", "current-load-list"] {
        let readback = READBACK.replace("minibuffer-local-map", name);
        let mut twin = Vm::new();
        let mut held = Vm::new();
        for vm in [&mut twin, &mut held] {
            vm.eval_source(GARBAGE).unwrap();
        }
        twin.eval_source(&format!("(progn {PAYLOAD} (setq {name} nil))")).unwrap();
        held.eval_source(&format!("(setq {name} {PAYLOAD})")).unwrap();
        let (twin_objects, twin_bytes) = settled(&mut twin);
        let (held_objects, held_bytes) = settled(&mut held);
        eprintln!("{name}: twin {twin_objects}/{twin_bytes}; held {held_objects}/{held_bytes}");
        assert!(
            held_objects >= twin_objects + 300 && held_bytes >= twin_bytes + 65_536,
            "{name} must keep its value live: twin {twin_objects}/{twin_bytes}, held {held_objects}/{held_bytes}"
        );
        assert_eq!(shown(&mut held, &readback), EXPECTED, "{name} must survive the moving collection");
        held.eval_source(GARBAGE).unwrap();
        settled(&mut held);
        assert_eq!(shown(&mut held, &readback), EXPECTED, "{name} after a second collection");
        for vm in [&mut twin, &mut held] {
            vm.set_collection_floor(1);
            let before = vm.heap_stats().collections;
            let inner = shown(vm, &format!("(let (({name} 'cv-inner)) {GARBAGE} {GARBAGE} {name})"));
            assert_eq!(inner, "cv-inner");
            assert!(vm.heap_stats().collections > before, "the fixture must collect inside the binding");
        }
        assert_eq!(shown(&mut held, &readback), EXPECTED, "{name}: the let-saved value is restored intact");
        held.eval_source(&format!("(setq {name} nil)")).unwrap();
        let (twin_after, twin_after_bytes) = settled(&mut twin);
        let (held_after, held_after_bytes) = settled(&mut held);
        assert_eq!((held_after, held_after_bytes), (twin_after, twin_after_bytes), "{name}: a dropped value must be reclaimed");
    }
}

/// The value a `let` saves for a forwarded variable is reachable only from
/// the binding stack while the body runs. Collection at every safepoint
/// (floor 1) with allocation inside the body must keep and forward it, and
/// both normal exit and a signal must restore it intact.
#[test]
fn forwarded_value_saved_by_let_survives_collection_inside_the_binding() {
    for exit in ["nil", "(signal 'error (list 'cv-boom))"] {
        let mut vm = Vm::new();
        vm.eval_source(GARBAGE).unwrap();
        vm.eval_source(&format!("(setq minibuffer-local-map {PAYLOAD})")).unwrap();
        vm.set_collection_floor(1);
        let before = vm.heap_stats().collections;
        let result = shown(
            &mut vm,
            &format!("(condition-case e (let ((minibuffer-local-map 'cv-inner)) {GARBAGE} {GARBAGE} (list minibuffer-local-map {exit})) (error e))"),
        );
        assert!(vm.heap_stats().collections > before, "the fixture must actually collect inside the binding");
        assert!(result == "(cv-inner nil)" || result == "(error cv-boom)", "{result}");
        assert_eq!(shown(&mut vm, READBACK), EXPECTED, "exit {exit}: the saved value must be restored intact");
    }
}

/// A per-buffer variable's default lives outside every buffer and every
/// value cell; `set-default' does not apply the predicate (GNU
/// set_default_internal), so it can hold a heap object that must be traced.
#[test]
fn per_buffer_default_survives_moving_collection() {
    let mut vm = Vm::new();
    vm.eval_source(GARBAGE).unwrap();
    vm.eval_source(&format!("(set-default 'fill-column {PAYLOAD})")).unwrap();
    settled(&mut vm);
    vm.eval_source(GARBAGE).unwrap();
    settled(&mut vm);
    assert_eq!(
        shown(&mut vm, &READBACK.replace("minibuffer-local-map", "(default-value 'fill-column)")),
        EXPECTED
    );
    assert_eq!(
        shown(&mut vm, &format!("(with-temp-buffer {})", READBACK.replace("minibuffer-local-map", "fill-column"))),
        EXPECTED,
        "a new buffer inherits the moved default"
    );
}

/// lread.c:5559 DEFVARs `values' and then clears its `declared_special', so
/// a closure captures a `let' of it. GNU 32.0.50 -Q --batch, same form:
/// (nil t nil 2).
#[test]
fn values_is_bound_but_not_special() {
    let mut vm = Vm::new();
    let printed = vm.eval_source("(list (special-variable-p 'values) (boundp 'values) values \
        (funcall (let ((values 2)) (lambda () values))))").unwrap().display().unwrap();
    assert_eq!(printed, "(nil t nil 2)");
}
