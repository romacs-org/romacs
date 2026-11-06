//! GNU src/minibuf.c `read-from-minibuffer' (minibuf.c:1309), `read-string'
//! (minibuf.c:1420), `read-buffer' and its table, the queries over the active
//! minibuffers that GNU's minibuffer.el makes, and fns.c `yes-or-no-p'.
//!
//! Tier: T2 (PRIMITIVE-DOCTRINE section 3). GNU's `read_minibuf' runs a
//! recursive command loop, i.e. it calls Lisp once per key, so the loop is Lisp
//! (`minibuf.el', the "Lisp half"), entered with one `Step::Call'. The Rust
//! half does exactly what GNU's C does outside that loop and in the same order:
//! the argument checks (minibuf.c:1373-1394, 612-632), then, after the loop,
//! `string_to_object' (minibuf.c:269) for READ and read-string's DEFAULT-VALUE
//! substitution (minibuf.c:1457). Values that must survive the loop wait in the
//! continuation's traced registers (`saved'), never in Rust locals, so a
//! collection inside a command typed at the prompt forwards them.
//!
//! The Lisp half is installed on first use with keyboard.c's (prims/keyboard.rs
//! `install'), by one `Step::Call' of `eval': a bare `Vm::new()' stays bare.
//!
//! Supported subset (DIVERGENCES.md VM2-MINIBUF1 lists what is not):
//! PROMPT, INITIAL-CONTENTS (string or (STRING . POSITION)), KEYMAP, READ,
//! HIST (variable set to nil when unbound; history recorded only through
//! GNU's own `add-to-history' when that is fbound), DEFAULT-VALUE (string,
//! list, or other, exactly as GNU uses it), INHERIT-INPUT-METHOD (accepted;
//! VM2 has no input methods). While `executing-kbd-macro' is non-nil the
//! command loop (prims/keyboard.rs) reads `unread-command-events' and then the
//! macro, GNU's batch prompt regime (`execute-kbd-macro', ert-x.el
//! `ert-simulate-keys'); with a host keyboard it reads that; otherwise VM2
//! behaves as batch GNU with an empty stdin (minibuf.c:659):
//! `(end-of-file "Error reading from stdin")'.
use super::*;

defsubr! { name = "read-from-minibuffer", gnu = "minibuf.c:1309", effect = Global,
fn read_from_minibuffer(cx, prompt: Value; optional initial_contents: Value, keymap: Value,
                        read: Value, hist: Value, default_value: Value,
                        inherit_input_method: Value) -> Step {
    prepare(cx, [prompt, initial_contents, keymap, read, hist, default_value, inherit_input_method], true)
}}

defsubr! { name = "read-string", gnu = "minibuf.c:1420", effect = Global,
fn read_string(cx, prompt: Value; optional initial_input: Value, history: Value,
               default_value: Value, inherit_input_method: Value) -> Step {
    // Fread_string calls Fread_from_minibuffer (prompt, initial, nil, nil,
    // history, default, inherit) directly, not through the symbol.
    prepare(cx, [prompt, initial_input, Value::NIL, Value::NIL, history, default_value, inherit_input_method], false)
}}

