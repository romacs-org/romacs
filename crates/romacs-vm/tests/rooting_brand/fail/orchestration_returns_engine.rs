// Orchestration handing the engine back to its raw caller.
use romacs_vm::brand::*;
/// A heap value of the stand-in engine: a word a collection may free or move.
pub struct Word(pub u64);
pub struct Engine { pub stack: Vec<Word>, pub collections: u32 }
struct K;
unsafe impl Key for K {}
/// The stand-in's collecting operation: the key holder's, taking the permission.
fn collect<'b>(m: &mut Mx<'_, 'b, Engine>, _gc: Gc<'_, 'b>) { m.raw(&K).collections += 1 }
fn body<'x, 'b>(_m: Mx<'x, 'b, Engine>, _gc: Gc<'_, 'b>, _: ()) {}
/// THEN hands the engine back: its result must be `'static`.
fn back<'x>(e: Eng<'x, Engine>, (): ()) -> Eng<'x, Engine> { e }
fn bad(e: Eng<'_, Engine>) -> u64 {
    let held = e.stack[0].0;
    let e = orchestrate(e, (), body, back);
    e.stack.len() as u64 + held
}
fn main() {}
