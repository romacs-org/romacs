//! First-class obarrays over the VM's stable symbol-slot arena.
//! The initial name table remains authoritative; its Lisp wrapper is lazy.
use super::*;
use crate::data::Object;
#[cfg(test)]
#[path = "obarray_tests.rs"]
mod tests;

#[derive(Clone)]
pub(super) struct Entry {
    bytes: Vec<u8>,
    chars: usize,
    pub(super) symbol: Value,
    next: Option<u64>,
    present: bool,
}
romacs_snapshot::snapshot_struct!(Entry { bytes, chars, symbol, next, present });
#[derive(Clone)]
pub(super) struct Obarray {
    pub(super) initial: bool,
    buckets: Vec<Option<u64>>,
    entries: HashMap<u64, Entry>,
    count: usize,
}
romacs_snapshot::snapshot_struct!(Obarray { initial, buckets, entries, count });
impl Obarray {
    pub(super) fn new(bits: u32, initial: bool) -> Self {
        Self { initial, buckets: vec![None; 1usize << bits], entries: HashMap::default(), count: 0 }
    }
    pub(super) fn len(&self) -> usize { self.count }
    pub(super) fn storage_bytes(&self, capacity: bool) -> usize {
        size_of::<Self>() + self.buckets.capacity() * size_of::<Option<u64>>()
            + self.entries.values().map(|e| size_of::<Entry>() + if capacity { e.bytes.capacity() } else { e.bytes.len() }).sum::<usize>()
    }
    pub(super) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for entry in self.entries.values_mut().filter(|e| e.present) { visit(&mut entry.symbol); }
    }
    pub(super) fn reclaim_removed(&mut self, live: &HashSet<usize>) {
        self.entries.retain(|_, e| e.present || live.contains(&(e.symbol.payload() as usize - 1)));
        self.entries.shrink_to_fit();
    }
    fn lookup(&self, bytes: &[u8], chars: usize) -> Option<Value> {
        let mut next = self.buckets[bucket(bytes, self.buckets.len())];
        while let Some(id) = next {
            let e = &self.entries[&id];
            if e.bytes == bytes && e.chars == chars { return Some(e.symbol); }
            next = e.next;
        }
        None
    }
    fn insert(&mut self, bytes: Vec<u8>, chars: usize, symbol: Value) {
        let i = bucket(&bytes, self.buckets.len());
        self.entries.insert(symbol.0, Entry { bytes, chars, symbol, next: self.buckets[i], present: true });
        self.buckets[i] = Some(symbol.0);
        self.count += 1;
        if self.count > self.buckets.len() {
            let size = self.buckets.len() * 2;
            let old = std::mem::replace(&mut self.buckets, vec![None; size]);
            for mut next in old {
                while let Some(id) = next {
                    let e = self.entries.get_mut(&id).unwrap();
                    next = e.next;
                    let i = bucket(&e.bytes, self.buckets.len());
                    e.next = self.buckets[i];
                    self.buckets[i] = Some(id);
                }
            }
        }
    }
    fn remove(&mut self, symbol: Value) -> bool {
        let Some(e) = self.entries.get(&symbol.0).filter(|e| e.present) else { return false };
        let i = bucket(&e.bytes, self.buckets.len());
        let successor = e.next;
        if self.buckets[i] == Some(symbol.0) { self.buckets[i] = successor; }
        else {
            let mut cursor = self.buckets[i];
            while let Some(id) = cursor {
                let e = self.entries.get_mut(&id).unwrap();
                if e.next == Some(symbol.0) { e.next = successor; break; }
                cursor = e.next;
            }
        }
        self.entries.get_mut(&symbol.0).unwrap().present = false;
        self.count -= 1;
        true
    }
}

// GNU fns.c hash_char_array and lisp.h reduce_emacs_uint_to_hash_hash/knuth_hash.
fn bucket(bytes: &[u8], size: usize) -> usize {
    let mut hash = bytes.len() as u64;
    let combine = |x: u64, y: u64| x.rotate_left(4).wrapping_add(y);
    if bytes.len() >= 8 {
        let mut i = 0;
        while i + 8 <= bytes.len() {
            hash = combine(hash, u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap()));
            i += 8.max(bytes.len() >> 3);
        }
        hash = combine(hash, u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap()));
    } else {
        let mut tail = 0u64;
        let mut i = 0;
        if bytes.len() >= 4 { tail = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as u64; i = 4; }
        if bytes.len() - i >= 2 { tail = (tail << 16) + u16::from_le_bytes(bytes[i..i+2].try_into().unwrap()) as u64; i += 2; }
        if i < bytes.len() { tail = (tail << 8) + bytes[i] as u64; }
        hash = combine(hash, tail);
    }
    let reduced = (hash ^ (hash >> 32)) as u32;
    ((reduced.wrapping_mul(2654435769) as u64) >> (32 - size.trailing_zeros())) as usize
}

