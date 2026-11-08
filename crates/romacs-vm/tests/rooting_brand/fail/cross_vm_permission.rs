// One VM's permission used by another VM's orchestration.
use romacs_vm::brand::*;
/// A heap value of the stand-in engine: a word a collection may free or move.
pub struct Word(pub u64);
pub struct Engine { pub stack: Vec<Word>, pub collections: u32 }
struct K;
unsafe impl Key for K {}
/// The stand-in's collecting operation: the key holder's, taking the permission.
fn collect<'b>(m: &mut Mx<'_, 'b, Engine>, _gc: Gc<'_, 'b>) { m.raw(&K).collections += 1 }
fn bad<'b1, 'b2>(mut a: Mx<'_, 'b1, Engine>, gb: Gc<'_, 'b2>) {
    collect(&mut a, gb);
}
fn main() {}
