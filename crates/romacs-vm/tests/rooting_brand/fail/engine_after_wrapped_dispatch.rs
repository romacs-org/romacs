// A raw handler orchestrating through a closure called in non-tail position, then using its engine.
use romacs_vm::brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn collects<'x, 'b>(mut m: Mx<'x, 'b, Engine>, mut gc: Gc<'_, 'b>, _: ()) -> u64 { collect(&mut m, gc.by_ref()); 0 }
fn then(_e: Eng<'_, Engine>, n: u64) -> u64 { n }
fn bad(e: Eng<'_, Engine>) -> u64 {
    let held = e.stack[0].0;
    let _out = (|| orchestrate(e, (), collects, then))();
    e.stack.len() as u64 + held
}
fn main() {}
