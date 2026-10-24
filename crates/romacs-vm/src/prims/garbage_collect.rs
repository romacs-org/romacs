//! Explicit collection is an engine request, never a collecting leaf.
use super::*;

defsubr! { name = "garbage-collect", gnu = "alloc.c:6030", effect = Global,
fn garbage_collect(_cx) -> Step {
    Ok(Step::Collect)
}}
