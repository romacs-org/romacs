//! What the rooting types (`romacs_brand`) leave to source checks, over the
//! converted modules (`CONVERTED`):
//!
//! - A dispatch moves the raw handler's engine (`Eng`), so nothing after it
//!   reaches the engine; a store through the raw register pointer, or a stale
//!   word returned, still could. So the handle has one name, `e`, and the bare
//!   `e` appears only as an argument of a call in tail position, outside
//!   closures: a handoff in any other shape (an alias, a rebinding, a capture,
//!   a call through a local) is rejected, not followed.
//! - A leaf could hand a Value to orchestration through global mutable state:
//!   no static or thread-local in the crate holds one, and converted modules
//!   declare no global mutable state and leak nothing.
//! - Minting a permission: one `Key` in the crate, and the `unchecked` mints
//!   and deferred runs in converted modules are counted, so they only go down.
#[path = "support/rust_tokens.rs"]
mod rust_tokens;
use rust_tokens::{tokens, Token};
use std::path::Path;

/// Modules converted to rooting as a type (`romacs_brand`): these checks cover
/// what the types leave.
const CONVERTED: &[&str] = &["autoload.rs", "command.rs", "prims/read.rs", "reader/mod.rs", "reader/parse.rs", "threaded.rs", "threaded_signal.rs", "threaded_lend.rs", "traversal.rs"];

fn source(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)).unwrap()
}

/// The rooting types, a crate of their own: held to these rules with the modules
/// that use them, and to no `Key`, since a key is its user's.
const BRAND: &str = "../../romacs-brand/src/lib.rs";

