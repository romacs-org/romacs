//! GNU print.c output entry points; callbacks cross the traced T2 boundary.
use super::*;
use std::io::Write;

// Each print first answers its unreadable objects (`answered'), then prints
// with SAVED = [OBJECT, ESCAPE, OVERRIDES, STREAM].
defsubr! { name = "prin1", gnu = "print.c:738", effect = Global,
fn prin1(cx, object: Value; optional stream: Value, overrides: Value) -> Step {
    let stream = destination(cx, stream)?;
    answered(cx, vec![object, Value::TRUE, overrides, stream], prin1_answered)
}}
fn prin1_answered(cx: &mut Leaf<'_>, answers: Value, saved: &[Value]) -> Next {
    let text = cx.print_text(saved[0], true, saved[2], saved[3], answers)?;
    output(cx, saved[0], text, saved[3])
}

defsubr! { name = "prin1-to-string", gnu = "print.c:795", effect = Global,
fn prin1_to_string(cx, object: Value; optional noescape: Value, overrides: Value) -> Step {
    let escape = if noescape == Value::NIL { Value::TRUE } else { Value::NIL };
    answered(cx, vec![object, escape, overrides, Value::NIL], to_string_answered)
}}
fn to_string_answered(cx: &mut Leaf<'_>, answers: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(cx.print_string(saved[0], saved[1] != Value::NIL, saved[2], answers)?))
}

defsubr! { name = "princ", gnu = "print.c:844", effect = Global,
fn princ(cx, object: Value; optional stream: Value) -> Step {
    let stream = destination(cx, stream)?;
    answered(cx, vec![object, Value::NIL, Value::NIL, stream], princ_answered)
}}
fn princ_answered(cx: &mut Leaf<'_>, answers: Value, saved: &[Value]) -> Next {
    let text = cx.princ_text(saved[0], saved[3], answers)?;
    output(cx, saved[0], text, saved[3])
}

defsubr! { name = "terpri", gnu = "print.c:627", effect = Global,
fn terpri(cx; optional stream: Value, ensure: Value) -> Step {
    if ensure != Value::NIL { return Err(Fault::Error("Unsupported function argument".into())); }
    let text = cx.string("\n");
    output(cx, Value::TRUE, text, stream)
}}

defsubr! { name = "print", gnu = "print.c:882", effect = Global,
fn print(cx, object: Value; optional stream: Value) -> Step {
    let stream = destination(cx, stream)?;
    answered(cx, vec![object, Value::TRUE, Value::NIL, stream], print_answered)
}}
fn print_answered(cx: &mut Leaf<'_>, answers: Value, saved: &[Value]) -> Next {
    let text = cx.print_text(saved[0], true, Value::NIL, saved[3], answers)?;
    let text = cx.string(&format!("\n{}\n", cx.string_text(text)?));
    output(cx, saved[0], text, saved[3])
}

/// print.c print_vectorlike_unreadable: while `print-unreadable-function'
/// (or the OVERRIDES of SAVED binding it) is a function, its value for each
/// unreadable object the print of SAVED meets, from
/// `romacs--print-unreadable' (rlisp/vm/prelude.el); THEN prints with that
/// list, or with nil when no call is due.
fn answered(cx: &mut Leaf<'_>, saved: Vec<Value>, then: Then) -> Next {
    let function = match overridden_function(cx, saved[2])? {
        Some(function) => function,
        None => cx.print_unreadable_function()?,
    };
    if function != Value::NIL && super::vars::function_p(cx, function)? {
        let objects = cx.print_unreadables(saved[0], saved[1] != Value::NIL, saved[2], saved[3])?;
        if !objects.is_empty() {
            let objects = objects.into_iter().rev().fold(Value::NIL, |list, object| cx.cons(object, list));
            let f = cx.intern("romacs--print-unreadable");
            return Ok(Step::Call { f, args: vec![function, objects, saved[1]], then, saved });
        }
    }
    then(cx, Value::NIL, &saved)
}

defsubr! { name = "write-char", gnu = "print.c:553", effect = Global,
fn write_char(cx, character: Value; optional stream: Value) -> Step {
    let number = character.integer_value().ok_or(Fault::Wrong("fixnump", character))?;
    let stream = destination(cx, stream)?;
    if stream != Value::TRUE && !matches!(cx.kind(stream), leaf::Kind::Buffer | leaf::Kind::ByteBuffer | leaf::Kind::Marker) {
        return Ok(Step::Call { f: stream, args: vec![Value::integer(i64::from(number as u32)).unwrap()], then: written, saved: vec![character] });
    }
    let code = u32::try_from(number).ok().filter(|&c| c <= crate::lstring::MAX_CHAR)
        .ok_or_else(|| Fault::Error("write-char outside the supported character range".into()))?;
    let mut bytes = [0; 5];
    let n = crate::lstring::char_string(code, &mut bytes);
    let text = cx.make_string(bytes[..n].to_vec(), true);
    output(cx, character, text, stream)
}}

defsubr! { name = "flush-standard-output", gnu = "print.c:917", effect = Global,
fn flush_standard_output(cx) {
    std::io::stdout().flush().map_err(|e| Fault::Error(e.to_string()))?;
    Ok(Value::NIL)
}}

