//! Native completion with callback state in traced continuation registers.
use super::*;
use crate::prim::Kind;
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Try,
    All,
    Test,
}

defsubr! { name = "try-completion", gnu = "minibuf.c:1615", effect = Global,
fn try_completion(cx, string: Value, collection: Value; optional predicate: Value) -> Step {
    prepare(cx, Mode::Try, string, collection, predicate)
}}
defsubr! { name = "all-completions", gnu = "minibuf.c:1846", effect = Global,
fn all_completions(cx, string: Value, collection: Value; optional predicate: Value) -> Step {
    prepare(cx, Mode::All, string, collection, predicate)
}}
defsubr! { name = "test-completion", gnu = "minibuf.c:2062", effect = Global,
fn test_completion(cx, string: Value, collection: Value; optional predicate: Value) -> Step {
    prepare(cx, Mode::Test, string, collection, predicate)
}}

fn number(n: usize) -> Value {
    Value::integer(n as i64).unwrap()
}
fn index(n: Value) -> usize {
    n.integer_value().unwrap() as usize
}
fn variable(cx: &mut Leaf<'_>, name: &str) -> std::result::Result<Value, Fault> {
    let symbol = cx.intern(name);
    Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL))
}
fn fold_case(cx: &mut Leaf<'_>) -> std::result::Result<bool, Fault> {
    Ok(variable(cx, "completion-ignore-case")? != Value::NIL)
}

struct State {
    mode: Mode,
    input: Value,
    collection: Value,
    predicate: Value,
    tail: Value,
    pending: Value,
    bucket: usize,
    index: usize,
    best: Value,
    best_len: usize,
    count: usize,
    matches: Value,
    name: Value,
    tortoise: Value,
    steps: i64,
}

impl State {
    fn save(self) -> Vec<Value> {
        vec![
            number(self.mode as usize),
            self.input,
            self.collection,
            self.predicate,
            self.tail,
            self.pending,
            number(self.bucket),
            number(self.index),
            self.best,
            number(self.best_len),
            number(self.count),
            self.matches,
            self.name,
            self.tortoise,
            Value::integer(self.steps).unwrap(),
        ]
    }

    fn restore(values: &[Value]) -> Self {
        let &[
            mode,
            input,
            collection,
            predicate,
            tail,
            pending,
            bucket,
            at,
            best,
            best_len,
            count,
            matches,
            name,
            tortoise,
            steps,
        ] = values
        else {
            unreachable!("completion saves fifteen registers")
        };
        Self {
            mode: match index(mode) {
                0 => Mode::Try,
                1 => Mode::All,
                2 => Mode::Test,
                _ => unreachable!(),
            },
            input,
            collection,
            predicate,
            tail,
            pending,
            bucket: index(bucket),
            index: index(at),
            best,
            best_len: index(best_len),
            count: index(count),
            matches,
            name,
            tortoise,
            steps: steps.integer_value().unwrap(),
        }
    }

    fn next(&mut self, cx: &mut Leaf<'_>) -> std::result::Result<Option<(Value, Value)>, Fault> {
        match cx.kind(self.collection) {
            Kind::HashTable => {
                let Some((at, key, value)) =
                    cx.completion_hash_next(self.collection, self.index)?
                else {
                    return Ok(None);
                };
                self.index = at + 1;
                Ok(Some((key, value)))
            }
            Kind::Obarray => {
                if self.pending == Value::UNBOUND {
                    return Ok(None);
                }
                let symbol = self.pending;
                let next = cx.obarray_next(self.collection, self.bucket, symbol);
                self.bucket = next.map_or(self.bucket, |(bucket, _)| bucket);
                self.pending = next.map_or(Value::UNBOUND, |(_, symbol)| symbol);
                Ok(Some((symbol, Value::NIL)))
            }
            _ => {
                if self.steps < 0 {
                    return Err(Fault::Circular(self.tail));
                }
                let Some((head, _)) = cx.pair(self.tail) else {
                    return Ok(None);
                };
                match super::fns_equal::for_each_tail_step(cx, self.tail, self.tortoise, self.steps)
                {
                    Ok((tail, tortoise, steps)) => {
                        self.tail = tail;
                        self.tortoise = tortoise;
                        self.steps = steps;
                    }
                    Err(Fault::Circular(tail)) => {
                        self.tail = tail;
                        self.steps = -1;
                    }
                    Err(error) => return Err(error),
                }
                Ok(Some((head, Value::NIL)))
            }
        }
    }
}

