//! GNU ccl.c's program registry: `register-ccl-program' (ccl.c:2245) with
//! resolve_symbol_ccl_program (ccl.c:1885). Programs register as GNU
//! records them; running them (`ccl-execute', `ccl-execute-on-string') is
//! not implemented (DIVERGENCES.md VM2-CCL1).
use super::*;
use crate::prim::leaf::Kind;

/// Vccl_program_table: entries [NAME CCL-PROG RESOLVEDP UPDATEDP].
const TABLE: &str = "#:vm2-ccl-program-table";
const HEADER_EOF: i64 = 1;
const HEADER_MAIN: usize = 2;

enum Resolved { Invalid, Unresolved, Program(Value) }

/// resolve_symbol_ccl_program: a copy of CCL with each embedded symbol
/// replaced by its index, Unresolved when an index is missing, Invalid for
/// anything that is not a CCL program.
fn resolve(cx: &mut Leaf<'_>, ccl: Value) -> std::result::Result<Resolved, Fault> {
    let len = cx.vector_len(ccl)?;
    if len <= HEADER_MAIN || len > i32::MAX as usize { return Ok(Resolved::Invalid); }
    let int = |v: Value| v.integer_value().is_some_and(|n| (i32::MIN as i64..=i32::MAX as i64).contains(&n));
    let index = |v: Value| v.integer_value().is_some_and(|n| (0..=i32::MAX as i64).contains(&n));
    let mut items = Vec::with_capacity(len);
    let mut unresolved = false;
    for i in 0..len {
        let item = cx.vector_get(ccl, Value::integer(i as i64).unwrap())?;
        if int(item) { items.push(item); continue; }
        let named = match cx.pair(item) {
            Some((symbol, property)) if cx.kind(symbol) == Kind::Symbol && cx.kind(property) == Kind::Symbol => {
                Some(cx.face_property(symbol, property)?).filter(|&v| index(v))
            }
            None if cx.kind(item) == Kind::Symbol => {
                let mut found = None;
                for property in ["translation-table-id", "code-conversion-map-id", "ccl-program-idx"] {
                    let key = cx.intern(property);
                    let v = cx.face_property(item, key)?;
                    if index(v) { found = Some(v); break; }
                }
                found
            }
            _ => return Ok(Resolved::Invalid),
        };
        unresolved |= named.is_none();
        items.push(named.unwrap_or(item));
    }
    let magnification = items[0].integer_value().unwrap_or(-1);
    let eof = items[HEADER_EOF as usize].integer_value().unwrap_or(-1);
    if !(0 <= magnification && 0 <= eof && eof as usize <= len) { return Ok(Resolved::Invalid); }
    Ok(if unresolved { Resolved::Unresolved } else { Resolved::Program(cx.vector_from(items)) })
}

defsubr! { name = "register-ccl-program", gnu = "ccl.c:2245", effect = Global,
fn register_ccl_program(cx, name: Value, ccl_prog: Value) {
    if cx.kind(name) != Kind::Symbol { return Err(Fault::Wrong("symbolp", name)); }
    let (program, resolved) = if ccl_prog == Value::NIL { (Value::NIL, Value::NIL) } else {
        if cx.kind(ccl_prog) != Kind::Vector { return Err(Fault::Wrong("vectorp", ccl_prog)); }
        match resolve(cx, ccl_prog)? {
            Resolved::Invalid => return Err(Fault::Error("Error in CCL program".into())),
            Resolved::Unresolved => (ccl_prog, Value::NIL),
            Resolved::Program(program) => (program, Value::TRUE),
        }
    };
    let table = match cx.internal_value(TABLE) { Some(table) => table, None => cx.vector_from(vec![Value::NIL; 32]) };
    let len = cx.vector_len(table)?;
    let mut entries = Vec::with_capacity(len + 1);
    for i in 0..len { entries.push(cx.vector_get(table, Value::integer(i as i64).unwrap())?); }
    let free = entries.iter().position(|&slot| cx.kind(slot) != Kind::Vector).unwrap_or(len);
    for (idx, &slot) in entries[..free].iter().enumerate() {
        if cx.vector_get(slot, Value::integer(0).unwrap())? == name {
            for (i, v) in [(1, program), (2, resolved), (3, Value::TRUE)] { cx.vector_set(slot, Value::integer(i).unwrap(), v)?; }
            return Value::integer(idx as i64).map_err(Fault::Error);
        }
    }
    let entry = cx.vector_from(vec![name, program, resolved, Value::TRUE]);
    if free == len { entries.push(entry); } else { entries[free] = entry; }
    let table = cx.vector_from(entries);
    cx.set_internal_value(TABLE, table);
    let idx = Value::integer(free as i64).map_err(Fault::Error)?;
    let key = cx.intern("ccl-program-idx");
    cx.put_symbol_property(name, key, idx)?;
    Ok(idx)
}}
