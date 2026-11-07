//! The heap boundary. `Value` is a tagged index into arenas that compaction
//! rewrites, so it is crate-private: no public item may name it (the crate
//! root denies `private_interfaces`). Embedders receive a [`Local`], which
//! holds a shared borrow of the `Vm` that produced it. Every operation that can
//! collect takes `&mut Vm`, so a `Local` cannot be alive across one: holding a
//! result across another evaluation or `collect_garbage` is a borrow error,
//! not silent corruption. A value that must outlive a collection is kept as a
//! [`Rooted`], which the collector traces and rewrites. See `GC.md`,
//! "Heap boundary".
use crate::coordinates::ByteOffset;
use crate::{Result, Value, Vm};
use std::cell::RefCell;
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

/// A Lisp value valid while its `Vm` is borrowed. It cannot be constructed
/// outside this crate, cannot be passed back into a `&mut Vm` call, and every
/// read goes through the `Vm` it came from.
#[derive(Clone, Copy)]
pub struct Local<'vm> {
    value: Value,
    vm: &'vm Vm,
}

impl<'vm> Local<'vm> {
    pub(crate) fn new(vm: &'vm Vm, value: Value) -> Self {
        Self { value, vm }
    }
    /// Keep this value across collections. The collector traces and
    /// rewrites it until the `Rooted` is dropped.
    pub fn root(self) -> Rooted {
        let slot = Arc::new(AtomicU64::new(self.value.0));
        let mut slots = self.vm.rooted.slots.borrow_mut();
        // Dropped handles are otherwise pruned only by `collect`; bound the
        // list when no collection runs (amortized: at each power of two).
        if slots.len() >= 64 && slots.len().is_power_of_two() {
            slots.retain(|w| w.strong_count() > 0);
        }
        slots.push(Arc::downgrade(&slot));
        drop(slots);
        Rooted {
            slot,
            vm: self.vm.rooted.id,
        }
    }
    pub fn is_nil(self) -> bool {
        self.value == Value::NIL
    }
    /// Exactly the symbol `t`.
    pub fn is_t(self) -> bool {
        self.value == Value::TRUE
    }
    pub fn as_integer(self) -> Result<i64> {
        self.value.as_integer()
    }
    pub fn display(self) -> Result<String> {
        self.vm.display(self.value)
    }
    pub fn cons_pair(self) -> Result<(Local<'vm>, Local<'vm>)> {
        let (car, cdr) = self.vm.cons_pair(self.value)?;
        Ok((Local::new(self.vm, car), Local::new(self.vm, cdr)))
    }
    /// Text of a string mapped from an attached image.
    pub fn image_string(self) -> Result<&'vm str> {
        self.vm.string_value(self.value)
    }
    pub fn typed_value(self) -> Result<serde_json::Value> {
        self.vm.typed_value(self.value)
    }
    /// `eq`: the same object (or immediate) of the same `Vm`.
    pub fn eq(self, other: Local<'_>) -> bool {
        std::ptr::eq(self.vm, other.vm) && self.value == other.value
    }
    /// Bytes of a unibyte or multibyte string.
    pub fn bytes(self) -> Result<&'vm [u8]> {
        self.vm.bytes_value(self.value)
    }
    /// Overlays of this buffer covering a UTF-8 byte boundary. The visitor
    /// receives each overlay as a `Local` of the same borrow.
    pub fn visit_overlays_at(
        self,
        position: ByteOffset,
        mut visitor: impl FnMut(Local<'vm>, Range<ByteOffset>),
    ) -> Result<()> {
        let vm = self.vm;
        vm.visit_overlays_at(self.value, position, |o, r| visitor(Local::new(vm, o), r))
    }
    /// Overlays of this buffer intersecting a half-open UTF-8 byte range.
    pub fn visit_overlays_in(
        self,
        range: Range<ByteOffset>,
        mut visitor: impl FnMut(Local<'vm>, Range<ByteOffset>),
    ) -> Result<()> {
        let vm = self.vm;
        vm.visit_overlays_in(self.value, range, |o, r| visitor(Local::new(vm, o), r))
    }
    /// Next overlay endpoint of this buffer strictly after `position`.
    pub fn next_overlay_change_byte(self, position: ByteOffset) -> Result<ByteOffset> {
        self.vm.next_overlay_change_byte(self.value, position)
    }
    /// One property of this overlay by symbol name; nil when absent.
    pub fn overlay_property_named(self, key: &str) -> Result<Local<'vm>> {
        Ok(Local::new(
            self.vm,
            self.vm.overlay_property_named(self.value, key)?,
        ))
    }
}

