//! GNU fns.c `random' (`prim/leaf/random.rs').
use super::*;

defsubr! { name = "random", gnu = "fns.c:77", effect = Global,
fn random(cx; optional limit: Value) {
    cx.random(limit)
}}
