//! Typed views of mem's 48-byte symbol; binding policy belongs to eval.
//! Localized/Forwarded aux words are untraced owner side-table indices.
//! Plain/Alias symbols may cache their immutable name hash there instead;
//! changing kind invalidates that cache before the owner installs an index.
use romacs_mem::SymbolRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
    Plain = 0,
    Alias = 1,
    Localized = 2,
    Forwarded = 3,
}
const KIND_MASK: u32 = 3;
pub const SPECIAL: u32 = 1 << 2;
pub const CONSTANT: u32 = 1 << 3;
pub const TRAPPED_WRITE: u32 = 1 << 4;
pub const INTERNED: u32 = 1 << 5;
// Bit 6 is mem's plain-value UNBOUND bit, not object policy.
pub const FUNCTION_UNBOUND: u32 = 1 << 7;
pub const INITIAL: u32 = 1 << 8;
pub const DECLARED_SPECIAL: u32 = 1 << 9;

pub fn kind(symbol: SymbolRef<'_>) -> Kind {
    match symbol.flags() & KIND_MASK {
        0 => Kind::Plain,
        1 => Kind::Alias,
        2 => Kind::Localized,
        _ => Kind::Forwarded,
    }
}
/// Change value representation, clearing aux on a change of kind. Install the
/// new Localized/Forwarded side-table index with `set_aux` afterwards.
pub fn set_kind(symbol: SymbolRef<'_>, kind: Kind) {
    let aux = if self::kind(symbol) == kind {
        symbol.aux()
    } else {
        0
    };
    symbol.set_flags_aux((symbol.flags() & !KIND_MASK) | kind as u32, aux);
}
pub fn has_flag(symbol: SymbolRef<'_>, flag: u32) -> bool {
    symbol.flags() & flag != 0
}
pub fn set_flag(symbol: SymbolRef<'_>, flag: u32, enabled: bool) {
    // Kind bits cannot be changed through the flag API.
    let flag = flag & !KIND_MASK;
    let flags = if enabled {
        symbol.flags() | flag
    } else {
        symbol.flags() & !flag
    };
    symbol.set_flags_aux(flags, symbol.aux());
}
