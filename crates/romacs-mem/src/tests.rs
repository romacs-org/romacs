use crate::*;
fn isolate(f: impl FnOnce(Heap) + Send + 'static) {
    spawn_isolate(HeapConfig::default(), f).join().unwrap();
}
#[test]
fn configured_heap_symbol_limit_is_typed_and_reusable() {
    spawn_isolate(
        HeapConfig {
            max_symbols: sym::BUILTIN_COUNT as usize + 1,
            ..Default::default()
        },
        |mut h| {
            let name = Root::new(&h, h.make_string(b"bounded", false).unwrap(), "name");
            let symbol = h.make_symbol(name.get(&h)).unwrap();
            let weak = WeakRef::new(&h, symbol);
            assert_eq!(h.make_symbol(name.get(&h)), Err(AllocError::SymbolLimit));
            h.collect(&mut ());
            assert!(weak.upgrade(&h).is_none());
            assert!(h.make_symbol(name.get(&h)).is_ok());
        },
    )
    .join()
    .unwrap();
}

#[test]
fn t0_words() {
    isolate(|h| {
        assert_eq!(std::mem::size_of::<Value<'_>>(), 8);
        assert_eq!(
            std::mem::size_of::<Frame>(),
            if cfg!(feature = "verify") { 24 } else { 16 }
        );
        for n in [Value::MIN_FIXNUM, -1, 0, 1, 2, Value::MAX_FIXNUM] {
            let v = h.fixnum(n).unwrap();
            assert_eq!(v.as_fixnum(), Some(n));
            assert_eq!(v.kind(), Kind::Fixnum);
        }
        assert!(h.fixnum(Value::MAX_FIXNUM + 1).is_none());
        assert!(h.fixnum(Value::MIN_FIXNUM - 1).is_none());
        assert!(
            h.fixnum(Value::MAX_FIXNUM)
                .unwrap()
                .checked_add(h.fixnum(1).unwrap())
                .is_none()
        );
        assert!(sym::NIL.is_nil());
        assert_eq!(sym::T.kind(), Kind::Symbol);
        for (tag, kind) in [
            (0, Kind::Symbol),
            (2, Kind::Fixnum),
            (3, Kind::Cons),
            (4, Kind::String),
            (5, Kind::Vectorlike),
            (6, Kind::Fixnum),
            (7, Kind::Float),
        ] {
            assert_eq!(Value::word(tag).kind(), kind);
        }
    });
}

// A0: actual frame/Args/root types, with a deliberately stub GC boundary.
struct World {
    stack: ValueStack,
    heap: Heap,
    calls: usize,
}
struct Parts<'a> {
    heap: &'a Heap,
    stack: &'a mut ValueStack,
    calls: &'a mut usize,
}
trait Host {
    fn parts(&mut self) -> Parts<'_>;
    fn call(&mut self, frame: Frame) -> Result<(), Flow>;
}
impl World {
    fn new(heap: Heap) -> Self {
        let stack = ValueStack::new(&heap);
        Self {
            heap,
            stack,
            calls: 0,
        }
    }
}
impl Host for World {
    fn parts(&mut self) -> Parts<'_> {
        Parts {
            heap: &self.heap,
            stack: &mut self.stack,
            calls: &mut self.calls,
        }
    }
    fn call(&mut self, frame: Frame) -> Result<(), Flow> {
        type HostFn = fn(&mut World, Frame) -> Result<(), Flow>;
        let table: [HostFn; 2] = [callback, throwing_bytecode];
        let op = frame.get(&self.heap, 0).as_fixnum().unwrap();
        table[op as usize](self, frame)
    }
}
fn funcall(host: &mut dyn Host, frame: Frame) -> Result<(), Flow> {
    host.call(frame)
}
fn pure(args: Args<'_>) -> Result<Value<'_>, Signal<'_>> {
    Ok(args.get(1))
}
fn throwing_bytecode(w: &mut World, f: Frame) -> Result<(), Flow> {
    w.heap.collect(&mut ());
    f.set(&mut w.stack, 0, w.heap.fixnum(77).unwrap());
    Err(Flow::Throw)
}
fn callback(w: &mut World, f: Frame) -> Result<(), Flow> {
    let local = Root::new(&w.heap, f.get(&w.heap, 1), "a0-local");
    let base = w.stack.len();
    w.stack.push(&w.heap, w.heap.fixnum(1).unwrap());
    w.stack.push(&w.heap, local.get(&w.heap));
    let inner = w.stack.frame(base, 2);
    let result = funcall(w, inner); // Host subr -> distinct inner bytecode frame -> throw.
    f.set(&mut w.stack, 0, inner.get(&w.heap, 0));
    w.stack.truncate(base);
    assert_eq!(f.get(&w.heap, 0).as_fixnum(), Some(77)); // outer frame remains valid
    assert_eq!(local.get(&w.heap).as_fixnum(), Some(42));
    assert_eq!(w.heap.epoch(), 1); // GC during callback, local kept through root.
    result
}
#[test]
fn a0_five_scenarios() {
    isolate(|heap| {
        let mut w = World::new(heap);
        w.stack.push(&w.heap, w.heap.fixnum(0).unwrap());
        w.stack.push(&w.heap, w.heap.fixnum(42).unwrap());
        let f = w.stack.frame(0, 2);
        {
            let host: &mut dyn Host = &mut w;
            let p = host.parts();
            *p.calls += 1;
            f.set(p.stack, 0, pure(Args::new(p.heap, f)).unwrap());
        }
        assert_eq!(w.calls, 1);
        assert_eq!(f.get(&w.heap, 0).as_fixnum(), Some(42));
        f.set(&mut w.stack, 0, w.heap.fixnum(0).unwrap());
        assert_eq!(funcall(&mut w, f), Err(Flow::Throw)); // outer bytecode -> table -> callback -> inner bytecode
        assert_eq!(f.get(&w.heap, 0).as_fixnum(), Some(77));
        assert_eq!(w.heap.root_count(), 0);
        drop(w.stack);
    });
}
#[test]
fn frame_checks_and_chunk_growth() {
    isolate(|h| {
        let mut s = ValueStack::new(&h);
        for _ in 0..65_537 {
            s.push(&h, sym::T);
        }
        let f = s.frame(65_536, 1);
        assert_eq!(f.get(&h, 0), sym::T);
        let mut wrong = ValueStack::new(&h);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.set(
                &mut wrong,
                0,
                sym::NIL
            )))
            .is_err()
        );
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.get(&h, 1))).is_err());
        s.truncate(0);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.get(&h, 0))).is_err());
        for _ in 0..65_537 {
            s.push(&h, sym::T);
        }
        #[cfg(feature = "verify")]
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.get(&h, 0))).is_err());
        drop(s);
        let mut next = ValueStack::new(&h);
        next.push(&h, sym::T);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.get(&h, 0))).is_err());
    });
}
#[test]
fn roots_reuse_and_owner() {
    isolate(|mut h| {
        let r = Root::new(&h, h.fixnum(9).unwrap(), "local");
        assert_eq!(r.owner(), "local");
        assert_eq!(h.root_census(), vec![("local", 1)]);
        h.safepoint();
        assert_eq!(r.get(&h).as_fixnum(), Some(9));
        drop(r);
        for _ in 0..100 {
            let _r = Root::new(&h, sym::T, "churn");
        }
        assert_eq!(h.root_count(), 0);
        assert_eq!(h.tables.roots.borrow().len(), 1);
    });
}
