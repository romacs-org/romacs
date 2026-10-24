use super::*;

defsubr! { name = "mapcan", gnu = "fns.c:3585", effect = Global,
fn mapcan(cx, function: Value, sequence: Value) -> Step {
    begin(function, sequence, Value::NIL)
}}

const R_FUNCTION: usize = 0;
const R_SEQUENCE: usize = 1;
const R_N: usize = 2;
const R_I: usize = 3;
const R_CURSOR: usize = 4;
const R_ACC: usize = 5;
const R_MAPC: usize = 6;
const REGISTERS: usize = 7;

const STRING_CHANGED: &str = "String's bytes changed under a mapping function";

fn fixnum(n: usize) -> Value {
    Value::integer(n as i64).expect("a sequence length is a fixnum")
}

fn count(v: Value) -> usize {
    v.integer_value().expect("saved count register") as usize
}

fn subr(name: &str) -> Value {
    (0..crate::prim::count())
        .find(|&i| crate::prim::descriptor(i).is_some_and(|d| d.name() == name))
        .map(|i| Value::reference(i, 6))
        .expect("subr is registered")
}

fn begin(function: Value, sequence: Value, mapc: Value) -> Next {
    let mut saved = vec![Value::NIL; REGISTERS];
    saved[R_FUNCTION] = function;
    saved[R_SEQUENCE] = sequence;
    saved[R_MAPC] = mapc;
    Ok(Step::Call { f: subr("length"), args: vec![sequence], then: measured, saved })
}

fn measured(cx: &mut Leaf<'_>, length: Value, saved: &[Value]) -> Next {
    let mut state = saved.to_vec();
    let sequence = state[R_SEQUENCE];
    if cx.kind(sequence) == leaf::Kind::CharTable {
        return Err(Fault::Wrong("listp", sequence));
    }
    state[R_N] = length;
    state[R_I] = fixnum(0);
    state[R_CURSOR] = if cx.pair(sequence).is_some() { sequence } else { fixnum(0) };
    walk(cx, state)
}

fn walk(cx: &mut Leaf<'_>, mut state: Vec<Value>) -> Next {
    let (sequence, n, i) = (state[R_SEQUENCE], count(state[R_N]), count(state[R_I]));
    if i >= n {
        return finish(cx, &state, i);
    }
    let element = if sequence == Value::NIL {
        return finish(cx, &state, 0);
    } else if cx.pair(sequence).is_some() {
        match cx.pair(state[R_CURSOR]) {
            Some((car, _)) => car,
            None => return finish(cx, &state, i),
        }
    } else if cx.kind(sequence) == leaf::Kind::BoolVector {
        if cx.bool_vector(sequence)?.get(i).unwrap() { Value::TRUE } else { Value::NIL }
    } else if cx.vector_len(sequence).is_ok() {
        cx.vector_get(sequence, fixnum(i))?
    } else if matches!(cx.kind(sequence), leaf::Kind::String | leaf::Kind::Bytes) {
        let at = count(state[R_CURSOR]);
        let (code, width) = cx.string_char_at(sequence, at)
            .ok_or_else(|| Fault::Error(STRING_CHANGED.into()))?;
        state[R_CURSOR] = fixnum(at + width);
        Value::integer(i64::from(code)).expect("a character is a fixnum")
    } else {
        return Err(Fault::Wrong("sequencep", sequence));
    };
    Ok(Step::Call { f: state[R_FUNCTION], args: vec![element], then: called, saved: state })
}

fn called(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let mut state = saved.to_vec();
    if state[R_MAPC] == Value::NIL {
        state[R_ACC] = cx.cons(value, state[R_ACC]);
    }
    if let Some((_, cdr)) = cx.pair(state[R_CURSOR]) {
        state[R_CURSOR] = cdr;
    }
    state[R_I] = fixnum(count(state[R_I]) + 1);
    walk(cx, state)
}

fn finish(cx: &mut Leaf<'_>, state: &[Value], nmapped: usize) -> Next {
    if state[R_MAPC] != Value::NIL {
        return Ok(Step::Done(state[R_SEQUENCE]));
    }
    let mut results = Vec::with_capacity(nmapped);
    let mut tail = state[R_ACC];
    while let Some((car, cdr)) = cx.pair(tail) {
        results.push(car);
        tail = cdr;
    }
    results.reverse();
    nconc(cx, &results).map(Step::Done)
}

fn nconc(cx: &mut Leaf<'_>, args: &[Value]) -> Out {
    let mut args = args.to_vec();
    let mut value = Value::NIL;
    for k in 0..args.len() {
        let list = args[k];
        if list == Value::NIL {
            continue;
        }
        if value == Value::NIL {
            value = list;
        }
        if k + 1 == args.len() {
            break;
        }
        if cx.pair(list).is_none() {
            return Err(Fault::Wrong("consp", list));
        }
        let (mut tail, mut last, mut tortoise, mut steps) = (list, list, list, 0);
        while cx.pair(tail).is_some() {
            last = tail;
            (tail, tortoise, steps) = super::fns_equal::for_each_tail_step(cx, tail, tortoise, steps)?;
        }
        let next = args[k + 1];
        cx.setcdr(last, next)?;
        if next == Value::NIL {
            args[k + 1] = last;
        }
    }
    Ok(value)
}

defsubr! { name = "mapc", gnu = "fns.c:3569", effect = Global,
fn mapc(cx, function: Value, sequence: Value) -> Step {
    begin(function, sequence, fixnum(1))
}}
