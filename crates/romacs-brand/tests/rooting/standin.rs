//! The stand-in engine the rooting fixtures share: `src/brand.rs`'s rules, proved
//! on the types themselves, with no VM behind them.
#![allow(dead_code)]
use romacs_brand::*;
/// A heap value of the stand-in engine: a word a collection may free or move.
pub struct Word(pub u64);
pub struct Engine { pub stack: Vec<Word>, pub collections: u32 }
pub struct K;
// SAFETY: the one key of each fixture's program, on a type only it names, used
// only to mint for code that holds no `Word`.
unsafe impl Key for K {}
/// The stand-in's collecting operation: the key holder's, taking the permission.
pub fn collect<'b>(m: &mut Mx<'_, 'b, Engine>, _gc: Gc<'_, 'b>) { m.raw(&K).collections += 1 }
/// The stand-in's request, through the one the VM's go through (`Mx::request`).
pub fn request<'b, B: Plain, R: Plain>(m: &mut Mx<'_, 'b, Engine>, gc: Gc<'_, 'b>, b: B,
    then: impl FnOnce(&mut Engine, B, Word) -> R + Copy + 'static) -> R {
    m.request(&K, gc, |e| { e.collections += 1; Word(1) }, b, then)
}
