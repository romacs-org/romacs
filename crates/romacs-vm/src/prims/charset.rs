use super::*;

defsubr! { name = "charset-plist", gnu = "charset.c:1295", effect = Reads,
fn charset_plist(cx, charset: Value) { cx.charset_plist(charset) }}

defsubr! { name = "set-charset-plist", gnu = "charset.c:1306", effect = Global,
fn set_charset_plist(cx, charset: Value, plist: Value) { cx.set_charset_plist(charset, plist) }}

defsubr! { name = "define-charset-alias", gnu = "charset.c:1281", effect = Global,
fn define_charset_alias(cx, alias: Value, charset: Value) { cx.charset_alias(alias, charset) }}


defsubr! { name = "unify-charset", gnu = "charset.c:1318", effect = Global,
fn unify_charset(cx, charset: Value; optional unify_map: Value, deunify: Value) { cx.unify_charset(charset, unify_map, deunify) }}

defsubr! { name = "charsetp", gnu = "charset.c:642", effect = Reads,
fn charsetp(cx, object: Value) { Ok(if cx.is_charset(object) { Value::TRUE } else { Value::NIL }) }}

defsubr! { name = "set-charset-priority", gnu = "charset.c:2165", effect = Global,
fn set_charset_priority(cx, first: Value; rest others) {
    let mut charsets = vec![first];
    charsets.extend_from_slice(others);
    cx.set_charset_priority(&charsets)
}}

defsubr! { name = "charset-priority-list", gnu = "charset.c:2146", effect = Reads,
fn charset_priority_list(cx; optional highestp: Value) { cx.charset_priority_list(highestp) }}

defsubr! { name = "decode-char", gnu = "charset.c:1828", effect = Reads,
fn decode_char(cx, charset: Value, code_point: Value) { cx.decode_char(charset, code_point) }}

defsubr! { name = "encode-char", gnu = "charset.c:1848", effect = Reads,
fn encode_char(cx, ch: Value, charset: Value) { cx.encode_char(ch, charset) }}

// map_charset_chars's calls are computed first, then made one at a time
// through T2. A walk reuses its cons, emptying its car after each call.
defsubr! { name = "map-charset-chars", gnu = "charset.c:787", effect = Global,
fn map_charset_chars(cx, function: Value, charset: Value; optional arg: Value, from_code: Value, to_code: Value) -> Step {
    let calls = cx.charset_map_calls(charset, from_code, to_code)?;
    let mut flat = Vec::with_capacity(3 * calls.len());
    for run in calls {
        let walk = run.walk.map_or(-1, |id| id as i64);
        flat.extend([run.start, run.end, walk].map(|n| Value::integer(n).expect("charset range is a fixnum")));
    }
    let calls = cx.vector_from(flat);
    next_charset_call(cx, &[function, arg, calls, Value::integer(0).unwrap(), Value::NIL])
}}
fn next_charset_call(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    let &[function, arg, calls, at, range] = saved else { unreachable!("five registers") };
    let at = at.integer_value().unwrap() as usize;
    if at * 3 >= cx.vector_len(calls)? { return Ok(Step::Done(Value::NIL)); }
    let field = |cx: &Leaf<'_>, k: usize| cx.vector_get(calls, Value::integer(k as i64).unwrap());
    let (start, end, walk) = (field(cx, 3 * at)?, field(cx, 3 * at + 1)?, field(cx, 3 * at + 2)?);
    let shared = at > 0 && walk != Value::integer(-1).unwrap() && field(cx, 3 * at - 1)? == walk;
    let range = if shared { cx.setcar(range, start)?; cx.setcdr(range, end)?; range } else { cx.cons(start, end) };
    Ok(Step::Call { f: function, args: vec![range, arg], then: charset_called,
        saved: vec![function, arg, calls, Value::integer(at as i64 + 1).unwrap(), range] })
}
fn charset_called(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let at = saved[3].integer_value().unwrap() as usize - 1;
    let walk = cx.vector_get(saved[2], Value::integer((3 * at + 2) as i64).unwrap())?;
    if walk != Value::integer(-1).unwrap() { cx.setcar(saved[4], Value::NIL)?; }
    next_charset_call(cx, saved)
}
