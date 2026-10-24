//! Native layouts. Only initialized objects are published as borrowed values.
#[cfg(test)]
use crate::raw;
use crate::{AllocError, Heap, Kind, PvecType, Value};
use std::{
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};

/// Lisp edge. All Stage A accesses are relaxed, with no barriers.
#[repr(transparent)]
pub struct Slot(AtomicU64);
impl Slot {
    pub(crate) fn new(value: Value<'_>) -> Self {
        Self(AtomicU64::new(value.0))
    }
    pub(crate) fn word(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
    #[inline]
    pub fn get<'h>(&self, heap: &'h Heap) -> Value<'h> {
        let word = self.0.load(Ordering::Relaxed);
        heap.assert_value(word);
        Value::word(word)
    }
    #[inline]
    pub fn set(&self, heap: &Heap, value: Value<'_>) {
        heap.assert_value(value.0);
        self.0.store(value.0, Ordering::Relaxed);
    }
}
#[repr(C)]
pub(crate) struct Cons {
    pub car: Slot,
    pub cdr: Slot,
}
#[repr(C)]
pub(crate) struct LispString {
    pub intervals: Slot,
    pub size: usize,
    pub size_byte: usize,
    pub data: *mut u8,
    pub cap: usize,
    pub multibyte: bool,
}
const UNBOUND: u64 = 1 << 6;
#[repr(C)]
pub(crate) struct Symbol {
    pub name: Slot,
    pub value: Slot,
    pub function: Slot,
    pub plist: Slot,
    pub next: Slot,
    pub flags: AtomicU64,
}
// The last word is the flags/aux pair. Atomic storage permits updates through
// borrowed typed refs without creating an aliased &mut Symbol.
#[repr(C)]
pub(crate) struct VectorHeader {
    pub pvec: PvecType,
    pub flags: u8,
    pub(crate) reserved: u16,
    pub lisp_slots: u32,
    pub raw_bytes: u64,
}
pub(crate) const PAYLOAD: u8 = 1;
pub(crate) const FOREIGN: u8 = 4;
pub(crate) const WEAK: u8 = 3;

/// Rust state that cannot own Lisp handles. `Send` excludes Value/Root/stacks;
/// `'static` excludes references to heap-borrowed typed views.
/// `Drop` must not call Lisp or access the heap.
pub trait Payload: Send + 'static {
    fn bytes(&self) -> usize;
}

/// Resource-bearing object words, not roots. S4 drops these before freeing cells.
#[derive(Clone, Copy)]
pub(crate) enum Resource {
    String(u64),
    Payload(u64),
    Foreign(u64),
}
impl Resource {
    /// # Safety
    /// The resource's initialized cell is still allocated and uniquely retired.
    pub(crate) unsafe fn drop(self) {
        match self {
            Self::String(word) => {
                // SAFETY: constructor transferred ownership of this Vec to header.
                let s = unsafe { &mut *((word & !7) as *mut LispString) };
                unsafe {
                    drop(Vec::from_raw_parts(s.data, s.size_byte, s.cap));
                }
            }
            Self::Payload(word) => {
                // SAFETY: initialized raw part contains exactly one Box<dyn Payload>.
                unsafe {
                    std::ptr::drop_in_place(raw_ptr(word).cast::<Box<dyn Payload>>());
                }
            }
            Self::Foreign(_) => unreachable!("foreign destruction requires heap hooks"),
        }
    }
}
/// # Safety
/// `word` points to an initialized VectorHeader and its complete slot prefix.
pub(crate) unsafe fn raw_ptr(word: u64) -> *mut u8 {
    let h = unsafe { &*((word & !7) as *const VectorHeader) };
    unsafe {
        ((word & !7) as *mut u8).add(16 + h.lisp_slots as usize * 8 + closure_code_bytes(h.pvec))
    }
}

fn closure_code_bytes(pvec: PvecType) -> usize {
    if pvec == PvecType::Closure { 8 } else { 0 }
}

/// # Safety
/// `word` points to an initialized closure with its private Slot.
pub(crate) unsafe fn code_slot(word: u64) -> *mut Slot {
    let h = unsafe { &*((word & !7) as *const VectorHeader) };
    unsafe {
        ((word & !7) as *mut u8)
            .add(16 + h.lisp_slots as usize * 8)
            .cast()
    }
}

