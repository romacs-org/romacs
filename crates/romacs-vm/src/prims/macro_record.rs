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

// (romacs--macro-cells &rest SYMBOLS): the function cells an expansion
// through SYMBOLS resolves now: each symbol's own and every alias link
// from it to the end of its chain, as a list of (SYMBOL . VERSION).  A
// non-symbol is ignored.  `macroexpand-1' and `macroexp--compiler-macro'
// read them as they resolve a head or handler, before it runs, and hand
// them to `romacs--record-macro-cells' once it expanded.  Not in GNU.
defsubr! { name = "romacs--macro-cells", gnu = "romacs:macro-record", effect = Reads,
fn macro_cells(cx; rest symbols) {
    let cells = cx.macro_cells(symbols);
    let mut list = Value::NIL;
    for &(slot, version) in cells.iter().rev() {
        let cell = cx.cons(symbol(slot), integer(version));
        list = cx.cons(cell, list);
    }
    Ok(list)
}}

// (romacs--record-macro-cells CELLS): the compile now running expanded a
// form through each of CELLS, a list `romacs--macro-cells' made, at the
// versions it holds.  Returns nil.  Not in GNU.
defsubr! { name = "romacs--record-macro-cells", gnu = "romacs:macro-record", effect = Global,
fn record_macro_cells(cx, cells: Value) {
    let mut uses = Vec::new();
    for cell in super::widen::list_elements(cx, cells)? {
        let (name, version) = cx.pair(cell).ok_or_else(|| Fault::Wrong("consp", cell))?;
        let version = version.as_integer().ok().and_then(|v| u64::try_from(v).ok()).ok_or_else(|| Fault::Wrong("natnump", version))?;
        uses.push((cx.macro_cell_slot(name)?, version));
    }
    cx.record_macro_cells(&uses);
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