impl PartialEq for Local<'_> {
    fn eq(&self, other: &Self) -> bool {
        Local::eq(*self, *other)
    }
}

impl fmt::Debug for Local<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The tagged bits, as `Value`'s Debug printed them: an observation
        // only. No public constructor accepts bits, so this cannot forge.
        f.debug_tuple("Local").field(&self.value).finish()
    }
}

impl PartialEq<Fixed> for Local<'_> {
    fn eq(&self, other: &Fixed) -> bool {
        self.value == other.0
    }
}

impl PartialEq<Local<'_>> for Fixed {
    fn eq(&self, other: &Local<'_>) -> bool {
        self.0 == other.value
    }
}

/// A value that no collection can move: `nil`, `t`, a fixnum, or a reference
/// into an immutable mapped image. It names no private heap object, so it may
/// be held for any length of time and passed back into the VM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fixed(Value);

impl Fixed {
    pub const NIL: Self = Self(Value::NIL);
    pub const T: Self = Self(Value::TRUE);
    pub fn integer(n: i64) -> Result<Self> {
        Value::integer(n).map(Self)
    }
    pub fn as_integer(self) -> Result<i64> {
        self.0.as_integer()
    }
    /// Executable-image references only: the IMAGE index bit is never a
    /// private index, and a frozen region (`frozen.rs`) can be reclaimed, so
    /// no `Fixed` may name one.
    pub(crate) fn image(value: Value) -> Self {
        debug_assert!(
            Vm::image_value(value) && crate::frozen::region_of(value.payload() as usize) == 0,
            "Fixed::image requires an executable-image reference"
        );
        Self(value)
    }
}

/// An argument passed into the VM: a value that cannot move, or a rooted one
/// (checked against the receiving VM).
#[derive(Clone, Copy, Debug)]
pub enum Arg<'r> {
    Fixed(Fixed),
    Rooted(&'r Rooted),
}

impl From<Fixed> for Arg<'_> {
    fn from(value: Fixed) -> Self {
        Arg::Fixed(value)
    }
}

impl<'r> From<&'r Rooted> for Arg<'r> {
    fn from(value: &'r Rooted) -> Self {
        Arg::Rooted(value)
    }
}

/// Payload of the hidden `Form::Object` variant: a heap object carried by a
/// macro expansion inside the crate. Not constructible or readable outside.
#[derive(Clone, Debug)]
pub struct FormObject(pub(crate) Arc<Rooted>);
romacs_snapshot::snapshot_tuple!(FormObject(f0));

impl FormObject {
    pub(crate) fn new(vm: &Vm, value: Value) -> Self {
        Self(Arc::new(Local::new(vm, value).root()))
    }

    pub(crate) fn value(&self, vm: &Vm) -> Result<Value> {
        Ok(self.0.get(vm)?.value)
    }
}

