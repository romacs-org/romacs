//! Background compiles of compiled Lisp (`crate::compiled_lisp`), the
//! switch lisp/session/romacs-precompile.el turns, and what the loads of
//! compiled Lisp took. Not in GNU, whose `.elc` files an installation
//! compiles ahead (ledger/divergences VM2-LOADPRECOMPILE).
use super::*;

// (romacs--compiled-lisp-background ON DELAY): with ON, compile in the
// background whenever the session has been idle DELAY seconds; without,
// start no more compiles. Non-nil if this process compiles Lisp at all.
defsubr! { name = "romacs--compiled-lisp-background", gnu = "romacs:compiled-lisp", effect = Global,
fn compiled_lisp_background(cx, on: Value, delay: Value) {
    cx.compiled_lisp_background(on != Value::NIL, delay)
}}

// Return what this process's loads that could run compiled Lisp took.
// The value is a plist.  `:taken' counts the loads that ran a stored
// artifact, `:compiled' those that ran one compiled for the load,
// `:source-on-miss' those that read source with compiled Lisp on in a
// process that compiles, and `:source-off' those that read source in a
// process that compiles nothing: compiled Lisp is off, or the store is
// read-only.  A load is counted once its input is in hand: an artifact
// chosen, or its source read.
//
// `:misses' is the latest 32 loads that read source with compiled Lisp
// on, newest first, each (FILE . REASON).  REASON is `deferred' (the file
// is queued for a background compile), `read-only' (no artifact, and this
// process compiles nothing), `failed' (a compile was tried here and gave
// no artifact that opened), `not-compiled' (its compile refused the file or
// signalled), `chain' (the file is being compiled up this compile's chain)
// or `unreadable' (the file could not be read for its artifact's key).
//
// `:store' is the store and `:identity' the compile identity that keys
// it, each nil while unknown.  Not in GNU.
defsubr! { name = "romacs-compiled-lisp-stats", gnu = "romacs:compiled-lisp", effect = Reads,
fn compiled_lisp_stats(cx) {
    let (stats, store, identity) = crate::compiled_lisp::stats();
    let int = |n: u64| Value::integer(i64::try_from(n).unwrap_or(i64::MAX)).expect("a count");
    let path = |cx: &mut Leaf<'_>, path: &std::path::Path| super::fileio::file_name_value(cx, path.as_os_str().as_encoded_bytes());
    let mut misses = Value::NIL;
    for (file, miss) in &stats.misses {
        let file = path(cx, file);
        let reason = cx.intern(miss.name());
        let pair = cx.cons(file, reason);
        misses = cx.cons(pair, misses);
    }
    let store = store.map_or(Value::NIL, |store| path(cx, store));
    let identity = identity.map_or(Value::NIL, |identity| cx.string(identity));
    let items = [
        ("taken", int(stats.taken)), ("compiled", int(stats.compiled)),
        ("source-on-miss", int(stats.source_on_miss)), ("source-off", int(stats.source_off)),
        ("misses", misses), ("store", store), ("identity", identity),
    ];
    let mut plist = Value::NIL;
    for (key, value) in items.into_iter().rev() {
        let key = cx.intern(&format!(":{key}"));
        plist = cx.cons(value, plist);
        plist = cx.cons(key, plist);
    }
    Ok(plist)
}}
