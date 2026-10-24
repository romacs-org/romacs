#[cfg(feature = "verify")]
use crate::Value;
use crate::{Heap, HeapConfig};

#[cfg(not(feature = "verify"))]
#[test]
fn sym_cell_needs_neither_allocator_nor_gc_borrow() {
    let heap = Heap::create(HeapConfig::default());
    let symbol = heap
        .make_symbol(heap.make_string(b"fast", false).unwrap())
        .unwrap();
    let _allocator = heap.allocator.borrow_mut();
    let _gc = heap.gc.borrow_mut();
    let cell = heap.symbol_cell(symbol).unwrap();
    let alias = heap.symbol_cell(symbol).unwrap();
    cell.set(heap.fixnum(42).unwrap());
    assert_eq!(alias.get().as_fixnum(), Some(42));
    alias.set(heap.nil());
    assert!(cell.get().is_nil());
}

#[cfg(feature = "verify")]
#[test]
#[should_panic(expected = "foreign/dead value")]
fn sym_cell_verify_rejects_unallocated_symbol() {
    let heap = Heap::create(HeapConfig::default());
    // Deliberate internal contract violation, checked before pointer formation.
    let _ = heap.symbol_cell(Value::word(123456 << 3));
}

#[cfg(feature = "verify")]
#[test]
#[should_panic(expected = "foreign/dead value")]
fn sym_cell_verify_rejects_reclaimed_symbol() {
    let mut heap = Heap::create(HeapConfig::default());
    let symbol = heap
        .make_symbol(heap.make_string(b"reclaimed", false).unwrap())
        .unwrap();
    let weak = crate::WeakRef::new(&heap, symbol);
    let word = symbol.0;
    heap.collect(&mut ());
    assert!(weak.upgrade(&heap).is_none());
    // Deliberate internal contract violation before ID reuse: occupancy, not generations.
    let _ = heap.symbol_cell(Value::word(word));
}

#[cfg(feature = "verify")]
#[test]
#[should_panic(expected = "foreign/dead value")]
fn sym_cell_verify_rejects_dead_store() {
    let mut heap = Heap::create(HeapConfig::default());
    let word = heap.make_string(b"dead", false).unwrap().0;
    heap.collect(&mut ());
    // Deliberate internal contract violation, checked before publishing an edge.
    heap.symbol_cell(heap.nil()).unwrap().set(Value::word(word));
}
