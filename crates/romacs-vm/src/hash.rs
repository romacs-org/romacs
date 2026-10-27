//! GNU-compatible Lisp hash tables (`src/fns.c` at the pinned GNU revision).
//!
//! Slot model: GNU keeps entries in a `key_and_value` slot array with a LIFO
//! free list, and `maphash` walks slot indices (`DOHASH_SAFE`). Iteration
//! order and mutation during iteration are therefore functions of the slot
//! model alone, which is reproduced exactly: growth sizes, free-list order
//! and the `clrhash` reset (only when the table has entries).
//!
//! Moving collection: a key's hash may read a private heap index, which
//! compaction changes. Such entries carry a `moving` bit; the collector
//! rewrites every key and value and marks the table stale when a moving key's
//! bits change, and the bucket index is rebuilt before the next lookup.
//! Content-hashed keys (numbers, symbols, strings, trees of those) are never
//! re-hashed. Slot indices never move, so iteration order is independent of
//! collection. No borrowed `&Value` outlives a single primitive call.
use super::*;
#[cfg(test)]
#[path = "fixnum_hash_tests.rs"]
mod fixnum_tag_tests;

/// Marks an unused slot. `UNBOUND` is never a Lisp-visible value, and tag 7
/// is neither marked nor rewritten by the collector.
pub(super) const UNUSED: Value = Value::UNBOUND;
const END: u32 = u32::MAX;
/// GNU's index cannot exceed 2^31 - 1 buckets: `:size` 2^30 and above fail.
const MAXIMUM_SIZE: i64 = 1 << 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Test {
    Eq,
    Eql,
    Equal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Weakness {
    None,
    Key,
    Value,
    KeyOrValue,
    KeyAndValue,
}
impl Weakness {
    /// GNU `keep_entry_p`.
    pub(super) fn keep(self, key: bool, value: bool) -> bool {
        match self {
            Self::None => true,
            Self::Key => key,
            Self::Value => value,
            Self::KeyOrValue => key || value,
            Self::KeyAndValue => key && value,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::None => "nil",
            Self::Key => "key",
            Self::Value => "value",
            Self::KeyOrValue => "key-or-value",
            Self::KeyAndValue => "key-and-value",
        }
    }
}

#[derive(Clone)]
pub(super) struct HashTable {
    pub(super) test: Test,
    pub(super) weakness: Weakness,
    /// Private buffer-local tables can compact their unobservable slot layout.
    pub(super) local_bindings: bool,
    /// GNU `table_size`: observable through `hash-table-size` and growth.
    size: usize,
    /// Materialized slots `0..keys.len()`; slots beyond are untouched and free.
    keys: Vec<Value>,
    values: Vec<Value>,
    hashes: Vec<u64>,
    moving: Vec<bool>,
    next: Vec<u32>,
    buckets: Vec<u32>,
    /// Freed slots, most recently freed last (GNU's free-list head).
    free: Vec<u32>,
    count: usize,
    stale: bool,
}
impl HashTable {
    pub(super) fn new(test: Test, weakness: Weakness, size: usize) -> Self {
        Self {
            test,
            weakness,
            local_bindings: false,
            size,
            keys: vec![],
            values: vec![],
            hashes: vec![],
            moving: vec![],
            next: vec![],
            buckets: vec![END; 8],
            free: vec![],
            count: 0,
            stale: false,
        }
    }
    pub(super) fn new_local_bindings() -> Self {
        let mut table = Self::new(Test::Eq, Weakness::Key, 65);
        table.local_bindings = true;
        table
    }
    /// Sweeping removes edges but ordinary GNU tables retain their slot array.
    /// Only private local tables shed dead slots and capacity after collection.
    pub(super) fn compact_local_bindings(&mut self) {
        if !self.local_bindings || self.count == self.keys.len() {
            return;
        }
        let mut compact = Self::new_local_bindings();
        for (i, key, value) in self.entries() {
            compact.insert(key, value, self.hashes[i], self.moving[i]);
        }
        // Existing hashes may refer to pre-compaction buffer indices. Preserve
        // invalidation until the normal lazy rehash can recompute them.
        compact.stale = self.stale;
        *self = compact;
    }
    pub(super) fn count(&self) -> usize {
        self.count
    }
    pub(super) fn size(&self) -> usize {
        self.size
    }
    pub(super) fn entry(&self, i: usize) -> Option<(Value, Value)> {
        let key = *self.keys.get(i)?;
        (key != UNUSED).then(|| (key, self.values[i]))
    }
    pub(super) fn entries(&self) -> impl Iterator<Item = (usize, Value, Value)> + '_ {
        self.keys
            .iter()
            .zip(&self.values)
            .enumerate()
            .filter(|(_, (k, _))| **k != UNUSED)
            .map(|(i, (k, v))| (i, *k, *v))
    }
    /// First used slot at or after `from` (one `DOHASH_SAFE` step).
    pub(super) fn next_used(&self, from: usize) -> Option<usize> {
        (from..self.keys.len()).find(|&i| self.keys[i] != UNUSED)
    }
    fn bucket(&self, hash: u64) -> usize {
        (mix(hash) as usize) & (self.buckets.len() - 1)
    }
    /// Head of the collision chain for `hash`; END when empty.
    fn chain(&self, hash: u64) -> u32 {
        assert!(!self.stale, "hash table lookup through a stale index");
        self.buckets[self.bucket(hash)]
    }
    /// GNU `hash_put` after a failed lookup, including `maybe_resize_hash_table`.
    fn insert(&mut self, key: Value, value: Value, hash: u64, moving: bool) {
        assert!(!self.stale && key != UNUSED);
        if self.free.is_empty() && self.keys.len() == self.size {
            let base = self.size.max(6);
            self.size = if self.size == 0 {
                6
            } else if base <= 64 {
                base * 4
            } else {
                base * 2
            };
        }
        let i = match self.free.pop() {
            Some(i) => i as usize,
            None => {
                self.keys.push(UNUSED);
                self.values.push(Value::NIL);
                self.hashes.push(0);
                self.moving.push(false);
                self.next.push(END);
                self.keys.len() - 1
            }
        };
        self.keys[i] = key;
        self.values[i] = value;
        self.hashes[i] = hash;
        self.moving[i] = moving;
        self.count += 1;
        if self.keys.len() * 2 > self.buckets.len() {
            self.rebuild();
        } else {
            let b = self.bucket(hash);
            self.next[i] = self.buckets[b];
            self.buckets[b] = i as u32;
        }
    }
    /// GNU `hash_remove_from_table` for a slot found by lookup.
    fn remove(&mut self, i: usize) {
        assert!(!self.stale);
        let b = self.bucket(self.hashes[i]);
        if self.buckets[b] == i as u32 {
            self.buckets[b] = self.next[i];
        } else {
            let mut at = self.buckets[b];
            while self.next[at as usize] != i as u32 {
                at = self.next[at as usize];
            }
            self.next[at as usize] = self.next[i];
        }
        self.release(i);
    }
    fn release(&mut self, i: usize) {
        self.keys[i] = UNUSED;
        self.values[i] = Value::NIL;
        self.moving[i] = false;
        self.next[i] = END;
        self.free.push(i as u32);
        self.count -= 1;
    }
    /// GNU `hash_clear`: a table with no entries keeps its free-list order.
    fn clear(&mut self) {
        if self.count == 0 {
            return;
        }
        self.keys.clear();
        self.values.clear();
        self.hashes.clear();
        self.moving.clear();
        self.next.clear();
        self.free.clear();
        self.buckets = vec![END; 8];
        self.count = 0;
        self.stale = false;
    }
    /// Collector removal of a dead weak entry; the chain is rebuilt lazily.
    pub(super) fn sweep(&mut self, i: usize) {
        self.release(i);
        self.stale = true;
    }
    fn rebuild(&mut self) {
        let wanted = (self.keys.len() * 2).next_power_of_two().max(8);
        self.buckets.clear();
        self.buckets.resize(wanted, END);
        for i in 0..self.keys.len() {
            self.next[i] = END;
            if self.keys[i] != UNUSED {
                let b = self.bucket(self.hashes[i]);
                self.next[i] = self.buckets[b];
                self.buckets[b] = i as u32;
            }
        }
    }
    /// Strong edges. Weak tables are visited by the collector's fixpoint.
    pub(super) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for i in 0..self.keys.len() {
            if self.keys[i] != UNUSED {
                visit(&mut self.keys[i]);
                visit(&mut self.values[i]);
            }
        }
    }
    /// Compaction forwarding. A moved index-hashed key invalidates the index.
    pub(super) fn forward(&mut self, rewrite: &mut impl FnMut(&mut Value)) {
        for i in 0..self.keys.len() {
            if self.keys[i] == UNUSED {
                continue;
            }
            let old = self.keys[i];
            rewrite(&mut self.keys[i]);
            rewrite(&mut self.values[i]);
            if self.moving[i] && self.keys[i] != old {
                self.stale = true;
            }
        }
    }
    pub(super) fn storage_bytes(&self, capacity: bool) -> usize {
        let per = 2 * size_of::<Value>() + size_of::<u64>() + size_of::<bool>() + 4;
        size_of::<Self>()
            + if capacity {
                self.keys.capacity() * per + self.buckets.capacity() * 4 + self.free.capacity() * 4
            } else {
                self.keys.len() * per + self.buckets.len() * 4 + self.free.len() * 4
            }
    }
}

