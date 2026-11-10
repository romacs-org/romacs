// The positive control: a value rooted in traced storage across a collection, read back by a later leaf.
use romacs_brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn body<'x, 'b>(mut m: Mx<'x, 'b, Engine>, mut gc: Gc<'_, 'b>, at: usize) -> u64 {
    // Root: the value goes to traced storage (the stand-in's stack), its index is plain.
    let rooted = m.leaf(at, |e, at| { let w = Word(e.stack[at].0 + 1); e.stack.push(w); e.stack.len() - 1 });
    collect(&mut m, gc.by_ref());
    m.leaf(rooted, |e, i| e.stack[i].0)
}
/// A request's value kept for the next request: rooted by a consuming leaf.
fn consuming<'x, 'b>(mut m: Mx<'x, 'b, Engine>, mut gc: Gc<'_, 'b>, (): ()) -> u64 {
    let kept = request(&mut m, gc.by_ref(), (), |e, (), value| { e.stack.push(value); e.stack.len() - 1 });
    request(&mut m, gc.by_ref(), kept, |e, i, _| e.stack[i].0)
}
/// Where raw mode goes next: here, the engine's count added in.
fn then(e: Eng<'_, Engine>, read: u64) -> u64 { read + e.collections as u64 }
fn run(e: Eng<'_, Engine>) -> u64 { orchestrate(e, 0usize, body, then) }
/// Orchestration with no permission: leaves only.
fn count<'x, 'b>(mut m: Mx<'x, 'b, Engine>, (): ()) -> usize { m.leaf((), |e, ()| e.stack.len()) }
fn main() {
    let mut engine = Engine { stack: vec![Word(41)], collections: 0 };
    let mut gc: Gc<'_, '_> = Gc::top(&K);
    assert_eq!(run(Eng::lend(&mut engine, gc.by_ref())), 43);
    assert_eq!(orchestrate(Eng::lend(&mut engine, gc.by_ref()), (), consuming, |_, read| read), 1);
    assert_eq!(without_gc(&mut engine, (), count), 3);
}