/// SOURCE's tokens with `#[cfg(test)]` items removed.
fn production(source: &str) -> Vec<Token> {
    use Token::{Ident, Punct};
    let toks = tokens(source);
    let is = |k: usize, t: &Token| toks.get(k) == Some(t);
    let mut keep = vec![true; toks.len()];
    let mut i = 0;
    while i < toks.len() {
        if is(i, &Punct('#')) && is(i + 1, &Punct('[')) && is(i + 2, &Ident("cfg".into())) && is(i + 4, &Ident("test".into())) && is(i + 6, &Punct(']')) {
            let mut j = i + 7;
            let mut depth = 0usize;
            while j < toks.len() {
                match &toks[j] {
                    Punct('{') => depth += 1,
                    Punct('}') if depth == 1 => break,
                    Punct('}') => depth -= 1,
                    Punct(';') if depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            keep[i..=j.min(toks.len() - 1)].fill(false);
            i = j + 1;
            continue;
        }
        i += 1;
    }
    toks.into_iter().zip(keep).filter_map(|(t, k)| k.then_some(t)).collect()
}

/// The matching close of each bracket opened at an index.
fn matches(toks: &[Token]) -> Vec<Option<usize>> {
    let mut close = vec![None; toks.len()];
    let mut open = vec![];
    for (i, t) in toks.iter().enumerate() {
        match t {
            Token::Punct('(' | '[' | '{') => open.push(i),
            Token::Punct(')' | ']' | '}') => {
                if let Some(o) = open.pop() {
                    close[o] = Some(i);
                }
            }
            _ => {}
        }
    }
    close
}

/// The innermost brace around index AT that encloses it.
fn enclosing_brace(toks: &[Token], close: &[Option<usize>], at: usize) -> Option<usize> {
    (0..at).rev().find(|&k| toks[k] == Token::Punct('{') && close[k].is_some_and(|c| c > at))
}

/// Whether the expression ending at the token before AFTER is in tail
/// position: only closes of tail blocks, `else` branches and match arms
/// follow it to the end of its function's body (BODY_END).
fn tail_after(toks: &[Token], close: &[Option<usize>], mut after: usize, body_end: usize) -> bool {
    use Token::{Ident, Punct};
    let arm = |o: usize| o >= 2 && toks[o - 1] == Punct('>') && toks[o - 2] == Punct('=');
    loop {
        if after == body_end {
            return true;
        }
        match &toks[after] {
            Punct('}') => {
                let block = (0..after).rev().find(|&k| close[k] == Some(after)).unwrap();
                if arm(block) {
                    // A match arm's block: the match's tail decides.
                    let Some(m) = enclosing_brace(toks, close, block) else { return false };
                    after = close[m].unwrap();
                    continue;
                }
                after += 1;
                // An if's then-branch: its else branches follow.
                while toks.get(after) == Some(&Ident("else".into())) {
                    let Some(brace) = (after..toks.len()).find(|&k| toks[k] == Punct('{')) else { return false };
                    after = close[brace].unwrap() + 1;
                }
            }
            // A match arm's expression: the match's tail decides.
            Punct(',') => {
                let Some(o) = (0..after).rev().find(|&k| matches!(toks[k], Punct('(' | '[' | '{')) && close[k].is_some_and(|c| c > after)) else { return false };
                if toks[o] != Punct('{') {
                    return false;
                }
                after = close[o].unwrap();
            }
            _ => return false,
        }
    }
}

/// The driver mints the handle for each handler and continues after an
/// orchestration (`route_escape`): the one named exception.
const DRIVER: &str = "execute_args_inner";

/// Closures in TOKS: (the first `|` of its parameters, the last token of its
/// body, whether its parameters bind `e`).
fn closures(toks: &[Token], close: &[Option<usize>]) -> Vec<(usize, usize, bool)> {
    use Token::{Ident, Punct};
    let mut out = vec![];
    for i in 1..toks.len() {
        // A closure's `|' follows an expression start, never an operand.
        let starts = matches!(&toks[i - 1], Punct('(' | ',' | '=' | '{' | ';' | '>')) || toks[i - 1] == Ident("move".into());
        if toks[i] != Punct('|') || !starts {
            continue;
        }
        let Some(head_end) = (i + 1..toks.len()).find(|&k| toks[k] == Punct('|')) else { continue };
        let binds_e = toks[i + 1..head_end].contains(&Ident("e".into()));
        // Past a return type to the body: a block, or one expression.
        let mut k = head_end + 1;
        while k < toks.len() && toks[k] != Punct('{') && !matches!(toks[k], Punct(',' | ')' | ';')) && !(toks[k] == Punct('-') && toks.get(k + 1) == Some(&Punct('>'))) {
            k += 1;
        }
        if toks.get(k) == Some(&Punct('-')) {
            k = (k..toks.len()).find(|&j| toks[j] == Punct('{')).unwrap_or(k);
        }
        let end = if toks.get(k) == Some(&Punct('{')) && k == head_end + 1 || toks.get(k) == Some(&Punct('{')) && toks[head_end + 1] == Punct('-') {
            close[k].unwrap_or(k)
        } else {
            // An expression body: to the `,', `)' or `;' that ends it.
            let mut depth = 0i32;
            let mut j = head_end + 1;
            while j < toks.len() {
                match toks[j] {
                    Punct('(' | '[' | '{') => depth += 1,
                    Punct(')' | ']' | '}') if depth == 0 => break,
                    Punct(')' | ']' | '}') => depth -= 1,
                    Punct(',' | ';') if depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            j.saturating_sub(1)
        };
        out.push((i, end, binds_e));
    }
    out
}

/// Uses of the raw handle in FILES other than as an argument of a call in
/// tail position, outside closures; and handles named other than `e`. With
/// the number of tail handoffs seen.
fn non_tail_handoffs(files: &[(String, Vec<Token>)]) -> (Vec<String>, usize) {
    use Token::{Ident, Punct};
    let mut problems = vec![];
    let mut handoffs = 0;
    for (file, toks) in files {
        let close = matches(toks);
        let closures = closures(toks, &close);
        for i in 0..toks.len() {
            if toks[i] != Ident("fn".into()) {
                continue;
            }
            // A function's name, or a macro's `$name`: its handlers too.
            let named = if toks.get(i + 1) == Some(&Punct('$')) { i + 2 } else { i + 1 };
            let Some(Ident(function)) = toks.get(named) else { continue };
            let Some(open_params) = (i + 2..toks.len()).find(|&k| toks[k] == Punct('(')) else { continue };
            let close_params = close[open_params].unwrap();
            // The handle's parameter, and its name.
            let mut takes = false;
            for k in open_params..close_params {
                if toks[k + 1] == Punct(':') && toks.get(k + 2) == Some(&Ident("Eng".into())) && toks.get(k + 3) == Some(&Punct('<')) {
                    takes = true;
                    // `_` drops it: the function hands nothing on.
                    if toks[k] != Ident("e".into()) && toks[k] != Ident("_".into()) {
                        problems.push(format!("{file}: {function} names its handle other than `e`"));
                    }
                }
            }
            let Some(open) = (close_params..toks.len()).find(|&k| matches!(toks[k], Punct('{' | ';'))) else { continue };
            if !takes || toks[open] != Punct('{') || function == DRIVER {
                continue;
            }
            let body_end = close[open].unwrap();
            for k in open + 1..body_end {
                if toks[k] != Ident("e".into()) {
                    continue;
                }
                let (prev, next) = (&toks[k - 1], toks.get(k + 1));
                // Not the bare handle: its field or method, another's field
                // named `e`, a borrow, a deref, a binding's own name, or
                // `let [mut] e = e`.
                let used_through = matches!(next, Some(Punct('.' | ':')))
                    || *prev == Punct('.') || *prev == Punct('&') || *prev == Punct('*')
                    || (*prev == Ident("mut".into()) && k >= 2 && toks[k - 2] == Punct('&'));
                let rebinds_itself = *prev == Punct('=') && k >= 3 && toks[k - 2] == Ident("e".into())
                    && (toks[k - 3] == Ident("let".into()) || (toks[k - 3] == Ident("mut".into()) && toks.get(k - 4) == Some(&Ident("let".into()))));
                let binding = (*prev == Ident("let".into()) || (*prev == Ident("mut".into()) && toks[k - 2] == Ident("let".into())))
                    && next == Some(&Punct('=')) && toks.get(k + 2) == Some(&Ident("e".into()));
                if used_through || rebinds_itself || binding {
                    continue;
                }
                // Inside a closure that binds its own `e` (a leaf's), it is
                // that closure's; in any other closure, a capture.
                let around: Vec<&(usize, usize, bool)> = closures.iter().filter(|&&(head, end, _)| head < k && k <= end).collect();
                if around.iter().any(|&&(_, _, own)| own) {
                    continue;
                }
                let captured = !around.is_empty();
                // An argument of a call: `CALLEE ( .. e .. )`, the call in
                // tail position.
                let in_call = matches!(prev, Punct('(' | ',')) && matches!(next, Some(Punct(',' | ')')));
                let call_open = (open..k).rev().find(|&j| toks[j] == Punct('(') && close[j].is_some_and(|c| c > k));
                let at_tail = in_call && call_open.is_some_and(|call_open| {
                    let mut start = call_open;
                    // The callee: a path, a turbofish, or a parenthesized pointer.
                    if start > 0 && toks[start - 1] == Punct(')') {
                        start = (0..start - 1).rev().find(|&j| close[j] == Some(start - 1)).unwrap_or(start);
                    } else if start > 0 && toks[start - 1] == Punct('>') {
                        let mut depth = 0;
                        let mut j = start - 1;
                        while j > 0 {
                            match toks[j] { Punct('>') => depth += 1, Punct('<') => { depth -= 1; if depth == 0 { break; } } _ => {} }
                            j -= 1;
                        }
                        start = j.saturating_sub(2);
                    }
                    // A path of names, a macro's `$name` among them.
                    while start > 0 && matches!(&toks[start - 1], Ident(_)) && !matches!(&toks[start - 1], Ident(w) if matches!(w.as_str(), "return" | "let" | "mut" | "else" | "unsafe")) {
                        start -= 1;
                        if start > 0 && toks[start - 1] == Punct('$') {
                            start -= 1;
                        }
                        while start >= 2 && toks[start - 1] == Punct(':') && toks[start - 2] == Punct(':') {
                            start -= 2;
                        }
                    }
                    let mut end = close[call_open].unwrap();
                    // Out through `unsafe { CALL }`: the block is the expression.
                    while start >= 2 && toks[start - 1] == Punct('{') && close[start - 1] == Some(end + 1) && toks[start - 2] == Ident("unsafe".into()) {
                        start -= 2;
                        end += 1;
                    }
                    // Past an attribute on the expression (`#[cfg(..)] unsafe { .. }`).
                    while toks[start - 1] == Punct(']') {
                        let attr = (0..start).rev().find(|&j| close[j] == Some(start - 1)).unwrap();
                        if attr == 0 || toks[attr - 1] != Punct('#') {
                            break;
                        }
                        start = attr - 1;
                    }
                    // A `return`'s value is the call itself: nothing after it.
                    let explicit = toks[start - 1] == Ident("return".into()) && matches!(toks.get(end + 1), Some(Punct(';' | '}' | ',')));
                    let block_tail = matches!(toks[start - 1], Punct('{' | ';' | '}')) || (toks[start - 1] == Punct('>') && toks[start - 2] == Punct('='));
                    explicit || (block_tail && tail_after(toks, &close, end + 1, body_end))
                });
                if captured || !at_tail {
                    problems.push(format!("{file}: {function} hands its engine on in a non-tail call"));
                } else {
                    handoffs += 1;
                }
            }
        }
    }
    (problems, handoffs)
}

#[test]
fn converted_modules_hand_their_engine_on_only_in_tail_position() {
    let files: Vec<(String, Vec<Token>)> = CONVERTED.iter().map(|f| (f.to_string(), production(&source(f)))).collect();
    let (problems, handoffs) = non_tail_handoffs(&files);
    assert!(handoffs > 100, "found the engine's handoffs: {handoffs}");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The check finds each way of using the handle's turn after a dispatch,
/// and passes the tail shapes the engine writes.
#[test]
fn the_tail_check_tells_handoffs_apart() {
    let source = r#"
        unsafe fn next(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value { ((*pc).handler)(e, r, pc) }
        unsafe fn tail(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            if c { return next(e, r, pc); }
            let x = 1;
            unsafe { if x == 1 { next(e, r, pc) } else { Value::NIL } }
        }
        unsafe fn arms(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            match k {
                A => next(e, r, pc),
                B => { stuff(); next(e, r, pc) }
                _ => return unsafe { next(e, r, pc) },
            }
        }
        unsafe fn bound(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            let v = next(e, r, pc);
            *r = held;
            v
        }
        unsafe fn statement(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            next(e, r, pc);
            Value::NIL
        }
        unsafe fn argument(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            wrap(next(e, r, pc))
        }
        unsafe fn wrapped(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            let out = (|| { return orchestrate(e, (), body) })();
            *r = held;
            out
        }
        unsafe fn bare_closure(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            let f = || next(e, r, pc);
            f()
        }
        fn rebound(e: Eng<'_, '_>) -> Value {
            let held = e.reg(0);
            let other = e;
            orchestrate(other, (), stress_collect, done);
            held
        }
        unsafe fn returned_method(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            return next(e, r, pc).max(held);
        }
        unsafe fn turbofish(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            let v = not_fixnums::<{ K }>(e, r, pc);
            v
        }
        unsafe fn not_fixnums<const K: u8>(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value { next(e, r, pc) }
        fn iterates(e: Eng<'_, '_>) -> Value { let x = list.iter().next(); next(e, x, y) }
        fn shadowed(e: Eng<'_, '_>, call: fn(u8) -> u8) -> Value { let bind = |x| x; let y = bind(call(1)); next(e, y, z) }
        unsafe fn aliased(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            let held = *r;
            let f = next;
            f(e, r, pc);
            held
        }
        unsafe fn pointer_local(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            let h = (*pc).handler;
            let v = h(e, r, pc);
            v
        }
        unsafe fn nested_shadow(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            { let next = || (); next(); }
            let v = next(e, r, pc);
            v
        }
        unsafe fn other_field(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            let v = ((*pc).step)(e, r, pc);
            v
        }
        unsafe fn renamed(h: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value { next(h, r, pc) }
        unsafe fn leaf_owns_e(mut m: Orch<'_, '_, '_>, e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
            let n = m.leaf((), |e, ()| fail(e, "x"));
            let mut e = e;
            e.budget = n;
            unsafe { if n == 0 { next(e, r, pc) } else { advance(e, r, pc) } }
        }
        macro_rules! predicate {
            ($name:ident) => {
                unsafe fn $name(e: Eng<'_, '_>, r: *mut Value, pc: *const Inst) -> Value {
                    let kept = *r;
                    next(e, r, pc);
                    *r = kept;
                    kept
                }
            };
        }
    "#;
    let (problems, _) = non_tail_handoffs(&[("seeded.rs".into(), tokens(source))]);
    let flagged: Vec<&str> = problems.iter().map(|p| p.split(": ").nth(1).unwrap().split(' ').next().unwrap()).collect();
    assert_eq!(flagged, ["bound", "statement", "argument", "wrapped", "bare_closure", "rebound", "returned_method", "turbofish",
                         "aliased", "pointer_local", "nested_shadow", "other_field", "renamed", "name"], "{problems:?}");
}

/// An orchestration body inlined into its dispatcher brings its borrowed
/// locals with it, and one escaping address stops every sibling call there:
/// each body (a function taking an `Orch` by value) stays out of line.
#[test]
fn orchestration_bodies_keep_their_own_frames() {
    let mut problems = vec![];
    for file in CONVERTED {
        let text = source(file);
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            let Some(at) = line.find("fn ") else { continue };
            let signature: String = lines[n..(n + 3).min(lines.len())].concat();
            let Some(params) = signature.split_once('(').map(|(_, p)| p) else { continue };
            let first = params.split(',').next().unwrap_or("");
            let by_value = first.contains(": Orch<") && !first.contains('&');
            if !by_value || line[..at].contains("//") {
                continue;
            }
            let attributes = lines[..n].iter().rev().take_while(|l| l.trim_start().starts_with("#[") || l.trim_start().starts_with("///"));
            if !attributes.into_iter().any(|l| l.contains("#[inline(never)]")) {
                problems.push(format!("{file}:{}: an orchestration body without #[inline(never)]", n + 1));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Statics (thread-locals' included) whose declared type names `Value`: each
/// `static [mut] NAME: TYPE` read as a whole, through its `=` or `;`.
fn statics_holding_value(source: &str) -> Vec<String> {
    use Token::{Ident, Punct};
    let toks = tokens(source);
    let mut found = vec![];
    for i in 0..toks.len() {
        if toks[i] != Ident("static".into()) {
            continue;
        }
        let mut k = i + 1;
        if toks.get(k) == Some(&Ident("mut".into())) {
            k += 1;
        }
        // A declaration, not a `'static' lifetime (`&'static a::B`): NAME
        // then one `:`.
        let (Some(Ident(name)), Some(Punct(':'))) = (toks.get(k), toks.get(k + 1)) else { continue };
        if toks.get(k + 2) == Some(&Punct(':')) {
            continue;
        }
        // Its type: to the declaration's own `=` or `;`, past bracket groups
        // and generic arguments (`Iterator<Item = Value>`); `->` closes none.
        let mut depth = 0i32;
        let mut end = k + 2;
        while end < toks.len() {
            match toks[end] {
                Punct('(' | '[' | '{' | '<') => depth += 1,
                Punct('>') if toks[end - 1] == Punct('-') => {}
                Punct(')' | ']' | '}' | '>') => depth -= 1,
                Punct('=' | ';') if depth == 0 => break,
                _ => {}
            }
            if depth < 0 {
                break;
            }
            end += 1;
        }
        if toks[k + 2..end].contains(&Ident("Value".into())) {
            found.push(name.clone());
        }
    }
    found
}

/// `Key for`, adjacent: every implementation of the key, however its header
/// is written (a path, generics), and nothing else names it so.
fn key_impls(source: &str) -> usize {
    tokens(source).windows(2).filter(|w| w[0] == Token::Ident("Key".into()) && w[1] == Token::Ident("for".into())).count()
}

/// The source checks below find what they look for, written any way.
#[test]
fn the_state_and_key_checks_find_their_targets() {
    let statics = r#"
        static SAVED: std::sync::Mutex<
            Option<Value>
        > = std::sync::Mutex::new(None);
        thread_local! {
            static KEPT: std::cell::Cell<Option<Value>> = const { std::cell::Cell::new(None) };
        }
        static mut RAW: [Value; 2] = [Value::NIL; 2];
        static NAMES: &[&str] = &["Value"];
        static PAIRED: ([u8; 1], std::sync::Mutex<Option<Value>>) = ([0], std::sync::Mutex::new(None));
        fn f() -> &'static str { "x" }
        fn pool() -> &'static rayon::Pool { static POOL: OnceLock<Value> = OnceLock::new(); x }
        static WALK: Mutex<Option<Box<dyn Iterator<Item = Value> + Send>>> = Mutex::new(None);
        static HOOK: Option<fn(u8) -> Value> = None;
    "#;
    assert_eq!(statics_holding_value(statics), ["SAVED", "KEPT", "RAW", "PAIRED", "POOL", "WALK", "HOOK"]);
    let keys = r#"
        unsafe impl Key for EngineKey {}
        unsafe impl crate::brand::Key for AnotherKey {}
        unsafe impl brand::Key for Third {}
        unsafe impl<T> Key for Generic<T> {}
        // unsafe impl Key for Commented {}
        fn key_for() {}
    "#;
    assert_eq!(key_impls(keys), 4);
}

#[test]
fn no_global_state_holds_a_value() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut problems = vec![];
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let relative = path.strip_prefix(&src).unwrap().to_string_lossy().into_owned();
            for name in statics_holding_value(&text) {
                problems.push(format!("{relative}: the static {name} holds a Value"));
            }
            if CONVERTED.contains(&relative.as_str()) {
                problems.extend(global_state(&relative, &text));
            }
        }
    }
    problems.extend(global_state("romacs-brand", &source(BRAND)));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn global_state(file: &str, text: &str) -> Vec<String> {
    let toks = production(text);
    GLOBAL_STATE.iter().filter(|(_, pattern)| count(&toks, pattern) > 0)
        .map(|(what, _)| format!("{file}: {what} in a converted module")).collect()
}

/// Global or shared mutable state, as token patterns.
const GLOBAL_STATE: &[(&str, &[&str])] = &[
    ("a thread-local", &["thread_local", "!"]),
    ("a mutable static", &["static", "mut"]),
    ("a leaked allocation", &["Box", ":", ":", "leak"]),
    ("a once cell", &["OnceLock", "<"]),
    ("a mutex", &["Mutex", "<"]),
    ("a ref cell", &["RefCell", "<"]),
    ("a cell", &["Cell", "<"]),
];

/// Occurrences of PATTERN (identifiers and punctuation) in TOKS.
fn count(toks: &[Token], pattern: &[&str]) -> usize {
    let at = |t: &Token, p: &str| match t {
        Token::Ident(name) => name == p,
        Token::Punct(c) => p.len() == 1 && p.starts_with(*c),
    };
    toks.windows(pattern.len()).filter(|w| w.iter().zip(pattern).all(|(t, p)| at(t, p))).count()
}

/// Mints and deferrals, counted in every converted module: each count only
/// goes down, and a module not listed has none. The census rows they answer
/// to are in the design note (§4.14).
const MINT_PATTERNS: &[&[&str]] = &[&["Gc", ":", ":", "unchecked"], &["threaded", ":", ":", "toplevel"], &["lend", ":", ":", "deferred"], &[".", "defer", "("]];
const MINTS: &[(&str, &[&str], usize)] = &[
    // The engine's entry for callers not yet converted (S1 gives them `Gc`).
    ("threaded.rs", &["Gc", ":", ":", "unchecked"], 1),
    // A lazy body's expansion runs under its builder's hold (`lend::deferred`).
    ("threaded.rs", &["lend", ":", ":", "deferred"], 1),
    // The embedder's command: its Rust caller holds no Value (`Toplevel` closes this).
    ("command.rs", &["threaded", ":", ":", "toplevel"], 1),
    // `autoload_do_load` for callers not yet converted: `macroexpand_steps` and
    // `expand_macro`, each keeping its rows.
    ("autoload.rs", &["threaded", ":", ":", "toplevel"], 1),
    // `read_source_raw` for callers not yet converted (eval_source, the
    // embedder's prepared lambdas and reads), each keeping its row.
    ("reader/mod.rs", &["threaded", ":", ":", "toplevel"], 1),
];

#[test]
fn permissions_come_from_one_key_and_counted_mints() {
    let mut keys = 0;
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                keys += key_impls(&std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    assert_eq!(keys, 1, "one key (`src/threaded_lend.rs`)");
    let brand = source(BRAND);
    assert!(brand.contains("pub unsafe trait Key"), "the rules read the rooting types");
    assert_eq!(key_impls(&brand), 0, "a key is its user's, not the rooting types'");
    let toks = production(&brand);
    for &pattern in MINT_PATTERNS {
        assert_eq!(count(&toks, pattern), 0, "romacs-brand mints `{}`", pattern.concat());
    }
    for &file in CONVERTED {
        let toks = production(&source(file));
        for &pattern in MINT_PATTERNS {
            let ratchet = MINTS.iter().find(|&&(f, p, _)| f == file && p == pattern).map_or(0, |&(_, _, n)| n);
            let found = count(&toks, pattern);
            let what = pattern.concat();
            assert!(found <= ratchet, "{file}: {found} of `{what}`, the ratchet says {ratchet}: convert the caller instead");
            assert_eq!(found, ratchet, "{file}: `{what}` went down to {found}; lower its count in MINTS");
        }
    }
}
