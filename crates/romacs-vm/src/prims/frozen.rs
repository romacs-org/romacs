//! Frozen values (vm-concurrency design §3.3, §4.2): `freeze` copies a value
//! graph into an immutable region, `thaw` copies one back into the private
//! heap, `frozen-p` tells the two apart. Not in GNU (DIVERGENCES.md
//! VM2-FROZEN).
use super::*;

defsubr! { name = "freeze", gnu = "romacs:frozen", effect = Global,
fn freeze(cx, object: Value) {
    cx.freeze(object)
}}

defsubr! { name = "frozen-p", gnu = "romacs:frozen", effect = Pure,
fn frozen_p(cx, object: Value) {
    Ok(if cx.frozen_p(object) { Value::TRUE } else { Value::NIL })
}}

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
