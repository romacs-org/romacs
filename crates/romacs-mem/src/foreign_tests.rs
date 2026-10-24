use crate::{raw::*, *};

pub(crate) trait Fixture: 'static {
    fn trace(&self, _: &mut Marker<'_>) {}
    fn weak(&mut self) -> Option<&mut dyn LegacyWeak> {
        None
    }
    fn bytes(&self) -> usize {
        0
    }
}
struct FixtureHooks<T>(std::marker::PhantomData<T>);
// SAFETY: place_fixture initializes exactly T after the overlay. Fixtures
// enumerate their edges, release borrows before collection, and never call Lisp.
unsafe impl<T: Fixture> ForeignHooks for FixtureHooks<T> {
    fn trace(&self, w: RawValue, m: &mut Marker<'_>) {
        unsafe { &*fixture_ptr::<T>(w) }.trace(m);
    }
    fn with_weak(&mut self, w: RawValue, f: &mut dyn FnMut(&mut dyn LegacyWeak)) {
        f(unsafe { &mut *fixture_ptr::<T>(w) }
            .weak()
            .expect("weak fixture"));
    }
    fn finalizer_function(&self, _: RawValue) -> RawValue {
        unreachable!()
    }
    fn drop(&mut self, w: RawValue) {
        unsafe {
            std::ptr::drop_in_place(fixture_ptr::<T>(w));
        }
    }
    fn bytes(&self, w: RawValue) -> usize {
        unsafe { &*fixture_ptr::<T>(w) }.bytes()
    }
    fn symbol_pinned(&self, _: RawValue) -> bool {
        false
    }
}
unsafe fn fixture_ptr<T>(w: RawValue) -> *mut T {
    unsafe { ((w.bits() & !7) as *mut u8).add(16).cast() }
}
pub(crate) fn install_fixture<T: Fixture>(h: &mut Heap) {
    unsafe {
        h.set_foreign_hooks(Box::new(FixtureHooks::<T>(std::marker::PhantomData)));
    }
}
pub(crate) fn place_fixture<T: Fixture>(h: &Heap, t: T, flags: ForeignFlags) -> Value<'_> {
    assert!(std::mem::align_of::<T>() <= 8);
    unsafe {
        let w = h
            .alloc_foreign(
                ForeignKind::Vectorlike,
                (16 + std::mem::size_of::<T>()).max(24),
                flags,
            )
            .unwrap();
        fixture_ptr::<T>(w).write(t);
        w.value(h)
    }
}
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

