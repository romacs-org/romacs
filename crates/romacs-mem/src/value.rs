use crate::Heap;
use std::marker::PhantomData;

/// GNU LSB tags; both integer tags decode to the same kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Symbol,
    Fixnum,
    Cons,
    String,
    Vectorlike,
    Float,
}

/// GNU's vectorlike discriminants, in lisp.h order. Semantics belong to L2+.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PvecType {
    NormalVector,
    Free,
    Bignum,
    Marker,
    Overlay,
    Finalizer,
    SymbolWithPos,
    MiscPtr,
    UserPtr,
    Process,
    Frame,
    Window,
    BoolVector,
    Buffer,
    HashTable,
    Obarray,
    Terminal,
    WindowConfiguration,
    Subr,
    Other,
    Xwidget,
    XwidgetView,
    Thread,
    Mutex,
    Condvar,
    ModuleFunction,
    NativeCompUnit,
    TsParser,
    TsNode,
    TsCompiledQuery,
    Sqlite,
    Closure,
    CharTable,
    SubCharTable,
    Record,
    Font,
    /// Eval-private immutable bytes; not a GNU-visible object kind.
    CodeBlock,
}

/// A single tagged word, valid until the next mutable heap boundary.
/// No raw-word constructor or extraction is public. The raw-pointer phantom
/// intentionally makes even immediate values `!Send` and `!Sync`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Value<'h>(pub(crate) u64, PhantomData<(&'h Heap, *const ())>);
impl<'h> Value<'h> {
    pub const MIN_FIXNUM: i64 = -(1_i64 << 61);
    pub const MAX_FIXNUM: i64 = (1_i64 << 61) - 1;
    pub(crate) const fn word(word: u64) -> Self {
        Self(word, PhantomData)
    }
    pub const fn fixnum(n: i64) -> Option<Self> {
        if n < Self::MIN_FIXNUM || n > Self::MAX_FIXNUM {
            None
        } else {
            Some(Self::word(((n as u64) << 2) | 2))
        }
    }
    pub const fn as_fixnum(self) -> Option<i64> {
        if self.0 & 3 == 2 {
            Some((self.0 as i64) >> 2)
        } else {
            None
        }
    }
    pub const fn kind(self) -> Kind {
        match self.0 & 7 {
            0 => Kind::Symbol,
            2 | 6 => Kind::Fixnum,
            3 => Kind::Cons,
            4 => Kind::String,
            5 => Kind::Vectorlike,
            7 => Kind::Float,
            _ => panic!("unused Lisp tag"),
        }
    }
    pub const fn is_nil(self) -> bool {
        self.0 == 0
    }
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        Self::fixnum(self.as_fixnum()?.checked_add(rhs.as_fixnum()?)?)
    }
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        Self::fixnum(self.as_fixnum()?.checked_sub(rhs.as_fixnum()?)?)
    }
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        Self::fixnum(self.as_fixnum()?.checked_mul(rhs.as_fixnum()?)?)
    }
}
impl std::fmt::Debug for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.as_fixnum() {
            Some(n) => write!(f, "Fixnum({n})"),
            None => write!(f, "{:?}({:#x})", self.kind(), self.0),
        }
    }
}
/// Heap-independent builtin words. L2 extends this table through defsym.
pub mod sym {
    use super::Value;
    pub const NIL: Value<'static> = Value::word(0);
    pub const T: Value<'static> = Value::word(8);
    /// Permanent unbound sentinel; preserve the inherited owner word 0x10.
    pub const UNBOUND: Value<'static> = Value::word(0x10);
    pub const BUILTIN_COUNT: u32 = 3;
}
