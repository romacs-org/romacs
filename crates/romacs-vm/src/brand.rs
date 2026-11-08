//! Rooting as a type. A heap `Value` is a plain word that a collection may free
//! or move, and rustc emits no stack maps, so a Value a Rust frame keeps across
//! a collection point is invisible to the collector. These types make keeping
//! one a compile error by splitting code into two modes:
//!
//! - Raw mode reads and writes Values but holds no permission to collect. A raw
//!   handler holds its engine as an [`Eng`], which it moves into its dispatch,
//!   so after a dispatch it has no engine left to use a Value with.
//! - Orchestration holds the permission ([`Gc`]) and no Value. Its raw work runs
//!   in leaves ([`Mx::leaf`]), which take and return only [`Plain`] data and
//!   cannot capture a place to smuggle a Value through. A Value needed after a
//!   collection point lives in traced storage (registers, a `RootFrame`, a
//!   binding) and a later leaf reads it back. Orchestration ends by naming,
//!   as plain data, where raw mode goes next ([`orchestrate`]).
//!
//! Every permission outside the instantiating crate's [`Key`] holder derives
//! from one already held: [`Gc::by_ref`], [`orchestrate`] (from an `Eng`),
//! [`Eng::lend`] (from a `Gc`), or `unsafe` [`Gc::unchecked`]. Brands are
//! invariant lifetimes, generative per orchestration, so permissions of two
//! VMs (isolates share slot indices) do not mix. Stored Values carry none.
//!
//! What the types cannot reject is a deliberate channel: a leaf writing a Value
//! to global mutable state, or re-encoding it as an integer, for orchestration
//! to read back. Those are named in the design note (§4.14) and kept out of
//! ordinary code by `tests/rooting_brand_rules.rs`.
//!
//! Generic over the engine, so `tests/compile_boundaries.rs` proves the rules
//! on these very types with a stand-in engine.
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

/// Invariant in 'b: one VM's brand cannot be widened or narrowed into another's.
type Invariant<'b> = PhantomData<fn(&'b ()) -> &'b ()>;

/// Permission to reach a collection point in the VM branded 'b. Zero-sized and
/// not `Copy`: a call that can collect takes it by value, lent with [`by_ref`],
/// so nothing that borrows it, and no leaf, outlives the lend.
///
/// [`by_ref`]: Gc::by_ref
pub struct Gc<'p, 'b>(PhantomData<(&'p mut (), Invariant<'b>)>);

impl<'b> Gc<'_, 'b> {
    /// Lend the permission to one call.
    #[inline(always)]
    pub fn by_ref(&mut self) -> Gc<'_, 'b> {
        Gc(PhantomData)
    }
    /// The top-level mint: only the holder of the crate's [`Key`] makes one.
    #[inline(always)]
    pub fn top<K: Key>(_key: &K) -> Gc<'static, 'b> {
        Gc(PhantomData)
    }
    /// A permission for code that does not hold one yet.
    ///
    /// # Safety
    /// The caller keeps no heap Value in a Rust local across what it passes
    /// this to (each call site says why, as the census row it replaces did).
    #[inline(always)]
    pub unsafe fn unchecked() -> Gc<'static, 'b> {
        Gc(PhantomData)
    }
}

/// The instantiating crate's authority to mint at its top level and to give its
/// own collecting operations the engine an [`Mx`] holds.
///
/// # Safety
/// Implemented once per crate, on a type only that crate's facade module can
/// name, and used only to mint for code that holds no Value.
pub unsafe trait Key {}

/// Data holding no unrooted heap Value: what leaves take and return.
///
/// # Safety
/// Implementing it promises the type holds no heap Value outside traced
/// storage (a root handle's index is fine; a `Value` is not).
pub unsafe trait Plain {}

