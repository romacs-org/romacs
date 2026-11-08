// A leaf orchestrating (so collecting) while it holds a heap value: it has &mut E, never an Eng.
use romacs_vm::brand::*;
/// A heap value of the stand-in engine: a word a collection may free or move.
pub struct Word(pub u64);
pub struct Engine { pub stack: Vec<Word>, pub collections: u32 }
struct K;
unsafe impl Key for K {}
/// The stand-in's collecting operation: the key holder's, taking the permission.
fn collect<'b>(m: &mut Mx<'_, 'b, Engine>, _gc: Gc<'_, 'b>) { m.raw(&K).collections += 1 }
fn body<'x, 'b>(_m: Mx<'x, 'b, Engine>, _gc: Gc<'_, 'b>, _: ()) -> u64 { 0 }
fn then(_e: Eng<'_, Engine>, n: u64) -> u64 { n }
fn bad<'b>(mut m: Mx<'_, 'b, Engine>) {
    m.leaf((), |e, ()| { orchestrate(e, (), body, then); });
}
fn main() {}
