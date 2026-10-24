use super::*;
#[test]
fn classes_and_injective_bitmaps() {
    assert_eq!(SYMBOLS_PER_PAGE, 1280);
    assert!(
        Allocator::new(crate::HeapConfig::default().max_symbols)
            .unwrap()
            .symbols
            .mapping
            .len
            <= 512 * 1024 * 1024
    );
    assert_eq!(Allocator::class(Kind::Float, 8), Ok(Some((1, 8))));
    assert_eq!(Allocator::class(Kind::Vectorlike, 8193), Ok(None));
    assert_eq!(
        Allocator::class(Kind::Cons, 15),
        Err(AllocError::InvalidSize)
    );
    for (kind, size) in [(Kind::Float, 8), (Kind::Cons, 16), (Kind::String, 48)]
        .into_iter()
        .chain(VECTOR_CLASSES.into_iter().map(|s| (Kind::Vectorlike, s)))
    {
        let mut a = Allocator::new(crate::HeapConfig::default().max_symbols).unwrap();
        let mut words = Vec::new();
        let count = (PAGE_BYTES - HEADER_BYTES) / size;
        for _ in 0..count {
            words.push(a.alloc(kind, size).unwrap());
        }
        assert_eq!(a.pages.iter().flatten().count(), 1);
        for (i, w) in words.iter().enumerate() {
            if i % 2 == 0 {
                assert!(a.mark(*w));
            }
        }
        a.sweep().unwrap();
        let p = a.pages[0].as_ref().unwrap();
        for i in 0..count {
            assert_eq!(
                p.allocated(i as u32),
                i % 2 == 0,
                "{kind:?} size={size} slot={i}"
            );
        }
        assert_eq!(
            a.stats().live_bytes,
            crate::sym::BUILTIN_COUNT as usize * 48 + count.div_ceil(2) * size
        );
        let base = p.base.as_ptr() as usize;
        let reused = a.alloc(kind, size).unwrap();
        assert!(words.contains(&reused));
        // Address masking identifies the page, including the final 8 B float cell.
        assert_eq!((reused as usize & !7) & !(PAGE_BYTES - 1), base);
    }
}
#[test]
fn configured_symbol_capacity_grows_exhausts_and_reuses() {
    for limit in [
        crate::sym::BUILTIN_COUNT as usize,
        SYMBOLS_PER_PAGE,
        SYMBOLS_PER_PAGE + 3,
    ] {
        let mut a = Allocator::new(limit).unwrap();
        assert_eq!(
            a.symbols.mapping.len,
            limit.div_ceil(SYMBOLS_PER_PAGE) * PAGE_BYTES
        );
        let base = a.symbol_base();
        let mut words = Vec::new();
        for index in crate::sym::BUILTIN_COUNT as usize..limit {
            let word = a.alloc(Kind::Symbol, 48).unwrap();
            assert_eq!(word >> 3, index as u64);
            words.push(word);
        }
        assert_eq!(a.symbol_base(), base);
        let before = a.stats();
        for _ in 0..2 {
            assert_eq!(a.alloc(Kind::Symbol, 48), Err(AllocError::SymbolLimit));
        }
        assert_eq!(a.stats().counts, before.counts);
        assert_eq!(a.stats().live_bytes, before.live_bytes);
        a.sweep().unwrap();
        if !words.is_empty() {
            assert!(words.contains(&a.alloc(Kind::Symbol, 48).unwrap()));
        }
    }
    for invalid in [
        0,
        crate::sym::BUILTIN_COUNT as usize - 1,
        SYMBOL_LIMIT + 1,
        usize::MAX,
    ] {
        assert!(matches!(
            Allocator::new(invalid),
            Err(AllocError::InvalidSize)
        ));
    }
}

