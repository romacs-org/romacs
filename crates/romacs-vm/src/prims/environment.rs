use super::*;

defsubr! { name = "getenv-internal", gnu = "callproc.c:1765", effect = Reads,
fn getenv_internal(cx, variable: Value; optional env: Value) {
    let variable = cx.string_view(variable)?.bytes().to_vec();
    let explicit = cx.pair(env).is_some();
    let environment = if explicit { env } else { environment_value(cx, "process-environment")? };
    crate::host::lisp_env(if explicit { crate::host::EnvList::Explicit } else { crate::host::EnvList::Process }, &variable);
    if let Some(value) = lookup(cx, &variable, environment)? {
        return Ok(match value {
            Some(bytes) => environment_string(cx, bytes),
            None if explicit => Value::TRUE,
            None => Value::NIL,
        });
    }
    if !explicit && variable.split(|&byte| byte == 0).next() == Some(b"DISPLAY") {
        let initial = environment_value(cx, "initial-environment")?;
        crate::host::lisp_env(crate::host::EnvList::Initial, &variable);
        if let Some(Some(bytes)) = lookup(cx, &variable, initial)? {
            return Ok(environment_string(cx, bytes));
        }
    }
    Ok(Value::NIL)
}}

fn environment_string(cx: &mut Leaf<'_>, bytes: Vec<u8>) -> Value {
    let multibyte = crate::environment::multibyte(&bytes);
    cx.make_string(bytes, multibyte)
}

/// GNU egetenv: VARIABLE's value in `process-environment', None when unset.
pub(super) fn egetenv(cx: &mut Leaf<'_>, variable: &str) -> std::result::Result<Option<Vec<u8>>, Fault> {
    let environment = environment_value(cx, "process-environment")?;
    crate::host::lisp_env(crate::host::EnvList::Process, variable.as_bytes());
    Ok(lookup(cx, variable.as_bytes(), environment)?.flatten())
}

fn environment_value(cx: &mut Leaf<'_>, name: &str) -> Out {
    let symbol = cx.intern(name);
    cx.variable_value(symbol)?.ok_or(Fault::VoidVariable(symbol))
}

fn lookup(cx: &Leaf<'_>, name: &[u8], mut tail: Value) -> std::result::Result<Option<Option<Vec<u8>>>, Fault> {
    let mut tortoise = tail;
    let mut steps = 0;
    while let Some((entry, _)) = cx.pair(tail) {
        if matches!(cx.kind(entry), leaf::Kind::String | leaf::Kind::Bytes) {
            let entry = cx.string_view(entry)?;
            let bytes = entry.bytes();
            if bytes.starts_with(name) {
                if bytes.len() == name.len() { return Ok(Some(None)); }
                if bytes.get(name.len()) == Some(&b'=') {
                    return Ok(Some(Some(bytes[name.len() + 1..].to_vec())));
                }
            }
        }
        (tail, tortoise, steps) = super::fns_equal::for_each_tail_step(cx, tail, tortoise, steps)?;
    }
    Ok(None)
}
