//! Compile-fail fixture for the T1 GC boundary (PRIMITIVE-DOCTRINE section 3).
//!
//! `src/prims/mod.rs` mounts this file as a sibling of the real primitive
//! domain modules only under `--cfg vm2_gc_probe="..."`, set by
//! `tests/primitive_gc_boundary.rs`. Each body below is a leaf primitive that
//! tries to reach a collection point; rustc must refuse every one.
use super::*;

// A leaf cannot name the running VM: `Leaf`'s only field is private to the
// `prim::leaf` module, so `collect`, `execute` and `load` are unreachable.
#[cfg(vm2_gc_probe = "leaf_field")]
defsubr! { name = "vm2-gc-probe-leaf-field", gnu = "none", effect = Pure,
fn vm2_gc_probe_leaf_field(cx, list: Value) {
    cx.vm.collect(&mut []);
    Ok(list)
}}

// Nor can it forge one: `#![forbid(unsafe_code)]` covers all of `src/prims`.
#[cfg(vm2_gc_probe = "unsafe_body")]
defsubr! { name = "vm2-gc-probe-unsafe-body", gnu = "none", effect = Pure,
fn vm2_gc_probe_unsafe_body(cx, list: Value) {
    let p: *const Value = &list;
    Ok(unsafe { *p })
}}
