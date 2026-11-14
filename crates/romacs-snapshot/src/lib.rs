//! What a Romacs world image may hold: values whose meaning survives a restore
//! into another process (`SnapshotSafe`).
//!
//! An image is the builder's heap, mapped back where it was. A value in it
//! means in the restored process what it meant in the builder when every
//! pointer it holds addresses the arena, and when it holds no resource of
//! the builder's process. Function pointers, references to any static
//! (string literals and constant tables too), pointers into a library's or
//! C's heap, descriptors, threads, children and handles of foreign
//! libraries all fail that. The one exception is not Romacs's to remove: a
//! plain type of the standard library or another crate may point into the
//! executable's read-only data, as an empty hash map points to its static
//! control group, and a restore maps that data where the builder had it
//! (romacs-vm's `world::arena::shadow_executable`). So nothing here admits
//! a `&'static` reference: what Romacs's own types hold of a static is its
//! name or a copy, which another build can read as well.
//!
//! The contract is a trait, implemented for a type only by listing it
//! field by field (`snapshot_struct!`, `snapshot_tuple!`) or variant by
//! variant (`snapshot_enum!`): the listing destructures every field, so a
//! field added and not listed does not compile, and each listed field's
//! type must be SnapshotSafe itself. A field whose type can hold a
//! resource, but whose value a VM at rest never does, is listed as at rest:
//! romacs-vm's `Vm::world_quiescent` checks each such field, and the object
//! inventory, before a capture, and its `world::image::capture` takes only
//! SnapshotSafe values. Each crate lists its own types, next to them.
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
/// Implement it only through `snapshot_struct!`, `snapshot_tuple!` or
/// `snapshot_enum!`, which check each field, or for a type whose
/// representation holds no pointer but into the global allocator's heap
/// or into immutable data, and no resource of the process.
pub unsafe trait SnapshotSafe {}

/// Require VALUE's type to be SnapshotSafe: what the listing macros call
/// for each field.
pub fn require<T: SnapshotSafe + ?Sized>(_value: &T) {}

/// `snapshot_struct!(Name { field, ... } at_rest { field, ... })`: Name is
/// SnapshotSafe, every field listed, each but the at-rest ones
/// SnapshotSafe. An at-rest field must be checked by `world_quiescent`.
#[macro_export]
macro_rules! snapshot_struct {
    ($name:ident { $($(#[$m:meta])* $field:ident),* $(,)? } $(at_rest { $($(#[$rm:meta])* $rest:ident),* $(,)? })?) => {
        // SAFETY: `fields` below names every field and requires each listed
        // one SnapshotSafe; at-rest fields are `world_quiescent`'s to check.
        unsafe impl $crate::SnapshotSafe for $name {}
        const _: () = {
            #[allow(dead_code, unused_variables, reason = "never called: it exists to be type-checked")]
            fn fields(value: &$name) {
                let $name { $($(#[$m])* $field,)* $($($(#[$rm])* $rest,)*)? } = value;
                $($(#[$m])* $crate::require($field);)*
            }
        };
    };
}

/// `snapshot_fields!(Name { field, ... } at_rest { field, ... })`: the
/// listing of `snapshot_struct!`, checked the same way, without making
/// Name SnapshotSafe. For a value that enters an image only through a
/// door that validates it, as a world's primary VM does through
/// romacs-vm's `world::image::capture`: SnapshotSafe would let one in
/// unvalidated, inside another value.
#[macro_export]
macro_rules! snapshot_fields {
    ($name:ident { $($(#[$m:meta])* $field:ident),* $(,)? } $(at_rest { $($(#[$rm:meta])* $rest:ident),* $(,)? })?) => {
        const _: () = {
            #[allow(dead_code, unused_variables, reason = "never called: it exists to be type-checked")]
            fn fields(value: &$name) {
                let $name { $($(#[$m])* $field,)* $($($(#[$rm])* $rest,)*)? } = value;
                $($(#[$m])* $crate::require($field);)*
            }
        };
    };
}

/// `snapshot_tuple!(Name(a, b))`: a tuple struct, every field listed.
#[macro_export]
macro_rules! snapshot_tuple {
    ($name:ident ( $($field:ident),* $(,)? )) => {
        // SAFETY: `fields` below names every field and requires each one
        // SnapshotSafe.
        unsafe impl $crate::SnapshotSafe for $name {}
        const _: () = {
            #[allow(dead_code, unused_variables, reason = "never called: it exists to be type-checked")]
            fn fields(value: &$name) {
                let $name($($field),*) = value;
                $($crate::require($field);)*
            }
        };
    };
}

/// `snapshot_enum!(Name { Name::A(x) => (x), Name::B { y, .. } => (y),
/// Name::C => () })`: every variant matched, the match exhaustive, each
/// binding SnapshotSafe. Bind nothing of an at-rest variant's payload.
#[macro_export]
macro_rules! snapshot_enum {
    ($name:ty { $($pat:pat => ($($bind:ident),*)),* $(,)? }) => {
        // SAFETY: `fields` below matches every variant, exhaustively, and
        // requires each binding SnapshotSafe.
        unsafe impl $crate::SnapshotSafe for $name {}
        const _: () = {
            #[allow(dead_code, unused_variables, reason = "never called: it exists to be type-checked")]
            fn fields(value: &$name) {
                match value { $($pat => { $($crate::require($bind);)* })* }
            }
        };
    };
}

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
plain!(ropey::Rope, num_bigint::BigInt, num_bigint::BigUint, serde_json::Value, serde_json::Number, rustc_hash::FxBuildHasher,
       foldhash::fast::FixedState);
// Not foldhash's RandomState or SeedableRandomState: their shared seed is,
// by default, foldhash's process-global one, in writable static storage
// each process draws anew. A builder that leaks its own seed to the heap
// vouches for that itself (romacs-vm's NameState).
// SAFETY: a JSON map is a heap tree of its keys and values.
unsafe impl<K: SnapshotSafe, V: SnapshotSafe> SnapshotSafe for serde_json::Map<K, V> {}

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

macro_rules! tuples {
    ($(($($t:ident),+)),* $(,)?) => {
        $(
            // SAFETY: a tuple holds its SnapshotSafe fields inline.
            unsafe impl<$($t: SnapshotSafe),+> SnapshotSafe for ($($t,)+) {}
        )*
    };
}
tuples!((A), (A, B), (A, B, C), (A, B, C, D), (A, B, C, D, E), (A, B, C, D, E, F), (A, B, C, D, E, F, G));
