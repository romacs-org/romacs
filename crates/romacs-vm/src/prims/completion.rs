//! Native completion with callback state in traced continuation registers.
use super::*;
use crate::prim::Kind;
use std::cmp::Ordering;
use super::callint::Verdict;

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

defsubr! { name = "romacs-command-symbols", gnu = "romacs:commands", effect = Reads,
fn romacs_command_symbols(cx) {
    // A verdict that only Lisp can give, or an error `commandp' would
    // signal, keeps the symbol: callers filter with `commandp' itself.
    for (slot, symbol) in cx.command_set_stale() {
        let member = !matches!(super::callint::command_verdict(cx, symbol, Value::NIL), Ok(Verdict::Known(false)));
        cx.command_set_mark(slot, member);
    }
    Ok(cx.command_set_symbols())
}}
defsubr! { name = "romacs-command-epoch", gnu = "romacs:commands", effect = Reads,
fn romacs_command_epoch(cx) {
    let epoch = i64::try_from(cx.command_set_epoch()).map_err(|_| Fault::Error("command epoch overflow".into()))?;
    Value::integer(epoch).map_err(Fault::Error)
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
    /// This walk's identity, for what it computes once (`walk_case_ranges').
    walk: u64,
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
            Value::integer(self.walk as i64).unwrap(),
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
            walk,
        ] = values
        else {
            unreachable!("completion saves sixteen registers")
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
            walk: walk.integer_value().unwrap() as u64,
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
        walk: cx.next_completion_walk(),
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

/// The `completion-regexp-list' regexps, through FOR_EACH_TAIL.
fn regexp_patterns(cx: &mut Leaf<'_>) -> std::result::Result<Vec<Value>, Fault> {
    let mut tail = variable(cx, "completion-regexp-list")?;
    let (mut slow, mut steps, mut patterns) = (tail, 0, vec![]);
    while let Some((pattern, _)) = cx.pair(tail) {
        patterns.push(pattern);
        (tail, slow, steps) = super::fns_equal::for_each_tail_step(cx, tail, slow, steps)?;
    }
    Ok(patterns)
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
    // No Lisp runs until the next predicate call, after which the walk
    // starts again: what the candidates are matched against holds till then.
    let length = cx.string_view(state.input)?.chars;
    let fold = fold_case(cx)?;
    let regexps = variable(cx, "completion-regexp-list")? != Value::NIL;
    // Compiled at the first candidate that reaches them, as GNU checks them.
    let mut matcher = None;
    let commandp = cx.intern("commandp");
    while let Some((candidate, associated)) = state.next(cx)? {
        let name = candidate_name(cx, state.collection, candidate)?;
        if !matches!(cx.kind(name), Kind::String | Kind::Bytes) {
            continue;
        }
        // Every name begins with the empty input.
        if length > 0 || state.mode == Mode::Test {
            let name_len = cx.string_chars(name).ok_or(Fault::Wrong("stringp", name))?;
            if name_len < length || (state.mode == Mode::Test && name_len != length) {
                continue;
            }
            let same = if fold {
                super::string_compare::prefix(cx, name, state.input, length, fold)?.0 == Ordering::Equal
            } else {
                cx.completion_same_prefix(name, state.input, length)?
            };
            if !same {
                continue;
            }
        }
        state.name = name;
        if state.mode == Mode::Test {
            if candidate == Value::NIL && cx.pair(state.collection).is_some() {
                return Ok(Step::Done(Value::NIL));
            }
            return test_found(cx, state, candidate, associated);
        }
        if regexps {
            if matcher.is_none() {
                let patterns = regexp_patterns(cx)?;
                matcher = Some(cx.completion_matcher(&patterns, fold, false, Some(state.walk))?);
            }
            let compiled = matcher.as_mut().unwrap();
            let matched = match compiled {
                Some(compiled) => cx.completion_matches(compiled, name)?,
                None => matches_regexps(cx, name)?,
            };
            if !matched { continue; }
        }
        if state.predicate != Value::NIL {
            // `commandp' answers most candidates without Lisp: those go on
            // in this loop, so a walk's depth never grows with its count.
            if state.predicate == commandp {
                match super::callint::command_verdict(cx, candidate, Value::NIL)? {
                    Verdict::Known(false) => continue,
                    Verdict::Known(true) => {
                        if accept(cx, &mut state)? {
                            return finish(cx, state);
                        }
                        continue;
                    }
                    Verdict::Ask(fun) => {
                        let f = cx.intern("interactive-form");
                        return Ok(Step::Call { f, args: vec![fun], then: tested, saved: state.save() });
                    }
                }
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

defsubr! { name = "completion--flex-cost-gotoh", gnu = "minibuf.c:2345", effect = Reads,
fn completion_flex_cost_gotoh(cx, pat: Value, str: Value) {
    let pattern = cx.string_view(pat)?;
    let candidate = cx.string_view(str)?;
    let width = candidate.chars + 1;
    // GNU's statically allocated matrices bound the work.
    if pattern.chars == 0 || candidate.chars == 0 || (pattern.chars + 1) * width > 128 * 512 {
        return Ok(Value::NIL);
    }
    let fold = fold_case(cx)?;
    // A pattern that is not a subsequence cannot match; GNU checks this on
    // the bytes, before the quadratic pass, only when case matters.
    if !fold && !is_subsequence(&pattern.bytes, &candidate.bytes) {
        return Ok(Value::NIL);
    }
    let folded = |cx: &mut Leaf<'_>, chars: Vec<u32>| -> std::result::Result<Vec<u32>, Fault> {
        if fold { chars.into_iter().map(|c| cx.case_mapping_character(c, true)).collect() } else { Ok(chars) }
    };
    let pattern = folded(cx, pattern.chars().collect())?;
    let candidate = folded(cx, candidate.chars().collect())?;
    match gotoh(&pattern, &candidate) {
        None => Ok(Value::NIL),
        Some((cost, positions)) => {
            let mut matches = Value::NIL;
            for &position in positions.iter().rev() {
                matches = cx.cons(Value::integer(position).unwrap(), matches);
            }
            Ok(cx.cons(Value::integer(cost).unwrap(), matches))
        }
    }
}}

fn is_subsequence(needle: &[u8], haystack: &[u8]) -> bool {
    let mut rest = needle.iter().peekable();
    for byte in haystack {
        if rest.peek() == Some(&byte) { rest.next(); }
    }
    rest.peek().is_none()
}

/// minibuf.c's modified Gotoh alignment of PATTERN against CANDIDATE, both
/// already case-folded: the cheapest cost of matching every pattern
/// character in order, opening a gap costing 10 and extending one 1, and
/// the candidate index each pattern character matched; None without a match.
/// Row and column -1 hold the costs before any match, as GNU's MAT macro.
fn gotoh(pattern: &[u32], candidate: &[u32]) -> Option<(i64, Vec<i64>)> {
    const GAP_OPEN: i32 = 10;
    const GAP_EXTEND: i32 = 1;
    const INFINITE: i32 = i32::MAX / 2;
    let width = candidate.len() + 1;
    let at = |i: isize, j: isize| ((i + 1) as usize) * width + (j + 1) as usize;
    let mut m = vec![INFINITE; (pattern.len() + 1) * width];
    let mut d = vec![INFINITE; m.len()];
    // Leading gaps cost half an opening; D[-1,-1] promotes a match at the start.
    d[..width].fill(GAP_OPEN / 2);
    d[0] = 0;
    // The first match of the previous row: no earlier column can match now.
    let mut start = 0;
    for (i, &p) in pattern.iter().enumerate() {
        let i = i as isize;
        let mut seen = false;
        for j in start..candidate.len() {
            let j = j as isize;
            if candidate[j as usize] == p {
                if !seen { seen = true; start = j as usize; }
                m[at(i, j)] = m[at(i - 1, j - 1)].min(d[at(i - 1, j - 1)]);
            }
            d[at(i, j)] = (m[at(i, j - 1)] + GAP_OPEN).min(d[at(i, j - 1)] + GAP_EXTEND);
        }
    }
    let last = pattern.len() as isize - 1;
    // The first cheapest match of the last pattern character.
    let (column, cost) = (0..candidate.len() as isize).map(|j| (j, m[at(last, j)]))
        .fold((-1, INFINITE), |best, (j, cost)| if cost < best.1 { (j, cost) } else { best });
    if column < 0 { return None; }
    // Walk back: each earlier character matched where a match beat a gap.
    let mut positions = vec![column as i64];
    let mut l = column;
    for i in (0..last).rev() {
        loop {
            l -= 1;
            if !(l >= 0 && m[at(i, l)] >= d[at(i, l)]) { break; }
        }
        positions.push(l as i64);
    }
    positions.reverse();
    Some((i64::from(cost), positions))
}
