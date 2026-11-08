// A heap value returned from a leaf into orchestration.
use romacs_vm::brand::*;
/// A heap value of the stand-in engine: a word a collection may free or move.
pub struct Word(pub u64);
pub struct Engine { pub stack: Vec<Word>, pub collections: u32 }
struct K;
unsafe impl Key for K {}
/// The stand-in's collecting operation: the key holder's, taking the permission.
fn collect<'b>(m: &mut Mx<'_, 'b, Engine>, _gc: Gc<'_, 'b>) { m.raw(&K).collections += 1 }
fn bad<'b>(mut m: Mx<'_, 'b, Engine>, mut gc: Gc<'_, 'b>) -> u64 {
    let w = m.leaf((), |e, ()| Word(e.stack[0].0));
    collect(&mut m, gc.by_ref());
    w.0
}
fn main() {}
