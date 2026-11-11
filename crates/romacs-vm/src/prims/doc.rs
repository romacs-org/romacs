//! GNU doc.c: docstrings of primitives live in etc/DOC, named by byte
//! offsets (Snarf-documentation); Lisp formats them through
//! substitute-command-keys.
use super::*;
use crate::prim::leaf::Kind;

fn doc_file(cx: &mut Leaf<'_>) -> std::result::Result<Option<std::path::PathBuf>, Fault> {
    let (dir, file) = (cx.intern("doc-directory"), cx.intern("internal-doc-file-name"));
    let dir = cx.variable_value(dir)?.unwrap_or(Value::NIL);
    let file = cx.variable_value(file)?.unwrap_or(Value::NIL);
    if !matches!(cx.kind(dir), Kind::String | Kind::Bytes) || !matches!(cx.kind(file), Kind::String | Kind::Bytes) {
        return Ok(None);
    }
    let file = cx.string_text(file)?;
    let path = std::path::Path::new(&file);
    Ok(Some(if path.is_absolute() { path.to_path_buf() } else { std::path::Path::new(&cx.string_text(dir)?).join(path) }))
}

/// Snarf-documentation's position for a primitive: the byte after its
/// "\037FNAME\n" record header, or None when DOC has no record for it.
fn subr_position(bytes: &[u8], name: &str) -> Option<usize> {
    let header = format!("\x1fF{name}\n");
    let header = header.as_bytes();
    bytes.windows(header.len()).position(|w| w == header).map(|at| at + header.len())
}

/// get_doc_string for a fixnum position: the text up to the next ^_, with
/// ^A^A, ^A0 and ^A_ unquoted; the data decide multibyteness.
fn doc_string(cx: &mut Leaf<'_>, position: i64) -> Out {
    let Some(path) = doc_file(cx)? else { return Ok(Value::NIL); };
    let Some(bytes) = cx.doc_file_bytes(&path) else {
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        return Ok(cx.string(&format!("Cannot open doc string file \"{name}\"\n")));
    };
    // Reading at or past the end yields the empty text, as lseek+read does.
    let start = (position.unsigned_abs() as usize).min(bytes.len());
    let end = bytes[start..].iter().position(|&b| b == 0x1f).map_or(bytes.len(), |n| start + n);
    let mut text = Vec::with_capacity(end - start);
    let mut from = bytes[start..end].iter().copied();
    while let Some(b) = from.next() {
        if b != 1 { text.push(b); continue; }
        match from.next() {
            Some(1) => text.push(1),
            Some(b'0') => text.push(0),
            Some(b'_') => text.push(0x1f),
            other => return Err(Fault::Error(format!(
                "Invalid data in documentation file -- \u{1} followed by code {:03o}", other.unwrap_or(0)))),
        }
    }
    let multibyte = crate::environment::multibyte(&text);
    Ok(cx.make_string(text, multibyte))
}

/// Snarf-documentation's variable records (doc.c:606): each "\037VNAME\n"
/// record's name and the position of its text, unless the text is a SKIP
/// placeholder.
fn variable_records(bytes: &[u8]) -> Vec<(&[u8], usize)> {
    let mut records = vec![];
    let mut at = 0;
    while let Some(found) = bytes[at..].windows(2).position(|w| w == b"\x1fV") {
        let name = at + found + 2;
        let Some(newline) = bytes[name..].iter().position(|&b| b == b'\n') else { break };
        let text = name + newline + 1;
        if !bytes[text..].starts_with(b"SKIP") { records.push((&bytes[name..name + newline], text)); }
        at = text;
    }
    records
}