/// splitmix64 finalizer, for bucket selection only. Hashes are not Lisp-visible.
fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}
/// GNU `sxhash_combine`.
fn combine(x: u64, y: u64) -> u64 {
    (x << 4).wrapping_add(x >> 60).wrapping_add(y)
}
fn hash_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}
const SXHASH_MAX_DEPTH: usize = 3;
const SXHASH_MAX_LEN: usize = 7;

/// Lisp-visible failures, converted to GNU-shaped signals at the boundary.
pub(super) enum HashFault {
    Error(&'static str, Option<Value>),
    Wrong(&'static str, Value),
    Arity(Primitive, usize),
    Internal(String),
}
type Hashed<T> = std::result::Result<T, HashFault>;

/// Comparison view of a Lisp object for GNU `internal_equal` and `sxhash`.
enum Shape<'a> {
    Atom,
    Cons,
    Float(u64),
    Bignum(&'a num_bigint::BigInt),
    Text { bytes: &'a [u8], chars: usize },
    Vector(&'a [Value]),
    BoolVector(&'a data::BoolVector),
    Record(&'a [Value]),
    Closure(usize, &'a [Value]),
    Marker(Value, Option<u32>),
    Overlay(Value, Option<(u32, u32)>, &'a [(Value, Value)]),
    Opaque,
}

pub(super) fn is_hash_primitive(p: Primitive) -> bool {
    (Primitive::MakeHashTable as usize..=Primitive::HashValueAt as usize).contains(&(p as usize))
}

impl Vm {
    fn moving_reference(v: Value) -> bool {
        !Self::image_value(v) && (matches!(v.tag(), 3..=5) || v.tag() == 0 && v != Value::NIL)
    }
    fn anchor_byte(&self, buffer: Value, anchor: anchors::Anchor) -> Option<u32> {
        match self.object_if(buffer)? {
            data::Object::Buffer(b) => b.anchors.resolve(anchor),
            _ => None,
        }
    }
    fn shape(&self, v: Value) -> Shape<'_> {
        match v.tag() {
            3 => Shape::Cons,
            4 => match self.closures.get(v.payload() as usize) {
                Some(c) => Shape::Closure(c.function, &c.captures),
                None => Shape::Opaque,
            },
            0 if v == Value::NIL => Shape::Atom,
            0 if Self::image_value(v) => match self.string_value(v) {
                Ok(s) => Shape::Text {
                    bytes: s.as_bytes(),
                    chars: s.chars().count(),
                },
                Err(_) => Shape::Opaque,
            },
            0 => match self.object_if(v) {
                Some(data::Object::Float(f)) => Shape::Float(f.to_bits()),
                Some(data::Object::Bignum(n)) => Shape::Bignum(n),
                Some(data::Object::String(s) | data::Object::RawString(s)) => Shape::Text {
                    bytes: s.bytes(),
                    chars: s.len(),
                },
                Some(data::Object::Bytes(b)) => Shape::Text {
                    bytes: b,
                    chars: b.len(),
                },
                Some(data::Object::Vector(a)) => Shape::Vector(a),
                Some(data::Object::BoolVector(a)) => Shape::BoolVector(a),
                Some(data::Object::Record(a)) => Shape::Record(a),
                Some(data::Object::Marker(m)) => Shape::Marker(
                    m.buffer,
                    m.anchor.and_then(|a| self.anchor_byte(m.buffer, a)),
                ),
                Some(data::Object::Overlay(o)) => Shape::Overlay(
                    o.buffer,
                    o.start.zip(o.end).and_then(|(a, b)| {
                        Some((
                            self.anchor_byte(o.buffer, a)?,
                            self.anchor_byte(o.buffer, b)?,
                        ))
                    }),
                    &o.properties,
                ),
                _ => Shape::Opaque,
            },
            _ => Shape::Atom,
        }
    }
    /// GNU `sxhash_obj`; `moving` is set when the hash reads a heap index.
    pub(super) fn sxhash(&self, v: Value, depth: usize, moving: &mut bool) -> u64 {
        self.sxhash_mode(v, depth, moving, false)
    }
    /// Public hashes cannot expose the arena indices repaired by table rehashing.
    pub(super) fn public_sxhash(&self, v: Value) -> u64 {
        self.sxhash_mode(v, 0, &mut false, true)
    }
    fn sxhash_mode(&self, v: Value, depth: usize, moving: &mut bool, public: bool) -> u64 {
        if depth > SXHASH_MAX_DEPTH {
            return 0;
        }
        match self.shape(v) {
            Shape::Atom if public && Self::moving_reference(v) => v.tag() as u64,
            Shape::Atom => v.0,
            Shape::Float(bits) => bits,
            Shape::Bignum(n) => hash_bytes(&n.to_signed_bytes_le()),
            Shape::Text { bytes, .. } => hash_bytes(bytes),
            Shape::BoolVector(bits) => combine(bits.len as u64, hash_bytes(&bits.bytes)),
            Shape::Cons => {
                let mut hash = 0;
                let mut tail = v;
                if depth < SXHASH_MAX_DEPTH {
                    for _ in 0..SXHASH_MAX_LEN {
                        let Ok((car, cdr)) = self.cons_pair(tail) else {
                            break;
                        };
                        hash = combine(hash, self.sxhash_mode(car, depth + 1, moving, public));
                        tail = cdr;
                    }
                }
                if tail != Value::NIL {
                    hash = combine(hash, self.sxhash_mode(tail, depth + 1, moving, public));
                }
                hash
            }
            Shape::Vector(items) | Shape::Record(items) => items
                .iter()
                .take(SXHASH_MAX_LEN)
                .fold(items.len() as u64, |h, &x| {
                    combine(h, self.sxhash_mode(x, depth + 1, moving, public))
                }),
            Shape::Closure(function, captures) => captures
                .iter()
                .take(SXHASH_MAX_LEN)
                .fold(function as u64, |h, &x| {
                    combine(h, self.sxhash_mode(x, depth + 1, moving, public))
                }),
            Shape::Marker(buffer, at) => {
                *moving |= Self::moving_reference(buffer);
                combine(if public { 0 } else { buffer.0 }, at.map_or(0, u64::from))
            }
            Shape::Overlay(_, range, _) => {
                let (a, b) = range.unwrap_or((0, 0));
                combine(a as u64, b as u64)
            }
            Shape::Opaque => {
                *moving |= Self::moving_reference(v);
                if public { 0 } else { v.0 }
            }
        }
    }
    /// The table's hash function: (hash, depends on a moving heap index).
    fn hash_key(&self, test: Test, key: Value) -> (u64, bool) {
        let mut moving = false;
        let hash = match (test, self.shape(key)) {
            (Test::Equal, _) => self.sxhash(key, 0, &mut moving),
            (Test::Eql, Shape::Float(bits)) => bits,
            (Test::Eql, Shape::Bignum(n)) => hash_bytes(&n.to_signed_bytes_le()),
            _ => {
                moving = Self::moving_reference(key);
                key.0
            }
        };
        (hash, moving)
    }
    /// GNU `Feql`: floats by bit pattern, so 0.0 and -0.0 differ and NaNs
    /// with identical bits match.
    pub(super) fn eql(&self, a: Value, b: Value) -> bool {
        a == b
            || matches!((self.shape(a), self.shape(b)),
                        (Shape::Float(x), Shape::Float(y)) if x == y)
            || matches!((self.shape(a), self.shape(b)),
                        (Shape::Bignum(x), Shape::Bignum(y)) if x == y)
    }
    fn keys_match(&self, test: Test, a: Value, b: Value) -> Hashed<bool> {
        Ok(match test {
            Test::Eq => a == b,
            Test::Eql => self.eql(a, b),
            Test::Equal => self.equal(a, b)?,
        })
    }
    /// GNU `internal_equal` (plain `equal`): cycle-aware and depth-limited.
    pub(super) fn equal(&self, a: Value, b: Value) -> Hashed<bool> {
        self.equal_at(a, b, 0, &mut HashMap::default(), false)
    }
    /// internal_equal with EQUAL_INCLUDING_PROPERTIES: strings must also
    /// carry equal text properties at every position.
    pub(crate) fn equal_including_properties(&self, a: Value, b: Value) -> Hashed<bool> {
        self.equal_at(a, b, 0, &mut HashMap::default(), true)
    }
    fn equal_at(
        &self,
        mut a: Value,
        mut b: Value,
        mut depth: usize,
        seen: &mut HashMap<u64, Vec<u64>>,
        props: bool,
    ) -> Hashed<bool> {
        loop {
            if depth > 10 {
                if depth > 200 {
                    return Err(HashFault::Error("Stack overflow in equal", None));
                }
                if matches!(
                    self.shape(a),
                    Shape::Cons
                        | Shape::Vector(_)
                        | Shape::Record(_)
                        | Shape::Closure(..)
                        | Shape::Marker(..)
                        | Shape::Overlay(..)
                        | Shape::Opaque
                ) {
                    let pairs = seen.entry(a.0).or_default();
                    if pairs.contains(&b.0) {
                        return Ok(true);
                    }
                    pairs.push(b.0);
                }
            }
            if a == b {
                return Ok(true);
            }
            match (self.shape(a), self.shape(b)) {
                (Shape::Float(x), Shape::Float(y)) => return Ok(x == y),
                (Shape::Bignum(x), Shape::Bignum(y)) => return Ok(x == y),
                (Shape::BoolVector(x), Shape::BoolVector(y)) => return Ok(x == y),
                (Shape::Text { bytes: x, chars: m }, Shape::Text { bytes: y, chars: n }) => {
                    return Ok(m == n && x == y && (!props || self.string_intervals_equal(a, b)?));
                }
                (Shape::Cons, Shape::Cons) => {
                    let (tail, other, verdict) = self.equal_spine(a, b, depth, seen, false, props)?;
                    if let Some(verdict) = verdict {
                        return Ok(verdict);
                    }
                    a = tail;
                    b = other;
                    depth += 1;
                }
                (Shape::Vector(x), Shape::Vector(y))
                | (Shape::Record(x), Shape::Record(y)) => return self.equal_items(x, y, depth, seen, props),
                (Shape::Closure(f, x), Shape::Closure(g, y)) => {
                    return Ok(f == g && self.equal_items(x, y, depth, seen, props)?);
                }
                (Shape::Marker(x, p), Shape::Marker(y, q)) => {
                    return Ok(x == y && (x == Value::NIL || p == q));
                }
                (Shape::Overlay(x, r, p), Shape::Overlay(y, s, q)) => {
                    if x != y || r != s || p.len() != q.len() {
                        return Ok(false);
                    }
                    for (&(k, v), &(l, w)) in p.iter().zip(q) {
                        if !self.equal_at(k, l, depth + 1, seen, props)?
                            || !self.equal_at(v, w, depth + 1, seen, props)?
                        {
                            return Ok(false);
                        }
                    }
                    return Ok(true);
                }
                _ => return Ok(false),
            }
        }
    }
    /// compare_string_intervals: the same properties, with `equal' values,
    /// over every run of either string (the texts are already equal).
    fn string_intervals_equal(&self, a: Value, b: Value) -> Hashed<bool> {
        let internal = |_: crate::prim::Fault| HashFault::Internal("text property runs".into());
        let mut starts = self.property_run_starts(a).map_err(internal)?;
        starts.extend(self.property_run_starts(b).map_err(internal)?);
        starts.sort_unstable();
        starts.dedup();
        let len = self.str_view(a).map_or(0, |v| v.chars as i64);
        for at in starts.into_iter().filter(|&at| at < len) {
            let x = self.annotation_raw(a, at as usize).map_err(HashFault::Internal)?;
            let y = self.annotation_raw(b, at as usize).map_err(HashFault::Internal)?;
            if x.len() != y.len() { return Ok(false); }
            for (key, value) in &x {
                let Some((_, other)) = y.iter().find(|(k, _)| k == key) else { return Ok(false) };
                if !self.equal(*value, *other)? { return Ok(false); }
            }
        }
        Ok(true)
    }
    fn equal_items(
        &self,
        x: &[Value],
        y: &[Value],
        depth: usize,
        seen: &mut HashMap<u64, Vec<u64>>,
        props: bool,
    ) -> Hashed<bool> {
        if x.len() != y.len() {
            return Ok(false);
        }
        for (&x, &y) in x.iter().zip(y) {
            if !self.equal_at(x, y, depth + 1, seen, props)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// GNU's `FOR_EACH_TAIL` walk of two lists. Returns the final non-cons
    /// tails to compare, or a verdict. `b_circular` is the
    /// `internal_equal_cycle` mode: a cycle in `a` then proves equality.
    fn equal_spine(
        &self,
        mut a: Value,
        mut b: Value,
        depth: usize,
        seen: &mut HashMap<u64, Vec<u64>>,
        b_circular: bool,
        props: bool,
    ) -> Hashed<(Value, Value, Option<bool>)> {
        // GNU `FOR_EACH_TAIL_STEP_CYCLEP`: the tortoise starts at the head and
        // only advances at power-of-two steps that are multiples of 4096.
        let mut tortoise = a;
        let mut steps = 0u64;
        while a.tag() == 3 {
            if b.tag() != 3 {
                return Ok((a, b, Some(false)));
            }
            let (x, a_next) = self.cons_pair(a).map_err(HashFault::Internal)?;
            let (y, b_next) = self.cons_pair(b).map_err(HashFault::Internal)?;
            if !self.equal_at(x, y, depth + 1, seen, props)? {
                return Ok((a, b, Some(false)));
            }
            b = b_next;
            if a_next == b {
                return Ok((a, b, Some(true)));
            }
            a = a_next;
            if a == tortoise {
                // Cycle in `a`: GNU `internal_equal_cycle` walks `b` instead.
                let verdict = if b_circular {
                    true
                } else {
                    b.tag() == 3 && self.equal_spine(b, a, depth, seen, true, props)?.2 == Some(true)
                };
                return Ok((a, b, Some(verdict)));
            }
            steps += 1;
            if steps.is_multiple_of(4096) && steps.is_power_of_two() {
                tortoise = a;
            }
        }
        if b_circular {
            return Ok((a, b, Some(false)));
        }
        Ok((a, b, None))
    }

    fn table_index(&self, v: Value) -> Hashed<usize> {
        match self.object_if(v) {
            Some(data::Object::HashTable(_)) if !Self::image_value(v) => {
                Ok(v.payload() as usize - 1)
            }
            _ => Err(HashFault::Wrong("hash-table-p", v)),
        }
    }
    fn table(&self, v: Value) -> Hashed<&HashTable> {
        let i = self.table_index(v)?;
        match &self.objects[i] {
            data::Object::HashTable(t) => Ok(t),
            _ => unreachable!("checked hash table"),
        }
    }
    fn table_mut(&mut self, i: usize) -> &mut HashTable {
        match &mut self.objects[i] {
            data::Object::HashTable(t) => t,
            _ => unreachable!("checked hash table"),
        }
    }
    /// Re-hash keys whose hash read a heap index, then rebuild the chains.
    fn fresh_table(&mut self, v: Value) -> Hashed<usize> {
        let i = self.table_index(v)?;
        let table = self.table(v)?;
        if table.stale {
            let test = table.test;
            let hashes: Vec<_> = table
                .entries()
                .filter(|(slot, _, _)| table.moving[*slot])
                .map(|(slot, key, _)| (slot, self.hash_key(test, key)))
                .collect();
            let table = self.table_mut(i);
            for (slot, (hash, moving)) in hashes {
                table.hashes[slot] = hash;
                table.moving[slot] = moving;
            }
            table.rebuild();
            table.stale = false;
        }
        Ok(i)
    }
    /// GNU `hash_find`: `eq` first, then the stored hash and the test.
    fn table_lookup(
        &mut self,
        table: Value,
        key: Value,
    ) -> Hashed<(usize, Option<usize>, u64, bool)> {
        let i = self.fresh_table(table)?;
        let t = self.table(table)?;
        let (hash, moving) = self.hash_key(t.test, key);
        let mut at = t.chain(hash);
        while at != END {
            let slot = at as usize;
            let stored = t.keys[slot];
            if key == stored || t.hashes[slot] == hash && self.keys_match(t.test, key, stored)? {
                return Ok((i, Some(slot), hash, moving));
            }
            at = t.next[slot];
        }
        Ok((i, None, hash, moving))
    }
    pub(super) fn hash_call(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
        error: &mut Option<String>,
        signal: &mut Option<(Value, Value)>,
    ) -> Option<Value> {
        let fault = match self.hash_primitive(p, start, n) {
            Ok(v) => return Some(v),
            Err(fault) => fault,
        };
        match self.hash_fault_signal(fault) {
            Ok((name, condition, data)) => {
                *error = Some(name.into());
                *signal = Some((condition, data));
            }
            Err(message) => *error = Some(message),
        }
        None
    }
    /// Ordinary `Result` adapter for callers without a signal channel.
    pub(super) fn hash_primitive_result(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        let (mut error, mut signal) = (None, None);
        self.hash_call(p, start, n, &mut error, &mut signal)
            .ok_or_else(|| error.unwrap_or_else(|| "hash table error".into()))
    }
    pub(super) fn declaration_hash_get(
        &mut self,
        table: Value,
        key: Value,
    ) -> Result<Option<Value>> {
        let (i, found, _, _) = self
            .table_lookup(table, key)
            .map_err(|_| "hash table required")?;
        Ok(found.map(|slot| self.table_mut(i).values[slot]))
    }

    pub(crate) fn completion_hash_find(&mut self, table: Value, key: Value) -> std::result::Result<Option<(Value, Value)>, prim::Fault> {
        let (index, found, _, _) = self.table_lookup(table, key).map_err(|fault| match fault {
            HashFault::Wrong(predicate, value) => prim::Fault::Wrong(predicate, value),
            HashFault::Error(message, value) => match value {
                Some(value) if message == "circular-list" => prim::Fault::Circular(value),
                _ => prim::Fault::Error(message.into()),
            },
            HashFault::Internal(message) => prim::Fault::Error(message),
            HashFault::Arity(_, _) => unreachable!(),
        })?;
        Ok(found.and_then(|slot| self.table_mut(index).entry(slot)))
    }
    pub(super) fn declaration_hash_remove(&mut self, table: Value, key: Value) -> Result<()> {
        let (i, found, _, _) = self.table_lookup(table, key).map_err(|_| "hash table required")?;
        if let Some(slot) = found {
            let value = self.table_mut(i).values[slot];
            self.table_mut(i).remove(slot);
            if let Some(binding) = value.index_if(7).and_then(|n| n.checked_sub(1)) {
                if self.local_binding_slots.contains(&binding) && !self.dynamic.iter().any(|(active, _, _, _)| *active == binding) {
                    self.slots[binding].value = Value::UNBOUND;
                }
            }
        }
        Ok(())
    }
    pub(super) fn declaration_hash_put(
        &mut self,
        table: Value,
        key: Value,
        value: Value,
    ) -> Result<()> {
        let (i, found, hash, moving) = self
            .table_lookup(table, key)
            .map_err(|_| "hash table required")?;
        let table = self.table_mut(i);
        match found {
            Some(slot) => table.values[slot] = value,
            None => table.insert(key, value, hash, moving),
        }
        Ok(())
    }
    fn hash_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Hashed<Value> {
        use Primitive::*;
        let arity = match p {
            MakeHashTable => true,
            Gethash => (2..=3).contains(&n),
            Puthash | DefineHashTableTest => n == 3,
            Remhash | Maphash | HashTableContainsP | HashNext | HashKeyAt | HashValueAt => n == 2,
            _ => n == 1,
        };
        if !arity {
            return Err(HashFault::Arity(p, n));
        }
        let args = self.stack[start..start + n].to_vec();
        let truth = |b: bool| if b { Value::TRUE } else { Value::NIL };
        match p {
            MakeHashTable => self.make_hash_table(&args),
            HashTableP => Ok(truth(self.table_index(args[0]).is_ok())),
            Gethash => {
                let (i, found, _, _) = self.table_lookup(args[1], args[0])?;
                Ok(match found {
                    Some(slot) => self.table_mut(i).values[slot],
                    None => args.get(2).copied().unwrap_or(Value::NIL),
                })
            }
            HashTableContainsP => Ok(truth(self.table_lookup(args[1], args[0])?.1.is_some())),
            Puthash => {
                let (key, value) = (args[0], args[1]);
                let (i, found, hash, moving) = self.table_lookup(args[2], key)?;
                let table = self.table_mut(i);
                match found {
                    Some(slot) => table.values[slot] = value,
                    None => table.insert(key, value, hash, moving),
                }
                Ok(value)
            }
            Remhash => {
                let (i, found, _, _) = self.table_lookup(args[1], args[0])?;
                if let Some(slot) = found {
                    self.table_mut(i).remove(slot);
                }
                Ok(Value::NIL)
            }
            Clrhash => {
                let i = self.table_index(args[0])?;
                self.table_mut(i).clear();
                Ok(args[0])
            }
            HashTableCount => Ok(Value::integer(self.table(args[0])?.count() as i64).unwrap()),
            HashTableSize => Ok(Value::integer(self.table(args[0])?.size() as i64).unwrap()),
            HashTableTest => {
                let name = match self.table(args[0])?.test {
                    Test::Eq => "eq",
                    Test::Eql => "eql",
                    Test::Equal => "equal",
                };
                Ok(self.intern(name))
            }
            HashTableWeakness => {
                let name = self.table(args[0])?.weakness.name();
                Ok(if name == "nil" {
                    Value::NIL
                } else {
                    self.intern(name)
                })
            }
            HashTableRehashSize | HashTableRehashThreshold => {
                self.table(args[0])?;
                let nominal = if p == HashTableRehashSize {
                    1.5
                } else {
                    0.8125
                };
                Ok(self.allocate_object(data::Object::Float(nominal)))
            }
            CopyHashTable => {
                let copy = self.table(args[0])?.clone();
                Ok(self.allocate_object(data::Object::HashTable(Box::new(copy))))
            }
            HashTableKeys | HashTableValues => {
                // subr-x pushes in maphash order, so the result is reversed.
                let t = self.table(args[0])?;
                let mut items: Vec<_> = t
                    .entries()
                    .map(|(_, k, v)| if p == HashTableKeys { k } else { v })
                    .collect();
                items.reverse();
                Ok(self.list_value(&items))
            }
            Maphash => {
                // Direct calls are lowered to the slot walk in `expand`. A
                // primitive cannot call Lisp: an indirect call that would have
                // to invoke FUNCTION fails explicitly rather than skip entries.
                if self.table(args[1])?.count() == 0 {
                    return Ok(Value::NIL);
                }
                Err(HashFault::Error(
                    "VM2 cannot call maphash indirectly on a non-empty table",
                    Some(args[0]),
                ))
            }
            DefineHashTableTest => {
                let property = self.intern("hash-table-test");
                let value = self.list_value(&args[1..]);
                self.put_symbol_property(args[0], property, value)
                    .map_err(HashFault::Internal)?;
                Ok(value)
            }
            HashNext => {
                let from = args[1]
                    .as_integer()
                    .map_err(|_| HashFault::Wrong("integerp", args[1]))?;
                let next = self.table(args[0])?.next_used(from.max(0) as usize);
                Ok(next.map_or(Value::NIL, |i| Value::integer(i as i64).unwrap()))
            }
            HashKeyAt | HashValueAt => {
                let at = args[1]
                    .as_integer()
                    .map_err(|_| HashFault::Wrong("integerp", args[1]))?;
                // Primitive calls never collect, so the slot found by HashNext
                // is still the one the lowered loop reads.
                let (k, v) = self
                    .table(args[0])?
                    .entry(at.max(0) as usize)
                    .ok_or_else(|| HashFault::Internal("maphash slot vanished".into()))?;
                Ok(if p == HashKeyAt { k } else { v })
            }
            _ => unreachable!("hash primitive dispatch"),
        }
    }
    /// GNU `hash_table_from_plist` (lread.c:3153), for the reader's
    /// `#s(hash-table ...)`. PLIST is everything after `hash-table`. Only
    /// `test`, `weakness`, `purecopy` and `data` are consulted, each by
    /// `plist_get` (first pair wins; a trailing key without a value is
    /// ignored), so older printers' `size`/`rehash-size` are ignored; the
    /// data checks precede `make-hash-table`'s own argument checks.
    pub(super) fn hash_table_from_plist(&mut self, plist: &[Value]) -> Hashed<Value> {
        let get = |name: Value| {
            plist
                .chunks_exact(2)
                .find(|pair| pair[0] == name)
                .map_or(Value::NIL, |pair| pair[1])
        };
        let mut params = vec![];
        for (key, keyword) in [("test", ":test"), ("weakness", ":weakness"), ("purecopy", ":purecopy")] {
            let value = get(self.intern(key));
            if value != Value::NIL {
                params.push(self.intern(keyword));
                params.push(value);
            }
        }
        let data = get(self.intern("data"));
        if data != Value::NIL && data.tag() != 3 {
            return Err(HashFault::Error("Hash table data is not a list", None));
        }
        // list_length: FOR_EACH_TAIL, then CHECK_LIST_END on the final tail.
        let mut items = vec![];
        let mut tail = data;
        while tail.tag() == 3 {
            let (car, cdr) = self.cons_pair(tail).map_err(HashFault::Internal)?;
            items.push(car);
            tail = cdr;
        }
        if tail != Value::NIL {
            return Err(HashFault::Wrong("listp", tail));
        }
        if items.len() % 2 == 1 {
            return Err(HashFault::Error("Hash table data length is odd", None));
        }
        params.push(self.intern(":size"));
        params.push(Value::integer(items.len() as i64 / 2).map_err(HashFault::Internal)?);
        let table = self.make_hash_table(&params)?;
        for pair in items.chunks_exact(2) {
            let (i, found, hash, moving) = self.table_lookup(table, pair[0])?;
            let t = self.table_mut(i);
            match found {
                Some(slot) => t.values[slot] = pair[1],
                None => t.insert(pair[0], pair[1], hash, moving),
            }
        }
        Ok(table)
    }
    /// The `(CONDITION . DATA)` a hash fault signals; `Err` carries an
    /// internal fault's text, which has no Lisp condition.
    pub(super) fn hash_fault_signal(
        &mut self,
        fault: HashFault,
    ) -> Result<(&'static str, Value, Value)> {
        let (condition, data) = match fault {
            HashFault::Wrong(predicate, value) => {
                ("wrong-type-argument", vec![self.intern(predicate), value])
            }
            HashFault::Arity(p, n) => (
                "wrong-number-of-arguments",
                vec![
                    Value::reference(p as usize, 6),
                    Value::integer(n as i64).unwrap(),
                ],
            ),
            HashFault::Error(message, datum) => {
                let text = self.allocate_object(data::Object::String(Box::new(
                    data::LispString::new(message),
                )));
                let mut values = vec![text];
                values.extend(datum);
                ("error", values)
            }
            HashFault::Internal(message) => return Err(message),
        };
        let symbol = self.intern(condition);
        let data = self.list_value(&data);
        Ok((condition, symbol, data))
    }
    /// GNU `Fmake_hash_table`: pairs are consumed from the end, so the first
    /// occurrence of a keyword wins and the last invalid keyword is reported.
    fn make_hash_table(&mut self, args: &[Value]) -> Hashed<Value> {
        if args.len() % 2 == 1 {
            return Err(HashFault::Error("Odd number of arguments", None));
        }
        let (mut test, mut weakness, mut size) = (Value::NIL, Value::NIL, Value::NIL);
        for pair in args.chunks_exact(2).rev() {
            let keyword = if pair[0].tag() == 7 && pair[0] != UNUSED {
                self.symbol_names
                    .get((pair[0].0 >> 3) as usize - 1)
                    .map(String::as_str)
            } else {
                None
            };
            match keyword {
                Some(":test") => test = pair[1],
                Some(":weakness") => weakness = pair[1],
                Some(":size") => size = pair[1],
                Some(":rehash-threshold" | ":rehash-size" | ":purecopy") => {}
                _ => {
                    return Err(HashFault::Error("Invalid keyword argument", Some(pair[0])));
                }
            }
        }
        let test = if test == Value::NIL || test == self.intern("eql") {
            Test::Eql
        } else if test == self.intern("eq") {
            Test::Eq
        } else if test == self.intern("equal") {
            Test::Equal
        } else {
            let property = self.intern("hash-table-test");
            let user = self.symbol_property(test, property).unwrap_or(Value::NIL);
            let defined = self.cons_pair(user).is_ok_and(|(_, tail)| tail.tag() == 3);
            return Err(HashFault::Error(
                if defined {
                    "VM2 does not support user-defined hash table tests"
                } else {
                    "Invalid hash table test"
                },
                Some(test),
            ));
        };
        let size = if size == Value::NIL {
            0
        } else {
            match size.integer_value() {
                Some(n) if n >= 0 => n,
                _ => return Err(HashFault::Error("Invalid hash table size", Some(size))),
            }
        };
        let weak = if weakness == Value::NIL {
            Weakness::None
        } else if weakness == self.intern("key") {
            Weakness::Key
        } else if weakness == self.intern("value") {
            Weakness::Value
        } else if weakness == self.intern("key-or-value") {
            Weakness::KeyOrValue
        } else if weakness == Value::TRUE || weakness == self.intern("key-and-value") {
            Weakness::KeyAndValue
        } else {
            return Err(HashFault::Error(
                "Invalid hash table weakness",
                Some(weakness),
            ));
        };
        if size >= MAXIMUM_SIZE {
            return Err(HashFault::Error("Hash table too large", None));
        }
        let table = HashTable::new(test, weak, size as usize);
        Ok(self.allocate_object(data::Object::HashTable(Box::new(table))))
    }
    /// GNU 30+ printed representation: `eql`, nil weakness and size are implicit.
    pub(super) fn display_table(&self, t: &HashTable, depth: usize) -> Result<String> {
        let bytes = self.print_table(t, None, |vm, v| vm.display_depth(v, depth + 1).map(String::into_bytes))?;
        String::from_utf8(bytes).map_err(|e| e.to_string())
    }
    /// GNU `print_object`'s PVEC_HASH_TABLE arm (print.c:2631-2675): the
    /// prefix `#s(hash-table`, ` test NAME` unless `eql`, ` weakness NAME`
    /// unless nil, then ` data (K V ...)` only when the table has entries,
    /// in entry order. ELEMENT prints each key and value with the CALLER's
    /// printer, so `prin1`/`princ` and their escapes apply inside the table
    /// exactly as they do outside it.
    /// `#s(hash-table ...)`, with at most LIMIT entries (print-length) then
    /// "...".
    pub(super) fn print_table(
        &self,
        t: &HashTable,
        limit: Option<usize>,
        mut element: impl FnMut(&Self, Value) -> Result<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let mut out = "#s(hash-table".to_owned();
        match t.test {
            Test::Eq => out.push_str(" test eq"),
            Test::Equal => out.push_str(" test equal"),
            Test::Eql => {}
        }
        if t.weakness != Weakness::None {
            out.push_str(" weakness ");
            out.push_str(t.weakness.name());
        }
        let mut out = out.into_bytes();
        if t.count() > 0 {
            let mut items = vec![];
            let shown = limit.unwrap_or(usize::MAX).min(t.count());
            for (_, k, v) in t.entries().take(shown) {
                items.push(element(self, k)?);
                items.push(element(self, v)?);
            }
            if shown < t.count() { items.push(b"...".to_vec()); }
            out.extend_from_slice(b" data (");
            out.extend(items.join(&b' '));
            out.push(b')');
        }
        out.push(b')');
        Ok(out)
    }
}
