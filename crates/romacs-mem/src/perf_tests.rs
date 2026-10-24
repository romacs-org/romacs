use crate::*;

#[test]
fn debt_query_does_not_borrow_allocator_or_gc() {
    spawn_isolate(Default::default(), |h| {
        let _allocator = h.allocator.borrow_mut();
        let _gc = h.gc.borrow_mut();
        assert_eq!(h.allocation_debt(), 0);
    })
    .join()
    .unwrap();
}

#[cfg(not(feature = "verify"))]
#[test]
fn symbol_metadata_does_not_borrow_allocator() {
    spawn_isolate(Default::default(), |h| {
        let sym = h
            .make_symbol(h.make_string(b"plain", false).unwrap())
            .unwrap();
        let _allocator = h.allocator.borrow_mut();
        assert!(!h.as_symbol(sym).unwrap().is_bound());
    })
    .join()
    .unwrap();
}
