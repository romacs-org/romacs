//! `Schema` for the standard library's and other crates' data types the
//! listings hold: what each owns and contains, for a walk. Byte counts are
//! exact for sequences, text and boxes; for hash tables, B-trees, ropes,
//! bignums and shared pointers they are estimates from their lengths and
//! capacities, without node overhead or padding.
use super::{EMPTY_HASH_TABLE, Schema, TAKEN, Walk};
use std::any::Any;
use std::mem::{size_of, size_of_val};

/// Values with nothing to visit and no allocation of their own.
macro_rules! leaves {
    ($($t:ty),* $(,)?) => {
        $(
            impl Schema for $t {
                const LEAF: bool = true;
                fn walk(&self, _: &mut Walk<'_>) {}
                fn any(&self) -> Option<&dyn Any> {
                    Some(self)
                }
            }
        )*
    };
}
leaves!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64, bool, char, (),
        std::time::Duration, std::time::SystemTime, std::num::NonZeroU32, std::num::NonZeroU64, std::num::NonZeroUsize,
        std::hash::RandomState, std::sync::atomic::AtomicBool, std::sync::atomic::AtomicU8, std::sync::atomic::AtomicU32,
        std::sync::atomic::AtomicU64, std::sync::atomic::AtomicUsize, std::sync::atomic::AtomicI64,
        rustc_hash::FxBuildHasher, foldhash::fast::FixedState, serde_json::Number);

