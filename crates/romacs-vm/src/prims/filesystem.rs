use super::*;

defsubr! { name = "file-system-info", gnu = "fileio.c:6659", effect = Global,
fn file_system_info(cx, filename: Value) -> Step {
    super::fileio::expanding(cx, filename, filesystem_expanded, vec![])
}}

fn filesystem_expanded(cx: &mut Leaf<'_>, expanded: Value, _: &[Value]) -> Next {
    let path = cx.string_text(expanded)?;
    let operation = cx.intern("file-system-info");
    let handler = super::fileio::find_file_name_handler(cx, expanded, operation)?;
    if handler != Value::NIL {
        return Ok(Step::Call { f: handler, args: vec![operation, expanded], then: filesystem_handler_returned, saved: vec![] });
    }
    let bytes = match cx.file_system_bytes(&path) {
        Ok(bytes) => bytes,
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR | libc::ENOSYS)) => return Ok(Step::Done(Value::NIL)),
        Err(error) => return Err(super::fileio::file_errno(cx, "Getting attributes", expanded, &error)),
    };
    let mut result = Value::NIL;
    for n in bytes.into_iter().rev() {
        let value = cx.bigint(n.into());
        result = cx.cons(value, result);
    }
    Ok(Step::Done(result))
}

fn filesystem_handler_returned(cx: &mut Leaf<'_>, result: Value, _: &[Value]) -> Next {
    if result == Value::NIL || cx.pair(result).is_some() { Ok(Step::Done(result)) }
    else { Err(Fault::Error("Invalid handler in `file-name-handler-alist'".into())) }
}
