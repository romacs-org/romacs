//! One heap authority: Obarray [count, buckets], buckets chained by symbol.next.
//! Views borrow the heap, so GC cannot run while an operation is in progress.
use crate::symbol::{self, INTERNED};
use romacs_mem::{AllocError, Heap, PvecType, SymbolRef, Value, VectorRef};

#[derive(Clone, Copy)]
pub struct Obarray<'h> {
    heap: &'h Heap,
    value: Value<'h>,
    object: VectorRef<'h>,
}

fn hash(bytes: &[u8], multibyte: bool) -> usize {
    bytes.iter().fold(
        0xcbf29ce484222325_u64 ^ u64::from(multibyte && !bytes.is_ascii()),
        |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3),
    ) as usize
}

impl<'h> Obarray<'h> {
    /// Create the initial obarray, whose colon-prefixed symbols are keywords.
    pub fn new_initial(heap: &'h Heap) -> Result<Self, AllocError> {
        Self::create(heap, &[1])
    }
    pub fn new(heap: &'h Heap) -> Result<Self, AllocError> {
        Self::create(heap, &[])
    }
    fn create(heap: &'h Heap, metadata: &[u8]) -> Result<Self, AllocError> {
        let buckets = heap.make_vectorlike(PvecType::NormalVector, &[heap.nil(); 16], &[])?;
        let value = heap.make_vectorlike(
            PvecType::Obarray,
            &[heap.fixnum(0).unwrap(), buckets],
            metadata,
        )?;
        Ok(Self {
            heap,
            value,
            object: heap.as_vectorlike(value).unwrap(),
        })
    }
    /// Reborrow a previously created obarray, typically from its owner's Root.
    /// Other vectorlike types are rejected. Do not mutate its layout via mem.
    pub fn from_value(heap: &'h Heap, value: Value<'h>) -> Option<Self> {
        let ob = heap.as_vectorlike(value)?;
        if ob.pvec_type() != PvecType::Obarray || ob.len() != 2 {
            return None;
        }
        let count = ob.get(0)?.as_fixnum()?;
        let buckets = heap.as_vectorlike(ob.get(1)?)?;
        (count >= 0
            && buckets.pvec_type() == PvecType::NormalVector
            && buckets.len().is_power_of_two())
        .then_some(Self {
            heap,
            value,
            object: ob,
        })
    }
    pub fn value(self) -> Value<'h> {
        self.value
    }
    fn object(self) -> VectorRef<'h> {
        self.object
    }
    fn buckets(self) -> VectorRef<'h> {
        self.heap
            .as_vectorlike(self.object().get(1).unwrap())
            .expect("buckets")
    }
    pub fn len(self) -> usize {
        self.object().get(0).unwrap().as_fixnum().unwrap() as usize
    }
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
    fn set_len(self, n: usize) {
        self.object().set(0, self.heap.fixnum(n as i64).unwrap());
    }
    pub fn intern_soft(self, name: &[u8], multibyte: bool) -> Option<Value<'h>> {
        let buckets = self.buckets();
        let hash = hash(name, multibyte);
        let index = hash & (buckets.len() - 1);
        self.lookup(buckets.get(index).unwrap(), name, multibyte, hash as u32)
    }
    // Redirect records own aux. Never compare their indices to name hashes,
    // nor move them to a different bucket based on that untraced metadata.
    fn name_hash(self, symbol: SymbolRef<'h>) -> u32 {
        let (flags, aux) = symbol.flags_aux();
        // Both redirect kinds have bit 1 set; Plain/Alias do not.
        if flags & 2 == 0 && aux != 0 {
            aux
        } else {
            let name = self.heap.as_string(symbol.name()).expect("symbol name");
            hash(name.bytes(), name.multibyte()) as u32
        }
    }
    fn lookup(
        self,
        mut head: Value<'h>,
        name: &[u8],
        multibyte: bool,
        hash: u32,
    ) -> Option<Value<'h>> {
        while !head.is_nil() {
            let symbol = self.heap.as_symbol(head).expect("bucket symbol");
            let (flags, aux) = symbol.flags_aux();
            // A matching cached hash needs no kind check. Redirect indices and
            // cleared caches must go straight to the immutable name instead.
            if aux != hash && aux != 0 && flags & 2 == 0 {
                head = symbol.next();
                continue;
            }
            let stored = self.heap.as_string(symbol.name()).expect("symbol name");
            if stored.bytes() == name && (name.is_ascii() || stored.multibyte() == multibyte) {
                return Some(head);
            }
            head = symbol.next();
        }
        None
    }
    /// Intern bytes, allocating a name string on a miss.
    pub fn intern(self, name: &[u8], multibyte: bool) -> Result<Value<'h>, AllocError> {
        self.intern_name(name, multibyte, None)
    }
    /// GNU's intern path: reuse an already allocated Lisp name string.
    pub fn intern_string(self, name: Value<'h>) -> Result<Value<'h>, AllocError> {
        let string = self.heap.as_string(name).ok_or(AllocError::InvalidSize)?;
        self.intern_name(string.bytes(), string.multibyte(), Some(name))
    }
    fn intern_name(
        self,
        name: &[u8],
        multibyte: bool,
        value: Option<Value<'h>>,
    ) -> Result<Value<'h>, AllocError> {
        let hash = hash(name, multibyte);
        let mut buckets = self.buckets();
        let index = hash & (buckets.len() - 1);
        if let Some(symbol) = self.lookup(buckets.get(index).unwrap(), name, multibyte, hash as u32)
        {
            return Ok(symbol);
        }
        let keyword = name.first() == Some(&b':') && self.object.raw_bytes() == Some(&[1]);
        let name = match value {
            Some(name) => name,
            None => self.heap.make_string(name, multibyte)?,
        };
        let value = self.heap.make_symbol(name)?;
        let count = self.len();
        if count >= buckets.len() {
            buckets = self.grow(buckets)?;
        }
        let index = hash & (buckets.len() - 1);
        let symbol = self.heap.as_symbol(value).unwrap();
        symbol.set_next(buckets.get(index).unwrap());
        // Name is immutable: aux caches its hash for growth and chain filtering.
        symbol.set_flags_aux(symbol.flags() | INTERNED, hash as u32);
        if keyword {
            symbol.set_value(value);
            symbol::set_flag(symbol, symbol::CONSTANT, true);
        }
        buckets.set(index, value);
        self.set_len(count + 1);
        Ok(value)
    }
    fn grow(self, old: VectorRef<'h>) -> Result<VectorRef<'h>, AllocError> {
        let size = old.len().checked_mul(2).ok_or(AllocError::MemoryFull)?;
        let value =
            self.heap
                .make_vectorlike(PvecType::NormalVector, &vec![self.heap.nil(); size], &[])?;
        let new = self.heap.as_vectorlike(value).unwrap();
        for i in 0..old.len() {
            let mut head = old.get(i).unwrap();
            while !head.is_nil() {
                let symbol = self.heap.as_symbol(head).unwrap();
                let next = symbol.next();
                let index = self.name_hash(symbol) as usize & (size - 1);
                symbol.set_next(new.get(index).unwrap());
                new.set(index, head);
                head = next;
            }
            // A retained old bucket vector must not keep stale chain roots.
            old.set(i, self.heap.nil());
        }
        self.object().set(1, value);
        Ok(new)
    }
    /// Remove by name, optionally requiring exact symbol identity (GNU's
    /// symbol argument to unintern). A different same-named symbol is a miss.
    pub fn unintern(self, name: &[u8], multibyte: bool, exact: Option<Value<'h>>) -> bool {
        let buckets = self.buckets();
        let index = hash(name, multibyte) & (buckets.len() - 1);
        let mut head = buckets.get(index).unwrap();
        let mut previous = None;
        while !head.is_nil() {
            let symbol = self.heap.as_symbol(head).unwrap();
            let stored = self.heap.as_string(symbol.name()).unwrap();
            if stored.bytes() == name && (name.is_ascii() || stored.multibyte() == multibyte) {
                if exact.is_some_and(|value| value != head) {
                    return false;
                }
                if let Some(previous) = previous {
                    self.heap
                        .as_symbol(previous)
                        .unwrap()
                        .set_next(symbol.next());
                } else {
                    buckets.set(index, symbol.next());
                }
                symbol.set_next(self.heap.nil());
                symbol::set_flag(symbol, INTERNED, false);
                self.set_len(self.len() - 1);
                return true;
            }
            previous = Some(head);
            head = symbol.next();
        }
        false
    }
    /// Kernel mapatoms; no Lisp callbacks. Collect values before mutating chains.
    pub fn for_each(self, visit: &mut dyn FnMut(Value<'h>)) {
        let buckets = self.buckets();
        for i in 0..buckets.len() {
            let mut head = buckets.get(i).unwrap();
            while !head.is_nil() {
                let next = self.heap.as_symbol(head).unwrap().next();
                visit(head);
                head = next;
            }
        }
    }
}