impl Heap {
    pub(crate) fn valid_word(&self, word: u64) -> bool {
        word & 3 == 2
            || self.external_symbol(word)
            || self.static_leaf(word)
            || self.allocator.borrow().address(word).is_some()
    }
    pub(crate) fn assert_value(&self, word: u64) {
        #[cfg(feature = "verify")]
        assert!(self.valid_word(word), "foreign/dead value");
        #[cfg(not(feature = "verify"))]
        let _ = word;
    }
    fn object(&self, value: Value<'_>, kind: Kind) -> Option<NonNull<u8>> {
        if value.kind() != kind
            || self.static_leaf(value.0)
            || (kind == Kind::Symbol && self.config.external_symbols)
        {
            return None;
        }
        if cfg!(feature = "verify") {
            return self.allocator.borrow().address(value.0);
        }
        if kind == Kind::Symbol {
            return self.symbol_cell(value).map(|c| c.symbol.ptr.cast());
        }
        // Mutator Values come from initialized constructors, rooted storage or
        // the unsafe raw bridge. Collector rebranding must validate allocation
        // before typed reads: compact_undo uses undo_value for owner words and
        // every followed edge; other collector reads use marked/registered cells.
        // The heap borrow then excludes reclamation for the returned typed view.
        // Symbols still need arena-index translation.
        NonNull::new((value.0 & !7) as *mut u8)
    }
    pub fn cons<'h>(&'h self, car: Value<'h>, cdr: Value<'h>) -> Result<Value<'h>, AllocError> {
        self.assert_value(car.0);
        self.assert_value(cdr.0);
        let word = self.allocator.borrow_mut().alloc(Kind::Cons, 16)?;
        // SAFETY: exclusive fresh cell, correct size and alignment.
        unsafe {
            ((word & !7) as *mut Cons).write(Cons {
                car: Slot::new(car),
                cdr: Slot::new(cdr),
            });
        }
        Ok(Value::word(word))
    }
    pub fn float(&self, value: f64) -> Result<Value<'_>, AllocError> {
        let word = self.allocator.borrow_mut().alloc(Kind::Float, 8)?;
        // SAFETY: exclusive fresh 8-byte float cell.
        unsafe {
            ((word & !7) as *mut f64).write(value);
        }
        Ok(Value::word(word))
    }
    /// Store bytes without compaction; multibyte strings use GNU internal encoding.
    /// Character count is derived, never supplied independently of the bytes.
    pub fn make_string(&self, bytes: &[u8], multibyte: bool) -> Result<Value<'_>, AllocError> {
        let nchars = if multibyte {
            let mut rest = bytes;
            let mut count = 0;
            while !rest.is_empty() {
                let len = romacs_chars::multibyte_length(rest, true);
                if len == 0 {
                    return Err(AllocError::InvalidSize);
                }
                rest = &rest[len..];
                count += 1;
            }
            count
        } else {
            bytes.len()
        };
        let mut data = bytes.to_vec();
        let word = self
            .allocator
            .borrow_mut()
            .alloc_with_counts(Kind::String, 48, nchars)?;
        let s = LispString {
            intervals: Slot::new(self.nil()),
            size: nchars,
            size_byte: data.len(),
            data: data.as_mut_ptr(),
            cap: data.capacity(),
            multibyte,
        };
        // SAFETY: fresh 48-byte aligned cell, data ownership transferred exactly once.
        unsafe {
            ((word & !7) as *mut LispString).write(s);
        }
        let capacity = data.capacity();
        std::mem::forget(data);
        self.resources.borrow_mut().push(Resource::String(word));
        self.account_new_string(capacity);
        Ok(Value::word(word))
    }
    pub fn make_symbol<'h>(&'h self, name: Value<'h>) -> Result<Value<'h>, AllocError> {
        if self.config.external_symbols {
            return Err(AllocError::InvalidSize);
        }
        if self.as_string(name).is_none() {
            return Err(AllocError::InvalidSize);
        }
        let word = self.allocator.borrow_mut().alloc(Kind::Symbol, 48)?;
        let ptr = self.allocator.borrow().address(word).expect("new symbol");
        // SAFETY: fresh arena cell with 48-byte Symbol layout.
        unsafe {
            ptr.cast::<Symbol>().as_ptr().write(Symbol {
                name: Slot::new(name),
                value: Slot::new(self.nil()),
                function: Slot::new(self.nil()),
                plist: Slot::new(self.nil()),
                next: Slot::new(self.nil()),
                flags: AtomicU64::new(UNBOUND),
            });
        }
        Ok(Value::word(word))
    }
    pub(crate) fn vector<'h>(
        &'h self,
        pvec: PvecType,
        values: &[Value<'h>],
        raw_bytes: usize,
        flags: u8,
    ) -> Result<Value<'h>, AllocError> {
        // Free is never an initialized published object.
        if pvec == PvecType::Free {
            return Err(AllocError::InvalidSize);
        }
        let n = u32::try_from(values.len()).map_err(|_| AllocError::MemoryFull)?;
        if raw_bytes as u128 >= 1_u128 << 62 {
            return Err(AllocError::MemoryFull);
        }
        let size = values
            .len()
            .checked_mul(8)
            .and_then(|v| v.checked_add(16))
            .and_then(|v| v.checked_add(closure_code_bytes(pvec)))
            .and_then(|v| v.checked_add(raw_bytes))
            .ok_or(AllocError::MemoryFull)?;
        for value in values {
            self.assert_value(value.0);
        }
        let word =
            self.allocator
                .borrow_mut()
                .alloc_with_counts(Kind::Vectorlike, size, values.len())?;
        let ptr = (word & !7) as *mut VectorHeader;
        // SAFETY: fresh allocation fits header, prefix and raw tail; tail stays zeroed.
        unsafe {
            ptr.write(VectorHeader {
                pvec,
                flags,
                reserved: 0,
                lisp_slots: n,
                raw_bytes: raw_bytes as u64,
            });
            let slots = ptr.add(1).cast::<Slot>();
            for (i, value) in values.iter().enumerate() {
                slots.add(i).write(Slot::new(*value));
            }
            if pvec == PvecType::Closure {
                code_slot(word).write(Slot::new(self.nil()));
            }
        }
        Ok(Value::word(word))
    }
    /// Copy an eval-defined encoding into an immutable, edge-free GC leaf.
    /// Eval owns the encoding (code, argdesc, depth and switch sets); mem never decodes it.
    pub fn make_code_block(&self, bytes: &[u8]) -> Result<Value<'_>, AllocError> {
        let value = self.vector(PvecType::CodeBlock, &[], bytes.len(), 0)?;
        // SAFETY: fresh raw tail fits the bytes; source and destination are disjoint.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), raw_ptr(value.0), bytes.len());
        }
        Ok(value)
    }
    /// Construct an immutable leaf certified by the owning bytecode verifier.
    /// # Safety
    /// The complete encoding, operand bounds, stack cap and constant indices
    /// must be verified before publication; consumers may omit those checks.
    pub unsafe fn make_verified_code_block(&self, bytes: &[u8]) -> Result<Value<'_>, AllocError> {
        let v = self.make_code_block(bytes)?;
        // SAFETY: fresh, exclusively owned header, initialized by make_code_block.
        unsafe {
            (*((v.0 & !7) as *mut VectorHeader)).reserved = 1;
        }
        Ok(v)
    }
    pub fn make_vectorlike<'h>(
        &'h self,
        pvec: PvecType,
        values: &[Value<'h>],
        raw: &[u8],
    ) -> Result<Value<'h>, AllocError> {
        if matches!(pvec, PvecType::Finalizer | PvecType::CodeBlock) {
            return Err(AllocError::InvalidSize);
        }
        let value = self.vector(pvec, values, raw.len(), 0)?;
        // SAFETY: new vector's tail has exactly raw.len() bytes; disjoint source.
        unsafe {
            std::ptr::copy_nonoverlapping(raw.as_ptr(), raw_ptr(value.0), raw.len());
        }
        Ok(value)
    }
    pub fn make_payload<'h>(
        &'h self,
        pvec: PvecType,
        values: &[Value<'h>],
        payload: Box<dyn Payload>,
    ) -> Result<Value<'h>, AllocError> {
        if matches!(pvec, PvecType::Finalizer | PvecType::CodeBlock) {
            return Err(AllocError::InvalidSize);
        }
        let value = self.vector(
            pvec,
            values,
            std::mem::size_of::<Box<dyn Payload>>(),
            PAYLOAD,
        )?;
        // SAFETY: raw tail is aligned for and sized to the owned fat pointer.
        unsafe {
            raw_ptr(value.0).cast::<Box<dyn Payload>>().write(payload);
        }
        self.resources.borrow_mut().push(Resource::Payload(value.0));
        self.account_resource(Resource::Payload(value.0));
        Ok(value)
    }
    pub fn as_cons<'h>(&'h self, value: Value<'h>) -> Option<ConsRef<'h>> {
        Some(ConsRef {
            heap: self,
            ptr: self.object(value, Kind::Cons)?.cast(),
        })
    }
    pub fn as_float(&self, value: Value<'_>) -> Option<f64> {
        let ptr = self.object(value, Kind::Float)?.cast::<f64>();
        // SAFETY: only initialized, live Float cells are published.
        Some(unsafe { *ptr.as_ref() })
    }
    pub fn as_string<'h>(&'h self, value: Value<'h>) -> Option<StringRef<'h>> {
        let ptr = self.object(value, Kind::String)?;
        // SAFETY: branded values name live cells; object also rejects static
        // leaves, whose owner layout is not a native/page-backed string.
        if unsafe { crate::alloc::Allocator::foreign_string(value.0) } {
            return None;
        }
        Some(StringRef {
            heap: self,
            ptr: ptr.cast(),
        })
    }
    /// A plain value cell: callers own binding policy and GC roots.
    #[inline]
    pub fn symbol_cell<'h>(&'h self, value: Value<'h>) -> Option<SymbolCell<'h>> {
        if value.0 & 7 != 0 || self.config.external_symbols {
            return None;
        }
        #[cfg(feature = "verify")]
        self.assert_value(value.0);
        let index = (value.0 >> 3) as usize;
        let cell_bytes = std::mem::size_of::<Symbol>();
        let gap = crate::alloc::PAGE_BYTES - crate::alloc::SYMBOLS_PER_PAGE * cell_bytes;
        let offset = crate::alloc::HEADER_BYTES
            + index * cell_bytes
            + (index / crate::alloc::SYMBOLS_PER_PAGE) * gap;
        // SAFETY: safe Values name initialized live cells in this isolate (raw
        // rebranding is unsafe). The reserved symbol mapping never moves; its
        // page/slot layout is fixed. &Heap excludes sweep, and allocation never
        // collects. Only Slot atomics are mutated, never an aliased &mut Symbol.
        let ptr = unsafe { NonNull::new_unchecked(self.symbol_base.as_ptr().add(offset).cast()) };
        Some(SymbolCell {
            symbol: SymbolRef { heap: self, ptr },
        })
    }
    pub fn as_symbol<'h>(&'h self, value: Value<'h>) -> Option<SymbolRef<'h>> {
        Some(SymbolRef {
            heap: self,
            ptr: self.object(value, Kind::Symbol)?.cast(),
        })
    }
    pub fn as_vectorlike<'h>(&'h self, value: Value<'h>) -> Option<VectorRef<'h>> {
        Some(VectorRef {
            heap: self,
            ptr: self.object(value, Kind::Vectorlike)?.cast(),
        })
    }
}