impl Vm {
    pub(super) fn initial_obarray(&mut self) -> Value {
        let slot = self.slot("obarray");
        // The slot constructor installs the lazy wrapper exactly once.
        self.slots[slot].value
    }
    pub(super) fn resolve_obarray(&mut self, mut value: Value) -> std::result::Result<Value, prim::Fault> {
        if value == Value::NIL { value = self.initial_obarray(); }
        if matches!(self.object_if(value), Some(Object::Obarray(_))) { return Ok(value); }
        let first = match self.object_if(value) { Some(Object::Vector(v)) => v.first().copied(), _ => None };
        if let Some(first) = first {
            if matches!(self.object_if(first), Some(Object::Obarray(_))) { return Ok(first); }
            if first.integer_value() == Some(0) {
                let table = self.allocate_object(Object::Obarray(Box::new(Obarray::new(0, false))));
                if Self::image_value(value) { return Err(self.read_only_fault(value)); }
                if let Object::Vector(v) = self.object_mut(value).map_err(prim::Fault::Error)? { v[0] = table; }
                return Ok(table);
            }
        }
        Err(prim::Fault::Wrong("obarrayp", value))
    }
    pub(super) fn obarray_lookup_value(&self, table: Value, bytes: &[u8], chars: usize) -> Option<Value> {
        let Object::Obarray(table) = self.object_if(table)? else { return None };
        if table.initial {
            let name = symbol_key(bytes, chars)?;
            match name.as_str() { "nil" => Some(Value::NIL), "t" => Some(Value::TRUE), _ => self.names.get(&name).map(|&s| Value::reference(s + 1, 7)) }
        } else { table.lookup(bytes, chars) }
    }
    pub(super) fn obarray_intern(&mut self, table: Value, name: Value) -> std::result::Result<Value, prim::Fault> {
        let view = self.str_view(name).ok_or(prim::Fault::Wrong("stringp", name))?;
        let (bytes, chars) = (view.bytes.to_vec(), view.chars);
        if let Some(found) = self.obarray_lookup_value(table, &bytes, chars) { return Ok(found); }
        let initial = matches!(self.object_if(table), Some(Object::Obarray(t)) if t.initial);
        let text = symbol_key(&bytes, chars).ok_or(prim::Fault::NotUnicode(name))?;
        let text = text.as_str();
        let symbol = if initial { self.intern(text) } else {
            let id = expand::generated_id();
            let slot = if let Some(slot) = self.obarray_free_slots.pop() { slot } else {
                let slot = self.slots.len();
                self.slots.push(Slot::new()); self.symbol_names.push(String::new()); slot
            };
            self.generated_slots.insert(slot, id);
            self.generated_symbols.insert(id, slot);
            let symbol = Value::reference(slot + 1, 7);
            let slot = symbol.payload() as usize - 1;
            self.symbol_names[slot] = text.to_owned();
            self.obarray_symbol_names.insert(slot, name);
            symbol
        };
        if Self::image_value(table) { return Err(self.read_only_fault(table)); }
        if let Object::Obarray(t) = self.object_mut(table).map_err(prim::Fault::Error)? { t.insert(bytes, chars, symbol); }
        Ok(symbol)
    }
    pub(super) fn obarray_unintern(&mut self, table: Value, symbol: Value) -> bool {
        let initial = matches!(self.object_if(table), Some(Object::Obarray(t)) if t.initial);
        if initial {
            let name = if symbol == Value::NIL { "nil".to_owned() } else if symbol == Value::TRUE { "t".to_owned() } else { self.symbol_names[symbol.payload() as usize - 1].clone() };
            if let Some(&slot) = self.names.get(&name).filter(|&&slot| Value::reference(slot + 1, 7) == symbol) {
                self.names.remove(&name);
                self.set_initial_obarray(slot, false);
                return true;
            }
            return false;
        }
        // A frozen obarray holds what it holds: nothing to remove.
        if Self::image_value(table) { return false; }
        if let Ok(Object::Obarray(t)) = self.object_mut(table) { t.remove(symbol) } else { false }
    }
    pub(super) fn obarray_next(&self, table: Value, bucket_index: usize, current: Value) -> Option<(usize, Value)> {
        let Object::Obarray(t) = self.object_if(table)? else { return None };
        if t.initial {
            // Stable slot cursor: removal of an earlier name cannot shift it.
            if bucket_index == 0 { return Some((1, Value::NIL)); }
            if bucket_index == 1 { return Some((2, Value::TRUE)); }
            for slot in bucket_index - 2..self.slots.len() {
                if self.in_initial_obarray(slot) && !matches!(self.symbol_names[slot].as_str(), "nil" | "t") {
                    return Some((slot + 3, Value::reference(slot + 1, 7)));
                }
            }
            return None;
        }
        if current != Value::UNBOUND {
            if let Some(symbol) = t.entries.get(&current.0).and_then(|e| e.next).and_then(|id| t.entries.get(&id)).map(|e| e.symbol) { return Some((bucket_index, symbol)); }
        }
        for i in bucket_index..t.buckets.len() {
            if let Some(id) = t.buckets[i] { return Some((i + 1, t.entries[&id].symbol)); }
        }
        None
    }
}

/// The symbol-table key of a name's BYTES (CHARS characters): multibyte
/// internal text through symkey.rs; a unibyte name must be ASCII.
pub(super) fn symbol_key(bytes: &[u8], chars: usize) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(bytes) && !text.contains(crate::symkey::ESC) { return Some(text.to_owned()); }
    (chars < bytes.len() || bytes.is_ascii()).then(|| crate::symkey::from_internal(bytes))
}
