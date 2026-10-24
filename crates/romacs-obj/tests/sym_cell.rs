use romacs_mem::{HeapConfig, Root, SymbolMode, WeakRef, spawn_isolate};
use romacs_obj::sym_cell;

#[test]
fn read_write_restore_agree_with_symbol_ref_across_pages() {
    spawn_isolate(HeapConfig::default(), |heap| {
        for i in 0..2600 {
            let symbol = heap
                .make_symbol(heap.make_string(b"dynamic", false).unwrap())
                .unwrap();
            let old = heap.as_symbol(symbol).unwrap();
            let cell = sym_cell(&heap, symbol).unwrap();
            let saved = cell.get();
            assert_eq!(saved, old.value());
            cell.set(heap.fixnum(i).unwrap());
            assert_eq!(old.value().as_fixnum(), Some(i));
            old.set_value(heap.fixnum(-i).unwrap());
            assert_eq!(cell.get().as_fixnum(), Some(-i));
            cell.set(saved);
            assert_eq!(old.value(), saved);
        }
        assert!(sym_cell(&heap, heap.fixnum(0).unwrap()).is_none());
        assert!(sym_cell(&heap, heap.make_string(b"not-symbol", false).unwrap()).is_none());
    })
    .join()
    .unwrap();
}

#[test]
fn binding_roots_survive_gc_then_release_without_interim_pin() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        heap.set_symbol_mode(SymbolMode::Interim);
        let symbol = heap
            .make_symbol(heap.make_string(b"binding", false).unwrap())
            .unwrap();
        let symbol_weak = WeakRef::new(&heap, symbol);
        let owner = Root::new(&heap, symbol, "binding-symbol");
        let old = heap.make_string(b"saved", false).unwrap();
        let old_weak = WeakRef::new(&heap, old);
        let saved = Root::new(&heap, old, "binding-old");
        let bound = heap.make_string(b"bound", false).unwrap();
        let bound_weak = WeakRef::new(&heap, bound);
        sym_cell(&heap, symbol).unwrap().set(bound);
        heap.collect(&mut ());
        assert!(symbol_weak.id().resolve(&heap).is_some());
        assert!(old_weak.id().resolve(&heap).is_some());
        assert!(bound_weak.id().resolve(&heap).is_some());
        let cell = sym_cell(&heap, owner.get(&heap)).unwrap();
        assert_eq!(cell.get(), bound_weak.id().resolve(&heap).unwrap());
        cell.set(saved.get(&heap));
        drop(saved);
        heap.collect(&mut ());
        assert!(bound_weak.id().resolve(&heap).is_none());
        assert!(old_weak.id().resolve(&heap).is_some());
        assert_eq!(
            heap.as_symbol(owner.get(&heap)).unwrap().value(),
            old_weak.id().resolve(&heap).unwrap()
        );
        drop(owner);
        heap.collect(&mut ());
        assert!(
            symbol_weak.id().resolve(&heap).is_none(),
            "plain stores must not sticky-pin symbols"
        );
        assert!(old_weak.id().resolve(&heap).is_none());
    })
    .join()
    .unwrap();
}
