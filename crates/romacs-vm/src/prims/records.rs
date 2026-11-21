//! GNU alloc.c records are distinct vectorlikes; their first slot is the type.
use super::*;

defsubr! { name = "record", gnu = "alloc.c:3516", effect = Pure,
fn record(cx, tag: Value; rest slots) {
    let mut values = Vec::with_capacity(slots.len() + 1);
    values.push(tag);
    values.extend_from_slice(slots);
    cx.record(values)
}}

defsubr! { name = "make-record", gnu = "alloc.c:3499", effect = Pure,
fn make_record(cx, tag: Value, slots: Value, init: Value) {
    let n = slots.integer_value().and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| Fault::Wrong("wholenump", slots))?;
    // PSEUDOVECTOR_SIZE_BITS is 12 in the pinned GNU lisp.h.
    let count = n.checked_add(1).ok_or_else(|| Fault::Wrong("wholenump", slots))?;
    if count > 4095 {
        return Err(Fault::Error(format!("Attempt to allocate a record of {count} slots; max is 4095")));
    }
    let mut values = vec![init; count];
    values[0] = tag;
    cx.record(values)
}}

defsubr! { name = "recordp", gnu = "data.c:405", effect = Pure,
fn recordp(cx, object: Value) {
    Ok(if cx.kind(object) == leaf::Kind::Record { Value::TRUE } else { Value::NIL })
}}

// The live records whose slot 0 is one of TAGS, a list: a struct's
// instances and its descendants' in one walk of the object table. Frozen
// records are not among them. One no longer reachable that no collection
// has freed yet may be, as a weak table still holds it: call
// `garbage-collect' first to leave such records out.
defsubr! { name = "romacs-records-tagged", gnu = "romacs:records", effect = Reads,
fn romacs_records_tagged(cx, tags: Value) {
    let tags = super::widen::list_elements(cx, tags)?;
    Ok(cx.records_tagged(&tags))
}}

// Give each record of the list RECORDS new slots in place, keeping its
// identity: TAG, then one slot per element of the vector MAP. An integer
// there takes the record's old slot at that index; nil takes FILL's value
// at the same position. FILL is nil (every such slot nil), one vector as
// long as MAP for every record, or a list of such vectors, one a record.
// Every record is checked before any changes: a frozen record signals the
// read-only error and leaves all of them as they were. The number reshaped.
defsubr! { name = "romacs-records-reshape", gnu = "romacs:records", effect = MutatesArgs,
fn romacs_records_reshape(cx, records: Value, tag: Value, map: Value, fill: Value) {
    use crate::prim::leaf::Source;
    let records = super::widen::list_elements(cx, records)?;
    let map = cx.vector_values(map)?.into_iter().map(|v| {
        if v == Value::NIL { return Ok(Source::Fill) }
        v.integer_value().and_then(|n| usize::try_from(n).ok()).map(Source::Old).ok_or(Fault::Wrong("natnump", v))
    }).collect::<std::result::Result<Vec<_>, Fault>>()?;
    let row = |cx: &Leaf<'_>, v: Value| -> std::result::Result<Vec<Value>, Fault> {
        let row = cx.vector_values(v)?;
        if row.len() != map.len() { return Err(Fault::Wrong("vectorp", v)) }
        Ok(row)
    };
    let rows: Vec<Vec<Value>> = if fill == Value::NIL {
        vec![vec![Value::NIL; map.len()]]
    } else if cx.kind(fill) == leaf::Kind::Vector {
        vec![row(cx, fill)?]
    } else {
        let rows = super::widen::list_elements(cx, fill)?.into_iter().map(|v| row(cx, v)).collect::<std::result::Result<Vec<_>, Fault>>()?;
        if rows.len() != records.len() { return Err(Fault::Wrong("listp", fill)) }
        rows
    };
    let fills: Vec<&[Value]> = (0..records.len()).map(|n| rows[if rows.len() == 1 { 0 } else { n }].as_slice()).collect();
    let n = cx.records_reshape(&records, tag, &map, &fills)?;
    Ok(i64::try_from(n).ok().and_then(Value::fixnum).unwrap_or(Value::NIL))
}}