/// Fread_from_minibuffer's checks, then read_minibuf's, in GNU's order; then
/// the call into the Lisp half. `minibuffer` selects the caller's tail:
/// `read-from-minibuffer' (READ) or `read-string' (DEFAULT-VALUE).
fn prepare(cx: &mut Leaf<'_>, args: [Value; 7], minibuffer: bool) -> Next {
    let [prompt, initial, keymap, read, hist, default, inherit] = args;
    // barf_if_interaction_inhibited (minibuf.c:1303).
    let inhibit = cx.intern("inhibit-interaction");
    if cx.variable_value(inhibit)?.is_some_and(|v| v != Value::NIL) {
        let condition = cx.intern("inhibited-interaction");
        return Err(Fault::Signal(condition, Value::NIL));
    }
    if cx.string_view(prompt).is_err() {
        return Err(Fault::Wrong("stringp", prompt));
    }
    // get_keymap (keymap, 1, 0); nil means the current `minibuffer-local-map'.
    let keymap = if keymap == Value::NIL {
        let local = cx.intern("minibuffer-local-map");
        cx.variable_value(local)?.unwrap_or(Value::NIL)
    } else {
        cx.keymap(keymap).ok_or(Fault::Wrong("keymapp", keymap))?
    };
    // HIST: a symbol, or (HISTVAR . HISTPOS) through car-safe; nil means
    // `minibuffer-history'. HISTPOS feeds only the history commands, which
    // are out of scope, so it is parsed and dropped.
    let histvar = if cx.kind(hist) == leaf::Kind::Symbol { hist } else { cx.pair(hist).map_or(Value::NIL, |(car, _)| car) };
    let histvar = if histvar == Value::NIL { cx.intern("minibuffer-history") } else { histvar };
    // read_minibuf (minibuf.c:612-632): INITIAL and its backup position,
    // converted to a (non-positive) distance from the end of the input.
    let (initial, pos) = match cx.pair(initial) {
        Some((string, backup)) => {
            let chars = cx.string_view(string).map_err(|_| Fault::Wrong("stringp", string))?.chars as i64;
            let pos = if backup == Value::NIL {
                0
            } else {
                let n = fixnum(backup)?;
                if n < 1 { -chars } else { n - 1 - chars }
            };
            (string, pos)
        }
        None if initial == Value::NIL => (Value::NIL, 0),
        None => {
            cx.string_view(initial).map_err(|_| Fault::Wrong("stringp", initial))?;
            (initial, 0)
        }
    };
    let pos = Value::integer(pos).map_err(|_| Fault::Wrong("fixnump", Value::NIL))?;
    // The Lisp half (minibuf.el, with keyboard.c's command loop) is installed
    // on first use; then the arguments are checked again, unchanged.
    if !super::keyboard::is_installed(cx)? {
        let flag = if minibuffer { Value::TRUE } else { Value::NIL };
        return super::keyboard::install(cx, keyboard_ready, [args.to_vec(), vec![flag]].concat());
    }
    let helper = cx.intern("romacs--read-minibuf");
    let call = [prompt, keymap, initial, pos, histvar, default, inherit];
    let saved = vec![read, default, call[0], call[1], call[2], call[3], call[4], call[5], call[6]];
    Ok(Step::Call { f: helper, args: call.to_vec(), then: tail(minibuffer), saved })
}

/// After the keyboard half's install, prepare again from the raw arguments.
fn keyboard_ready(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let &[prompt, initial, keymap, read, hist, default, inherit, caller] = saved else {
        unreachable!("keyboard install saves eight registers")
    };
    prepare(cx, [prompt, initial, keymap, read, hist, default, inherit], caller != Value::NIL)
}

fn tail(minibuffer: bool) -> Then {
    if minibuffer { read_done } else { string_done }
}

/// Fread_from_minibuffer's tail: read_minibuf's `if (expflag)` (minibuf.c:1104).
fn read_done(cx: &mut Leaf<'_>, val: Value, saved: &[Value]) -> Next {
    let (read, default) = (saved[0], saved[1]);
    if read == Value::NIL {
        return Ok(Step::Done(val));
    }
    string_to_object(cx, val, default).map(Step::Done)
}

/// Fread_string's tail (minibuf.c:1457): an empty answer becomes DEFAULT-VALUE,
/// or its car when it is a cons; whatever type that is, GNU returns it as is.
fn string_done(cx: &mut Leaf<'_>, val: Value, saved: &[Value]) -> Next {
    let default = saved[1];
    let empty = cx.string_view(val).is_ok_and(|s| s.chars == 0);
    if empty && default != Value::NIL {
        return Ok(Step::Done(cx.pair(default).map_or(default, |(car, _)| car)));
    }
    Ok(Step::Done(val))
}

