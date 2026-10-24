//! GNU fns.c sequence callbacks. All state crossing Lisp calls is traced.
use super::*;

fn native(name: &str) -> Value {
    (0..crate::prim::count())
        .find(|&i| crate::prim::descriptor(i).is_some_and(|d| d.name() == name))
        .map(|i| Value::reference(i, 6))
        .expect("required native primitive is registered")
}
fn number(n: usize) -> Value {
    Value::integer(n as i64).expect("sequence index")
}
fn index(v: Value) -> usize {
    v.integer_value().expect("saved index") as usize
}

defsubr! { name = "mapconcat", gnu = "fns.c:3484", effect = Global,
fn mapconcat(cx, function: Value, sequence: Value; optional separator: Value) -> Step {
    Ok(Step::Call { f: native("length"), args: vec![sequence], then: map_measured,
        saved: vec![function, sequence, separator] })
}}

// function, sequence, separator, length, index, list/byte cursor, mapped results.
fn map_measured(cx: &mut Leaf<'_>, length: Value, saved: &[Value]) -> Next {
    if cx.kind(saved[1]) == leaf::Kind::CharTable {
        return Err(Fault::Wrong("listp", saved[1]));
    }
    let cursor = if cx.pair(saved[1]).is_some() {
        saved[1]
    } else {
        number(0)
    };
    let mut state = saved.to_vec();
    let results = cx.vector(&vec![Value::NIL; index(length)]);
    state.extend([length, number(0), cursor, results]);
    let seq = state[1];
    if state[0] == cx.intern("identity") {
        if let Ok(values) = cx.vector_values(seq) {
            state[6] = cx.vector(&values);
            state[4] = length;
        } else if cx.pair(seq).is_some() {
            let mut values = Vec::new();
            let mut tail = seq;
            while let Some((car, cdr)) = cx.pair(tail) {
                values.push(car);
                tail = cdr;
            }
            state[6] = cx.vector(&values);
            state[4] = length;
        }
    }
    map_next(cx, state)
}
fn map_next(cx: &mut Leaf<'_>, mut state: Vec<Value>) -> Next {
    let i = index(state[4]);
    if i >= index(state[3]) {
        let mut parts = Vec::new();
        let values = cx.vector_values(state[6])?;
        for (i, &part) in values[..index(state[3])].iter().enumerate() {
            if i != 0 {
                parts.push(state[2]);
            }
            parts.push(part);
        }
        return super::widen::concat(cx, &parts).map(Step::Done);
    }
    let seq = state[1];
    let element = if cx.pair(seq).is_some() {
        let Some((car, _)) = cx.pair(state[5]) else {
            state[3] = state[4];
            return map_next(cx, state);
        };
        car
    } else if matches!(cx.kind(seq), leaf::Kind::String | leaf::Kind::Bytes) {
        let (code, width) = cx.string_char_at(seq, index(state[5])).ok_or_else(|| {
            Fault::Error("String's bytes changed under a mapping function".into())
        })?;
        state[5] = number(index(state[5]) + width);
        number(code as usize)
    } else {
        super::fns_access::elt(cx, seq, number(i))?
    };
    Ok(Step::Call {
        f: state[0],
        args: vec![element],
        then: map_called,
        saved: state,
    })
}
fn map_called(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let mut state = saved.to_vec();
    cx.vector_set(state[6], state[4], value)?;
    if let Some((_, cdr)) = cx.pair(state[5]) {
        state[5] = cdr;
    }
    state[4] = number(index(state[4]) + 1);
    map_next(cx, state)
}

