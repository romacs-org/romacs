//! What a Romacs world image may hold: values whose meaning survives a restore
//! into another process (`SnapshotSafe`).
//!
//! An image is the builder's heap, mapped back where it was. A value in it
//! means in the restored process what it meant in the builder when every
//! pointer it holds addresses the arena, or read-only data of the
//! executable, which a restore maps where the builder had it
//! (romacs-vm's `world::arena::shadow_executable`); and when it holds no resource of the
//! builder's process. Function pointers, references to writable statics,
//! pointers into a library's or C's heap, descriptors, threads, children
//! and handles of foreign libraries all fail that.
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
        unsafe impl $crate::SnapshotSafe for $name {}
        const _: () = {
            #[allow(dead_code, unused_variables)]
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
            #[allow(dead_code, unused_variables)]
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
        unsafe impl $crate::SnapshotSafe for $name {}
        const _: () = {
            #[allow(dead_code, unused_variables)]
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
        unsafe impl $crate::SnapshotSafe for $name {}
        const _: () = {
            #[allow(dead_code, unused_variables)]
            fn fields(value: &$name) {
                match value { $($pat => { $($crate::require($bind);)* })* }
            }
        };
    };
}

// The standard library's data types: their representations hold no
// pointer but into the global allocator's heap, which is the arena.
macro_rules! plain {
    ($($t:ty),* $(,)?) => { $(unsafe impl SnapshotSafe for $t {})* };
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
unsafe impl<K: SnapshotSafe, V: SnapshotSafe> SnapshotSafe for serde_json::Map<K, V> {}

/// A literal: in the executable's read-only data.
unsafe impl SnapshotSafe for &'static str {}

unsafe impl<T: SnapshotSafe> SnapshotSafe for [T] {}
unsafe impl<T: SnapshotSafe, const N: usize> SnapshotSafe for [T; N] {}
unsafe impl<T: SnapshotSafe> SnapshotSafe for Vec<T> {}
unsafe impl<T: SnapshotSafe> SnapshotSafe for std::collections::VecDeque<T> {}
unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for Box<T> {}
unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for std::rc::Rc<T> {}
unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for std::rc::Weak<T> {}
unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for std::sync::Arc<T> {}
unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for std::sync::Weak<T> {}
unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for std::cell::Cell<T> {}
unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for std::cell::RefCell<T> {}
unsafe impl<T: SnapshotSafe> SnapshotSafe for std::cell::OnceCell<T> {}
unsafe impl<T: SnapshotSafe> SnapshotSafe for std::sync::OnceLock<T> {}
/// A futex word and a poison flag: no resource of the process. A VM at
/// rest holds none locked, since no other thread runs in it.
unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for std::sync::Mutex<T> {}
unsafe impl<T: SnapshotSafe + ?Sized> SnapshotSafe for std::sync::RwLock<T> {}
unsafe impl<T: SnapshotSafe> SnapshotSafe for Option<T> {}
unsafe impl<T: SnapshotSafe, E: SnapshotSafe> SnapshotSafe for Result<T, E> {}
unsafe impl<T: SnapshotSafe> SnapshotSafe for std::ops::Range<T> {}
unsafe impl<T: SnapshotSafe> SnapshotSafe for std::ops::RangeInclusive<T> {}
unsafe impl<T: ?Sized> SnapshotSafe for std::marker::PhantomData<T> {}
unsafe impl<T: SnapshotSafe + ToOwned + ?Sized> SnapshotSafe for std::borrow::Cow<'static, T> where T::Owned: SnapshotSafe {}
unsafe impl<H> SnapshotSafe for std::hash::BuildHasherDefault<H> {}
unsafe impl<K: SnapshotSafe, V: SnapshotSafe, S: SnapshotSafe> SnapshotSafe for std::collections::HashMap<K, V, S> {}
unsafe impl<K: SnapshotSafe, S: SnapshotSafe> SnapshotSafe for std::collections::HashSet<K, S> {}
unsafe impl<K: SnapshotSafe, V: SnapshotSafe> SnapshotSafe for std::collections::BTreeMap<K, V> {}
unsafe impl<K: SnapshotSafe> SnapshotSafe for std::collections::BTreeSet<K> {}
unsafe impl<T: SnapshotSafe> SnapshotSafe for std::collections::BinaryHeap<T> {}

macro_rules! tuples {
    ($(($($t:ident),+)),* $(,)?) => { $(unsafe impl<$($t: SnapshotSafe),+> SnapshotSafe for ($($t,)+) {})* };
}
tuples!((A), (A, B), (A, B, C), (A, B, C, D), (A, B, C, D, E), (A, B, C, D, E, F), (A, B, C, D, E, F, G));
