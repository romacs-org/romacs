use super::*;
use crate::{HeapConfig, spawn_isolate, sym};
use std::{
    mem::{align_of, offset_of, size_of},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
fn native_layouts() {
    assert_eq!(size_of::<Slot>(), 8);
    assert_eq!(align_of::<Slot>(), 8);
    assert_eq!(size_of::<Cons>(), 16);
    assert_eq!(offset_of!(Cons, cdr), 8);
    assert_eq!(size_of::<f64>(), 8);
    assert_eq!(size_of::<LispString>(), 48);
    assert_eq!(offset_of!(LispString, data), 24);
    assert_eq!(offset_of!(LispString, cap), 32);
    assert_eq!(size_of::<Symbol>(), 48);
    assert_eq!(offset_of!(Symbol, flags), 40);
    assert_eq!(size_of::<VectorHeader>(), 16);
    assert_eq!(offset_of!(VectorHeader, lisp_slots), 4);
    assert_eq!(offset_of!(VectorHeader, raw_bytes), 8);
    assert_eq!(size_of::<Box<dyn Payload>>(), 16);
}
#[test]
fn typed_roundtrip_and_prefix_tail() {
    spawn_isolate(HeapConfig::default(), |h| {
        let one = h.fixnum(1).unwrap();
        let c = h.cons(one, h.nil()).unwrap();
        let r = h.as_cons(c).unwrap();
        assert_eq!(r.car(), one);
        r.set_car(sym::T);
        r.set_cdr(c);
        assert_eq!(r.car(), sym::T);
        assert_eq!(r.cdr(), c);
        assert!(h.as_string(c).is_none());
        let f = h.float(-0.0).unwrap();
        assert_eq!(h.as_float(f).unwrap().to_bits(), (-0.0_f64).to_bits());
        let s = h.make_string("αβ".as_bytes(), true).unwrap();
        let string = h.as_string(s).unwrap();
        assert_eq!(string.chars(), 2);
        assert_eq!(string.bytes(), "αβ".as_bytes());
        assert!(string.multibyte());
        string.set_intervals(c);
        assert_eq!(string.intervals(), c);
        let empty = h.make_string(&[], false).unwrap();
        assert!(h.as_string(empty).unwrap().bytes().is_empty());
        assert!(h.make_string(&[255], true).is_err());
        let symbol = h.make_symbol(s).unwrap();
        let sr = h.as_symbol(symbol).unwrap();
        assert_eq!(sr.name(), s);
        sr.set_value(c);
        sr.set_function(f);
        sr.set_plist(s);
        sr.set_next(sym::T);
        sr.set_flags_aux(0x12345678, 0x87654321);
        assert_eq!(
            (sr.value(), sr.function(), sr.plist(), sr.next()),
            (c, f, s, sym::T)
        );
        assert_eq!((sr.flags(), sr.aux()), (0x12345678, 0x87654321));
        assert!(h.make_symbol(c).is_err());
        let tail = [0xff; 19];
        let v = h
            .make_vectorlike(PvecType::Record, &[c, s, symbol], &tail)
            .unwrap();
        let vr = h.as_vectorlike(v).unwrap();
        assert_eq!(vr.pvec_type(), PvecType::Record);
        assert_eq!((vr.len(), vr.raw_len()), (3, 19));
        assert_eq!(vr.raw_bytes(), Some(tail.as_slice()));
        assert_eq!(vr.get(2), Some(symbol));
        assert!(vr.set(1, f));
        assert_eq!(vr.get(1), Some(f));
        assert!(!vr.set(3, c));
        assert_eq!(vr.get(3), None);
        assert_eq!(vr.raw_bytes(), Some(tail.as_slice()));
        assert!(h.make_vectorlike(PvecType::Finalizer, &[], &[]).is_err());
        let large = h
            .make_vectorlike(PvecType::NormalVector, &vec![c; 2048], &[42; 7])
            .unwrap();
        let lr = h.as_vectorlike(large).unwrap();
        assert_eq!(lr.get(2047), Some(c));
        assert_eq!(lr.raw_bytes(), Some([42; 7].as_slice()));
        assert_eq!(h.allocation_stats().counts.string_chars, 2);
    })
    .join()
    .unwrap();
}
struct Data(Arc<AtomicUsize>);
impl Payload for Data {
    fn bytes(&self) -> usize {
        71
    }
}
impl Drop for Data {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}
#[test]
fn payload_tail_owns_one_box() {
    let drops = Arc::new(AtomicUsize::new(0));
    let count = drops.clone();
    spawn_isolate(HeapConfig::default(), move |h| {
        let value = h
            .make_payload(PvecType::UserPtr, &[sym::T], Box::new(Data(count)))
            .unwrap();
        let r = h.as_vectorlike(value).unwrap();
        assert_eq!(r.get(0), Some(sym::T));
        assert_eq!(r.payload().unwrap().bytes(), 71);
        assert!(r.raw_bytes().is_none());
        assert!(r.set(0, h.nil()));
        assert_eq!(r.payload().unwrap().bytes(), 71);
    })
    .join()
    .unwrap();
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}
struct WeakOld(Vec<(raw::RawValue, raw::RawValue)>);
// SAFETY: this fixture enumerates all stored pairs and retains their order/count.
unsafe impl raw::LegacyWeak for WeakOld {
    fn weakness(&self) -> raw::Weakness {
        raw::Weakness::Key
    }
    fn entries(&self, visit: &mut dyn FnMut(raw::RawValue, raw::RawValue)) {
        for &(key, value) in &self.0 {
            visit(key, value);
        }
    }
    fn remove_dead(&mut self, keep: &dyn Fn(raw::RawValue, raw::RawValue) -> bool) -> u32 {
        let before = self.0.len();
        self.0.retain(|&(key, value)| keep(key, value));
        (before - self.0.len()) as u32
    }
}
// SAFETY: no strong edges, weak entries exposed only through the adapter.
impl crate::foreign_tests::Fixture for WeakOld {
    fn trace(&self, _: &mut raw::Marker<'_>) {}
    fn weak(&mut self) -> Option<&mut dyn raw::LegacyWeak> {
        Some(self)
    }
}
#[test]
fn weak_adapter_contract_and_allocation_gate() {
    use raw::LegacyWeak;
    spawn_isolate(HeapConfig::default(), |mut h| {
        crate::foreign_tests::install_fixture::<WeakOld>(&mut h);
        let nil = raw::RawValue::from_value(h.nil());
        let t = raw::RawValue::from_value(sym::T);
        let mut old = WeakOld(vec![(nil, t), (t, nil)]);
        let mut pairs = Vec::new();
        old.entries(&mut |k, v| pairs.push((k, v)));
        assert_eq!(pairs, old.0);
        assert_eq!(old.weakness(), raw::Weakness::Key);
        assert_eq!(old.remove_dead(&|k, _| k == t), 1);
        assert_eq!(old.0, vec![(t, nil)]);
        // SAFETY: adapter obeys the contract; S5 processes its weak entries.
        let value = crate::foreign_tests::place_fixture(&h, old, raw::ForeignFlags::WEAK_TABLE);
        assert_eq!(h.as_vectorlike(value).unwrap().pvec_type(), PvecType::Other);
    })
    .join()
    .unwrap();
}
struct Old {
    edge: raw::RawValue,
    drops: Arc<AtomicUsize>,
}
// SAFETY: test payload reports its only strong edge and has no weak entries.
impl crate::foreign_tests::Fixture for Old {
    fn trace(&self, marker: &mut raw::Marker<'_>) {
        marker.mark(self.edge);
    }
}
impl Drop for Old {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}
#[test]
fn legacy_closure_traces_code_and_payload_edges() {
    let drops = Arc::new(AtomicUsize::new(0));
    let count = drops.clone();
    spawn_isolate(HeapConfig::default(), move |mut h| {
        crate::foreign_tests::install_fixture::<Old>(&mut h);
        let edge = h.cons(h.nil(), h.nil()).unwrap();
        let weak_edge = crate::WeakRef::new(&h, edge);
        let code = h.make_code_block(b"opaque-code").unwrap();
        let weak_code = crate::WeakRef::new(&h, code);
        let graph = h.cons(edge, code).unwrap();
        let closure = crate::foreign_tests::place_fixture(
            &h,
            Old {
                edge: raw::RawValue::from_value(graph),
                drops: count,
            },
            raw::ForeignFlags::NONE,
        );
        let root = crate::Root::new(&h, closure, "foreign closure");
        assert!(h.as_vectorlike(closure).unwrap().code().is_none());
        h.collect(&mut ());
        assert!(weak_edge.upgrade(&h).is_some());
        assert!(weak_code.upgrade(&h).is_some());
        drop(root);
        h.collect(&mut ());
        assert!(weak_edge.upgrade(&h).is_none());
        assert!(weak_code.upgrade(&h).is_none());
    })
    .join()
    .unwrap();
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn code_block_pointer_shaped_bytes_do_not_retain_object() {
    spawn_isolate(HeapConfig::default(), |mut h| {
        let victim = h.cons(h.nil(), h.nil()).unwrap();
        let weak = crate::WeakRef::new(&h, victim);
        let bytes = victim.0.to_ne_bytes();
        let closure = h
            .make_vectorlike(PvecType::Closure, &[h.nil(); 4], &[])
            .unwrap();
        let root = crate::Root::new(&h, closure, "opaque-closure");
        assert!(
            h.as_vectorlike(closure)
                .unwrap()
                .set_code(h.make_code_block(&bytes).unwrap())
        );
        h.collect(&mut ());
        assert!(weak.upgrade(&h).is_none());
        let code = h.as_vectorlike(root.get(&h)).unwrap().code().unwrap();
        assert_eq!(
            h.as_vectorlike(code).unwrap().raw_bytes(),
            Some(bytes.as_slice())
        );
    })
    .join()
    .unwrap();
}

#[test]
fn legacy_adapter_reports_edges_and_drops() {
    let drops = Arc::new(AtomicUsize::new(0));
    let count = drops.clone();
    spawn_isolate(HeapConfig::default(), move |mut h| {
        crate::foreign_tests::install_fixture::<Old>(&mut h);
        let edge = raw::RawValue::from_value(h.cons(h.nil(), h.nil()).unwrap());
        // SAFETY: edge belongs to h; Old obeys the documented adapter contract.
        let value = crate::foreign_tests::place_fixture(
            &h,
            Old { edge, drops: count },
            raw::ForeignFlags::NONE,
        );
        let r = h.as_vectorlike(value).unwrap();
        assert!(r.raw_bytes().is_none() && r.payload().is_none());
        let mut work = Vec::new();
        h.edges(value.0, &mut work);
        assert_eq!(work, vec![edge]);
    })
    .join()
    .unwrap();
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}
