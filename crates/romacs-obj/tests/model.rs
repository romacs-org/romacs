use romacs_mem::{HeapConfig, Root, WeakRef, spawn_isolate};
use romacs_obj::{Obarray, symbol};
use std::collections::HashMap;

#[test]
fn model_with_gc_between_operations() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        let root = Root::new(&heap, Obarray::new(&heap).unwrap().value(), "model-obarray");
        // Weak identity witnesses never give the model a second strong registry.
        let mut model: HashMap<(Vec<u8>, bool), WeakRef> = HashMap::new();
        let mut random = 71_u64;
        for step in 0..20000 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let name = format!("symbol-{}-é", (random >> 16) % 1000).into_bytes();
            let mb = random & 8 != 0;
            let key = (name, mb);
            let ob = Obarray::from_value(&heap, root.get(&heap)).unwrap();
            match random % 3 {
                0 => {
                    let actual = ob.intern(&key.0, mb).unwrap();
                    if let Some(expected) = model.get(&key) {
                        assert_eq!(Some(actual), expected.upgrade(&heap));
                    } else {
                        model.insert(key, WeakRef::new(&heap, actual));
                    }
                }
                1 => assert_eq!(
                    ob.intern_soft(&key.0, mb),
                    model.get(&key).and_then(|v| v.upgrade(&heap))
                ),
                _ => assert_eq!(ob.unintern(&key.0, mb, None), model.remove(&key).is_some()),
            }
            assert_eq!(ob.len(), model.len());
            if step % 512 == 511 {
                heap.collect(&mut ());
                let ob = Obarray::from_value(&heap, root.get(&heap)).unwrap();
                let mut count = 0;
                ob.for_each(&mut |value| {
                    let s = heap.as_symbol(value).unwrap();
                    let name = heap.as_string(s.name()).unwrap();
                    assert_eq!(
                        model[&(name.bytes().to_vec(), name.multibyte())].upgrade(&heap),
                        Some(value)
                    );
                    count += 1;
                });
                assert_eq!(count, model.len());
            }
        }
        let ob = Obarray::from_value(&heap, root.get(&heap)).unwrap();
        let name = heap.make_string(b"only-obarray", false).unwrap();
        let value = ob.intern_string(name).unwrap();
        assert_eq!(heap.as_symbol(value).unwrap().name(), name);
        assert_eq!(ob.intern_string(name).unwrap(), value);
        assert!(ob.intern_string(heap.nil()).is_err());
        let weak = WeakRef::new(&heap, value);
        heap.collect(&mut ());
        assert!(weak.upgrade(&heap).is_some());
        let ob = Obarray::from_value(&heap, root.get(&heap)).unwrap();
        assert!(ob.unintern(b"only-obarray", false, None));
        heap.collect(&mut ());
        assert!(weak.upgrade(&heap).is_none());
    })
    .join()
    .unwrap();
}

#[test]
fn flags_cells_identity_and_raw_names() {
    spawn_isolate(HeapConfig::default(), |heap| {
        let ob = Obarray::new(&heap).unwrap();
        let symbol = ob.intern(b"", false).unwrap();
        let s = heap.as_symbol(symbol).unwrap();
        for kind in [
            symbol::Kind::Plain,
            symbol::Kind::Alias,
            symbol::Kind::Localized,
            symbol::Kind::Forwarded,
        ] {
            symbol::set_kind(s, kind);
            symbol::set_flag(s, symbol::SPECIAL, true);
            assert_eq!(symbol::kind(s), kind);
            assert!(symbol::has_flag(s, symbol::SPECIAL));
            assert!(symbol::has_flag(s, symbol::INTERNED));
            symbol::set_flag(s, symbol::SPECIAL, false);
            assert!(!symbol::has_flag(s, symbol::SPECIAL));
            assert!(symbol::has_flag(s, symbol::INTERNED));
        }
        s.set_value(heap.fixnum(17).unwrap());
        s.set_function(heap.fixnum(29).unwrap());
        s.set_plist(heap.cons(heap.fixnum(1).unwrap(), heap.nil()).unwrap());
        assert_eq!(s.value().as_fixnum(), Some(17));
        assert_eq!(s.function().as_fixnum(), Some(29));
        assert_eq!(heap.as_cons(s.plist()).unwrap().car().as_fixnum(), Some(1));
        let other = heap.make_symbol(s.name()).unwrap();
        assert!(!ob.unintern(b"", false, Some(other)));
        assert!(ob.unintern(b"", false, Some(symbol)));
        assert!(!symbol::has_flag(s, symbol::INTERNED));
        assert!(s.next().is_nil());
        let raw = ob.intern(&[0xc1, 0xbf], true).unwrap();
        assert_eq!(
            heap.as_string(heap.as_symbol(raw).unwrap().name())
                .unwrap()
                .chars(),
            1
        );
        assert_ne!(ob.intern(&[0xff], false).unwrap(), raw);
        assert_eq!(
            ob.intern(b"ascii", false).unwrap(),
            ob.intern(b"ascii", true).unwrap()
        );
        assert!(ob.intern(&[0xff], true).is_err());
        assert_eq!(ob.len(), 3);
    })
    .join()
    .unwrap();
}