// State: original, lessp, key, reverse, inplace, items, keys, i, j.
// Small inputs append phase, descending, binary low and binary high.
defsubr! { name = "sort", gnu = "fns.c:2447", effect = Global,
fn sort(cx, sequence: Value; rest options) -> Step {
    let (mut lessp, mut key, mut reverse, mut inplace) = (Value::NIL, Value::NIL, Value::NIL, Value::NIL);
    if options.len() == 1 {
        lessp = options[0];
        inplace = Value::TRUE;
    } else if options.len() % 2 != 0 {
        return Err(Fault::Error("Invalid argument list".into()));
    } else {
        for pair in options.chunks_exact(2) {
            let name = pair[0];
            match name {
                name if name == cx.intern(":lessp") => lessp = pair[1],
                name if name == cx.intern(":key") => key = pair[1],
                name if name == cx.intern(":reverse") => reverse = pair[1],
                name if name == cx.intern(":in-place") => inplace = pair[1],
                _ => {
                    let condition = cx.intern("error");
                    let message = cx.string("Invalid keyword argument");
                    let data = cx.cons(pair[0], Value::NIL);
                    let data = cx.cons(message, data);
                    return Err(Fault::Signal(condition, data));
                }
            }
        }
    }
    if sequence == Value::NIL { return Ok(Step::Done(sequence)); }
    let values = if cx.pair(sequence).is_some() {
        let mut values = Vec::new();
        let mut seen = HashSet::new();
        let mut tail = sequence;
        while let Some((car, cdr)) = cx.pair(tail) {
            if !seen.insert(tail.0) { return Err(Fault::Circular(sequence)); }
            values.push(car);
            tail = cdr;
        }
        if tail != Value::NIL { return Err(Fault::Wrong("listp", tail)); }
        values
    } else { cx.vector_values(sequence).map_err(|_| Fault::Wrong("list-or-vector-p", sequence))? };
    if values.is_empty() { return Ok(Step::Done(sequence)); }
    // GNU sort.c reverses the input before computing keys, then reverses
    // the sorted result. For lengths below 64 minrun is the whole input.
    let mut values = values;
    if values.len() < 64 && reverse != Value::NIL { values.reverse(); }
    let items = cx.vector(&values);
    let keys = cx.vector(&values);
    if lessp == Value::NIL { lessp = cx.intern("value<"); }
    let state = vec![sequence, lessp, key, reverse, inplace, items, keys, number(0), number(0)];
    if values.len() < 2 { return sort_finish(cx, &state); }
    sort_key(cx, state)
}}
fn sort_key(cx: &mut Leaf<'_>, mut state: Vec<Value>) -> Next {
    let n = cx.vector_len(state[5])?;
    let i = index(state[7]);
    if state[2] == Value::NIL || state[2] == cx.intern("identity") || i == n {
        state[7] = number(1);
        state[8] = number(1);
        if n < 64 {
            // phase (initial pair/run/binary), descending, binary low/high.
            state.extend([number(0), Value::NIL, number(0), number(0)]);
            return sort_small_compare(cx, state);
        }
        return sort_compare(cx, state);
    }
    let element = cx.vector_get(state[5], number(i))?;
    Ok(Step::Call {
        f: state[2],
        args: vec![element],
        then: sort_keyed,
        saved: state,
    })
}
fn sort_keyed(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let mut state = saved.to_vec();
    cx.vector_set(state[6], state[7], value)?;
    state[7] = number(index(state[7]) + 1);
    sort_key(cx, state)
}
fn sort_compare(cx: &mut Leaf<'_>, mut state: Vec<Value>) -> Next {
    let n = cx.vector_len(state[5])?;
    let (i, j) = (index(state[7]), index(state[8]));
    if i >= n {
        return sort_finish(cx, &state);
    }
    if j == 0 {
        state[7] = number(i + 1);
        state[8] = number(i + 1);
        return sort_compare(cx, state);
    }
    let mut args = vec![
        cx.vector_get(state[6], number(j))?,
        cx.vector_get(state[6], number(j - 1))?,
    ];
    if state[3] != Value::NIL {
        args.swap(0, 1);
    }
    Ok(Step::Call {
        f: state[1],
        args,
        then: sort_compared,
        saved: state,
    })
}
fn sort_compared(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let mut state = saved.to_vec();
    let (i, j) = (index(state[7]), index(state[8]));
    if before != Value::NIL {
        for slot in [5, 6] {
            let a = cx.vector_get(state[slot], number(j))?;
            let b = cx.vector_get(state[slot], number(j - 1))?;
            cx.vector_set(state[slot], number(j - 1), a)?;
            cx.vector_set(state[slot], number(j), b)?;
        }
        state[8] = number(j - 1);
    } else {
        state[7] = number(i + 1);
        state[8] = number(i + 1);
    }
    sort_compare(cx, state)
}
// GNU sort.c count_run + binarysort, restricted to one-minrun inputs.
// All callback-visible values remain in traced state; no heap indices are
// retained in Rust across calls. Large inputs retain the partial fallback.
fn sort_small_compare(cx: &mut Leaf<'_>, state: Vec<Value>) -> Next {
    let i = index(state[7]);
    let other = if index(state[9]) < 2 {
        i - 1
    } else {
        index(state[11]) + ((index(state[12]) - index(state[11])) >> 1)
    };
    let args = vec![
        cx.vector_get(state[6], number(i))?,
        cx.vector_get(state[6], number(other))?,
    ];
    Ok(Step::Call { f: state[1], args, then: sort_small_compared, saved: state })
}
fn sort_reverse_run(cx: &mut Leaf<'_>, state: &[Value], end: usize) -> std::result::Result<(), Fault> {
    for slot in [5, 6] {
        for i in 0..end / 2 {
            let j = end - i - 1;
            let a = cx.vector_get(state[slot], number(i))?;
            let b = cx.vector_get(state[slot], number(j))?;
            cx.vector_set(state[slot], number(i), b)?;
            cx.vector_set(state[slot], number(j), a)?;
        }
    }
    Ok(())
}
fn sort_small_compared(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let mut state = saved.to_vec();
    let n = cx.vector_len(state[5])?;
    let i = index(state[7]);
    let phase = index(state[9]);
    if phase < 2 {
        if phase == 0 { state[10] = before; }
        if phase == 0 || (before != Value::NIL) == (state[10] != Value::NIL) {
            state[9] = number(1);
            state[7] = number(i + 1);
            if i + 1 < n { return sort_small_compare(cx, state); }
            if state[10] != Value::NIL { sort_reverse_run(cx, &state, n)?; }
            return sort_finish(cx, &state);
        }
        // GNU restarts comparisons at binarysort's midpoint, without reusing
        // the failed run result; callbacks can observe both comparisons.
        if state[10] != Value::NIL { sort_reverse_run(cx, &state, i)?; }
        state[9] = number(2);
        state[11] = number(0);
        state[12] = number(i);
        return sort_small_compare(cx, state);
    }
    let low = index(state[11]);
    let high = index(state[12]);
    let mid = low + ((high - low) >> 1);
    if before != Value::NIL { state[12] = number(mid); }
    else { state[11] = number(mid + 1); }
    if state[11] != state[12] { return sort_small_compare(cx, state); }
    let at = index(state[11]);
    for slot in [5, 6] {
        let pivot = cx.vector_get(state[slot], number(i))?;
        for j in (at + 1..=i).rev() {
            let value = cx.vector_get(state[slot], number(j - 1))?;
            cx.vector_set(state[slot], number(j), value)?;
        }
        cx.vector_set(state[slot], number(at), pivot)?;
    }
    if i + 1 == n { return sort_finish(cx, &state); }
    state[7] = number(i + 1);
    state[11] = number(0);
    state[12] = number(i + 1);
    sort_small_compare(cx, state)
}
fn sort_finish(cx: &mut Leaf<'_>, state: &[Value]) -> Next {
    let mut values = cx.vector_values(state[5])?;
    if values.len() < 64 && state[3] != Value::NIL {
        values.reverse();
        for (i, &value) in values.iter().enumerate() {
            cx.vector_set(state[5], number(i), value)?;
        }
    }
    let original = state[0];
    let result = if state[4] != Value::NIL {
        if cx.pair(original).is_some() {
            let mut tail = original;
            for value in values {
                let Some((_, cdr)) = cx.pair(tail) else { break };
                cx.setcar(tail, value)?;
                tail = cdr;
            }
        } else {
            for (i, value) in values.into_iter().enumerate() {
                cx.vector_set(original, number(i), value)?;
            }
        }
        original
    } else if cx.pair(original).is_some() {
        values
            .into_iter()
            .rev()
            .fold(Value::NIL, |tail, car| cx.cons(car, tail))
    } else {
        state[5]
    };
    Ok(Step::Done(result))
}
