//! GNU fns.c sequence callbacks. All state crossing Lisp calls is traced.
use super::*;

fn number(n: usize) -> Value {
    Value::integer(n as i64).expect("sequence index")
}
fn index(v: Value) -> usize {
    v.integer_value().expect("saved index") as usize
}

// State: original, lessp, key, reverse, inplace, items, keys, i, j. Small
// inputs append phase, descending, binary low/high; larger ones sort_merge's.
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
        let mut seen = HashSet::default();
        let mut tail = sequence;
        // Nothing is written yet: polled anywhere.
        let mut steps = crate::quit::Steps::default();
        while let Some((car, cdr)) = cx.pair(tail) {
            if steps.tick() { cx.quittable(Leaf::native_poll)?; }
            if !seen.insert(tail.0) { return Err(Fault::Circular(sequence)); }
            values.push(car);
            tail = cdr;
        }
        if tail != Value::NIL { return Err(Fault::Wrong("listp", tail)); }
        values
    } else { cx.vector_values(sequence).map_err(|_| Fault::Wrong("list-or-vector-p", sequence))? };
    if values.is_empty() { return Ok(Step::Done(sequence)); }
    // GNU sort.c reverses the input before computing keys, then reverses
    // the sorted result, which keeps a reversed sort stable.
    let mut values = values;
    if reverse != Value::NIL { values.reverse(); }
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
        // Its sort stops consistently: the input is written only at the end.
        if cx.quittable(|cx| sort_native(cx, &state))? { return sort_finish(cx, &state); }
        state[7] = number(1);
        state[8] = number(1);
        if n < 64 {
            // phase (initial pair/run/binary), descending, binary low/high.
            state.extend([number(0), Value::NIL, number(0), number(0)]);
            return sort_small_compare(cx, state);
        }
        let destination = [cx.vector(&vec![Value::NIL; n]), cx.vector(&vec![Value::NIL; n])];
        state[8] = number(0);
        state.extend([number(1), number(0)]);
        state.extend(destination);
        return sort_merge(cx, state);
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
/// A stable sort of ORDER by COMPARE that polls as it goes: runs of RUN
/// elements sorted alone, then merged pairwise a level at a time, a poll
/// every RUN merged elements. Nothing outside ORDER is written.
pub(crate) fn sort_polled<T: Copy>(cx: &mut Leaf<'_>, order: &mut [T], run: usize,
                        mut compare: impl FnMut(&mut Leaf<'_>, &T, &T) -> std::cmp::Ordering) -> std::result::Result<(), Fault> {
    use std::cmp::Ordering::Less;
    for chunk in order.chunks_mut(run) {
        chunk.sort_by(|a, b| compare(cx, a, b));
        cx.native_poll()?;
    }
    // Each level merges from one buffer into the other, so no level copies
    // back; the last result is copied into ORDER if it ended in the other.
    let mut other: Vec<T> = order.to_vec();
    let (mut from_order, mut width) = (true, run);
    while width < order.len() {
        let (source, target): (&[T], &mut [T]) = if from_order { (&*order, &mut other[..]) } else { (&other[..], &mut *order) };
        for start in (0..source.len()).step_by(2 * width) {
            let (mid, end) = ((start + width).min(source.len()), (start + 2 * width).min(source.len()));
            let (mut left, mut right, mut out) = (start, mid, start);
            while left < mid && right < end {
                // Stable: the right element goes first only when strictly less.
                if compare(cx, &source[right], &source[left]) == Less {
                    target[out] = source[right];
                    right += 1;
                } else {
                    target[out] = source[left];
                    left += 1;
                }
                out += 1;
                if (out - start).is_multiple_of(run) { cx.native_poll()?; }
            }
            target[out..out + mid - left].copy_from_slice(&source[left..mid]);
            out += mid - left;
            target[out..end].copy_from_slice(&source[right..end]);
        }
        from_order = !from_order;
        width *= 2;
    }
    if !from_order { order.copy_from_slice(&other); }
    Ok(())
}

/// Fixnums a sort's run holds between two polls: about 10 ms.
const SORT_RUN: usize = 1 << 18;
/// Strings a run holds: a comparison reads both strings' text, about six
/// times a fixnum's, so about 15 ms.
const STRING_RUN: usize = 1 << 14;

/// The orders `sort_native` takes natively: by string, or by fixnum
/// ascending (`<`, `value<`) or descending (`>`).
enum Native { Strings, Ascending, Descending }

/// The native order of a predicate that resolves to a builtin subr, when
/// every key is of a type it compares without signaling. GNU sort.c marks
/// this point as where keys would be checked for faster comparison.
fn sort_native(cx: &mut Leaf<'_>, state: &[Value]) -> std::result::Result<bool, Fault> {
    // The predicate resolved as a call would: aliases followed; a macro
    // (`(macro . F)`, rebuilt from the slot's flag) or an autoload ends the
    // walk, and the general path calls it, signalling as GNU does. `nil`
    // and `t` end it too: the general path's dispatcher does not read their
    // cells (romacs#309), and the two paths must agree. This fallback goes
    // when #309 is fixed.
    let mut f = state[1];
    let mut name = None;
    for _ in 0..64 {
        if f == Value::NIL || f == Value::TRUE { return Ok(false); }
        if let Some(found) = crate::prim::name(f) { name = Some(found); break; }
        if !matches!(cx.kind(f), leaf::Kind::Symbol) { return Ok(false); }
        match cx.symbol_definition(f) { Ok(next) if next != Value::NIL => f = next, _ => return Ok(false) }
    }
    let keys = cx.vector_values(state[6])?;
    let fixnums = || keys.iter().all(|k| k.integer_value().is_some());
    let strings = |symbols: bool| keys.iter().all(|&k| {
        matches!(cx.kind(k), leaf::Kind::String | leaf::Kind::Bytes)
            || symbols && matches!(cx.kind(k), leaf::Kind::Symbol)
    });
    let native = match name {
        Some("string-lessp") if strings(true) => Native::Strings,
        Some("value<") if strings(false) => Native::Strings,
        Some("<" | "value<") if fixnums() => Native::Ascending,
        Some(">") if fixnums() => Native::Descending,
        _ => return Ok(false),
    };
    // The keys' order, sorted with no Lisp to poll: polled between runs
    // (`sort_polled`), nothing written to the input before it ends.
    let mut order: Vec<usize> = (0..keys.len()).collect();
    // Fixnum keys read once, not at each comparison.
    let ints = || -> Vec<i64> { keys.iter().map(|k| k.integer_value().expect("checked fixnums")).collect() };
    // Each a stable sort: keys neither before the other keep their order,
    // as GNU's stable sort keeps them for any predicate.
    match native {
        // Validated above, so the comparison cannot fail.
        Native::Strings => sort_polled(cx, &mut order, STRING_RUN, |cx, a, b| cx.string_order(keys[*a], keys[*b]).unwrap_or(std::cmp::Ordering::Equal))?,
        Native::Ascending => { let ints = ints(); sort_polled(cx, &mut order, SORT_RUN, |_, a, b| ints[*a].cmp(&ints[*b]))?; }
        Native::Descending => { let ints = ints(); sort_polled(cx, &mut order, SORT_RUN, |_, a, b| ints[*b].cmp(&ints[*a]))?; }
    }
    let items = cx.vector_values(state[5])?;
    // Into the sort's own vectors: polled anywhere.
    let mut steps = crate::quit::Steps::default();
    for (to, &from) in order.iter().enumerate() {
        if steps.tick() { cx.native_poll()?; }
        cx.vector_set(state[5], number(to), items[from])?;
        cx.vector_set(state[6], number(to), keys[from])?;
    }
    Ok(true)
}
// A stable bottom-up merge sort for Lisp predicates on inputs past GNU's
// single minrun: O(n log n) calls, where GNU's timsort also gallops.
// State 7..12: width, left cursor, right cursor, output cursor, and the
// destination items and keys, swapped with the source after each pass.
fn sort_merge(cx: &mut Leaf<'_>, mut state: Vec<Value>) -> Next {
    let n = cx.vector_len(state[5])?;
    loop {
        let width = index(state[7]);
        if width >= n { return sort_finish(cx, &state); }
        let out = index(state[10]);
        if out == n {
            state.swap(5, 11);
            state.swap(6, 12);
            state[7] = number(2 * width);
            state[8] = number(0);
            state[9] = number((2 * width).min(n));
            state[10] = number(0);
            continue;
        }
        let low = out - out % (2 * width);
        let (mid, high) = ((low + width).min(n), (low + 2 * width).min(n));
        let (left, right) = (index(state[8]), index(state[9]));
        if left < mid && right < high {
            let args = vec![cx.vector_get(state[6], number(right))?, cx.vector_get(state[6], number(left))?];
            return Ok(Step::Call { f: state[1], args, then: sort_merged, saved: state });
        }
        let (from, to) = if left < mid { (left, mid) } else { (right, high) };
        for (at, i) in (out..).zip(from..to) { sort_move(cx, &state, i, at)?; }
        state[8] = number(high);
        state[9] = number((high + width).min(n));
        state[10] = number(high);
    }
}
fn sort_merged(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let mut state = saved.to_vec();
    // Only a strictly earlier right element moves first, keeping ties stable.
    let side = if before != Value::NIL { 9 } else { 8 };
    let (from, at) = (index(state[side]), index(state[10]));
    sort_move(cx, &state, from, at)?;
    state[side] = number(from + 1);
    state[10] = number(at + 1);
    sort_merge(cx, state)
}
fn sort_move(cx: &mut Leaf<'_>, state: &[Value], from: usize, at: usize) -> std::result::Result<(), Fault> {
    for (source, destination) in [(5, 11), (6, 12)] {
        let value = cx.vector_get(state[source], number(from))?;
        cx.vector_set(state[destination], number(at), value)?;
    }
    Ok(())
}
// GNU sort.c count_run + binarysort, restricted to one-minrun inputs.
// All callback-visible values remain in traced state; no heap indices are
// retained in Rust across calls.
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
    if state[3] != Value::NIL {
        values.reverse();
        for (i, &value) in values.iter().enumerate() {
            cx.vector_set(state[5], number(i), value)?;
        }
    }
    let original = state[0];
    let result = if state[4] != Value::NIL {
        if cx.pair(original).is_some() {
            let mut tail = original;
            // NOPOLL: the list's length, one store each; a quit would leave
            // it half sorted.
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
