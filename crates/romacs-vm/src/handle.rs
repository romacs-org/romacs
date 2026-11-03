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
        self.collect(&mut []);
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
        self.profile_collection_begin();
        let start = crate::host::collection_clock();
        self.collect_rooted(roots);
        // What the collection and the work before it freed goes back.
        crate::world::arena::trim();
        let elapsed = start.map_or(0, |start| start.elapsed().as_nanos() as u64);
        self.count_collection(elapsed);
        self.profile_collection_end();
    }
    fn collect_rooted(&mut self, roots: &mut [Value]) {
        let live: Vec<Arc<AtomicU64>> = {
            let slots = self.rooted.slots.get_mut();
            slots.retain(|w| w.strong_count() > 0);
            slots.iter().filter_map(Weak::upgrade).collect()
        };
        if live.is_empty() {
            return self.collect_marked(roots);
        }
        let mut all = roots.to_vec();
        all.extend(live.iter().map(|s| Value(s.load(Ordering::Relaxed))));
        self.collect_marked(&mut all);
        roots.copy_from_slice(&all[..roots.len()]);
        for (slot, value) in live.iter().zip(&all[roots.len()..]) {
            slot.store(value.0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Form, Vm};
    use serde_json::json;

    #[test]
    fn form_object_constants_follow_compaction_and_release_roots() {
        let mut vm = Vm::new();
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

    #[test]
    fn dropped_rooted_slots_are_bounded_without_collection() {
        let mut vm = Vm::new();
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