/// Unsized text, met through a pointer that owns it.
macro_rules! unsized_text {
    ($($t:ty),* $(,)?) => {
        $(
            impl Schema for $t {
                const LEAF: bool = true;
                fn walk(&self, _: &mut Walk<'_>) {}
                fn any(&self) -> Option<&dyn Any> {
                    None
                }
            }
        )*
    };
}
unsized_text!(str, std::path::Path, std::ffi::OsStr);

/// Owned text: an allocation of its capacity.
macro_rules! owned_text {
    ($($t:ty),* $(,)?) => {
        $(
            impl Schema for $t {
                fn walk(&self, w: &mut Walk<'_>) {
                    w.own(self.capacity());
                }
                fn any(&self) -> Option<&dyn Any> {
                    Some(self)
                }
            }
        )*
    };
}
owned_text!(String, std::path::PathBuf, std::ffi::OsString, ropey::Rope);

impl Schema for num_bigint::BigUint {
    fn walk(&self, w: &mut Walk<'_>) {
        w.own(self.iter_u64_digits().len() * size_of::<u64>());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl Schema for num_bigint::BigInt {
    fn walk(&self, w: &mut Walk<'_>) {
        w.own(self.iter_u64_digits().len() * size_of::<u64>());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl Schema for serde_json::Value {
    fn walk(&self, w: &mut Walk<'_>) {
        match self {
            serde_json::Value::String(text) => w.value(text),
            serde_json::Value::Array(items) => w.value(items),
            serde_json::Value::Object(map) => w.value(map),
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
        }
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl Schema for serde_json::Map<String, serde_json::Value> {
    fn walk(&self, w: &mut Walk<'_>) {
        w.own(self.len() * size_of::<(String, serde_json::Value)>());
        w.elements(self.keys());
        w.elements(self.values());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<T: Schema> Schema for [T] {
    fn walk(&self, w: &mut Walk<'_>) {
        w.elements(self.iter());
    }
    fn any(&self) -> Option<&dyn Any> {
        None
    }
}

impl<T: Schema, const N: usize> Schema for [T; N] {
    fn walk(&self, w: &mut Walk<'_>) {
        w.elements(self.iter());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

/// Sequences: an allocation of their capacity, and their elements.
macro_rules! sequences {
    ($($t:ident)::+; $($rest:tt)*) => {
        impl<T: Schema> Schema for $($t)::+<T> {
            fn walk(&self, w: &mut Walk<'_>) {
                w.own(self.capacity() * size_of::<T>());
                w.elements(self.iter());
            }
            fn any(&self) -> Option<&dyn Any> {
                Some(self)
            }
        }
        sequences!($($rest)*);
    };
    () => {};
}
sequences!(Vec; std::collections::VecDeque; std::collections::BinaryHeap;);

impl<T: Schema> Schema for Option<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        if let Some(value) = self {
            w.value(value);
        }
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<T: Schema, E: Schema> Schema for Result<T, E> {
    fn walk(&self, w: &mut Walk<'_>) {
        match self {
            Ok(value) => w.value(value),
            Err(error) => w.value(error),
        }
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<T: Schema> Schema for std::cell::OnceCell<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        if let Some(value) = self.get() {
            w.value(value);
        }
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<T: Schema> Schema for std::sync::OnceLock<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        if let Some(value) = self.get() {
            w.value(value);
        }
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<T: Schema> Schema for std::ops::Range<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        w.value(&self.start);
        w.value(&self.end);
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<T: Schema> Schema for std::ops::RangeInclusive<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        w.value(self.start());
        w.value(self.end());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

/// An empty standard hash table points its control bytes at a static of
/// the executable; another holds an allocation of its buckets.
fn hash_table<E>(w: &mut Walk<'_>, capacity: usize) {
    if capacity == 0 {
        w.note(EMPTY_HASH_TABLE);
    } else {
        w.own(capacity * (size_of::<E>() + 1));
    }
}

impl<K: Schema, V: Schema, S: 'static> Schema for std::collections::HashMap<K, V, S> {
    fn walk(&self, w: &mut Walk<'_>) {
        hash_table::<(K, V)>(w, self.capacity());
        w.elements(self.keys());
        w.elements(self.values());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<K: Schema, S: 'static> Schema for std::collections::HashSet<K, S> {
    fn walk(&self, w: &mut Walk<'_>) {
        hash_table::<K>(w, self.capacity());
        w.elements(self.iter());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

/// A B-tree's nodes, counted by the entries they hold.
impl<K: Schema, V: Schema> Schema for std::collections::BTreeMap<K, V> {
    fn walk(&self, w: &mut Walk<'_>) {
        w.own(self.len() * size_of::<(K, V)>());
        w.elements(self.keys());
        w.elements(self.values());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<K: Schema> Schema for std::collections::BTreeSet<K> {
    fn walk(&self, w: &mut Walk<'_>) {
        w.own(self.len() * size_of::<K>());
        w.elements(self.iter());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<T: Schema + ?Sized> Schema for Box<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        w.own(size_of_val(&**self));
        w.value(&**self);
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

/// Shared pointers: the allocation and its value, the first time the walk
/// meets them; its two counts beside the value.
macro_rules! shared {
    ($($t:ident)::+; $($rest:tt)*) => {
        impl<T: Schema + ?Sized> Schema for $($t)::+<T> {
            fn walk(&self, w: &mut Walk<'_>) {
                if w.first_share($($t)::+::as_ptr(self).cast::<u8>() as usize) {
                    w.own(size_of_val(&**self) + 2 * size_of::<usize>());
                    w.value(&**self);
                }
            }
            fn any(&self) -> Option<&dyn Any> {
                Some(self)
            }
        }
        shared!($($rest)*);
    };
    () => {};
}
shared!(std::rc::Rc; std::sync::Arc;);

/// A weak pointer's value is its strong pointers' to walk.
macro_rules! weak {
    ($($t:ident)::+; $($rest:tt)*) => {
        impl<T: Schema + ?Sized> Schema for $($t)::+<T> {
            fn walk(&self, _: &mut Walk<'_>) {}
            fn any(&self) -> Option<&dyn Any> {
                Some(self)
            }
        }
        weak!($($rest)*);
    };
    () => {};
}
weak!(std::rc::Weak; std::sync::Weak;);

impl<T: Schema + Copy> Schema for std::cell::Cell<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        w.value(&self.get());
    }
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<T: Schema + ?Sized> Schema for std::cell::RefCell<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        match self.try_borrow() {
            Ok(value) => w.value(&*value),
            Err(_) => w.note(TAKEN),
        }
    }
    fn any(&self) -> Option<&dyn Any> {
        None
    }
}

impl<T: Schema + ?Sized> Schema for std::sync::Mutex<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        match self.try_lock() {
            Ok(value) => w.value(&*value),
            Err(_) => w.note(TAKEN),
        }
    }
    fn any(&self) -> Option<&dyn Any> {
        None
    }
}

impl<T: Schema + ?Sized> Schema for std::sync::RwLock<T> {
    fn walk(&self, w: &mut Walk<'_>) {
        match self.try_read() {
            Ok(value) => w.value(&*value),
            Err(_) => w.note(TAKEN),
        }
    }
    fn any(&self) -> Option<&dyn Any> {
        None
    }
}

impl<T: ?Sized + 'static> Schema for std::marker::PhantomData<T> {
    const LEAF: bool = true;
    fn walk(&self, _: &mut Walk<'_>) {}
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl<H: 'static> Schema for std::hash::BuildHasherDefault<H> {
    const LEAF: bool = true;
    fn walk(&self, _: &mut Walk<'_>) {}
    fn any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
