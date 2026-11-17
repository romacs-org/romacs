//! What a Romacs world image may hold: values whose meaning survives a restore
//! into another process (`SnapshotSafe`).
//!
//! An image is the builder's heap, mapped back where it was. A value in it
//! means in the restored process what it meant in the builder when every
//! pointer it holds addresses the arena, and when it holds no resource of
//! the builder's process. Function pointers, references to any static
//! (string literals and constant tables too), pointers into a library's or
//! C's heap, descriptors, threads, children and handles of foreign
//! libraries all fail that. So nothing here admits a `&'static` reference:
//! what Romacs's own types hold of a static is its name or a copy. The one
//! such address a plain type may hold, an empty standard hash table's
//! control bytes, a capture removes (`Settle`), and the root's at-rest
//! fields the restoring process remakes (`Schema::renew`).
//!
//! The contract is a trait, derived from a type's definition
//! (`#[derive(Snapshot)]`, romacs-snapshot-derive): the derive meets every
//! field of every variant, so none is left out, and each one's type must be
//! SnapshotSafe itself. A field of the image's root whose type can hold a
//! resource, but whose value a VM at rest never does, is declared at rest
//! (`#[snapshot(at_rest)]`, a root's alone): romacs-vm's
//! `Vm::world_quiescent` checks each such field, and the object inventory,
//! before a capture, and its `world::image::capture` takes only
//! SnapshotSafe values. An enum variant declared at rest is the one payload
//! no walk reaches: the type keeps it inert in an image (romacs-vm's
//! `Object::foreign_resource`), and should one hold an address of the
//! executable anyway, following it faults where the restore could reserve
//! that range (romacs-vm's `world::arena::guard_executable`; a taken range
//! is left alone). Each crate derives it for its own types.
//!
//! No capture-time scan backs this up. The heap holds padding and unused
//! capacity no value initialized; a B2 image has thousands of words there
//! that look like code and stack addresses (DESIGN-M1-addendum A10.1), so
//! a scan could not tell a pointer from stale bytes, and reading those
//! bytes as values would itself be undefined.

/// A value of this type, in a world image, means the same in the process
/// that maps the image (see the module documentation).
///
/// # Safety
/// Implement it only through `#[derive(Snapshot)]`, which checks each
/// field, or for a type whose representation holds no pointer but into the
/// global allocator's heap and no resource of the process.
pub unsafe trait SnapshotSafe {}

mod schema;
pub use schema::{EMPTY_HASH_TABLE, Notes, Schema, Settle, TAKEN, Walk};
pub use romacs_snapshot_derive::Snapshot;

// The standard library's data types: their representations hold no
// pointer but into the global allocator's heap, which is the arena.
macro_rules! plain {
    ($($t:ty),* $(,)?) => {
        $(
            // SAFETY: each type the invocations list, on the grounds their
            // comments give.
            unsafe impl SnapshotSafe for $t {}
        )*
    };
}
plain!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64, bool, char, (), str, String,
       std::path::PathBuf, std::path::Path, std::ffi::OsString, std::ffi::OsStr, std::time::Duration, std::time::SystemTime,
       std::num::NonZeroU32, std::num::NonZeroU64, std::num::NonZeroUsize, std::hash::RandomState,
       std::sync::atomic::AtomicBool, std::sync::atomic::AtomicU8, std::sync::atomic::AtomicU32,
       std::sync::atomic::AtomicU64, std::sync::atomic::AtomicUsize, std::sync::atomic::AtomicI64);

// Other crates' data types the VM holds, on the same grounds: a bignum's
// digits, a JSON tree, a rope's nodes, and hash builders' seeds.
plain!(ropey::Rope, num_bigint::BigInt, num_bigint::BigUint, rustc_hash::FxBuildHasher,
       foldhash::fast::FixedState);
// Not foldhash's RandomState or SeedableRandomState: their shared seed is,
// by default, foldhash's process-global one, in writable static storage
// each process draws anew. A builder that leaks its own seed to the heap
// vouches for that itself (romacs-vm's NameState).

// Containers: their own storage is the global allocator's heap or inline,
// and every pointer they hold beyond that is one of their elements'.
macro_rules! containers {
    ($(<$($param:ident),+> $t:ty;)*) => {
        $(
            // SAFETY: as above, given that each element type is SnapshotSafe.
            unsafe impl<$($param: SnapshotSafe),+> SnapshotSafe for $t {}
        )*
    };
}
containers! {
    <T> [T]; <T> Vec<T>; <T> std::collections::VecDeque<T>; <T> std::collections::BinaryHeap<T>;
    <T> std::cell::OnceCell<T>; <T> std::sync::OnceLock<T>; <T> Option<T>;
    <T> std::ops::Range<T>; <T> std::ops::RangeInclusive<T>;
    <T, E> Result<T, E>;
    <K, V, S> std::collections::HashMap<K, V, S>; <K, S> std::collections::HashSet<K, S>;
    <K, V> std::collections::BTreeMap<K, V>; <K> std::collections::BTreeSet<K>;
}
// SAFETY: an array holds its elements inline.
unsafe impl<T: SnapshotSafe, const N: usize> SnapshotSafe for [T; N] {}

// Owning and shared pointers and cells, of possibly unsized values: they
// point into the global allocator's heap, or hold the value inline.
macro_rules! pointers {
    ($($t:ident)::+; $($rest:tt)*) => {
        // SAFETY: as above, given that the pointee type is SnapshotSafe.
        unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for $($t)::+<T> {}
        pointers!($($rest)*);
    };
    () => {};
}
pointers! {
    Box; std::rc::Rc; std::rc::Weak; std::sync::Arc; std::sync::Weak;
    std::cell::Cell; std::cell::RefCell;
    // A futex word and a poison flag: no resource of the process. A VM at
    // rest holds none locked, since no other thread runs in it.
    std::sync::Mutex; std::sync::RwLock;
}
// SAFETY: no value at all.
unsafe impl<T: ?Sized> SnapshotSafe for std::marker::PhantomData<T> {}
// SAFETY: a zero-sized marker for a hasher type.
unsafe impl<H> SnapshotSafe for std::hash::BuildHasherDefault<H> {}

use std::any::Any;

macro_rules! tuples {
    ($(($($t:ident $v:ident),+)),* $(,)?) => {
        $(
            // SAFETY: a tuple holds its SnapshotSafe fields inline.
            unsafe impl<$($t: SnapshotSafe),+> SnapshotSafe for ($($t,)+) {}
            impl<$($t: SnapshotSafe + Schema),+> Schema for ($($t,)+) {
                fn walk(&self, w: &mut Walk<'_>) {
                    let ($($v,)+) = self;
                    $(w.field(stringify!($v), $v);)+
                }
                fn any(&self) -> Option<&dyn Any> {
                    Some(self)
                }
                fn settle(&mut self, s: &mut Settle) {
                    let ($($v,)+) = self;
                    $($v.settle(s);)+
                }
            }
        )*
    };
}
tuples!((A a), (A a, B b), (A a, B b, C c), (A a, B b, C c, D d), (A a, B b, C c, D d, E e), (A a, B b, C c, D d, E e, F f),
        (A a, B b, C c, D d, E e, F f, G g));

