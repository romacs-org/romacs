//! Frozen values (vm-concurrency design §3.3, §4.2): `freeze` copies a value
//! graph into an immutable region, `thaw` copies one back into the private
//! heap, `frozen-p` tells the two apart. Not in GNU (ledger/divergences
//! VM2-FROZEN).
use super::*;

// Freeze OBJECT: an immutable copy of its whole value graph in a new frozen region,
// or OBJECT itself when it is a region's root already, a fixnum, nil or t.  A symbol is not
// copied: one in the initial obarray is the same symbol, its value, function and plist as
// mutable as ever, and any other a new uninterned symbol of its name.  A graph holding a
// value that cannot be frozen signals (error "Cannot freeze" VALUE PATH).  Writing into a
// frozen value signals an error; `thaw' gives a mutable copy back.
defsubr! { name = "freeze", gnu = "romacs:frozen", effect = Global,
fn freeze(cx, object: Value) {
    cx.freeze(object)
}}

// Whether OBJECT is frozen: in a region `freeze' made, or one a frozen value taken
// from another isolate was placed in.
defsubr! { name = "frozen-p", gnu = "romacs:frozen", effect = Pure,
fn frozen_p(cx, object: Value) {
    Ok(if cx.frozen_p(object) { Value::TRUE } else { Value::NIL })
}}

// A private, mutable copy of the frozen value graph OBJECT; a value that is not frozen
// is returned as it is.
defsubr! { name = "thaw", gnu = "romacs:frozen", effect = Global,
fn thaw(cx, object: Value) {
    cx.thaw(object)
}}

// The attached regions, for tests and introspection: a list of
// (ID CONSES OBJECTS BYTES) per live region, by id.
defsubr! { name = "romacs--frozen-regions", gnu = "romacs:frozen", effect = Reads,
fn frozen_regions(cx) {
    Ok(cx.frozen_regions())
}}

// Build a seed (design O7m §2.6): the entries ROOTS name, each one of (variable SYM [FILE]),
// (function SYM [FILE]), (plist SYM) and (after-load KEY FUNCS), with a function and a plist
// entry for each symbol they mention whose definition does not cross by name, encoded once as
// one code graph within LIMIT nodes of work, refused work included.  TABLE is the zygote's, a
// plist whose :incarnation the seed names.  Returns (BACKING . REFUSALS): BACKING is sent as it
// is, and arrives as private, mutable copies; each refusal is (KIND NAME REASON . DETAIL), its
// entry left out.
defsubr! { name = "romacs--seed-build", gnu = "romacs:frozen", effect = Global,
fn seed_build(cx, roots: Value, table: Value, limit: Value) {
    cx.seed_build(roots, table, limit)
}}

// The request-owned functions alive and the cache slots they own, as (FUNCTIONS CACHES): what
// placed code (a seed's) adds while held and gives back once a collection finds it dropped.  For
// tests and introspection.
defsubr! { name = "romacs--request-code-counts", gnu = "romacs:frozen", effect = Reads,
fn request_code_counts(cx) {
    Ok(cx.request_code_counts())
}}
