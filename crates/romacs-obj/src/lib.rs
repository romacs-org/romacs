//! Unwired object primitives. Allocation never collects or calls Lisp.
//! Keep returned values in a mem Root/Frame across mutable heap boundaries.
pub mod builtins;
pub mod obarray;
pub mod symbol;
pub use obarray::Obarray;

/// Access an indexed symbol's plain value cell without binding policy.
/// `idx` is a branded symbol Value, not an unchecked integer or a GC root.
/// Reborrow it from its owner after each mutable heap boundary.
#[inline]
pub fn sym_cell<'h>(
    heap: &'h romacs_mem::Heap,
    idx: romacs_mem::Value<'h>,
) -> Option<romacs_mem::SymbolCell<'h>> {
    heap.symbol_cell(idx)
}
