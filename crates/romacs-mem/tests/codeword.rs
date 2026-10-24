use romacs_mem::{
    AllocError, Heap, HeapConfig, Payload, PvecType, Root, Value, WeakRef, spawn_isolate,
};

fn closure(heap: &Heap) -> Value<'_> {
    heap.make_vectorlike(
        PvecType::Closure,
        &[
            heap.fixnum(0).unwrap(),
            heap.nil(),
            heap.nil(),
            heap.fixnum(1).unwrap(),
        ],
        &[7, 8],
    )
    .unwrap()
}

#[test]
fn private_word_does_not_change_visible_projection() {
    spawn_isolate(HeapConfig::default(), |heap| {
        let fun = closure(&heap);
        let vector = heap.as_vectorlike(fun).unwrap();
        let before: Vec<_> = (0..vector.len()).map(|i| vector.get(i).unwrap()).collect();
        let debug_before = format!("{before:?}");
        assert_eq!(vector.code(), Some(heap.nil()));
        let mut source = vec![192, 135, 255];
        let block = heap.make_code_block(&source).unwrap();
        source.fill(0);
        assert!(vector.set_code(block));
        assert_eq!(vector.code(), Some(block));
        assert_eq!(vector.len(), 4);
        assert_eq!(vector.get(4), None);
        assert!(!vector.set(4, heap.nil()));
        assert_eq!(vector.raw_bytes(), Some(&[7, 8][..]));
        let after: Vec<_> = (0..vector.len()).map(|i| vector.get(i).unwrap()).collect();
        assert_eq!(before, after);
        assert_eq!(debug_before, format!("{after:?}"));
        let blob = heap.as_vectorlike(block).unwrap();
        assert_eq!(blob.len(), 0);
        assert!(!blob.set(0, fun));
        assert_eq!(blob.raw_bytes(), Some(&[192, 135, 255][..]));
        assert_eq!(blob.code(), None);
    })
    .join()
    .unwrap();
}

#[test]
fn code_survives_with_shared_closures_and_dies_with_last_owner() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        let prototype = Root::new(&heap, closure(&heap), "prototype");
        let instance = Root::new(&heap, closure(&heap), "instance");
        let block = heap.make_code_block(&[192, 135]).unwrap();
        let weak = WeakRef::new(&heap, block);
        let proto = heap.as_vectorlike(prototype.get(&heap)).unwrap();
        assert!(proto.set_code(block));
        assert!(
            heap.as_vectorlike(instance.get(&heap))
                .unwrap()
                .set_code(proto.code().unwrap())
        );
        heap.collect(&mut ());
        assert!(
            weak.upgrade(&heap).is_some(),
            "private word must trace code"
        );
        drop(prototype);
        heap.collect(&mut ());
        assert!(
            weak.upgrade(&heap).is_some(),
            "shared instance must retain code"
        );
        drop(instance);
        heap.collect(&mut ());
        assert!(
            weak.upgrade(&heap).is_none(),
            "code is not a permanent root"
        );
    })
    .join()
    .unwrap();
}

#[test]
fn only_leaf_blocks_can_be_stored() {
    spawn_isolate(HeapConfig::default(), |heap| {
        let fun = heap.as_vectorlike(closure(&heap)).unwrap();
        let block = heap.make_code_block(&[]).unwrap();
        assert_eq!(
            heap.as_vectorlike(block).unwrap().pvec_type(),
            PvecType::CodeBlock
        );
        assert!(fun.set_code(block));
        for invalid in [
            heap.fixnum(7).unwrap(),
            closure(&heap),
            heap.make_string(b"x", false).unwrap(),
        ] {
            assert!(!fun.set_code(invalid));
            assert_eq!(fun.code(), Some(block));
        }
        assert!(!heap.as_vectorlike(block).unwrap().set_code(block));
        assert_eq!(
            heap.make_vectorlike(PvecType::CodeBlock, &[heap.nil()], &[]),
            Err(AllocError::InvalidSize)
        );
        assert_eq!(
            heap.make_payload(PvecType::CodeBlock, &[], Box::new(Bytes)),
            Err(AllocError::InvalidSize)
        );
    })
    .join()
    .unwrap();
}

struct Bytes;
impl Payload for Bytes {
    fn bytes(&self) -> usize {
        23
    }
}

#[test]
fn payload_tail_and_code_are_disjoint() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        let fun = heap
            .make_payload(PvecType::Closure, &[heap.nil()], Box::new(Bytes))
            .unwrap();
        let root = Root::new(&heap, fun, "payload closure");
        let vector = heap.as_vectorlike(fun).unwrap();
        assert_eq!(vector.code(), Some(heap.nil()));
        let block = heap.make_code_block(&[1, 2]).unwrap();
        let weak = WeakRef::new(&heap, block);
        assert!(vector.set_code(block));
        assert_eq!(vector.payload().unwrap().bytes(), 23);
        assert!(vector.raw_bytes().is_none());
        heap.collect(&mut ());
        assert!(weak.upgrade(&heap).is_some());
        assert_eq!(
            heap.as_vectorlike(root.get(&heap))
                .unwrap()
                .payload()
                .unwrap()
                .bytes(),
            23
        );
        drop(root);
        assert_eq!(heap.collect(&mut ()).dropped_payloads, 1);
        assert!(weak.upgrade(&heap).is_none());
    })
    .join()
    .unwrap();
}

#[test]
fn closure_cell_cost_includes_one_private_word() {
    spawn_isolate(HeapConfig::default(), |heap| {
        for (slots, normal_bytes, closure_bytes) in [(4, 48, 64), (5, 64, 64), (6, 64, 80)] {
            let values = vec![heap.nil(); slots];
            let before = heap.allocation_stats().live_bytes;
            heap.make_vectorlike(PvecType::NormalVector, &values, &[])
                .unwrap();
            let middle = heap.allocation_stats().live_bytes;
            heap.make_vectorlike(PvecType::Closure, &values, &[])
                .unwrap();
            let after = heap.allocation_stats().live_bytes;
            assert_eq!(middle - before, normal_bytes);
            assert_eq!(after - middle, closure_bytes);
        }
    })
    .join()
    .unwrap();
}

#[test]
fn clearing_code_releases_blob() {
    spawn_isolate(HeapConfig::default(), |mut heap| {
        let root = Root::new(&heap, closure(&heap), "closure");
        let block = heap.make_code_block(&[]).unwrap();
        let weak = WeakRef::new(&heap, block);
        let fun = heap.as_vectorlike(root.get(&heap)).unwrap();
        assert!(fun.set_code(block));
        assert_eq!(fun.code(), Some(block));
        assert!(fun.set_code(heap.nil()));
        heap.collect(&mut ());
        assert!(weak.upgrade(&heap).is_none());
    })
    .join()
    .unwrap();
}
