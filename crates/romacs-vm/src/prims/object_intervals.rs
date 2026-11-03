//! GNU fns.c object-intervals over the VM's property runs.
use super::*;

defsubr! { name = "object-intervals", gnu = "fns.c:6663", effect = Reads,
fn object_intervals(cx, object: Value) { cx.object_intervals(object) }}