/// GNU `string_to_object' (minibuf.c:269).
fn string_to_object(cx: &mut Leaf<'_>, mut val: Value, default: Value) -> Out {
    if cx.string_view(val).is_ok_and(|s| s.chars == 0) {
        if cx.string_view(default).is_ok() {
            val = default;
        } else if let Some((car, _)) = cx.pair(default).filter(|&(car, _)| cx.string_view(car).is_ok()) {
            val = car;
        }
    }
    let read = cx.read_from_string(val, Value::NIL, Value::NIL)?;
    let (object, end) = cx.pair(read).expect("read-from-string returns (OBJECT . POSITION)");
    let end = end.integer_value().expect("read position is a fixnum") as usize;
    let view = cx.string_view(val)?;
    if end != view.chars {
        // Ignore trailing whitespace; any other trailing junk is an error.
        let text: Vec<char> = view.chars().map(|c| char::from_u32(c).unwrap_or('\u{fffd}')).collect();
        if text[end..].iter().any(|&c| !matches!(c, ' ' | '\t' | '\n')) {
            let condition = cx.intern("invalid-read-syntax");
            let message = cx.string("Trailing garbage following expression");
            let data = cx.cons(message, Value::NIL);
            return Err(Fault::Signal(condition, data));
        }
    }
    Ok(object)
}

fn fixnum(v: Value) -> std::result::Result<i64, Fault> {
    v.integer_value().ok_or(Fault::Wrong("fixnump", v))
}

use super::keyboard::{enter, DONE};

defsubr! { name = "innermost-minibuffer-p", gnu = "minibuf.c:424", effect = Global,
fn innermost_minibuffer_p(cx; optional buffer: Value) -> Step {
    enter(cx, "romacs--innermost-minibuffer-p", vec![buffer], DONE)
}}

defsubr! { name = "minibuffer-innermost-command-loop-p", gnu = "minibuf.c:438", effect = Global,
fn minibuffer_innermost_command_loop_p(cx; optional buffer: Value) -> Step {
    enter(cx, "romacs--minibuffer-innermost-command-loop-p", vec![buffer], DONE)
}}

defsubr! { name = "minibuffer-prompt", gnu = "minibuf.c:2248", effect = Global,
fn minibuffer_prompt(cx) -> Step { enter(cx, "romacs--minibuffer-prompt", vec![], DONE) }}

defsubr! { name = "minibuffer-prompt-end", gnu = "minibuf.c:505", effect = Global,
fn minibuffer_prompt_end(cx) -> Step { enter(cx, "romacs--minibuffer-prompt-end", vec![], DONE) }}

defsubr! { name = "minibuffer-contents", gnu = "minibuf.c:527", effect = Global,
fn minibuffer_contents(cx) -> Step { enter(cx, "romacs--minibuffer-contents", vec![Value::TRUE], DONE) }}

defsubr! { name = "minibuffer-contents-no-properties", gnu = "minibuf.c:537", effect = Global,
fn minibuffer_contents_no_properties(cx) -> Step {
    enter(cx, "romacs--minibuffer-contents", vec![Value::NIL], DONE)
}}

// Fread_buffer binds `completion-ignore-case' around the reader, so it is Lisp.
defsubr! { name = "read-buffer", gnu = "minibuf.c:1522", effect = Global,
fn read_buffer(cx, prompt: Value; optional def: Value, require_match: Value, predicate: Value) -> Step {
    enter(cx, "romacs--read-buffer", vec![prompt, def, require_match, predicate], DONE)
}}

// Fread_coding_system binds `completion-ignore-case' around the reader.
defsubr! { name = "read-coding-system", gnu = "coding.c:8625", effect = Global,
fn read_coding_system(cx, prompt: Value; optional default_coding_system: Value) -> Step {
    enter(cx, "romacs--read-coding-system", vec![prompt, default_coding_system], DONE)
}}

defsubr! { name = "internal-complete-buffer", gnu = "minibuf.c:2156", effect = Global,
fn internal_complete_buffer(cx, string: Value, predicate: Value, flag: Value) -> Step {
    enter(cx, "romacs--internal-complete-buffer", vec![string, predicate, flag], DONE)
}}

defsubr! { name = "yes-or-no-p", gnu = "fns.c:3612", effect = Global,
fn yes_or_no_p(cx, prompt: Value) -> Step { enter(cx, "romacs--yes-or-no-p", vec![prompt], DONE) }}