#[repr(C)]
struct Old {
    header: [u8; 16],
    type_tag: u8,
    edges: Vec<RawValue>,
}
#[derive(Default)]
struct Observed {
    drops: [usize; 2],
    pinned: BTreeSet<u64>,
}
struct Hooks(Rc<RefCell<Observed>>);
// SAFETY: all fixtures initialize Old before publishing; edges are complete,
// weak entries live in separate fixtures, and hooks never access the heap.
unsafe impl ForeignHooks for Hooks {
    fn trace(&self, w: RawValue, m: &mut Marker<'_>) {
        // SAFETY: fixture installs only Old objects.
        let old = unsafe { &*((w.bits() & !7) as *const Old) };
        if old.type_tag != 78 {
            for &edge in &old.edges {
                m.mark(edge);
            }
        }
    }
    fn with_weak(&mut self, _: RawValue, _: &mut dyn FnMut(&mut dyn LegacyWeak)) {
        panic!("not weak")
    }
    fn finalizer_function(&self, w: RawValue) -> RawValue {
        unsafe { &*((w.bits() & !7) as *const Old) }.edges[0]
    }
    fn drop(&mut self, w: RawValue) {
        let ptr = (w.bits() & !7) as *mut Old;
        // SAFETY: uniquely retired owner fields, header has no Rust destructor.
        unsafe {
            self.0.borrow_mut().drops[usize::from(w.bits() & 7 == 5)] += 1;
            std::ptr::drop_in_place(std::ptr::addr_of_mut!((*ptr).edges));
        }
    }
    fn bytes(&self, w: RawValue) -> usize {
        unsafe { &*((w.bits() & !7) as *const Old) }
            .edges
            .capacity()
            * 8
    }
    fn symbol_pinned(&self, w: RawValue) -> bool {
        self.0.borrow().pinned.contains(&w.bits())
    }
    fn is_marker(&self, w: RawValue) -> bool {
        // SAFETY: owner installed only initialized Old cells.
        unsafe { &*((w.bits() & !7) as *const Old) }.type_tag == 3
    }
}
fn owner(config: HeapConfig) -> (Heap, Rc<RefCell<Observed>>) {
    // SAFETY: tests keep heap and fixtures on this thread.
    let mut h = unsafe { Heap::for_legacy_owner(config) };
    let observed = Rc::new(RefCell::new(Observed::default()));
    unsafe { h.set_foreign_hooks(Box::new(Hooks(observed.clone()))) };
    (h, observed)
}
fn old(h: &Heap, kind: ForeignKind, edges: Vec<RawValue>, flags: ForeignFlags) -> RawValue {
    // SAFETY: owner fields initialized without touching the mem overlay.
    unsafe {
        let w = h
            .alloc_foreign(kind, std::mem::size_of::<Old>(), flags)
            .unwrap();
        let ptr = (w.bits() & !7) as *mut Old;
        std::ptr::addr_of_mut!((*ptr).type_tag).write(
            if matches!(flags, ForeignFlags::FINALIZER) {
                78
            } else {
                77
            },
        );
        std::ptr::addr_of_mut!((*ptr).edges).write(edges);
        w
    }
}
struct Roots(Vec<RawValue>);
impl Trace for Roots {
    fn trace(&self, m: &mut Marker<'_>) {
        for &w in &self.0 {
            m.mark(w);
        }
    }
}
#[test]
fn overlay_canary_and_hidden_tail() {
    let (mut h, _) = owner(HeapConfig::default());
    // SAFETY: byte-only fixture needs no destructor fields; use separate no-op hooks.
    struct Canary;
    unsafe impl ForeignHooks for Canary {
        fn trace(&self, _: RawValue, _: &mut Marker<'_>) {}
        fn with_weak(&mut self, _: RawValue, _: &mut dyn FnMut(&mut dyn LegacyWeak)) {
            unreachable!()
        }
        fn finalizer_function(&self, _: RawValue) -> RawValue {
            unreachable!()
        }
        fn drop(&mut self, _: RawValue) {}
        fn bytes(&self, _: RawValue) -> usize {
            0
        }
        fn symbol_pinned(&self, _: RawValue) -> bool {
            false
        }
    }
    unsafe {
        h.set_foreign_hooks(Box::new(Canary));
    }
    for (kind, n) in [ForeignKind::Vectorlike, ForeignKind::String]
        .into_iter()
        .flat_map(|kind| [24, 56, 9000].map(|n| (kind, n)))
    {
        let w = unsafe { h.alloc_foreign(kind, n, ForeignFlags::NONE).unwrap() };
        unsafe {
            ((w.bits() & !7) as *mut u8)
                .add(16)
                .write_bytes(0xa5, n - 16);
        }
        let mut roots = Roots(vec![w]);
        for _ in 0..100 {
            h.collect(&mut roots);
        }
        assert!(h.is_foreign(w));
        if matches!(kind, ForeignKind::Vectorlike) {
            let v = h.as_vectorlike(unsafe { w.value(&h) }).unwrap();
            assert_eq!(
                (v.pvec_type(), v.len(), v.raw_bytes()),
                (PvecType::Other, 0, None)
            );
            assert!(v.get(0).is_none() && v.payload().is_none() && v.code().is_none());
        } else {
            assert!(h.as_string(unsafe { w.value(&h) }).is_none());
            let native = h.make_string(b"native", false).unwrap();
            assert!(!h.is_foreign(RawValue::from_value(native)));
            assert_eq!(h.as_string(native).unwrap().bytes(), b"native");
        }
        assert!(
            unsafe { std::slice::from_raw_parts(((w.bits() & !7) as *const u8).add(16), n - 16) }
                .iter()
                .all(|&b| b == 0xa5)
        );
    }
}
#[test]
fn foreign_string_hook_survival_and_release() {
    let (mut h, seen) = owner(HeapConfig::default());
    let c = h.cons(h.nil(), h.nil()).unwrap();
    let weak = WeakRef::new(&h, c);
    let s = old(
        &h,
        ForeignKind::String,
        vec![RawValue::from_value(c)],
        ForeignFlags::NONE,
    );
    assert!(h.is_foreign(s));
    assert!(h.as_string(unsafe { s.value(&h) }).is_none());
    let mut roots = Roots(vec![s]);
    h.collect(&mut roots);
    assert!(weak.upgrade(&h).is_some());
    unsafe {
        (*((s.bits() & !7) as *mut Old)).edges.clear();
    }
    h.collect(&mut roots);
    assert!(weak.upgrade(&h).is_none());
    roots.0.clear();
    h.collect(&mut roots);
    assert_eq!(seen.borrow().drops, [1, 0]);
}
#[test]
fn mixed_model_stress() {
    let (mut h, seen) = owner(HeapConfig::default());
    let mut nodes: Vec<Option<(RawValue, Vec<usize>, usize)>> = Vec::new();
    let mut roots = Roots(Vec::new());
    let mut seed = 0x1357_2468_u64;
    let mut expected = [0; 2];
    for op in 0..20_000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let live: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.as_ref().map(|_| i))
            .collect();
        let ids: Vec<usize> = if live.is_empty() {
            vec![]
        } else {
            vec![live[seed as usize % live.len()]]
        };
        let edges: Vec<_> = ids.iter().map(|&i| nodes[i].as_ref().unwrap().0).collect();
        let kind = op % 3;
        let w = if kind == 2 {
            let a = edges
                .first()
                .copied()
                .unwrap_or(RawValue::from_value(h.nil()));
            unsafe { h.alloc_cons_raw(a, RawValue::from_value(h.nil())).unwrap() }
        } else {
            old(
                &h,
                if kind == 0 {
                    ForeignKind::String
                } else {
                    ForeignKind::Vectorlike
                },
                edges,
                ForeignFlags::NONE,
            )
        };
        nodes.push(Some((w, ids, kind)));
        if seed & 7 == 0 {
            roots.0.push(w);
        }
        if roots.0.len() > 8 {
            roots.0.remove(0);
        }
        if op % 512 == 511 || op == 19_999 {
            let mut reachable = BTreeSet::new();
            let mut todo: Vec<usize> = nodes
                .iter()
                .enumerate()
                .filter_map(|(i, n)| n.as_ref().filter(|n| roots.0.contains(&n.0)).map(|_| i))
                .collect();
            while let Some(i) = todo.pop() {
                if reachable.insert(i) {
                    todo.extend(&nodes[i].as_ref().unwrap().1);
                }
            }
            for (i, n) in nodes.iter_mut().enumerate() {
                if !reachable.contains(&i)
                    && let Some((_, _, k)) = n.take()
                    && k < 2
                {
                    expected[k] += 1;
                }
            }
            h.collect(&mut roots);
            assert_eq!(seen.borrow().drops, expected, "cycle {op}");
            let bytes: usize = nodes
                .iter()
                .flatten()
                .filter(|n| n.2 < 2)
                .map(|n| {
                    unsafe { &*((n.0.bits() & !7) as *const Old) }
                        .edges
                        .capacity()
                        * 8
                })
                .sum();
            assert_eq!(h.accounted_bytes(), h.allocation_stats().live_bytes + bytes);
        }
    }
    assert!(expected[0] > 0 && expected[1] > 0);
}
#[test]
fn foreign_finalizers_two_pass_and_once() {
    let (mut h, seen) = owner(HeapConfig::default());
    let f = h.cons(h.nil(), h.nil()).unwrap();
    let weak = WeakRef::new(&h, f);
    let a = old(
        &h,
        ForeignKind::Vectorlike,
        vec![RawValue::from_value(f)],
        ForeignFlags::FINALIZER,
    );
    let b = old(
        &h,
        ForeignKind::Vectorlike,
        vec![a],
        ForeignFlags::FINALIZER,
    );
    h.as_cons(f).unwrap().set_car(unsafe { b.value(&h) });
    h.collect(&mut ());
    assert!(weak.upgrade(&h).is_some());
    assert_eq!(
        h.run_finalizers(|h, root| {
            h.collect(&mut ());
            assert!(root.get(h).kind() != Kind::Fixnum);
        }),
        2
    );
    h.collect(&mut ());
    assert_eq!(h.run_finalizers(|_, _| panic!("repeat")), 0);
    assert!(weak.upgrade(&h).is_none());
    assert_eq!(seen.borrow().drops, [0, 2]);
}
#[test]
fn external_symbols_mark_pin_and_reject_native() {
    let (mut h, seen) = owner(HeapConfig {
        external_symbols: true,
        ..HeapConfig::default()
    });
    let name = h.make_string(b"name", false).unwrap();
    assert!(h.make_symbol(name).is_err());
    let words = [800, 808, 816];
    seen.borrow_mut().pinned.insert(words[1]);
    let mut tables = Vec::new();
    for bits in words {
        let t = h.make_weak_pairs(Weakness::Key, 1).unwrap();
        h.as_weak_pairs(t)
            .unwrap()
            .set(0, unsafe { RawValue::from_bits(bits).value(&h) }, h.nil());
        tables.push(Root::new(&h, t, "external table"));
    }
    let mut roots = Roots(vec![unsafe { RawValue::from_bits(words[2]) }]);
    h.collect(&mut roots);
    for (i, t) in tables.iter().enumerate() {
        assert_eq!(
            h.as_weak_pairs(t.get(&h)).unwrap().len(),
            usize::from(i != 0)
        );
    }
    assert!(
        h.as_symbol(unsafe { RawValue::from_bits(words[2]).value(&h) })
            .is_none()
    );
    assert_eq!(seen.borrow().drops, [0, 0]);
    roots.0.clear();
    seen.borrow_mut().pinned.clear();
    h.collect(&mut roots);
    assert!(
        tables
            .iter()
            .all(|t| h.as_weak_pairs(t.get(&h)).unwrap().is_empty())
    );
}
#[test]
fn registered_static_leaf() {
    let (mut h, _) = owner(HeapConfig::default());
    unsafe {
        h.add_static_range(0x1000, 32);
    }
    h.collect(&mut Roots(vec![unsafe { RawValue::from_bits(0x1005) }]));
    for bits in [0x1004, 0x1005, 0x1007] {
        let w = unsafe { RawValue::from_bits(bits) };
        let pair = unsafe {
            h.alloc_cons_raw(w, RawValue::from_value(h.nil()))
                .unwrap()
                .value(&h)
        };
        let leaf = h.as_cons(pair).unwrap().car();
        assert!(h.as_vectorlike(leaf).is_none());
        assert!(h.as_string(leaf).is_none());
        assert!(h.as_float(leaf).is_none());
    }
}
#[test]
#[should_panic(expected = "Dangling edge")]
fn unregistered_static_panics() {
    let (mut h, _) = owner(HeapConfig::default());
    h.collect(&mut Roots(vec![unsafe { RawValue::from_bits(0x1005) }]));
}

