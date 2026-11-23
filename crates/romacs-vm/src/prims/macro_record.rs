//! The macro record (`crate::macro_record`): which compiled functions
//! expanded through a function cell, and what writes to such cells would
//! make stale. Not in GNU.
use crate::prim::{Fault, Leaf};
use crate::Value;
use crate::macro_record::Named;

fn symbol(slot: usize) -> Value {
    Value::reference(slot + 1, 7)
}

fn list(cx: &mut Leaf<'_>, values: &[Value]) -> Value {
    values.iter().rev().fold(Value::NIL, |tail, &value| cx.cons(value, tail))
}

/// N as a fixnum; nil past the fixnums, which no count reaches.
fn integer(n: u64) -> Value {
    i64::try_from(n).ok().and_then(|n| Value::integer(n).ok()).unwrap_or(Value::NIL)
}

// (romacs--macro-capture &rest SYMBOLS): open a capture of the function
// cells a call through SYMBOLS resolves through: each symbol's own and
// every alias link from it to the end of its chain.  They are read now,
// and again whenever a call of one of SYMBOLS autoloads its definition,
// so a capture open across the call holds the cells that ran.  Returns
// the capture's depth, for `romacs--macro-capture-end'.  A non-symbol is
// left out.  `macroexpand-1' and `macroexp--compiler-macro' open one as
// they resolve a head or apply a handler.  Not in GNU.
defsubr! { name = "romacs--macro-capture", gnu = "romacs:macro-record", effect = Global,
fn macro_capture(cx; rest symbols) {
    let depth = cx.open_macro_capture(symbols);
    Ok(integer(depth as u64))
}}

// (romacs--macro-capture-end DEPTH RECORD): close the capture at DEPTH,
// and any left open above it; with RECORD non-nil, the compile now running
// expanded a form through its cells.  Returns nil.  Not in GNU.
defsubr! { name = "romacs--macro-capture-end", gnu = "romacs:macro-record", effect = Global,
fn macro_capture_end(cx, depth: Value, record: Value) {
    let depth = depth.as_integer().ok().and_then(|d| usize::try_from(d).ok()).ok_or_else(|| Fault::Wrong("natnump", depth))?;
    cx.close_macro_capture(depth, record != Value::NIL);
    Ok(Value::NIL)
}}

// Return the symbols whose function cell holds code that expanded a form
// through SYMBOL's function cell: as a macro, as an alias link on the way
// to one, or as a compiler macro's function.  The code includes the
// lambdas a definition makes and the function an advice wrapper holds.
// With TRANSITIVE, return instead those a redefinition of SYMBOL would make
// stale: also the callers of each macro whose expander is stale.  Sorted
// by name.  Not in GNU.
defsubr! { name = "romacs-who-macroexpands", gnu = "romacs:macro-record", effect = Reads,
fn who_macroexpands(cx, name: Value; optional transitive: Value) {
    let callers: Vec<Value> = cx.who_macroexpands(name, transitive != Value::NIL)?.into_iter().map(symbol).collect();
    Ok(list(cx, &callers))
}}

// Return the writes to function cells that some compiled code expanded
// through, and what each would make stale were stale code re-expanded.
// The value is a plist.  `:events' counts those writes.  `:log' is the
// latest 64, oldest first, each (SYMBOL FUNCTIONS ROUNDS NANOSECONDS
// JOINED), SYMBOL being the symbol's name as a string when it was not
// interned in the initial obarray (a collected one's slot may name another
// symbol now): the functions still live whose code expanded the definition
// that write replaced, the scan's rounds, the time it took now (0 where no
// clock runs, as in a world build), and JOINED, the macros whose
// expanders are among them, whose callers it would reach too.  The scan
// runs as this is read.  `:functions', `:scan-ns' and `:max-scan-ns' sum
// and bound the log's.  Not in GNU.
defsubr! { name = "romacs-macro-invalidation-stats", gnu = "romacs:macro-record", effect = Reads,
fn macro_invalidation_stats(cx) {
    let (events, writes) = cx.macro_invalidations();
    let mut log = Vec::with_capacity(writes.len());
    for write in &writes {
        let joined: Vec<Value> = write.joined.iter().copied().map(symbol).collect();
        let joined = list(cx, &joined);
        let named = match &write.symbol {
            Named::Interned(slot) => symbol(*slot),
            Named::Name(name) => cx.string(name),
        };
        let fields = [named, integer(write.functions as u64), integer(write.rounds as u64), integer(write.ns), joined];
        log.push(list(cx, &fields));
    }
    let log = list(cx, &log);
    let functions = writes.iter().map(|w| w.functions as u64).sum();
    let scan_ns = writes.iter().map(|w| w.ns).sum();
    let max_scan_ns = writes.iter().map(|w| w.ns).max().unwrap_or(0);
    let items = [("events", integer(events)), ("functions", integer(functions)),
                 ("scan-ns", integer(scan_ns)), ("max-scan-ns", integer(max_scan_ns)), ("log", log)];
    let mut plist = Vec::with_capacity(items.len() * 2);
    for (key, value) in items {
        plist.push(cx.intern(&format!(":{key}")));
        plist.push(value);
    }
    Ok(list(cx, &plist))
}}