fn prepare(
    cx: &mut Leaf<'_>,
    mode: Mode,
    input: Value,
    mut collection: Value,
    predicate: Value,
) -> Next {
    if mode == Mode::Test {
        cx.string_view(input)?;
    }
    if cx.kind(collection) == Kind::Vector {
        collection = cx.resolve_obarray(collection)?;
    }
    let ordinary = matches!(cx.kind(collection), Kind::Obarray | Kind::HashTable)
        || collection == Value::NIL
        || (cx.pair(collection).is_some() && !super::vars::function_p(cx, collection)?);
    cx.string_view(input)?;
    if !ordinary {
        let flag = match mode {
            Mode::Try => Value::NIL,
            Mode::All => Value::TRUE,
            Mode::Test => cx.intern("lambda"),
        };
        return Ok(Step::Call {
            f: collection,
            args: vec![input, predicate, flag],
            then: delegated,
            saved: vec![],
        });
    }
    let first = if cx.kind(collection) == Kind::Obarray {
        cx.obarray_next(collection, 0, Value::UNBOUND)
    } else {
        None
    };
    let state = State {
        mode,
        input,
        collection,
        predicate,
        tail: collection,
        pending: first.map_or(Value::UNBOUND, |(_, symbol)| symbol),
        bucket: first.map_or(0, |(bucket, _)| bucket),
        index: 0,
        best: Value::NIL,
        best_len: 0,
        count: 0,
        matches: Value::NIL,
        name: Value::NIL,
        tortoise: collection,
        steps: 0,
    };
    if mode == Mode::Test {
        test_start(cx, state)
    } else {
        walk(cx, state)
    }
}

fn delegated(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(value))
}

fn candidate_name(
    cx: &mut Leaf<'_>,
    collection: Value,
    candidate: Value,
) -> std::result::Result<Value, Fault> {
    let name = if matches!(cx.kind(collection), Kind::HashTable | Kind::Obarray) {
        candidate
    } else {
        cx.pair(candidate).map_or(candidate, |(key, _)| key)
    };
    if cx.kind(name) == Kind::Symbol {
        cx.symbol_name_value(name)
    } else {
        Ok(name)
    }
}

fn matches_regexps(cx: &mut Leaf<'_>, name: Value) -> std::result::Result<bool, Fault> {
    let mut tail = variable(cx, "completion-regexp-list")?;
    let (mut slow, mut steps) = (tail, 0);
    let fold = fold_case(cx)?;
    while let Some((pattern, _)) = cx.pair(tail) {
        if !cx.completion_regexp_match(pattern, name, fold)? {
            return Ok(false);
        }
        (tail, slow, steps) = super::fns_equal::for_each_tail_step(cx, tail, slow, steps)?;
    }
    Ok(true)
}

fn walk(cx: &mut Leaf<'_>, mut state: State) -> Next {
    while let Some((candidate, associated)) = state.next(cx)? {
        let name = candidate_name(cx, state.collection, candidate)?;
        if !matches!(cx.kind(name), Kind::String | Kind::Bytes) {
            continue;
        }
        let length = cx.string_view(state.input)?.chars;
        let name_len = cx.string_view(name)?.chars;
        if name_len < length || (state.mode == Mode::Test && name_len != length) {
            continue;
        }
        let fold = fold_case(cx)?;
        if super::string_compare::prefix(cx, name, state.input, length, fold)?.0 != Ordering::Equal
        {
            continue;
        }
        state.name = name;
        if state.mode == Mode::Test {
            if candidate == Value::NIL && cx.pair(state.collection).is_some() {
                return Ok(Step::Done(Value::NIL));
            }
            return test_found(cx, state, candidate, associated);
        }
        if !matches_regexps(cx, name)? {
            continue;
        }
        if state.predicate != Value::NIL {
            if state.predicate == cx.intern("commandp") {
                return super::callint::command_predicate(
                    cx,
                    candidate,
                    Value::NIL,
                    tested,
                    state.save(),
                );
            }
            let args = if cx.kind(state.collection) == Kind::HashTable {
                vec![candidate, associated]
            } else {
                vec![candidate]
            };
            return Ok(Step::Call {
                f: state.predicate,
                args,
                then: tested,
                saved: state.save(),
            });
        }
        if accept(cx, &mut state)? {
            return finish(cx, state);
        }
    }
    finish(cx, state)
}

