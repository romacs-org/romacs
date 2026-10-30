//! GNU editfns.c Fmessage through xdisp.c message3: log the text to the
//! `messages-buffer-name' buffer, then show it unless `inhibit-message'.
//!
//! The log is message_dolog's Lisp half (rlisp/vm/message.el), installed by
//! one `Step::Call' of `eval' the first time a message is logged, as
//! insdel.rs installs its own.
use super::*;
use std::io::Write;

const LISP_HALF: &str = include_str!("../../../../rlisp/vm/message.el");
const DOLOG: &str = "romacs--message-dolog";

// GNU has no message catalog yet: MSGID when N is the fixnum 1, else
// MSGID-PLURAL.
defsubr! { name = "ngettext", gnu = "editfns.c:3180", effect = Pure,
fn ngettext(cx, msgid: Value, msgid_plural: Value, n: Value) {
    cx.string_view(msgid)?;
    cx.string_view(msgid_plural)?;
    cx.bigint_value(n).ok_or(Fault::Wrong("integerp", n))?;
    Ok(if n.integer_value() == Some(1) { msgid } else { msgid_plural })
}}

defsubr! { name = "message", gnu = "editfns.c:3197", effect = Global,
fn message(cx, format_string: Value; rest args) -> Step {
    // GNU message1 (0): nil or "" clears the echo area and logs nothing;
    // in batch, message_to_stderr writes an empty line.
    let empty = cx.string_bytes(format_string).is_ok_and(|bytes| bytes.is_empty());
    if format_string == Value::NIL || empty {
        if !inhibited(cx)? {
            show(cx, Value::NIL)?;
            let noninteractive = cx.intern("noninteractive");
            if cx.variable_value(noninteractive)?.unwrap_or(Value::NIL) != Value::NIL { eprintln!(); }
        }
        return Ok(Step::Done(format_string));
    }
    let mut all = vec![format_string];
    all.extend_from_slice(args);
    let format = crate::prim::legacy_value(crate::Primitive::Format).expect("format is registered");
    Ok(Step::Call { f: format, args: all, then: formatted, saved: vec![] })
}}

fn formatted(cx: &mut Leaf<'_>, text: Value, _: &[Value]) -> Next {
    let limit = cx.intern("message-log-max");
    if cx.variable_value(limit)?.unwrap_or(Value::NIL) == Value::NIL { return logged(cx, Value::NIL, &[text]); }
    let dolog = cx.intern(DOLOG);
    if cx.symbol_function(dolog)? == Value::NIL {
        let source = cx.string(LISP_HALF);
        let form = cx.read_from_string(source, Value::NIL, Value::NIL)?;
        let (form, _) = cx.pair(form).expect("read-from-string returns (OBJECT . POSITION)");
        return Ok(Step::Call { f: cx.intern("eval"), args: vec![form, Value::TRUE], then: installed, saved: vec![text] });
    }
    Ok(Step::Call { f: dolog, args: vec![text], then: logged, saved: vec![text] })
}

fn installed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern(DOLOG), args: vec![saved[0]], then: logged, saved: vec![saved[0]] })
}

fn logged(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let text = saved[0];
    if !inhibited(cx)? {
        show(cx, text)?;
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(&cx.string_bytes(text)?).and_then(|()| err.write_all(b"\n"));
    }
    Ok(Step::Done(text))
}

fn inhibited(cx: &mut Leaf<'_>) -> std::result::Result<bool, Fault> {
    let inhibit = cx.intern("inhibit-message");
    Ok(cx.variable_value(inhibit)?.unwrap_or(Value::NIL) != Value::NIL)
}

/// The echo area holds the last message; nil clears it.
fn show(cx: &mut Leaf<'_>, text: Value) -> std::result::Result<(), Fault> {
    let echo = cx.intern(crate::prim::ECHO_AREA_MESSAGE);
    cx.variable_set(echo, text)
}
