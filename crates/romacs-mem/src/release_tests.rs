use super::*;
#[test]
fn madvise_releases_only_payload_after_two_cycles() {
    let mut a = Allocator::new(crate::HeapConfig::default().max_symbols).unwrap();
    let live = a.alloc(Kind::Cons, 16).unwrap();
    for _ in 0..(PAGE_BYTES - HEADER_BYTES) / 8 {
        a.alloc(Kind::Float, 8).unwrap();
    }
    let payload = PAGE_BYTES + HEADER_BYTES;
    assert_eq!(
        a.chunks[0]
            .as_ref()
            .unwrap()
            .mapping
            .resident_pages(payload, PAGE_BYTES - HEADER_BYTES),
        15
    );
    assert!(a.mark(live));
    a.sweep().unwrap();
    assert_eq!(
        a.chunks[0]
            .as_ref()
            .unwrap()
            .mapping
            .resident_pages(payload, PAGE_BYTES - HEADER_BYTES),
        15
    );
    assert!(a.mark(live));
    a.sweep().unwrap();
    let m = &a.chunks[0].as_ref().unwrap().mapping;
    assert_eq!(m.resident_pages(payload, PAGE_BYTES - HEADER_BYTES), 0);
    assert_eq!(m.resident_pages(PAGE_BYTES, HEADER_BYTES), 1);
    assert!(a.pages[1].as_ref().unwrap().h().released);
    let f = a.alloc(Kind::String, 48).unwrap();
    assert!(a.mark(f));
    assert_eq!(a.pages.iter().flatten().count(), 2);
    assert_eq!(a.pages[1].as_ref().unwrap().h().size, 48);
}
