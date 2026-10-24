//! Complete steps 1-8 benchmark; timings informational, no wall-clock test gate.
use romacs_mem::{raw::*, *};
#[repr(C)]
struct Old {
    header: [u8; 16],
    pairs: Vec<(RawValue, RawValue)>,
}
// SAFETY: reports all weak entries, removal preserves count/order; no heap/Lisp access.
unsafe impl LegacyWeak for Old {
    fn weakness(&self) -> Weakness {
        Weakness::Key
    }
    fn entries(&self, f: &mut dyn FnMut(RawValue, RawValue)) {
        for &(k, v) in &self.pairs {
            f(k, v)
        }
    }
    fn remove_dead(&mut self, keep: &dyn Fn(RawValue, RawValue) -> bool) -> u32 {
        let n = self.pairs.len();
        self.pairs.retain(|&(k, v)| keep(k, v));
        (n - self.pairs.len()) as u32
    }
}
// SAFETY: no strong fields; all pairs belong to containing heap; no interior borrows.
struct Hooks;
// SAFETY: cells contain initialized Old; all pairs are weak and no heap access occurs.
unsafe impl ForeignHooks for Hooks {
    fn trace(&self, _: RawValue, _: &mut Marker<'_>) {}
    fn with_weak(&mut self, w: RawValue, f: &mut dyn FnMut(&mut dyn LegacyWeak)) {
        f(unsafe { &mut *((w.bits() & !7) as *mut Old) });
    }
    fn finalizer_function(&self, _: RawValue) -> RawValue {
        unreachable!()
    }
    fn drop(&mut self, w: RawValue) {
        unsafe {
            std::ptr::drop_in_place((w.bits() & !7) as *mut Old);
        }
    }
    fn bytes(&self, w: RawValue) -> usize {
        unsafe { &*((w.bits() & !7) as *const Old) }
            .pairs
            .capacity()
            * 16
    }
    fn symbol_pinned(&self, _: RawValue) -> bool {
        false
    }
}
struct Bytes;
impl Payload for Bytes {
    fn bytes(&self) -> usize {
        0
    }
}
struct Owner {
    undo: RawValue,
    markers: Vec<RawValue>,
}
impl Trace for Owner {
    fn trace(&self, _: &mut Marker<'_>) {}
    fn undo_heads(&mut self, f: &mut dyn FnMut(&mut RawValue)) {
        f(&mut self.undo)
    }
    fn unchain_markers(&mut self, marked: &dyn Fn(RawValue) -> bool) {
        self.markers.retain(|v| marked(*v))
    }
}
fn main() {
    spawn_isolate(HeapConfig::default(),|mut heap| {
        unsafe {heap.set_foreign_hooks(Box::new(Hooks));}
        let mut head=heap.nil();
        for _ in 0..4096 {
            let s=heap.make_string(&[b'x';1024],false).unwrap();
            let v=heap.make_vectorlike(PvecType::NormalVector,&[s,heap.fixnum(17).unwrap()],&[]).unwrap();
            head=heap.cons(v,head).unwrap();
        }
        let strong=Root::new(&heap,head,"bench strong");
        let mut times=Vec::new(); let mut live=0;
        for _ in 0..7 {
            let mut keep=Vec::new(); let mut pairs=Vec::new(); let mut undo=heap.nil(); let mut markers=Vec::new();
            let native=heap.make_weak_pairs(Weakness::Key,48).unwrap();
            for i in 0..48 {
                let key=heap.cons(heap.nil(),heap.nil()).unwrap();
                let value=heap.make_vectorlike(PvecType::NormalVector,&[heap.fixnum(i).unwrap()],&[]).unwrap();
                if i<16 {keep.push(Root::new(&heap,key,"bench key"));}
                if i>=32 {heap.make_finalizer(key).unwrap();}
                heap.as_weak_pairs(native).unwrap().set(i as usize,key,value);
                pairs.push((RawValue::from_value(key),RawValue::from_value(value)));
            }
            let native=Root::new(&heap,native,"bench native");
            // SAFETY: these words belong to heap; Old obeys the weak adapter contract.
            let old=unsafe {
                let w=heap.alloc_foreign(ForeignKind::Vectorlike,std::mem::size_of::<Old>(),ForeignFlags::WEAK_TABLE).unwrap();
                let ptr=(w.bits() & !7) as *mut Old;
                std::ptr::addr_of_mut!((*ptr).pairs).write(pairs);
                w.value(&heap)
            };
            let old=Root::new(&heap,old,"bench legacy");
            for i in 0..16 {
                let marker=heap.make_vectorlike(PvecType::Marker,&[],&[]).unwrap(); markers.push(RawValue::from_value(marker));
                if i%2==0 {keep.push(Root::new(&heap,marker,"bench marker"));}
                let entry=heap.cons(marker,heap.fixnum(i).unwrap()).unwrap(); undo=heap.cons(entry,undo).unwrap();
                heap.make_payload(PvecType::Other,&[],Box::new(Bytes)).unwrap();
            }
            let mut owner=Owner{undo:RawValue::from_value(undo),markers};
            let report=heap.collect(&mut owner);
            assert!(report.freed_objects>=16 && report.dropped_payloads>=16);
            assert_eq!(heap.as_weak_pairs(native.get(&heap)).unwrap().len(),32);
            assert_eq!(owner.markers.len(),8);
            times.push(report.record.core_stop_ns); live=report.record.accounted_after;
            assert_eq!(heap.run_finalizers(|_,_|{}),16);
            drop((native,old,keep));
        }
        times.sort_unstable();
        println!("complete steps 1-8: live={live} median_ns={} ns_per_live_mib={:.0} cycles=7 weak_native=48 weak_legacy=48 undo=16 doomed=16 eager_drops>=16",times[3],times[3] as f64/(live as f64/1048576.0));
        drop(strong);assert!(heap.collect(&mut ()).freed_objects>0);
    }).join().unwrap();
}