impl Vm {
    /// Collect now. No embedder-held `Local` can exist across this call, so
    /// there is nothing outside the VM to root or rewrite.
    pub fn collect_garbage(&mut self) {
        self.collection_log.explicit = true;
        self.collect(&mut []);
    }
    /// Collect now and compact the tables: every live entry moves down and
    /// no entry is left free. A capture takes this (`publish_world'); other
    /// collections free in place and compact only on demand (gc.rs
    /// `compacts').
    pub fn collect_compacting(&mut self) {
        self.free.compact = true;
        self.collect(&mut []);
    }
    /// The command loop's idle collection: as `collect_garbage', but
    /// abandoned, with nothing collected or counted, if the host has a key
    /// press queued while it is still marking, which changes nothing but
    /// caches: the key waits for no collection that has not begun moving
    /// the heap. Whether it collected.
    pub fn collect_garbage_until_key(&mut self) -> bool {
        self.collect_until_key(&mut [], true)
    }
    /// Evaluate a UTF-8 source file sequentially in this persistent strand.
    /// This call may mark and compact; the result borrows the VM.
    pub fn eval_source(&mut self, source: &str) -> Result<Local<'_>> {
        let value = self.eval_source_raw(source)?;
        Ok(Local::new(self, value))
    }
    pub fn eval_form(&mut self, form: &crate::Form) -> Result<Local<'_>> {
        let value = self.eval_form_raw(form)?;
        Ok(Local::new(self, value))
    }
    /// Execute a live zero-argument function. Invalid or retired IDs return an
    /// `"invalid function"` error; IDs returned by `compile` remain live.
    pub fn execute(&mut self, function: usize) -> Result<Local<'_>> {
        let value = self.execute_raw(function)?;
        Ok(Local::new(self, value))
    }
    pub fn load_file(&mut self, path: &std::path::Path) -> Result<Local<'_>> {
        let value = self.load_file_raw(path)?;
        Ok(Local::new(self, value))
    }
    pub fn install_lisp_prelude(&mut self) -> Result<Local<'_>> {
        let value = self.install_lisp_prelude_raw()?;
        Ok(Local::new(self, value))
    }
    pub fn invoke_prepared(
        &mut self,
        handle: crate::PreparedHandle,
        args: &[Arg<'_>],
    ) -> Result<Local<'_>> {
        let args = args
            .iter()
            .map(|a| match a {
                Arg::Fixed(f) => Ok(f.0),
                Arg::Rooted(r) => r.get(self).map(|l| l.value),
            })
            .collect::<Result<Vec<Value>>>()?;
        let value = self.invoke_prepared_raw(handle, &args)?;
        Ok(Local::new(self, value))
    }
    /// Read one datum, returning its UTF-8 byte end offset. Comments and
    /// whitespace alone return None.
    pub fn read_source(&mut self, source: &str) -> Result<Option<(Local<'_>, usize)>> {
        let read = self.read_source_raw(source)?;
        Ok(read.map(|(value, end)| (Local::new(self, value), end)))
    }
    pub fn declare_special(&mut self, name: &str, value: Fixed) {
        self.declare_special_raw(name, value.0);
    }
    /// Bind a special variable to a rooted heap value of this VM.
    pub fn declare_special_rooted(&mut self, name: &str, value: &Rooted) -> Result<()> {
        let value = value.get(self)?.value;
        self.declare_special_raw(name, value);
        Ok(())
    }
}

impl crate::ImageRegion {
    pub fn reference(&self, index: usize) -> Result<Fixed> {
        self.reference_raw(index).map(Fixed::image)
    }
    pub fn string_reference(&self, index: usize) -> Result<Fixed> {
        self.string_reference_raw(index).map(Fixed::image)
    }
}

impl crate::ImageBuilder {
    pub fn string(&mut self, text: &str) -> Fixed {
        Fixed::image(self.string_raw(text))
    }
    pub fn cons(&mut self, car: Fixed, cdr: Fixed) -> Fixed {
        Fixed::image(self.cons_raw(car.0, cdr.0))
    }
}

static NEXT_VM: AtomicU64 = AtomicU64::new(1);

/// The VM side of `Rooted`: weak references to every live handle's slot.
/// Only `collect` reads it; a dropped handle's weak entry is pruned there.
pub(crate) struct RootSet {
    id: u64,
    slots: RefCell<Vec<Weak<AtomicU64>>>,
}
romacs_snapshot::snapshot_struct!(RootSet { id, slots });

impl RootSet {
    /// No embedder handle is live.
    pub(crate) fn is_empty(&self) -> bool {
        self.slots.borrow().iter().all(|slot| slot.strong_count() == 0)
    }
}

impl Default for RootSet {
    fn default() -> Self {
        Self {
            id: NEXT_VM.fetch_add(1, Ordering::Relaxed),
            slots: RefCell::default(),
        }
    }
}

/// A value kept alive and current across collections. It holds no borrow, so
/// it can be stored anywhere; reading it requires the `Vm` it belongs to,
/// which yields an ordinary `Local`. Dropping it releases the root.
pub struct Rooted {
    slot: Arc<AtomicU64>,
    vm: u64,
}
romacs_snapshot::snapshot_struct!(Rooted { slot, vm });

