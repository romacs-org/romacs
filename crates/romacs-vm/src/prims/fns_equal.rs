//! `equal`-based alist and sequence primitives, following GNU src/fns.c
//! `Fassoc` and `Fdelete`, and data.c `Fnlistp`.
use super::*;

defsubr! { name = "assoc", gnu = "fns.c:2006", effect = Global,
fn assoc(cx, key: Value, alist: Value; optional testfn: Value) -> Step {
    assoc_scan(cx, key, alist, testfn, alist, alist, 0)
}}

defsubr! { name = "delete", gnu = "fns.c:2137", effect = MutatesArgs,
fn delete(cx, elt: Value, seq: Value) {
    delete_equal(cx, elt, seq)
}}

defsubr! { name = "nlistp", gnu = "data.c:342", effect = Pure,
fn nlistp(cx, object: Value) {
    not_list(cx, object)
}}

// Tier (PRIMITIVE-DOCTRINE section 3, candidate C5): a nil TESTFN compares with
// `eq' then `equal' (both C in GNU) and the whole walk stays a Rust leaf. A
// non-nil TESTFN is a Lisp call per element, which a Rust loop must never make,
// so that walk is a T2 state machine: each candidate is one `Step::Call' of
// `(funcall TESTFN (car ELT) KEY)', GNU's argument order, and every Value the
// walk needs afterwards (KEY, ALIST, TESTFN, the tail, GNU's FOR_EACH_TAIL
// tortoise, the candidate element and the step count) waits in the
// continuation's traced registers, so a collection inside TESTFN forwards them.
// C5's further fast path for the symbols `eq'/`equal' is NOT taken: GNU calls
// them through `calln', so a redefined or advised `eq' would be bypassed.
//
// GNU's `eq_comparable_value (key) && NILP (testfn)' shortcut to `Fassq' is
// not separate here: for such a KEY, `equal' agrees with `eq' and cannot
// signal, so the one loop below gives the same element and the same errors.
fn assoc_scan(cx: &mut Leaf<'_>, key: Value, alist: Value, testfn: Value,
              mut tail: Value, mut tortoise: Value, mut steps: i64) -> Next {
    while let Some((element, _)) = cx.pair(tail) {
        if let Some((candidate, _)) = cx.pair(element) {
            if testfn != Value::NIL {
                let steps = Value::integer(steps).expect("FOR_EACH_TAIL count is a fixnum");
                return Ok(Step::Call {
                    f: testfn,
                    args: vec![candidate, key],
                    then: assoc_tested,
                    saved: vec![key, alist, testfn, tail, tortoise, element, steps],
                });
            }
            if candidate == key || equal(cx, candidate, key)? {
                return Ok(Step::Done(element));
            }
        }
        (tail, tortoise, steps) = for_each_tail_step(cx, tail, tortoise, steps)?;
    }
    // CHECK_LIST_END (tail, alist): GNU names the whole list.
    if tail != Value::NIL { return Err(Fault::Wrong("listp", alist)); }
    Ok(Step::Done(Value::NIL))
}

// Like GNU, the element returned is the one read before the call, and the walk
// continues from the tail's cdr as it is AFTER the call (TESTFN may mutate).
fn assoc_tested(cx: &mut Leaf<'_>, matched: Value, saved: &[Value]) -> Next {
    let &[key, alist, testfn, tail, tortoise, element, steps] = saved else {
        unreachable!("assoc saves seven registers")
    };
    if matched != Value::NIL {
        return Ok(Step::Done(element));
    }
    let steps = steps.integer_value().expect("saved FOR_EACH_TAIL count");
    let (tail, tortoise, steps) = for_each_tail_step(cx, tail, tortoise, steps)?;
    assoc_scan(cx, key, alist, testfn, tail, tortoise, steps)
}

