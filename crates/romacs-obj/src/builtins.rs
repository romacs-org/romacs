//! The builtin indices currently reserved by mem. Extend mem's reservation
//! and this single list together when later object slices add standard atoms.
use romacs_mem::{Value, sym};
macro_rules! builtins {
    ($($name:ident = $index:literal),* $(,)?) => {
        $(pub const $name: Value<'static> = sym::$name;)*
        pub const INDICES: &[(&str, u32)] = &[$((stringify!($name), $index)),*];
    };
}
builtins! { NIL = 0, T = 1, UNBOUND = 2 }
