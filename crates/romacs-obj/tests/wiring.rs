use romacs_mem::{HeapConfig, Root, WeakRef, raw::RawValue, spawn_isolate};
use romacs_obj::{Obarray, builtins, symbol};

#[test]
fn unbound_sentinel_is_reserved_at_index_two() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        assert_eq!(RawValue::from_value(builtins::UNBOUND).bits(), 0x10);
        assert_eq!(builtins::INDICES[2], ("UNBOUND", 2));
        assert!(heap.as_symbol(builtins::UNBOUND).is_some());
        let name = heap.make_string(b"first", false).unwrap();
        let first = heap.make_symbol(name).unwrap();
        assert_eq!(RawValue::from_value(first).bits(), 3 << 3);
        heap.collect(&mut ());
        assert!(heap.as_symbol(builtins::UNBOUND).is_some());
        let next = heap
            .make_symbol(heap.make_string(b"reused", false).unwrap())
            .unwrap();
        assert_ne!(next, builtins::UNBOUND);
    })
    .join()
    .unwrap();
}

#[test]
fn object_flags_are_disjoint_and_preserve_mem_binding_state() {
    let flags = [
        symbol::SPECIAL,
        symbol::CONSTANT,
        symbol::TRAPPED_WRITE,
        symbol::INTERNED,
        symbol::FUNCTION_UNBOUND,
        symbol::INITIAL,
        symbol::DECLARED_SPECIAL,
    ];
    for (i, flag) in flags.iter().enumerate() {
        assert_eq!(flag.count_ones(), 1);
        assert_eq!(flag & ((1 << 6) | 3), 0);
        for other in &flags[..i] {
            assert_eq!(flag & other, 0);
        }
    }
    spawn_isolate(HeapConfig::default(), move |heap| {
        let value = heap
            .make_symbol(heap.make_string(b"flags", false).unwrap())
            .unwrap();
        let s = heap.as_symbol(value).unwrap();
        for flag in flags {
            symbol::set_flag(s, flag, true);
            assert!(!s.is_bound());
            s.set_value(heap.nil());
            assert!(symbol::has_flag(s, flag));
            symbol::set_flag(s, flag, false);
            assert!(s.is_bound());
            s.makunbound();
        }
    })
    .join()
    .unwrap();
}

#[test]
fn redirect_aux_is_untraced_and_not_an_obarray_hash() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        let root = Root::new(&heap, Obarray::new(&heap).unwrap().value(), "aux-obarray");
        let mut witnesses = Vec::new();
        for (name, kind) in [
            (b"localized".as_slice(), symbol::Kind::Localized),
            (b"forwarded".as_slice(), symbol::Kind::Forwarded),
        ] {
            let ob = Obarray::from_value(&heap, root.get(&heap)).unwrap();
            let value = ob.intern(name, false).unwrap();
            let dead = heap
                .make_symbol(heap.make_string(b"not-an-edge", false).unwrap())
                .unwrap();
            let word = RawValue::from_value(dead).bits() as u32;
            let index = if kind == symbol::Kind::Localized {
                word >> 3
            } else {
                word
            };
            let s = heap.as_symbol(value).unwrap();
            symbol::set_kind(s, kind);
            s.set_aux(index);
            assert_eq!(ob.intern_soft(name, false), Some(value));
            witnesses.push((
                name,
                WeakRef::new(&heap, value),
                WeakRef::new(&heap, dead),
                index,
            ));
        }
        let ob = Obarray::from_value(&heap, root.get(&heap)).unwrap();
        for i in 0..128 {
            ob.intern(format!("grow-{i}").as_bytes(), false).unwrap();
        }
        heap.collect(&mut ());
        for (name, live, dead, index) in witnesses {
            assert!(
                dead.upgrade(&heap).is_none(),
                "aux must not trace a symbol-shaped word"
            );
            let value = live.upgrade(&heap).unwrap();
            let ob = Obarray::from_value(&heap, root.get(&heap)).unwrap();
            assert_eq!(ob.intern_soft(name, false), Some(value));
            let s = heap.as_symbol(value).unwrap();
            assert_eq!(s.aux(), index);
            symbol::set_kind(s, symbol::Kind::Plain);
            assert_eq!(s.aux(), 0, "redirect index must not become a name hash");
            assert_eq!(ob.intern_soft(name, false), Some(value));
            assert!(ob.unintern(name, false, Some(value)));
        }
    })
    .join()
    .unwrap();
}
