//! GNU composite.c primitives.
use super::*;

// GNU replaces its gstring cache and clears the face cache. VM2 composes no
// characters and keeps neither cache (DIVERGENCES.md VM2-COMPOSITE1).
defsubr! { name = "clear-composition-cache", gnu = "composite.c:692", effect = Global,
fn clear_composition_cache(cx) {
    let _ = cx;
    Ok(Value::NIL)
}}
