//! GNU reader entry point; delegates to the VM reader, not an evaluator shim.
use super::*;

defsubr! { name = "read-from-string", gnu = "lread.c:2511", effect = Global,
fn read_from_string(call, string: Value; optional start: Value, end: Value) -> Reader {
    from_string(call)
}}

defsubr! { name = "read", gnu = "lread.c:2457", effect = Global,
fn read(call; optional stream: Value) -> Reader {
    from_stream(call)
}}

use crate::prim::{Leaf, ReadCall, land};

/// `read-from-string': STRING's characters START to END read under the
/// engine's permission, as (OBJECT . POSITION).
fn from_string(call: ReadCall<'_, '_, '_, '_>) -> bool {
    let ReadCall { m, gc, start, argc, out } = call;
    let Some(spec) = m.leaf((start, argc, out), |vm, (start, argc, out)| {
        let arg = |i: usize| if i < argc { vm.stack[start + i] } else { Value::NIL };
        let (string, from, to) = (arg(0), arg(1), arg(2));
        match Leaf::new(vm).read_range(string, from, to) {
            Ok((a, b)) => {
                vm.external_roots.push(string);
                Some((vm.external_roots.len() - 1, a, b))
            }
            Err(fault) => {
                land(vm, out, Err(fault));
                None
            }
        }
    }) else { return true };
    crate::reader::read_string(m, Some(gc), spec, (out, spec.0, spec.1), |vm, (out, root, a), read| {
        let outcome = Leaf::new(vm).string_read(read, a);
        vm.external_roots.truncate(root);
        land(vm, out, outcome)
    })
}

/// `read': the next object of STREAM (`standard-input''s value for nil), read
/// under the engine's permission.
fn from_stream(call: ReadCall<'_, '_, '_, '_>) -> bool {
    let ReadCall { m, gc, start, argc, out } = call;
    let Some(root) = m.leaf((start, argc, out), |vm, (start, argc, out)| {
        let stream = if argc > 0 { vm.stack[start] } else { Value::NIL };
        match Leaf::new(vm).read_stream(stream) {
            Ok(stream) => {
                vm.external_roots.push(stream);
                Some(vm.external_roots.len() - 1)
            }
            Err(fault) => {
                land(vm, out, Err(fault));
                None
            }
        }
    }) else { return true };
    crate::reader::read_stream(m, gc, root, (out, root), |vm, (out, root), read| {
        let stream = vm.external_roots[root];
        vm.external_roots.truncate(root);
        let outcome = Leaf::new(vm).stream_read(read, stream);
        land(vm, out, outcome)
    })
}