/// One audited list of the standard types that are plain.
macro_rules! plain {
    ($([$($generics:tt)*] $t:ty),* $(,)?) => {
        // SAFETY: each listed type is a scalar, text, or a container of types
        // that are themselves `Plain`; none can hold a `Value`.
        $(unsafe impl<$($generics)*> Plain for $t {})*
    };
}
plain!(
    [] (), [] bool, [] char, [] u8, [] u16, [] u32, [] u64, [] u128, [] usize,
    [] i8, [] i16, [] i32, [] i64, [] i128, [] isize, [] f32, [] f64, [] String, [] str,
    [T: Plain + ?Sized] &T, [T: Plain + ?Sized] Box<T>, [T: Plain] Option<T>,
    [T: Plain, E: Plain] Result<T, E>, [T: Plain] Vec<T>, [T: Plain] [T], [T: Plain, const N: usize] [T; N],
    [A: Plain, B: Plain] (A, B), [A: Plain, B: Plain, C: Plain] (A, B, C),
    [A: Plain, B: Plain, C: Plain, D: Plain] (A, B, C, D),
);

/// Raw mode's engine: what a handler gets and moves into its dispatch. Not
/// `Copy`; made only from a lent permission ([`Eng::lend`]) or by
/// [`orchestrate`] for its continuation, so a leaf can neither dispatch, drive
/// nor orchestrate. In machine code it is the `&mut E` it wraps.
#[repr(transparent)]
pub struct Eng<'x, E>(&'x mut E);

impl<'x, E> Eng<'x, E> {
    /// The driver's handle for a run it holds the permission for.
    #[inline(always)]
    pub fn lend(e: &'x mut E, _gc: Gc<'x, '_>) -> Self {
        Eng(e)
    }
}
impl<E> Deref for Eng<'_, E> {
    type Target = E;
    #[inline(always)]
    fn deref(&self) -> &E {
        self.0
    }
}
impl<E> DerefMut for Eng<'_, E> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut E {
        self.0
    }
}

/// An engine (or VM) in orchestration: a facade with no raw accessor but the
/// key holder's. It holds no Value; its raw work runs in leaves.
pub struct Mx<'e, 'b, E> {
    e: &'e mut E,
    _brand: Invariant<'b>,
}

impl<'b, E> Mx<'_, 'b, E> {
    /// Raw work: plain data in, plain data out. The closure may capture only
    /// `Copy`, `'static` data, so it cannot write a Value to a captured place
    /// nor take the permission with it.
    #[inline(always)]
    pub fn leaf<A: Plain, R: Plain>(&mut self, a: A, f: impl FnOnce(&mut E, A) -> R + Copy + 'static) -> R {
        f(self.e, a)
    }
    /// The engine, for the key holder's collecting operations.
    #[inline(always)]
    pub fn raw<K: Key>(&mut self, _key: &K) -> &mut E {
        self.e
    }
    /// A collecting request, for the key holder: RUN may collect, and what it
    /// returns goes straight to the leaf THEN, whose result is plain. So nothing
    /// RUN returned outlives the request in orchestration.
    #[inline(always)]
    pub fn request<K: Key, T, B: Plain, R: Plain>(&mut self, _key: &K, _gc: Gc<'_, 'b>, run: impl FnOnce(&mut E) -> T, b: B,
        then: impl FnOnce(&mut E, B, T) -> R + Copy + 'static) -> R {
        let out = run(self.e);
        then(self.e, b, out)
    }
}

/// Raw mode to orchestration and back, consuming the handle. BODY is a
/// function pointer, so it captures nothing of the caller's; it returns, as
/// plain data, where raw mode goes next, and THEN takes the engine there.
/// THEN's result is `'static`, so it cannot hand the engine back to the raw
/// caller. BODY returns before THEN runs: its frame, whatever it borrowed, is
/// gone before the dispatch, which can be a sibling call.
#[inline(always)]
pub fn orchestrate<'x, E, A: Plain, N: Plain, R: 'static>(e: Eng<'x, E>, a: A,
    body: for<'m, 'b, 'g> fn(Mx<'m, 'b, E>, Gc<'g, 'b>, A) -> N, then: fn(Eng<'x, E>, N) -> R) -> R {
    let Eng(e) = e;
    let next = body(Mx { e: &mut *e, _brand: PhantomData }, Gc(PhantomData), a);
    then(Eng(e), next)
}
