// Orchestration handing the engine back to its raw caller.
use romacs_vm::brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn body<'x, 'b>(_m: Mx<'x, 'b, Engine>, _gc: Gc<'_, 'b>, _: ()) {}
/// THEN hands the engine back: its result must be `'static`.
fn back<'x>(e: Eng<'x, Engine>, (): ()) -> Eng<'x, Engine> { e }
fn bad(e: Eng<'_, Engine>) -> u64 {
    let held = e.stack[0].0;
    let e = orchestrate(e, (), body, back);
    e.stack.len() as u64 + held
}
fn main() {}
