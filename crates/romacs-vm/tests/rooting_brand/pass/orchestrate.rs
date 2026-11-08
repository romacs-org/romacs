// The positive control: a value rooted in traced storage across a collection, read back by a later leaf.
use romacs_vm::brand::*;
/// A heap value of the stand-in engine: a word a collection may free or move.
pub struct Word(pub u64);
pub struct Engine { pub stack: Vec<Word>, pub collections: u32 }
struct K;
unsafe impl Key for K {}
/// The stand-in's collecting operation: the key holder's, taking the permission.
fn collect<'b>(m: &mut Mx<'_, 'b, Engine>, _gc: Gc<'_, 'b>) { m.raw(&K).collections += 1 }
fn body<'x, 'b>(mut m: Mx<'x, 'b, Engine>, mut gc: Gc<'_, 'b>, at: usize) -> u64 {
    // Root: the value goes to traced storage (the stand-in's stack), its index is plain.
    let rooted = m.leaf(at, |e, at| { let w = Word(e.stack[at].0 + 1); e.stack.push(w); e.stack.len() - 1 });
    collect(&mut m, gc.by_ref());
    m.leaf(rooted, |e, i| e.stack[i].0)
}
/// Where raw mode goes next: here, the engine's count added in.
fn then(e: Eng<'_, Engine>, read: u64) -> u64 { read + e.collections as u64 }
fn run(e: Eng<'_, Engine>) -> u64 { orchestrate(e, 0usize, body, then) }
fn main() {
    let mut engine = Engine { stack: vec![Word(41)], collections: 0 };
    let mut gc: Gc<'_, '_> = Gc::top(&K);
    assert_eq!(run(Eng::lend(&mut engine, gc.by_ref())), 43);
}