#[derive(Clone, Copy)]
pub struct ConsRef<'h> {
    heap: &'h Heap,
    ptr: NonNull<Cons>,
}
impl<'h> ConsRef<'h> {
    fn object(self) -> &'h Cons {
        unsafe { self.ptr.as_ref() }
    } // SAFETY: live cell, heap borrowed.
    pub fn car(self) -> Value<'h> {
        self.object().car.get(self.heap)
    }
    pub fn cdr(self) -> Value<'h> {
        self.object().cdr.get(self.heap)
    }
    pub fn set_car(self, value: Value<'h>) {
        self.object().car.set(self.heap, value);
    }
    pub fn set_cdr(self, value: Value<'h>) {
        self.object().cdr.set(self.heap, value);
    }
}
#[derive(Clone, Copy)]
pub struct StringRef<'h> {
    heap: &'h Heap,
    ptr: NonNull<LispString>,
}
impl<'h> StringRef<'h> {
    fn object(self) -> &'h LispString {
        unsafe { self.ptr.as_ref() }
    } // SAFETY: heap borrow prevents reclamation.
    pub fn bytes(self) -> &'h [u8] {
        let s = self.object();
        // SAFETY: owned Vec storage is immutable and lives until cell destruction.
        unsafe { std::slice::from_raw_parts(s.data, s.size_byte) }
    }
    pub fn chars(self) -> usize {
        self.object().size
    }
    pub fn multibyte(self) -> bool {
        self.object().multibyte
    }
    pub fn intervals(self) -> Value<'h> {
        self.object().intervals.get(self.heap)
    }
    pub fn set_intervals(self, value: Value<'h>) {
        self.object().intervals.set(self.heap, value);
    }
}
/// Plain symbol value storage; unlike SymbolRef writes, stores do not pin.
/// Neither this view nor its values are roots across a mutable GC boundary.
#[derive(Clone, Copy)]
pub struct SymbolCell<'h> {
    symbol: SymbolRef<'h>,
}
impl<'h> SymbolCell<'h> {
    /// Metadata view of this already-addressed, branded symbol cell.
    #[inline]
    pub fn symbol(self) -> SymbolRef<'h> {
        self.symbol
    }
    #[inline]
    pub fn get(self) -> Value<'h> {
        self.symbol.object().value.get(self.symbol.heap)
    }
    #[inline]
    pub fn set(self, value: Value<'h>) {
        self.symbol.object().value.set(self.symbol.heap, value);
        let flags = &self.symbol.object().flags;
        flags.store(flags.load(Ordering::Relaxed) & !UNBOUND, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy)]
