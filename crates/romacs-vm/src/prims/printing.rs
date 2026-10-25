//! GNU print.c output entry points; callbacks cross the traced T2 boundary.
use super::*;
use std::io::Write;

defsubr! { name = "prin1", gnu = "print.c:738", effect = Global,
fn prin1(cx, object: Value; optional stream: Value, overrides: Value) -> Step {
    let stream = destination(cx, stream)?; let text = cx.print_text(object, true, overrides, stream)?;
    output(cx, object, text, stream)
}}

defsubr! { name = "princ", gnu = "print.c:844", effect = Global,
fn princ(cx, object: Value; optional stream: Value) -> Step {
    let stream = destination(cx, stream)?;
    let text = cx.princ_text(object, stream)?;
    output(cx, object, text, stream)
}}

defsubr! { name = "terpri", gnu = "print.c:627", effect = Global,
fn terpri(cx; optional stream: Value, ensure: Value) -> Step {
    if ensure != Value::NIL { return Err(Fault::Error("Unsupported function argument".into())); }
    let text = cx.string("\n");
    output(cx, Value::TRUE, text, stream)
}}

defsubr! { name = "print", gnu = "print.c:882", effect = Global,
fn print(cx, object: Value; optional stream: Value) -> Step {
    let stream = destination(cx, stream)?; let text = cx.print_text(object, true, Value::NIL, stream)?;
    let text = cx.string(&format!("\n{}\n", cx.string_text(text)?));
    output(cx, object, text, stream)
}}

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
fn destination(cx: &mut Leaf<'_>, stream: Value) -> Out {
    if stream == Value::NIL {
        let symbol = cx.intern("standard-output");
        let stream = cx.variable_value(symbol)?.unwrap_or(Value::TRUE);
        Ok(if stream == Value::NIL { Value::TRUE } else { stream })
    } else { Ok(stream) }
}
fn output(cx: &mut Leaf<'_>, result: Value, text: Value, stream: Value) -> Next {
    let stream = destination(cx, stream)?;
    if stream == Value::TRUE {
        let text = cx.string_text(text)?;
        std::io::stdout().write_all(text.as_bytes()).map_err(|e| Fault::Error(e.to_string()))?;
        return Ok(Step::Done(result));
    }
    if matches!(cx.kind(stream), leaf::Kind::Buffer | leaf::Kind::ByteBuffer | leaf::Kind::Marker) {
        cx.print_into_buffer(text, stream)?;
        return Ok(Step::Done(result));
    }
    next_char(cx, vec![result, text, stream, Value::integer(0).unwrap()])
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