fn written(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next { Ok(Step::Done(saved[0])) }
/// Printing with change hooks to run: the insertion's bracket, in Lisp.
fn print_hooked(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let insert = crate::prim::subr_named("romacs--insert-printed").expect("print_finish subr");
    let args = vec![saved[2], saved[1], insert];
    Ok(Step::Call { f: cx.intern("romacs--print-insert"), args, then: written, saved: vec![saved[0]] })
}
fn destination(cx: &mut Leaf<'_>, stream: Value) -> Out {
    if stream == Value::NIL {
        let symbol = cx.intern("standard-output");
        let stream = cx.variable_value(symbol)?.unwrap_or(Value::TRUE);
        Ok(if stream == Value::NIL { Value::TRUE } else { stream })
    } else { Ok(stream) }
}
fn output(cx: &mut Leaf<'_>, result: Value, text: Value, stream: Value) -> Next {
    let stream = destination(cx, stream)?;
    if stream == Value::TRUE && echo_area_shown(cx)? {
        return echo_print(cx, result, text);
    }
    if stream == Value::TRUE {
        // print.c strout: the string's bytes as they are (a raw-byte
        // character as its internal two-byte form, as GNU writes it).
        let bytes = cx.string_view(text)?.bytes().to_vec();
        std::io::stdout().write_all(&bytes).map_err(|e| Fault::Error(e.to_string()))?;
        return Ok(Step::Done(result));
    }
    if matches!(cx.kind(stream), leaf::Kind::Buffer | leaf::Kind::ByteBuffer | leaf::Kind::Marker) {
        if !cx.print_runs_lisp(text, stream)? {
            cx.print_into_buffer(text, stream)?;
            return Ok(Step::Done(result));
        }
        // The printed text carries no properties, as insert_1_both's.
        let view = cx.string_view(text)?;
        let (bytes, multibyte) = (view.bytes().to_vec(), view.is_multibyte());
        let saved = vec![result, cx.make_string(bytes, multibyte), stream];
        if super::insdel::half_missing(cx)? { return super::insdel::install_half(cx, print_hooked, saved); }
        return print_hooked(cx, Value::NIL, &saved);
    }
    next_char(cx, vec![result, text, stream, Value::integer(0).unwrap()])
}
/// print.c PRINTPREPARE: `t` is the echo area unless `noninteractive`; in
/// VM2, when the embedder renders one (`Vm::set_echo_area_host`).
fn echo_area_shown(cx: &mut Leaf<'_>) -> std::result::Result<bool, Fault> {
    let noninteractive = cx.intern("noninteractive");
    Ok(cx.hidden_value(crate::prim::ECHO_AREA_HOST) != Value::NIL
        && cx.variable_value(noninteractive)?.unwrap_or(Value::NIL) == Value::NIL)
}
/// print.c `printchar_to_stream` into the echo area: a print extends the
/// message printing made (`message_buf_print`); after any other message,
/// it starts a new one. Either way the text is logged, as part of a line.
fn echo_print(cx: &mut Leaf<'_>, result: Value, text: Value) -> Next {
    let shown = cx.hidden_value(crate::prim::ECHO_AREA_MESSAGE);
    let extended = shown != Value::NIL && shown == cx.hidden_value(crate::prim::ECHO_AREA_PRINTED);
    let prefix = if extended { cx.string_text(shown)? } else { String::new() };
    let echoed = cx.string(&(prefix + &cx.string_text(text)?));
    cx.set_hidden_value(crate::prim::ECHO_AREA_MESSAGE, echoed);
    cx.set_hidden_value(crate::prim::ECHO_AREA_PRINTED, echoed);
    super::message::log_printed(cx, text, extended, result)
}
fn next_char(cx: &mut Leaf<'_>, mut saved: Vec<Value>) -> Next {
    let at = saved[3].integer_value().unwrap() as usize;
    match cx.string_char_at(saved[1], at) {
        None => Ok(Step::Done(saved[0])),
        Some((code, width)) => {
            saved[3] = Value::integer((at + width) as i64).unwrap();
            Ok(Step::Call { f: saved[2], args: vec![Value::integer(code as i64).unwrap()], then: emitted, saved })
        }
    }
}
fn emitted(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    next_char(cx, saved.to_vec())
}

/// print.c print_create_variable_mapping's override keys.
const OVERRIDE_KEYS: [&str; 17] = ["length", "level", "circle", "quoted", "escape-newlines", "escape-control-characters",
    "escape-nonascii", "escape-multibyte", "charset-text-property", "unreadable-function", "unreadeable-function",
    "gensym", "continuous-numbering", "number-table", "float-format", "integers-as-characters", "symbols-bare"];

/// print.c print_bind_overrides's checks of OVERRIDES, and the
/// `print-unreadable-function' they bind: the last one given, nil after
/// a `t', which binds every default; None when none is.
fn overridden_function(cx: &mut Leaf<'_>, overrides: Value) -> std::result::Result<Option<Value>, Fault> {
    if overrides == Value::NIL { return Ok(None); }
    if overrides == Value::TRUE { return Ok(Some(Value::NIL)); }
    if cx.pair(overrides).is_none() { return Err(Fault::Wrong("consp", overrides)); }
    let (mut function, mut tail) = (None, overrides);
    while let Some((setting, next)) = cx.pair(tail) {
        if setting == Value::TRUE {
            function = Some(Value::NIL);
        } else {
            let (key, value) = cx.pair(setting).ok_or(Fault::Wrong("consp", setting))?;
            let name = if cx.kind(key) == leaf::Kind::Symbol { cx.symbol_name(key)? } else { String::new() };
            // Fassq of an unknown key is nil, which is not a symbol here.
            if !OVERRIDE_KEYS.contains(&name.as_str()) { return Err(Fault::Wrong("symbolp", Value::NIL)); }
            if name.starts_with("unread") { function = Some(value); }
        }
        if next != Value::NIL && cx.pair(next).is_none() { return Err(Fault::Wrong("consp", tail)); }
        tail = next;
    }
    Ok(function)
}