pub struct SymbolRef<'h> {
    heap: &'h Heap,
    ptr: NonNull<Symbol>,
}
impl<'h> SymbolRef<'h> {
    #[inline]
    fn object(self) -> &'h Symbol {
        unsafe { self.ptr.as_ref() }
    } // SAFETY: live arena cell borrowed from heap.
    pub fn name(self) -> Value<'h> {
        self.object().name.get(self.heap)
    }
    pub fn value(self) -> Value<'h> {
        self.object().value.get(self.heap)
    }
    pub fn function(self) -> Value<'h> {
        self.object().function.get(self.heap)
    }
    pub fn plist(self) -> Value<'h> {
        self.object().plist.get(self.heap)
    }
    pub fn next(self) -> Value<'h> {
        self.object().next.get(self.heap)
    }
    fn pin_on_write(self) {
        if self.heap.gc.borrow().mode_is_interim() {
            let word = self.heap.allocator.borrow().symbol_word(self.ptr);
            self.heap.pin_symbol(Value::word(word));
        }
    }
    /// Whether the plain value cell has a binding (bound nil is distinct).
    pub fn is_bound(self) -> bool {
        self.object().flags.load(Ordering::Relaxed) & UNBOUND == 0
    }
    /// Clear a plain binding without changing its redirect or other flags.
    pub fn makunbound(self) {
        self.object().value.set(self.heap, self.heap.nil());
        let flags = &self.object().flags;
        flags.store(flags.load(Ordering::Relaxed) | UNBOUND, Ordering::Relaxed);
    }
    pub fn set_value(self, value: Value<'h>) {
        self.pin_on_write();
        self.object().value.set(self.heap, value);
        let flags = &self.object().flags;
        flags.store(flags.load(Ordering::Relaxed) & !UNBOUND, Ordering::Relaxed);
    }
    pub fn set_function(self, value: Value<'h>) {
        self.pin_on_write();
        self.object().function.set(self.heap, value);
    }
    pub fn set_plist(self, value: Value<'h>) {
        self.pin_on_write();
        self.object().plist.set(self.heap, value);
    }
    pub fn set_next(self, value: Value<'h>) {
        self.object().next.set(self.heap, value);
    }
    /// Read the packed metadata once when policy needs both halves.
    #[inline]
    pub fn flags_aux(self) -> (u32, u32) {
        let word = self.object().flags.load(Ordering::Relaxed);
        (word as u32, (word >> 32) as u32)
    }
    pub fn flags(self) -> u32 {
        self.object().flags.load(Ordering::Relaxed) as u32
    }
    /// Untraced owner metadata, not a Lisp edge. Localized/forwarded symbols
    /// use it as an index into owner-managed side tables; those tables must
    /// root any Lisp edges themselves. Mem never interprets this word.
    pub fn aux(self) -> u32 {
        (self.object().flags.load(Ordering::Relaxed) >> 32) as u32
    }
    /// Replace owner metadata without changing flags, including boundness.
    pub fn set_aux(self, aux: u32) {
        self.set_flags_aux(self.flags(), aux);
    }
    pub fn set_flags_aux(self, flags: u32, aux: u32) {
        self.object()
            .flags
            .store(flags as u64 | ((aux as u64) << 32), Ordering::Relaxed);
    }
}
#[derive(Clone, Copy)]
pub struct VectorRef<'h> {
    heap: &'h Heap,
    ptr: NonNull<VectorHeader>,
}
impl<'h> VectorRef<'h> {
    fn header(self) -> &'h VectorHeader {
        unsafe { self.ptr.as_ref() }
    } // SAFETY: initialized header, heap borrow.
    /// Eval-private edge, excluded from the visible slot prefix and raw bytes.
    pub fn code(self) -> Option<Value<'h>> {
        Some(self.private_code()?.get(self.heap))
    }
    /// Store nil or an immutable CodeBlock from this heap. Invalid inputs leave it unchanged.
    pub fn set_code(self, value: Value<'h>) -> bool {
        let Some(slot) = self.private_code() else {
            return false;
        };
        if !value.is_nil() {
            if value.kind() != Kind::Vectorlike {
                return false;
            }
            let Some(ptr) = self.heap.allocator.borrow().address(value.0) else {
                return false;
            };
            // SAFETY: this heap owns the initialized vectorlike allocation.
            if unsafe { ptr.cast::<VectorHeader>().as_ref().pvec } != PvecType::CodeBlock {
                return false;
            }
        }
        slot.set(self.heap, value);
        true
    }
    fn private_code(self) -> Option<&'h Slot> {
        if self.pvec_type() != PvecType::Closure {
            return None;
        }
        // SAFETY: every closure constructor initializes one private Slot after the prefix.
        Some(unsafe { &*code_slot(self.ptr.as_ptr() as u64) })
    }
    /// Whether an unsafe verifier constructor certified this leaf.
    pub fn is_verified_code(self) -> bool {
        self.header().pvec == PvecType::CodeBlock && self.header().reserved == 1
    }
    /// Stable initialized Slot words. The caller must keep the vector rooted
    /// across collection and use atomic access if another thread can write.
    pub fn slots_ptr(self) -> *const u64 {
        // SAFETY: every vector header is followed by its initialized Slot prefix.
        unsafe { self.ptr.as_ptr().add(1).cast() }
    }
    pub fn pvec_type(self) -> PvecType {
        self.header().pvec
    }
    pub fn len(self) -> usize {
        self.header().lisp_slots as usize
    }
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
    pub fn raw_len(self) -> usize {
        self.header().raw_bytes as usize
    }
    fn slot(self, index: usize) -> Option<&'h Slot> {
        if index >= self.len() {
            return None;
        }
        // SAFETY: prefix contains exactly lisp_slots initialized Slots.
        Some(unsafe { &*self.ptr.as_ptr().add(1).cast::<Slot>().add(index) })
    }
    pub fn get(self, index: usize) -> Option<Value<'h>> {
        Some(self.slot(index)?.get(self.heap))
    }
    pub fn set(self, index: usize, value: Value<'h>) -> bool {
        let Some(slot) = self.slot(index) else {
            return false;
        };
        slot.set(self.heap, value);
        true
    }
    /// Raw data is read-only; owned fat pointers are never exposed as bytes.
    pub fn raw_bytes(self) -> Option<&'h [u8]> {
        if self.header().flags != 0 {
            return None;
        }
        // SAFETY: initialized immutable raw tail; heap borrow prevents freeing.
        Some(unsafe {
            std::slice::from_raw_parts(raw_ptr(self.ptr.as_ptr() as u64), self.raw_len())
        })
    }
    pub fn payload(self) -> Option<&'h dyn Payload> {
        if self.header().flags != PAYLOAD {
            return None;
        }
        // SAFETY: PAYLOAD constructor wrote the Box and it remains owned by heap.
        Some(unsafe { &**raw_ptr(self.ptr.as_ptr() as u64).cast::<Box<dyn Payload>>() })
    }
}

#[cfg(test)]
#[path = "object_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sym_cell_tests.rs"]
mod sym_cell_tests;