impl Rooted {
    pub fn get<'vm>(&self, vm: &'vm Vm) -> Result<Local<'vm>> {
        if vm.rooted.id != self.vm {
            return Err("rooted value belongs to another VM".into());
        }
        Ok(Local::new(vm, Value(self.slot.load(Ordering::Relaxed))))
    }
}

impl fmt::Debug for Rooted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Rooted(..)")
    }
}

impl Vm {
    /// Every collection, automatic or explicit, enters here. Live `Rooted`
    /// slots are appended to the caller's roots and written back forwarded.
    pub(crate) fn collect(&mut self, roots: &mut [Value]) {
        self.collect_until_key(roots, false);
    }
    /// Collect, or with UNTIL_KEY stop while marking once the host has a key
    /// press queued, leaving the heap as it was: whether it collected.
    pub(crate) fn collect_until_key(&mut self, roots: &mut [Value], until_key: bool) -> bool {
        self.profile_collection_begin();
        let start = crate::host::collection_clock();
        let last = std::mem::take(&mut self.gc_times);
        let collected = self.collect_rooted(roots, until_key);
        if !collected {
            self.gc_times = last;
        }
        if collected {
            let mut lap = crate::gc::Lap::start();
            // What the collection and the work before it freed goes back.
            crate::world::arena::trim();
            lap.lap(&mut self.gc_times, crate::gc::Phase::Trim);
            let elapsed = start.map_or(0, |start| start.elapsed().as_nanos() as u64);
            self.count_collection(elapsed);
            self.note_collection(elapsed, until_key);
        }
        self.profile_collection_end();
        collected
    }
    fn collect_rooted(&mut self, roots: &mut [Value], until_key: bool) -> bool {
        let live: Vec<Arc<AtomicU64>> = {
            let slots = self.rooted.slots.get_mut();
            slots.retain(|w| w.strong_count() > 0);
            slots.iter().filter_map(Weak::upgrade).collect()
        };
        if live.is_empty() {
            return self.collect_marked(roots, until_key);
        }
        let mut all = roots.to_vec();
        all.extend(live.iter().map(|s| Value(s.load(Ordering::Relaxed))));
        if !self.collect_marked(&mut all, until_key) {
            return false;
        }
        roots.copy_from_slice(&all[..roots.len()]);
        for (slot, value) in live.iter().zip(&all[roots.len()..]) {
            slot.store(value.0, Ordering::Relaxed);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::{Form, Vm};
    use serde_json::json;

    #[test]
    fn form_object_constants_follow_compaction_and_release_roots() {
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
        for _ in 0..64 {
            vm.make_lisp_string(b"garbage".to_vec(), true);
        }
        let value = vm.make_lisp_string(b"retained".to_vec(), true);
        let form = super::FormObject::new(&vm, value);
        let clone = form.clone();
        vm.collect_garbage();
        let moved = clone.value(&vm).unwrap();
        assert_ne!(moved, value, "fixture must actually move the object");
        assert_eq!(vm.source_string(moved).unwrap(), "retained");
        drop(form);
        vm.collect_garbage();
        assert_eq!(vm.source_string(clone.value(&vm).unwrap()).unwrap(), "retained");
        drop(clone);
        vm.collect_garbage();
        assert!(vm.rooted.slots.borrow().is_empty());
    }

    /// A host whose key press shows from the Nth look on.
    struct KeyAfter(usize);
    impl crate::HostInput for KeyAfter {
        fn input_pending(&mut self) -> bool { false }
        fn key_pending(&mut self) -> bool {
            self.0 = self.0.saturating_sub(1);
            self.0 == 0
        }
        fn redisplay(&mut self, _: crate::HostFrame) -> std::result::Result<Vec<crate::WindowEnd>, String> { Ok(vec![]) }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> std::result::Result<crate::HostEvent, String> {
            Err("no events".into())
        }
    }

    /// A collection a key press abandoned is no collection: the report
    /// stays the last completed one's.
    #[test]
    fn an_abandoned_collection_keeps_the_last_report() {
        let mut vm = Vm::new();
        let mut kept = [vm.alloc_cons(crate::Value::NIL, crate::Value::NIL)];
        vm.collect(&mut kept);
        let before = vm.gc_times.entries;
        assert_ne!(before, [0; 4], "a report");
        vm.set_host_input(Some(Box::new(KeyAfter(1))));
        assert!(!vm.collect_garbage_until_key());
        assert_eq!(vm.gc_times.entries, before);
        vm.set_host_input(None);
    }

    /// An idle collection gives way to a key press that arrives while it
    /// marks, in a small heap and a large one: nothing is collected, counted
    /// or moved, and a later collection collects as ever.
    #[test]
    fn an_idle_collection_gives_way_to_a_key_press() {
        let abandoned = |vm: &mut Vm, looks| {
            vm.set_host_input(Some(Box::new(KeyAfter(looks))));
            let (objects, collections) = (vm.heap_objects(), vm.heap_stats().collections);
            assert!(!vm.collect_garbage_until_key(), "collected with a key press queued");
            assert_eq!((vm.heap_objects(), vm.heap_stats().collections), (objects, collections), "an abandoned collection changed the heap");
            vm.set_host_input(None);
        };
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.eval_source_raw("(dotimes (_ 2000) (cons 1 2))").unwrap();
        abandoned(&mut vm, 1);
        // Live values a collection must keep through an abandoned one.
        let kept = vm.eval_source("(setq kept (make-list 100000 (list 1 2)))").unwrap().root();
        vm.eval_source_raw("(dotimes (_ 20000) (cons 1 2))").unwrap();
        abandoned(&mut vm, 1);
        let objects = vm.heap_objects();
        assert!(vm.collect_garbage_until_key(), "no key, no collection");
        assert!(vm.heap_objects() < objects, "garbage stayed");
        assert_eq!(kept.get(&vm).unwrap().display().unwrap().len(), "(".len() + 100_000 * "(1 2) ".len());
        assert_eq!(vm.eval_source("(list (romacs--collect-idle) (length kept))").unwrap().display().unwrap(), "(t 100000)");
    }

    /// Each collection leaves one record, and a full log only counts.
    #[test]
    fn collections_leave_records_for_the_usage_log() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        let take = |vm: &mut Vm| {
            let taken = vm.take_collections();
            vm.display(taken).unwrap().split_whitespace().map(str::to_owned).collect::<Vec<_>>().join(" ")
        };
        take(&mut vm);
        // Durations vary: only the record's shape is compared.
        let shape = |text: String| text.split(' ').enumerate().map(|(i, w)| if i % 4 == 2 { "MS".to_owned() } else { w.to_owned() }).collect::<Vec<_>>().join(" ");
        vm.collect_garbage();
        assert_eq!(shape(take(&mut vm)), "(0 (explicit MS nil nil))");
        vm.set_host_input(Some(Box::new(KeyAfter(usize::MAX))));
        assert!(vm.collect_garbage_until_key());
        assert_eq!(shape(take(&mut vm)), "(0 (idle MS nil nil))");
        // The key arrives after the last point the collection gives way.
        vm.set_host_input(Some(Box::new(KeyAfter(2))));
        assert!(vm.collect_garbage_until_key());
        assert_eq!(shape(take(&mut vm)), "(0 (idle MS key nil))");
        vm.set_host_input(None);
        vm.eval_source_raw("(setq real-this-command 'forward-char)").unwrap();
        take(&mut vm);
        vm.collect(&mut []);
        assert_eq!(shape(take(&mut vm)), r#"(0 (automatic MS command "forward-char"))"#);
        // The loop's tail has cleared the running command.
        vm.eval_source_raw("(setq real-this-command nil real-last-command 'next-line)").unwrap();
        take(&mut vm);
        vm.collect(&mut []);
        assert_eq!(shape(take(&mut vm)), r#"(0 (automatic MS command "next-line"))"#);
        // Idle again: the command has ended, whatever the variable holds.
        vm.idle_since = Some(crate::host::monotonic());
        vm.set_host_input(Some(Box::new(KeyAfter(usize::MAX))));
        vm.collect(&mut []);
        assert_eq!(shape(take(&mut vm)), "(0 (automatic MS nil nil))");
        vm.set_host_input(None);
        for _ in 0..crate::gc::COLLECTION_RECORDS + 44 {
            vm.collect(&mut []);
        }
        let taken = take(&mut vm);
        assert!(taken.starts_with("(44 (automatic"), "{}", &taken[..40]);
        assert_eq!(taken.matches("automatic").count(), crate::gc::COLLECTION_RECORDS);
        assert_eq!(take(&mut vm), "(0)", "taken means emptied");
    }

    /// A host that counts its looks for a key press and never has one.
    struct Looks(std::rc::Rc<std::cell::Cell<usize>>);
    impl crate::HostInput for Looks {
        fn input_pending(&mut self) -> bool { false }
        fn key_pending(&mut self) -> bool {
            self.0.set(self.0.get() + 1);
            false
        }
        fn redisplay(&mut self, _: crate::HostFrame) -> std::result::Result<Vec<crate::WindowEnd>, String> { Ok(vec![]) }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> std::result::Result<crate::HostEvent, String> {
            Err("no events".into())
        }
    }

    /// A key press at an idle collection's last look, after its mark found
    /// a finalizer unreachable, leaves the finalizer where it was: only a
    /// collection that finishes queues it to run.
    #[test]
    fn an_abandoned_collection_dooms_no_finalizer() {
        let made = || {
            let mut vm = Vm::new();
            vm.set_gc_stress(0);
            vm.eval_source_raw("(progn (make-finalizer (lambda () nil)) nil)").unwrap();
            vm
        };
        // The looks a collection of this heap takes: the record's is the
        // last, and the one before it follows the mark.
        let looks = {
            let mut vm = made();
            let seen = std::rc::Rc::new(std::cell::Cell::new(0));
            vm.set_host_input(Some(Box::new(Looks(seen.clone()))));
            assert!(vm.collect_garbage_until_key());
            assert_eq!(vm.doomed_finalizers.len(), 1, "the fixture's finalizer is unreachable");
            seen.get() - 1
        };
        let mut vm = made();
        vm.set_host_input(Some(Box::new(KeyAfter(looks))));
        assert!(!vm.collect_garbage_until_key());
        assert_eq!((vm.finalizers.len(), vm.doomed_finalizers.len()), (1, 0), "an abandoned collection doomed a finalizer");
        vm.set_host_input(None);
        assert!(vm.collect_garbage_until_key());
        assert_eq!((vm.finalizers.len(), vm.doomed_finalizers.len()), (0, 1));
    }

    #[test]
    fn dropped_rooted_slots_are_bounded_without_collection() {
        let mut vm = Vm::new();
        // Counts collections, so no forced ones (`ROMACS_GC_STRESS`).
        vm.set_gc_stress(0);
        vm.set_collection_floor(usize::MAX);
        let before = vm.heap_stats().collections;
        for _ in 0..10_000 {
            drop(vm.eval_source("nil").unwrap().root());
        }
        assert_eq!(
            vm.heap_stats().collections,
            before,
            "fixture must not collect"
        );
        let slots = vm.rooted.slots.borrow().len();
        assert!(
            slots <= 128,
            "dropped Rooted slots retained without collection: {slots}"
        );
    }

    // Moved from tests/executor.rs: the public API can no longer carry one
    // VM's object into another (`Rooted::get` refuses), so the entry
    // validation behind it is exercised here with raw values.
    #[test]
    fn foreign_arena_reference_cannot_bypass_entry_validation() {
        let mut owner = Vm::new();
        let f = owner
            .compile(&Form::from_json(json!(["lambda", [], 42])).unwrap())
            .unwrap();
        let closure = owner.execute_raw(f).unwrap();
        for args in [
            json!(["funcall", "foreign"]),
            json!(["funcall", "foreign", 1, 2, 3]),
        ] {
            let mut other = Vm::new();
            other.declare_special_raw("foreign", closure);
            let f = other.compile(&Form::from_json(args).unwrap()).unwrap();
            assert!(
                other
                    .execute_raw(f)
                    .unwrap_err()
                    .contains("invalid closure")
            );
        }
    }
}