// Internal: the scan half of Snarf-documentation (doc.c:500), which
// lisp/romacs/romacs-loadup-bootstrap.el finishes in Lisp. FILENAME is in
// `doc-directory'; the value lists each variable record as (NAME . POSITION).
defsubr! { name = "romacs--doc-variable-records", gnu = "doc.c:500", effect = Reads,
fn doc_variable_records(cx, filename: Value) {
    let directory = cx.intern("doc-directory");
    let directory = cx.variable_value(directory)?.unwrap_or(Value::NIL);
    if !matches!(cx.kind(directory), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", directory)); }
    if !matches!(cx.kind(filename), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", filename)); }
    let path = std::path::Path::new(&cx.string_text(directory)?).join(cx.string_text(filename)?);
    let Some(bytes) = cx.doc_file_bytes(&path) else {
        return Err(Fault::Error(format!("Opening doc string file: No such file or directory, {}", path.display())));
    };
    let mut list = Value::NIL;
    for (name, position) in variable_records(&bytes).into_iter().rev() {
        let name = cx.make_string(name.to_vec(), crate::environment::multibyte(name));
        let position = Value::integer(position as i64).map_err(Fault::Error)?;
        let record = cx.cons(name, position);
        list = cx.cons(record, list);
    }
    Ok(list)
}}

defsubr! { name = "internal-subr-documentation", gnu = "doc.c:383", effect = Reads,
fn internal_subr_documentation(cx, function: Value) {
    if cx.kind(function) != Kind::Subr { return Ok(Value::TRUE); }
    // XSUBR (function)->doc: 0 when DOC has no record for the primitive.
    let name = crate::prim::name(function).unwrap_or("");
    let position = doc_file(cx)?.and_then(|path| cx.doc_file_bytes(&path))
        .and_then(|bytes| subr_position(&bytes, name)).unwrap_or(0);
    Ok(Value::integer(position as i64).map_err(Fault::Error)?)
}}

/// The DOC lookup and substitute-command-keys tail shared by both entries.
fn finish(cx: &mut Leaf<'_>, doc: Value, raw: Value, strings_only: bool) -> Next {
    let doc = if doc.integer_value() == Some(0) { Value::NIL } else { doc };
    let doc = if let Some(position) = doc.integer_value() {
        doc_string(cx, position)?
    } else if cx.pair(doc).is_some_and(|(_, pos)| pos.integer_value().is_some()) {
        return Err(super::fileio::refuse("documentation from a (FILE . POSITION) docstring"));
    } else { doc };
    let string = matches!(cx.kind(doc), Kind::String | Kind::Bytes);
    if raw != Value::NIL || (strings_only && !string) { return Ok(Step::Done(doc)); }
    Ok(Step::Call { f: cx.intern("substitute-command-keys"), args: vec![doc], then: done, saved: vec![] })
}
fn done(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next { Ok(Step::Done(value)) }

defsubr! { name = "documentation-property", gnu = "doc.c:402", effect = Global,
fn documentation_property(cx, symbol: Value, prop: Value; optional raw: Value) -> Step {
    let mut doc = cx.face_property(symbol, prop)?;
    if doc == Value::NIL && prop == cx.intern("variable-documentation") {
        // Findirect_variable: the end of SYMBOL's alias chain.
        let mut indirect = symbol;
        let mut seen = Vec::new();
        while let Some(next) = cx.variable_alias(indirect)? {
            if seen.contains(&next) { break; }
            seen.push(next);
            indirect = next;
        }
        if indirect != symbol { doc = cx.face_property(indirect, prop)?; }
    }
    let lookup = doc.integer_value().is_some() || cx.pair(doc).is_some_and(|(_, pos)| pos.integer_value().is_some());
    if !lookup && doc != Value::NIL && !matches!(cx.kind(doc), Kind::String | Kind::Bytes) {
        // A non-string property is a form to evaluate.
        return Ok(Step::Call { f: cx.intern("eval"), args: vec![doc, Value::NIL], then: evaluated, saved: vec![raw] });
    }
    finish(cx, doc, raw, true)
}}
// An evaluated property is the documentation itself, never a DOC position.
fn evaluated(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    if saved[0] != Value::NIL || !matches!(cx.kind(value), Kind::String | Kind::Bytes) { return Ok(Step::Done(value)); }
    Ok(Step::Call { f: cx.intern("substitute-command-keys"), args: vec![value], then: done, saved: vec![] })
}

defsubr! { name = "documentation", gnu = "doc.c:333", effect = Global,
fn documentation(cx, function: Value; optional raw: Value) -> Step {
    let property = cx.intern("function-documentation");
    if cx.kind(function) == Kind::Symbol && cx.face_property(function, property)? != Value::NIL {
        return Ok(Step::Call { f: cx.intern("documentation-property"), args: vec![function, property, raw], then: done, saved: vec![] });
    }
    let mut fun = function;
    let mut seen = Vec::new();
    while fun != Value::NIL && cx.kind(fun) == Kind::Symbol {
        if seen.contains(&fun) { return Err(Fault::CyclicFunctionIndirection(function)); }
        seen.push(fun);
        fun = cx.symbol_definition(fun)?;
    }
    if fun == Value::NIL { return Err(Fault::Signal(cx.intern("void-function"), cx.cons(function, Value::NIL))); }
    if let Some((head, body)) = cx.pair(fun) { if head == cx.intern("macro") { fun = body; } }
    Ok(Step::Call { f: property, args: vec![fun], then: documented, saved: vec![raw] })
}}
fn documented(cx: &mut Leaf<'_>, doc: Value, saved: &[Value]) -> Next { finish(cx, doc, saved[0], false) }