#[test]
fn words_and_current_thread_heap() {
    // SAFETY: fixtures own this thread and use only valid immediate words.
    unsafe {
        let h = Heap::for_legacy_owner(HeapConfig::default());
        for bits in [0, 8, 42, 0xffff_ffff_ffff_fffe] {
            assert_eq!(RawValue::from_bits(bits).bits(), bits);
        }
        let pair = h
            .alloc_cons_raw(RawValue::from_bits(0), RawValue::from_bits(8))
            .unwrap();
        assert_eq!(h.as_cons(pair.value(&h)).unwrap().cdr(), sym::T);
    }
}

#[test]
fn reviewer_foreign_marker_in_undo_is_compacted() {
    struct Undo(RawValue);
    impl Trace for Undo {
        fn trace(&self, _: &mut Marker<'_>) {}
        fn undo_heads(&mut self, visit: &mut dyn FnMut(&mut RawValue)) {
            visit(&mut self.0);
        }
    }
    let (mut h, seen) = owner(HeapConfig::default());
    let marker = old(&h, ForeignKind::Vectorlike, vec![], ForeignFlags::NONE);
    unsafe {
        (*((marker.bits() & !7) as *mut Old)).type_tag = 3;
    } // old Marker
    let pair = h
        .cons(unsafe { marker.value(&h) }, h.fixnum(1).unwrap())
        .unwrap();
    let head = h.cons(pair, h.nil()).unwrap();
    let mut undo = Undo(RawValue::from_value(head));
    h.collect(&mut undo);
    assert_eq!(
        undo.0.bits(),
        h.nil().0,
        "dead foreign marker must be removed from undo before strong marking"
    );
    assert_eq!(seen.borrow().drops, [0, 1]);
}

#[test]
fn reviewer_native_vector_child_through_foreign() {
    for kind in [ForeignKind::String, ForeignKind::Vectorlike] {
        let (mut h, seen) = owner(HeapConfig::default());
        let c = h.cons(h.nil(), h.nil()).unwrap();
        let v = h
            .make_vectorlike(PvecType::NormalVector, &[c], &[])
            .unwrap();
        let wc = WeakRef::new(&h, c);
        let wv = WeakRef::new(&h, v);
        let f = old(&h, kind, vec![RawValue::from_value(v)], ForeignFlags::NONE);
        let parent = h
            .make_vectorlike(PvecType::NormalVector, &[unsafe { f.value(&h) }], &[])
            .unwrap();
        let r = Root::new(&h, parent, "native-to-foreign-to-native");
        h.collect(&mut ());
        assert!(
            wv.upgrade(&h).is_some(),
            "native vector reachable only through foreign cell was freed"
        );
        assert!(wc.upgrade(&h).is_some());
        drop(r);
        h.collect(&mut ());
        assert!(wv.upgrade(&h).is_none() && wc.upgrade(&h).is_none());
        assert_eq!(seen.borrow().drops.iter().sum::<usize>(), 1);
    }
}

#[test]
fn mixed_foreign_native_undo_marker_liveness() {
    struct Undo {
        head: RawValue,
        chain: Vec<RawValue>,
        detached: usize,
    }
    impl Trace for Undo {
        fn trace(&self, _: &mut Marker<'_>) {}
        fn undo_heads(&mut self, visit: &mut dyn FnMut(&mut RawValue)) {
            visit(&mut self.head);
        }
        fn unchain_markers(&mut self, marked: &dyn Fn(RawValue) -> bool) {
            self.chain.retain(|w| {
                if marked(*w) {
                    true
                } else {
                    self.detached += 1;
                    false
                }
            });
        }
    }
    let (mut h, seen) = owner(HeapConfig::default());
    let mut markers = Vec::new();
    for _ in 0..2 {
        let marker = old(&h, ForeignKind::Vectorlike, vec![], ForeignFlags::NONE);
        unsafe {
            (*((marker.bits() & !7) as *mut Old)).type_tag = 3;
        }
        markers.push(marker);
    }
    for _ in 0..2 {
        markers.push(RawValue::from_value(
            h.make_vectorlike(PvecType::Marker, &[], &[]).unwrap(),
        ));
    }
    let live_foreign = Root::new(&h, unsafe { markers[1].value(&h) }, "live foreign marker");
    let live_native = Root::new(&h, unsafe { markers[3].value(&h) }, "live native marker");
    let data = old(&h, ForeignKind::Vectorlike, vec![], ForeignFlags::NONE);
    let mut entries = Vec::new();
    for word in markers.iter().chain([&data]) {
        entries.push(RawValue::from_value(
            h.cons(unsafe { word.value(&h) }, h.fixnum(1).unwrap())
                .unwrap(),
        ));
    }
    let tail = h.fixnum(42).unwrap();
    let mut head = tail;
    for entry in entries.iter().rev() {
        head = h.cons(unsafe { entry.value(&h) }, head).unwrap();
    }
    let mut undo = Undo {
        head: RawValue::from_value(head),
        chain: markers.clone(),
        detached: 0,
    };
    h.collect(&mut undo);
    let mut head = unsafe { undo.head.value(&h) };
    for i in [1, 3, 4] {
        let cell = h.as_cons(head).unwrap();
        assert_eq!(RawValue::from_value(cell.car()), entries[i]);
        head = cell.cdr();
    }
    assert_eq!(head.as_fixnum(), Some(42));
    assert_eq!(undo.chain, vec![markers[1], markers[3]]);
    assert_eq!(undo.detached, 2);
    assert_eq!(seen.borrow().drops, [0, 1]);
    drop((live_foreign, live_native));
    h.collect(&mut undo);
    assert_eq!(undo.chain.len(), 0);
    assert_eq!(undo.detached, 4);
    assert_eq!(seen.borrow().drops, [0, 2]);
}