fn tested(cx: &mut Leaf<'_>, accepted: Value, saved: &[Value]) -> Next {
    let mut state = State::restore(saved);
    if accepted != Value::NIL && accept(cx, &mut state)? {
        return finish(cx, state);
    }
    walk(cx, state)
}

fn accept(cx: &mut Leaf<'_>, state: &mut State) -> std::result::Result<bool, Fault> {
    if state.mode == Mode::All {
        state.matches = cx.cons(state.name, state.matches);
        return Ok(false);
    }
    let name_len = cx.string_view(state.name)?.chars;
    if state.best == Value::NIL {
        state.best = state.name;
        state.best_len = name_len;
        state.count = 1;
        return Ok(false);
    }
    let compare = state.best_len.min(name_len);
    let folded = fold_case(cx)?;
    let (_, matched) = super::string_compare::prefix(cx, state.best, state.name, compare, folded)?;
    let old_best = state.best;
    let best_length = cx.string_view(state.best)?.chars;
    let input_length = cx.string_view(state.input)?.chars;
    if folded {
        let shorter_exact = matched == name_len && matched < best_length;
        let candidate_case =
            super::string_compare::prefix(cx, state.name, state.input, input_length, false)?.0
                == Ordering::Equal;
        let old_case =
            super::string_compare::prefix(cx, state.best, state.input, input_length, false)?.0
                == Ordering::Equal;
        if shorter_exact
            || ((matched == name_len) == (matched == best_length) && candidate_case && !old_case)
        {
            state.best = state.name;
        }
    }
    if state.best_len != name_len
        || state.best_len != matched
        || (folded
            && super::string_compare::prefix(cx, old_best, state.name, compare, false)?.0
                != Ordering::Equal)
    {
        state.count = 2;
    }
    state.best_len = matched;
    Ok(matched <= input_length && !folded && state.count > 1)
}

fn finish(cx: &mut Leaf<'_>, state: State) -> Next {
    if state.mode == Mode::Test {
        return Ok(Step::Done(Value::NIL));
    }
    if state.mode == Mode::All {
        let (mut tail, mut result) = (state.matches, Value::NIL);
        while let Some((value, rest)) = cx.pair(tail) {
            result = cx.cons(value, result);
            tail = rest;
        }
        return Ok(Step::Done(result));
    }
    if state.best == Value::NIL {
        return Ok(Step::Done(Value::NIL));
    }
    if state.count == 1
        && cx
            .equal(state.best, state.input)
            .map_err(|_| Fault::Error("completion string comparison failed".into()))?
    {
        return Ok(Step::Done(Value::TRUE));
    }
    cx.string_slice(state.best, 0, state.best_len)
        .map(Step::Done)
}

fn test_start(cx: &mut Leaf<'_>, mut state: State) -> Next {
    if cx.kind(state.collection) == Kind::HashTable {
        if let Some((key, associated)) = cx.completion_hash_find(state.collection, state.input)? {
            return test_found(cx, state, key, associated);
        }
    } else if cx.kind(state.collection) == Kind::Obarray {
        if let Some(symbol) = cx.completion_obarray_find(state.collection, state.input)? {
            return test_found(cx, state, symbol, Value::NIL);
        }
        if !fold_case(cx)? {
            return Ok(Step::Done(Value::NIL));
        }
    }
    state.mode = Mode::Test;
    walk(cx, state)
}

fn test_found(cx: &mut Leaf<'_>, state: State, candidate: Value, associated: Value) -> Next {
    if !matches_regexps(cx, state.input)? {
        return Ok(Step::Done(Value::NIL));
    }
    if state.predicate == Value::NIL {
        return Ok(Step::Done(Value::TRUE));
    }
    let args = if cx.kind(state.collection) == Kind::HashTable {
        vec![candidate, associated]
    } else {
        vec![candidate]
    };
    Ok(Step::Call {
        f: state.predicate,
        args,
        then: delegated,
        saved: vec![],
    })
}
