//! GNU sqlite.c: SQLite databases and result sets (`prim/leaf/sqlite.rs`).
use super::*;

defsubr! { name = "sqlite-open", gnu = "sqlite.c:280", effect = Global,
fn sqlite_open(cx; optional file: Value, readonly: Value, disable_uri: Value) -> Step {
    if file == Value::NIL { return Ok(Step::Done(cx.sqlite_open(None, readonly, disable_uri)?)); }
    super::fileio::expanding(cx, file, open_expanded, vec![readonly, disable_uri])
}}

fn open_expanded(cx: &mut Leaf<'_>, file: Value, saved: &[Value]) -> Next {
    let path = cx.string_text(file)?.into_bytes();
    Ok(Step::Done(cx.sqlite_open(Some(path), saved[0], saved[1])?))
}

defsubr! { name = "sqlite-close", gnu = "sqlite.c:329", effect = Global,
fn sqlite_close(cx, db: Value) {
    cx.sqlite_close(db)
}}

defsubr! { name = "sqlite-execute", gnu = "sqlite.c:493", effect = Global,
fn sqlite_execute(cx, db: Value, query: Value; optional values: Value) {
    cx.sqlite_execute(db, query, values)
}}

defsubr! { name = "sqlite-select", gnu = "sqlite.c:579", effect = Global,
fn sqlite_select(cx, db: Value, query: Value; optional values: Value, return_type: Value) {
    cx.sqlite_select(db, query, values, return_type)
}}

defsubr! { name = "sqlite-execute-batch", gnu = "sqlite.c:664", effect = Global,
fn sqlite_execute_batch(cx, db: Value, statements: Value) {
    cx.sqlite_execute_batch(db, statements)
}}

defsubr! { name = "sqlite-transaction", gnu = "sqlite.c:675", effect = Global,
fn sqlite_transaction(cx, db: Value) {
    cx.sqlite_exec(db, b"begin")
}}

defsubr! { name = "sqlite-commit", gnu = "sqlite.c:683", effect = Global,
fn sqlite_commit(cx, db: Value) {
    cx.sqlite_exec(db, b"commit")
}}

defsubr! { name = "sqlite-rollback", gnu = "sqlite.c:691", effect = Global,
fn sqlite_rollback(cx, db: Value) {
    cx.sqlite_exec(db, b"rollback")
}}

defsubr! { name = "sqlite-pragma", gnu = "sqlite.c:699", effect = Global,
fn sqlite_pragma(cx, db: Value, pragma: Value) {
    cx.sqlite_pragma(db, pragma)
}}

defsubr! { name = "sqlite-load-extension", gnu = "sqlite.c:711", effect = Global,
fn sqlite_load_extension(cx, db: Value, module: Value) -> Step {
    cx.sqlite_database(db)?;
    if !matches!(cx.kind(module), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", module)); }
    if !cx.sqlite_extension_allowed(&cx.string_text(module)?) {
        return Err(cx.sqlite_extension_refused());
    }
    super::fileio::expanding(cx, module, extension_expanded, vec![db])
}}

fn extension_expanded(cx: &mut Leaf<'_>, module: Value, saved: &[Value]) -> Next {
    let path = cx.string_text(module)?;
    Ok(Step::Done(cx.sqlite_load_extension(saved[0], path.as_bytes())?))
}

defsubr! { name = "sqlite-next", gnu = "sqlite.c:792", effect = Global,
fn sqlite_next(cx, set: Value) {
    cx.sqlite_next(set)
}}

defsubr! { name = "sqlite-columns", gnu = "sqlite.c:815", effect = Reads,
fn sqlite_columns(cx, set: Value) {
    cx.sqlite_columns(set)
}}

defsubr! { name = "sqlite-more-p", gnu = "sqlite.c:823", effect = Reads,
fn sqlite_more_p(cx, set: Value) {
    cx.sqlite_more_p(set)
}}

defsubr! { name = "sqlite-finalize", gnu = "sqlite.c:835", effect = Global,
fn sqlite_finalize(cx, set: Value) {
    cx.sqlite_finalize(set)
}}

defsubr! { name = "sqlite-version", gnu = "sqlite.c:846", effect = Pure,
fn sqlite_version(cx) {
    Ok(cx.sqlite_version())
}}

defsubr! { name = "sqlitep", gnu = "sqlite.c:858", effect = Pure,
fn sqlitep(cx, object: Value) {
    Ok(cx.sqlitep(object))
}}

defsubr! { name = "sqlite-available-p", gnu = "sqlite.c:869", effect = Pure,
fn sqlite_available_p(cx) {
    let _ = cx;
    Ok(Value::TRUE)
}}