// One FOR_EACH_TAIL_STEP_CYCLEP (lisp.h:5899-5904): step, signal
// `(circular-list TAIL)' on meeting the tortoise, and teleport the tortoise at
// each power-of-two multiple of FOR_EACH_TAIL_THRESHOLD (4096).
pub(super) fn for_each_tail_step(cx: &Leaf<'_>, tail: Value, tortoise: Value, steps: i64)
    -> std::result::Result<(Value, Value, i64), Fault> {
    let tail = cx.pair(tail).expect("FOR_EACH_TAIL steps a cons").1;
    if tail == tortoise { return Err(Fault::Circular(tail)); }
    let steps = steps + 1;
    let teleport = steps & 4095 == 0 && (steps as u64).is_power_of_two();
    Ok((tail, if teleport { tail } else { tortoise }, steps))
}

fn equal(cx: &Leaf<'_>, a: Value, b: Value) -> std::result::Result<bool, Fault> {
    cx.equal(a, b).map_err(|e| match e {
        crate::hash::HashFault::Wrong(p, v) => Fault::Wrong(p, v),
        crate::hash::HashFault::Error("circular-list", Some(v)) => Fault::Circular(v),
        crate::hash::HashFault::Error(m, _) => Fault::Error(m.into()),
        _ => Fault::Error("equal failed".into()),
    })
}

// GNU Fdelete, branch for branch: nil, list (destructive, `equal' ELT first),
// vector and string (never destructive: the same object when nothing matched,
// else a new one), anything else `(wrong-type-argument sequencep SEQ)'. The new
// vector/string is a Leaf allocation, which never collects, and no Value is
// held across a wait point, so nothing here can be moved under us.
fn delete_equal(cx: &mut Leaf<'_>, elt: Value, seq: Value) -> Out {
    if seq == Value::NIL {
        return Ok(seq);
    }
    if cx.pair(seq).is_some() {
        return delete_from_list(cx, elt, seq);
    }
    if let Ok(items) = cx.vector_values(seq) {
        let mut kept = Vec::with_capacity(items.len());
        for &item in &items {
            // GNU compares (equal (aref SEQ i) ELT) here, ELT second.
            if !equal(cx, item, elt)? { kept.push(item); }
        }
        return Ok(if kept.len() == items.len() { seq } else { cx.vector(&kept) });
    }
    if let Ok(string) = cx.string_view(seq) {
        // !CHARACTERP (elt): nothing can match, SEQ itself is returned.
        let Some(target) = elt.integer_value()
            .filter(|c| (0..=crate::lstring::MAX_CHAR as i64).contains(c)) else {
            return Ok(seq);
        };
        let bytes = string.bytes();
        let mut kept = Vec::with_capacity(bytes.len());
        let (mut at, mut removed) = (0, false);
        while at < bytes.len() {
            // STRING_CHAR on multibyte storage, the raw byte on unibyte.
            let (c, width) = if string.is_multibyte() {
                crate::lstring::string_char(&bytes[at..])
            } else {
                (bytes[at] as u32, 1)
            };
            if c as i64 == target { removed = true; } else { kept.extend_from_slice(&bytes[at..at + width]); }
            at += width;
        }
        return Ok(if removed { cx.make_string(kept, string.is_multibyte()) } else { seq });
    }
    Err(Fault::Wrong("sequencep", seq))
}

fn delete_from_list(cx: &mut Leaf<'_>, elt: Value, mut seq: Value) -> Out {
    let (mut tail, mut previous, mut tortoise, mut steps) = (seq, Value::NIL, seq, 0);
    while let Some((car, cdr)) = cx.pair(tail) {
        if equal(cx, elt, car)? {
            if previous == Value::NIL { seq = cdr; } else { cx.setcdr(previous, cdr)?; }
        } else {
            previous = tail;
        }
        (tail, tortoise, steps) = for_each_tail_step(cx, tail, tortoise, steps)?;
    }
    // CHECK_LIST_END (tail, seq): SEQ as already shortened by the walk.
    if tail != Value::NIL { return Err(Fault::Wrong("listp", seq)); }
    Ok(seq)
}

// GNU Fnlistp: `(not (listp OBJECT))'.
fn not_list(cx: &mut Leaf<'_>, object: Value) -> Out {
    Ok(if object == Value::NIL || cx.pair(object).is_some() { Value::NIL } else { Value::TRUE })
}

defsubr! { name = "equal-including-properties", gnu = "fns.c:2822", effect = Pure,
fn equal_including_properties(cx, o1: Value, o2: Value) {
    let same = cx.equal_including_properties(o1, o2).map_err(|e| match e {
        crate::hash::HashFault::Wrong(p, v) => Fault::Wrong(p, v),
        crate::hash::HashFault::Error("circular-list", Some(v)) => Fault::Circular(v),
        crate::hash::HashFault::Error(m, _) => Fault::Error(m.into()),
        crate::hash::HashFault::Internal(m) => Fault::Error(m),
        _ => Fault::Error("equal-including-properties failed".into()),
    })?;
    Ok(if same { Value::TRUE } else { Value::NIL })
}}
