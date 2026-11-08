// A raw handler orchestrating through a closure called in non-tail position, then using its engine.
use romacs_vm::brand::*;
/// A heap value of the stand-in engine: a word a collection may free or move.
pub struct Word(pub u64);
pub struct Engine { pub stack: Vec<Word>, pub collections: u32 }
struct K;
unsafe impl Key for K {}
/// The stand-in's collecting operation: the key holder's, taking the permission.
fn collect<'b>(m: &mut Mx<'_, 'b, Engine>, _gc: Gc<'_, 'b>) { m.raw(&K).collections += 1 }
fn collects<'x, 'b>(mut m: Mx<'x, 'b, Engine>, mut gc: Gc<'_, 'b>, _: ()) -> u64 { collect(&mut m, gc.by_ref()); 0 }
fn then(_e: Eng<'_, Engine>, n: u64) -> u64 { n }
fn bad(e: Eng<'_, Engine>) -> u64 {
    let held = e.stack[0].0;
    let _out = (|| orchestrate(e, (), collects, then))();
    e.stack.len() as u64 + held
}
fn main() {}