#[test]
fn symbols_mapping_reuse_and_release() {
    let mut a = Allocator::new(crate::HeapConfig::default().max_symbols).unwrap();
    let mut words = Vec::new();
    for _ in 0..2560 {
        words.push(a.alloc(Kind::Symbol, 48).unwrap());
    }
    for w in &words {
        let index = (w >> 3) as usize;
        let expected = a.symbols.mapping.base.as_ptr() as usize
            + index / 1280 * PAGE_BYTES
            + HEADER_BYTES
            + index % 1280 * 48;
        assert_eq!(
            a.symbols.address(index as u32).unwrap().as_ptr() as usize,
            expected
        );
        if index == 1280 {
            assert!(a.mark(*w));
        }
    }
    a.sweep().unwrap();
    assert!(a.symbols.address(1280).is_some());
    assert!(a.symbols.address(crate::sym::BUILTIN_COUNT).is_none());
    assert!(a.symbols.address(0).is_some());
    let reuse = a.alloc(Kind::Symbol, 48).unwrap();
    assert!(words.contains(&reuse));
    a.sweep().unwrap();
    a.sweep().unwrap();
    assert_eq!(
        a.stats().committed_symbol_bytes,
        PAGE_BYTES + 2 * HEADER_BYTES
    );
    let word = a.alloc(Kind::Symbol, 48).unwrap();
    assert!(a.mark(word));
    a.sweep().unwrap();
    assert_eq!(
        a.stats().live_bytes,
        (crate::sym::BUILTIN_COUNT as usize + 1) * 48
    );
    assert_eq!(
        a.stats().counts.symbols,
        2562 + crate::sym::BUILTIN_COUNT as u64
    );
}
#[test]
fn large_objects_counters_and_page_release() {
    let mut a = Allocator::new(crate::HeapConfig::default().max_symbols).unwrap();
    let keep = a.alloc(Kind::Vectorlike, 9000).unwrap();
    let dead = a.alloc(Kind::Vectorlike, 20_000).unwrap();
    let c = a.alloc(Kind::Cons, 16).unwrap();
    let f = a.alloc(Kind::Float, 8).unwrap();
    let s = a.alloc(Kind::String, 48).unwrap();
    a.alloc_with_counts(Kind::String, 48, 16).unwrap();
    assert_eq!(
        a.stats().counts.memory_use_counts(),
        [1, 1, 3621, crate::sym::BUILTIN_COUNT as u64, 16, 0, 2]
    );
    assert!(!a.mark((c & !7) | 7));
    assert!(!a.mark(c + 8));
    assert!(a.mark(keep));
    a.sweep().unwrap();
    assert_eq!(a.stats().large_objects, 1);
    assert!(!a.mark(dead));
    assert!(!a.mark(f));
    assert!(!a.mark(s));
    assert_eq!(a.stats().mapped_bytes, CHUNK_BYTES + 12_288);
    assert!(a.mark(keep));
    a.sweep().unwrap();
    assert_eq!(a.stats().released_pages, 3);
    assert_eq!(a.stats().mapped_bytes, 12_288);
    assert_eq!(
        a.stats().live_bytes,
        9000 + crate::sym::BUILTIN_COUNT as usize * 48
    );
    assert_eq!(a.stats().bytes_since_cycle, 0);
    a.sweep().unwrap();
    assert_eq!(a.stats().mapped_bytes, 0);
    assert_eq!(
        a.alloc(Kind::Vectorlike, usize::MAX),
        Err(AllocError::MemoryFull)
    );
}
#[test]
fn released_page_reuse_with_live_chunk() {
    let mut a = Allocator::new(crate::HeapConfig::default().max_symbols).unwrap();
    let live = a.alloc(Kind::Cons, 16).unwrap();
    a.alloc(Kind::Float, 8).unwrap();
    for _ in 0..2 {
        assert!(a.mark(live));
        a.sweep().unwrap();
    }
    assert_eq!(a.stats().released_pages, 1);
    let f = a.alloc(Kind::Float, 8).unwrap();
    assert!(a.mark(f));
    assert!(a.mark(live));
    a.sweep().unwrap();
    assert_eq!(
        a.stats().live_bytes,
        24 + crate::sym::BUILTIN_COUNT as usize * 48
    );
    assert_eq!(a.stats().mapped_bytes, CHUNK_BYTES);
}

#[test]
fn committed_budget_distinguishes_reserved_released_and_large_storage() {
    let mut a = Allocator::new(crate::HeapConfig::default().max_symbols).unwrap();
    assert_eq!(a.stats().committed_bytes, PAGE_BYTES);
    let live = a.alloc(Kind::Cons, 16).unwrap();
    a.alloc(Kind::Float, 8).unwrap();
    let large = a.alloc(Kind::Vectorlike, 9000).unwrap();
    assert_eq!(a.stats().committed_bytes, 3 * PAGE_BYTES + 12_288);
    assert!(a.stats().mapped_bytes > a.stats().committed_bytes);
    for _ in 0..2 {
        assert!(a.mark(live));
        assert!(a.mark(large));
        a.sweep().unwrap();
    }
    assert_eq!(
        a.stats().committed_bytes,
        2 * PAGE_BYTES + HEADER_BYTES + 12_288
    );
    let float = a.alloc(Kind::Float, 8).unwrap();
    assert_eq!(a.stats().committed_bytes, 3 * PAGE_BYTES + 12_288);
    assert!(a.mark(float));
    assert!(a.mark(live));
    a.sweep().unwrap();
    assert_eq!(a.stats().committed_bytes, 3 * PAGE_BYTES);
}
