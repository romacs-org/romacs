//! Strand-owned literal and sequence storage. Tag zero with a nonzero index
//! denotes an object; nil remains zero. Strings use GNU internal encoding,
//! indexed by characters rather than bytes. No global mutator or table.
use super::*;
pub(crate) use crate::bool_vector::BoolVector;
#[cfg(test)]
#[path = "string_cursor_tests.rs"]
mod string_cursor_tests;

/// Payload bytes between the engine's polls for a collection
/// (`Vm::collect_poll').
const POLL_PAYLOAD: usize = 256 << 10;

/// The bytes OBJECT holds beyond its own slot, as GNU charges them to the
/// next collection: a string's text, a vector's or record's slots, a bool
/// vector's bits.
/// The slots a collection traces in OBJECT beyond the object itself.
pub(super) fn traced_slots(object: &Object) -> usize {
    match object {
        Object::Vector(slots) | Object::Record(slots) => slots.len(),
        _ => 0,
    }
}

/// What the memory profiler charges for OBJECT when it is made: the entry
/// and the storage `heap_stats' counts as live (GNU's malloc_probe sizes).
pub(super) fn profiled_bytes(object: &Object) -> usize {
    size_of::<Object>()
        + match object {
            Object::String(s) | Object::RawString(s) => s.storage_bytes(false),
            Object::Bytes(bytes) => bytes.len(),
            Object::Vector(slots) | Object::Record(slots) => slots.len() * size_of::<Value>(),
            Object::HashTable(table) => table.storage_bytes(false),
            _ => 0,
        }
}

pub(super) fn payload_bytes(object: &Object) -> usize {
    match object {
        Object::String(s) | Object::RawString(s) => s.bytes.len(),
        Object::Bytes(bytes) => bytes.len(),
        Object::BoolVector(bits) => bits.bytes.len(),
        Object::Vector(slots) | Object::Record(slots) => slots.len() * std::mem::size_of::<Value>(),
        _ => 0,
    }
}

#[derive(Clone)]
pub(super) enum Object {
    SyntaxTable(std::collections::BTreeMap<u32, romacs_regex_engine::SyntaxClass>),
    Float(f64),
    Bignum(Box<num_bigint::BigInt>),
    Window(Box<window::Window>),
    Frame(Box<frame::Frame>),
    Terminal(Box<frame::Terminal>),
    WindowConfiguration(Box<crate::window::WindowConfiguration>),
    Sqlite(Box<crate::sqlite::Handle>),
    Treesit(Box<crate::treesit::Object>),
    Process(Box<crate::subprocess::Process>),
    Finalizer(Value), // alloc.c's: its function, nil once called (gc.rs dooms it)
    /// The main thread's object, the only one a threadless build has
    /// (VM2-THREAD1): what `process-thread' answers.
    Thread,
    String(Box<LispString>),
    RawString(Box<LispString>),
    Vector(Vec<Value>),
    BoolVector(Box<BoolVector>),
    Record(Vec<Value>),
    CharTable(Box<char_table::CharTable>),
    SubCharTable(char_table::CharTableNodeHandle),
    MapKeymapCursor(Box<crate::map_keymap::Cursor>),
    Buffer(Box<annotations::AnnotatedBuffer>),
    Marker(Box<annotations::Marker>),
    Overlay(Box<annotations::Overlay>),
    Bytes(Vec<u8>),
    ByteBuffer(Box<bytes::ByteBuffer>),
    HashTable(Box<hash::HashTable>),
    Obarray(Box<obarray::Obarray>),
    /// A free table entry (gc.rs `sweep_tables`): no Value names it but a
    /// stale one, which finds no object here.
    Free,
}
romacs_snapshot::snapshot_enum!(Object {
    Object::SyntaxTable(f0) => (f0),
    Object::Float(f0) => (f0),
    Object::Bignum(f0) => (f0),
    Object::Window(f0) => (f0),
    Object::Frame(f0) => (f0),
    Object::Terminal(f0) => (f0),
    Object::WindowConfiguration(f0) => (f0),
    // At rest: `foreign_resource` refuses a live connection or statement,
    // whose handles are SQLite's, in its own heap.
    Object::Sqlite(f0) => (),
    // At rest: `foreign_resource` refuses every live treesit object.
    Object::Treesit(f0) => (),
    Object::Process(f0) => (f0),
    Object::Finalizer(f0) => (f0),
    Object::Thread => (),
    Object::String(f0) => (f0),
    Object::RawString(f0) => (f0),
    Object::Vector(f0) => (f0),
    Object::BoolVector(f0) => (f0),
    Object::Record(f0) => (f0),
    Object::CharTable(f0) => (f0),
    Object::SubCharTable(f0) => (f0),
    Object::MapKeymapCursor(f0) => (f0),
    Object::Buffer(f0) => (f0),
    Object::Marker(f0) => (f0),
    Object::Overlay(f0) => (f0),
    Object::Bytes(f0) => (f0),
    Object::ByteBuffer(f0) => (f0),
    Object::HashTable(f0) => (f0),
    Object::Obarray(f0) => (f0),
    // Never captured: an image's tables are dense (gc.rs `collect_compacting`).
    Object::Free => (),
});
const _: [(); 32] = [(); size_of::<Object>()];
impl Object {
    /// The resource of this process an object holds, which no other
    /// process could use, named; None for data. Every variant is named
    /// here, so a new one is classified before it compiles.
    pub(super) fn foreign_resource(&self) -> Option<&'static str> {
        match self {
            Object::Sqlite(handle) => handle.is_live().then_some("a live sqlite object"),
            Object::Treesit(t) => t.foreign_resource(),
            // Its pipes, pty and child are the subprocess table's, which a
            // VM at rest holds none of; a live process is one of them.
            Object::Process(process) => process.alive.then_some("a live process"),
            // The terminal's device is the terminal host's, which a VM at
            // rest holds none of; the object is its parameters.
            Object::Terminal(_) => None,
            Object::SyntaxTable(_) | Object::Float(_) | Object::Bignum(_) | Object::Window(_) | Object::Frame(_)
            | Object::WindowConfiguration(_) | Object::String(_) | Object::RawString(_) | Object::Vector(_)
            | Object::BoolVector(_) | Object::Record(_) | Object::CharTable(_) | Object::SubCharTable(_)
            | Object::MapKeymapCursor(_) | Object::Buffer(_) | Object::Marker(_) | Object::Overlay(_) | Object::Bytes(_)
            | Object::ByteBuffer(_) | Object::HashTable(_) | Object::Obarray(_) | Object::Finalizer(_) | Object::Thread | Object::Free => None,
        }
    }
}
/// String slots contain valid UTF-8; RawString slots contain all other bytes.
#[derive(Clone)]
pub(super) struct LispString {
    bytes: Vec<u8>,
    chars: usize,
    flags: u8,
    pub(crate) properties: crate::properties::Properties,
    cursors: [std::cell::Cell<(usize, usize)>; 2],
    #[cfg(test)]
    pub(super) scanned: std::cell::Cell<usize>,
    #[cfg(test)]
    pub(super) flag_scanned: std::cell::Cell<usize>,
}
romacs_snapshot::snapshot_struct!(LispString { bytes, chars, flags, properties, cursors, #[cfg(test)] scanned, #[cfg(test)] flag_scanned });
const MULTIBYTE: u8 = 1;
const ASCII: u8 = 2;
const UTF8: u8 = 4;
/// The heap object of STRING: a `String` when it is UTF-8, else a
/// `RawString` (a multibyte string with raw bytes).
/// Unibyte text as an object: ASCII keeps the string slot, which every
/// reader of text takes; only bytes above 127 are a byte string.
pub(crate) fn unibyte_object(bytes: Vec<u8>) -> Object {
    if bytes.is_ascii() { string_object(LispString::from_bytes(bytes, false)) } else { Object::Bytes(bytes) }
}

pub(crate) fn string_object(string: LispString) -> Object {
    let string = Box::new(string);
    if string.is_utf8() { Object::String(string) } else { Object::RawString(string) }
}

impl LispString {
    pub fn new(s: &str) -> Self {
        let chars=s.chars().count();
        Self {
            bytes:s.as_bytes().to_vec(), chars,
            flags: MULTIBYTE | UTF8 | if s.is_ascii() { ASCII } else { 0 },
            properties: crate::properties::Properties::new(),
            cursors:[std::cell::Cell::new((0,0)),std::cell::Cell::new((chars,s.len()))],
            #[cfg(test)]
            scanned: std::cell::Cell::new(0),
            #[cfg(test)]
            flag_scanned: std::cell::Cell::new(0),
        }
    }
    pub fn from_bytes(bytes: Vec<u8>, multibyte: bool) -> Self {
        if bytes.is_ascii() {
            let chars=bytes.len();
            return Self {
                bytes, chars, flags: ASCII | UTF8 | if multibyte { MULTIBYTE } else { 0 },
                properties: crate::properties::Properties::new(),
                cursors:[std::cell::Cell::new((0,0)),std::cell::Cell::new((chars,chars))],
                #[cfg(test)]
                scanned: std::cell::Cell::new(0),
            #[cfg(test)]
            flag_scanned: std::cell::Cell::new(0),
            };
        }
        let utf8 = std::str::from_utf8(&bytes);
        let is_utf8 = bytes.is_ascii() || (multibyte && utf8.is_ok());
        let chars = if !multibyte {
            bytes.len()
        } else if let Ok(text) = utf8 {
            text.chars().count()
        } else {
            let mut rest = bytes.as_slice();
            let mut count = 0;
            while !rest.is_empty() {
                let (_, width) =
                    crate::lstring::decode_char(rest).expect("valid internal string encoding");
                rest = &rest[width..];
                count += 1;
            }
            count
        };
        let byte_len = bytes.len();
        let is_ascii = bytes.is_ascii();

        Self {
            bytes,
            chars,
            flags: u8::from(multibyte) * MULTIBYTE | u8::from(is_ascii) * ASCII | u8::from(is_utf8) * UTF8,
            properties: crate::properties::Properties::new(),
            cursors: [
                std::cell::Cell::new((0, 0)),
                std::cell::Cell::new((chars, byte_len)),
            ],
            #[cfg(test)]
            scanned: std::cell::Cell::new(0),
            #[cfg(test)]
            flag_scanned: std::cell::Cell::new(0),
        }
    }
    pub fn as_str(&self) -> &str {
        assert!(self.is_utf8(), "String slot must contain UTF-8");
        // Construction and every byte mutation update is_utf8.
        unsafe { std::str::from_utf8_unchecked(&self.bytes) }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn is_utf8(&self) -> bool { self.flags & UTF8 != 0 }
    fn is_ascii(&self) -> bool { self.flags & ASCII != 0 }
    pub fn is_multibyte(&self) -> bool {
        self.flags & MULTIBYTE != 0
    }
    pub fn view(&self) -> crate::lstring::StrView<'_> {
        crate::lstring::StrView {
            bytes: &self.bytes,
            chars: self.chars,
            multibyte: self.is_multibyte(),
        }
    }
    pub fn len(&self) -> usize {
        self.chars
    }
    pub fn get(&self, c: coordinates::CharOffset) -> Option<char> {
        self.char_code(c).and_then(char::from_u32)
    }
    // Keep the byte-indexed path in callers: the full-range decoder otherwise
    // gives ASCII reads its call frame and Option<u32> return ABI overhead.
    #[inline(always)]
    pub fn char_code(&self, c: coordinates::CharOffset) -> Option<u32> {
        if self.flags & (MULTIBYTE | ASCII) != MULTIBYTE {
            return self.bytes.get(c.0).map(|&b| b as u32);
        }
        self.multibyte_char_code(c)
    }
    #[inline(never)]
    fn multibyte_char_code(&self, c: coordinates::CharOffset) -> Option<u32> {
        if c.0 >= self.chars { return None; }
        let (next, byte) = self.cursors[0].get();
        if c.0 == next {
            let (ch, width) = crate::lstring::string_char(&self.bytes[byte..]);
            self.cursors[0].set((next + 1, byte + width));
            return Some(ch);
        }
        self.byte_offset(c)
            .map(|b| crate::lstring::string_char(&self.bytes[b..]).0)
    }
    pub(super) fn byte_offset(&self, c: coordinates::CharOffset) -> Option<usize> {
        let target = c.0;
        if target > self.chars {
            return None;
        }
        if !self.is_multibyte() || self.is_ascii() {
            return Some(target);
        }
        let a = self.cursors[0].get();
        let z = self.cursors[1].get();
        let slot = usize::from(target.abs_diff(z.0) < target.abs_diff(a.0));
        let (mut character, mut byte) = self.cursors[slot].get();
        let distance = target.abs_diff(character);
        if target < distance {
            character = 0;
            byte = 0;
        }
        if self.chars - target < target.abs_diff(character) {
            character = self.chars;
            byte = self.bytes.len();
        }
        #[cfg(test)]
        self.scanned
            .set(self.scanned.get() + target.abs_diff(character));
        let bytes = &self.bytes;
        while character < target {
            byte += 1;
            while byte < bytes.len() && bytes[byte] & 0xc0 == 0x80 {
                byte += 1;
            }
            character += 1;
        }
        while character > target {
            byte -= 1;
            while bytes[byte] & 0xc0 == 0x80 {
                byte -= 1;
            }
            character -= 1;
        }
        self.cursors[slot].set((target, byte));
        Some(byte)
    }
    pub fn set(&mut self, c: coordinates::CharOffset, ch: char) -> Result<()> {
        self.set_code(c, ch as u32)
    }
    pub fn set_code(&mut self, c: coordinates::CharOffset, code: u32) -> Result<()> {
        if code > crate::lstring::MAX_CHAR {
            return Err("character required".into());
        }
        let old_code = self.char_code(c).ok_or("index out of range")?;
        let b = self.byte_offset(c).ok_or("index out of range")?;
        let old_width = if self.is_multibyte() {
            crate::lstring::string_char(&self.bytes[b..]).1
        } else {
            1
        };
        let mut encoded = [0; 5];
        let width = if self.is_multibyte() {
            crate::lstring::char_string(code, &mut encoded)
        } else {
            encoded[0] = u8::try_from(code)
                .map_err(|_| "Attempt to store non-byte value into unibyte string")?;
            1
        };
        self.properties.resize_character(
            coordinates::ByteOffset(b)..coordinates::ByteOffset(b + old_width),
            width,
        );
        self.bytes
            .splice(b..b + old_width, encoded[..width].iter().copied());
        // Only crossing a character class can invalidate these cached predicates.
        if code >= 128 {
            self.flags &= !ASCII;
        } else if old_code >= 128 {
            #[cfg(test)]
            self.flag_scanned.set(self.flag_scanned.get() + self.bytes.len());
            if self.bytes.is_ascii() { self.flags |= ASCII; }
        }
        let multibyte = self.is_multibyte();
        let valid_utf8_char = |c| c < 128 || (multibyte && char::from_u32(c).is_some());
        if !valid_utf8_char(code) {
            self.flags &= !UTF8;
        } else if !valid_utf8_char(old_code) {
            #[cfg(test)]
            self.flag_scanned.set(self.flag_scanned.get() + self.bytes.len());
            if std::str::from_utf8(&self.bytes).is_ok() { self.flags |= UTF8; }
        }
        self.cursors[0].set((c.0, b));
        self.cursors[1].set((self.chars, self.bytes.len()));
        Ok(())
    }
    pub fn storage_bytes(&self, capacity: bool) -> usize {
        size_of::<Self>()
            + self.properties.storage_bytes()
            + if capacity {
                self.bytes.capacity()
            } else {
                self.bytes.len()
            }
    }
}

/// Leaf fault construction never dispatches, so hot handlers can tail-call it.
#[cold]
#[inline(never)]
pub(super) fn type_error(e: &mut threaded::Engine<'_>, predicate: &str, value: Value) -> Value {
    let pred = e.vm.slot(predicate);
    let condition = e.vm.slot("wrong-type-argument");
    let data = e.vm.list_value(&[Value::reference(pred + 1, 7), value]);
    e.escape = Some(threaded::Escape::Signal(
        Value::reference(condition + 1, 7),
        data,
    ));
    Value::UNBOUND
}

impl Vm {
    pub(super) fn allocate_record(&mut self, values: Vec<Value>) -> Result<Value> {
        if values.len() > 4095 {
            return Err(format!("Attempt to allocate a record of {} slots; max is 4095", values.len()));
        }
        Ok(self.allocate_object(Object::Record(values)))
    }

    /// Parse a signed integer without narrowing through a machine integer.
    /// Small results retain the existing immediate fixnum representation.
    pub(crate) fn integer_from_radix(&mut self, digits: &str, radix: u32) -> Result<Value> {
        if !(2..=36).contains(&radix) {
            return Err("invalid integer radix".into());
        }
        let magnitude = digits.strip_prefix(['+', '-']).unwrap_or(digits);
        if magnitude.is_empty()
            || !magnitude.chars().all(|c| c.is_ascii() && c.to_digit(radix).is_some())
        {
            return Err("invalid integer digits".into());
        }
        let integer = num_bigint::BigInt::parse_bytes(digits.as_bytes(), radix)
            .ok_or("invalid integer digits")?;
        if let Ok(n) = i64::try_from(&integer) {
            if let Ok(value) = Value::integer(n) {
                return Ok(value);
            }
        }
        Ok(self.allocate_object(Object::Bignum(Box::new(integer))))
    }

    pub(crate) fn str_view(&self, value: Value) -> Option<crate::lstring::StrView<'_>> {
        if Self::image_string(value) {
            let text = self.string_value(value).ok()?;
            return Some(crate::lstring::StrView {
                bytes: text.as_bytes(),
                chars: text.chars().count(),
                // An image holds literals, whose kind is the reader's rule.
                multibyte: !text.is_ascii(),
            });
        }
        match self.object_if(value)? {
            Object::String(s) | Object::RawString(s) => Some(s.view()),
            Object::Bytes(bytes) => Some(crate::lstring::StrView {
                bytes,
                chars: bytes.len(),
                multibyte: false,
            }),
            _ => None,
        }
    }
    pub(crate) fn make_lisp_string(&mut self, bytes: Vec<u8>, multibyte: bool) -> Value {
        if !multibyte {
            return self.allocate_object(unibyte_object(bytes));
        }
        self.allocate_object(string_object(LispString::from_bytes(bytes, multibyte)))
    }
    /// Charge BYTES of payload to the next collection, and ask the engine
    /// to poll for one whenever the charge passes another `POLL_PAYLOAD'.
    pub(super) fn charge_payload(&mut self, bytes: usize) {
        let before = self.gc_payload_bytes;
        self.gc_payload_bytes = before.saturating_add(bytes);
        self.collect_poll |= self.gc_payload_bytes / POLL_PAYLOAD != before / POLL_PAYLOAD;
    }
    /// A new object: the lowest free entry of the table, else a new one
    /// (gc.rs `FreeEntries'). Every object enters here.
    pub(super) fn allocate_object(&mut self, object: Object) -> Value {
        self.charge_payload(payload_bytes(&object));
        self.free.allocated += profiled_bytes(&object) as u64;
        let index = match self.free.objects.pop() {
            Some(index) => {
                self.objects[index as usize] = object;
                index as usize
            }
            None => {
                self.objects.push(object);
                self.objects.len() - 1
            }
        };
        if let Some(referrers) = &mut self.referrers { referrers.list(index, &self.objects[index]); }
        Value::reference(index + 1, 0)
    }
    /// A new cons (CAR . CDR), as `allocate_object' makes an object.
    #[inline]
    pub(super) fn alloc_cons(&mut self, car: Value, cdr: Value) -> Value {
        self.free.allocated += size_of::<(Value, Value)>() as u64;
        let index = match self.free.conses.pop() {
            Some(index) => {
                self.conses[index as usize] = (car, cdr);
                index as usize
            }
            None => {
                self.conses.push((car, cdr));
                self.conses.len() - 1
            }
        };
        Value::reference(index, 3)
    }
    /// A new cell holding VALUE: its index (a cell Value is tag 5).
    #[inline]
    pub(super) fn alloc_cell(&mut self, value: Value) -> usize {
        self.free.allocated += size_of::<Value>() as u64;
        match self.free.cells.pop() {
            Some(index) => {
                self.cells[index as usize] = value;
                index as usize
            }
            None => {
                self.cells.push(value);
                self.cells.len() - 1
            }
        }
    }
    /// A new closure.
    #[inline]
    pub(super) fn alloc_closure(&mut self, closure: Closure) -> Value {
        self.free.allocated += (size_of::<Closure>() + (closure.captures.len() + closure.metadata.len()) * size_of::<Value>()) as u64;
        let index = match self.free.closures.pop() {
            Some(index) => {
                self.closures[index as usize] = closure;
                index as usize
            }
            None => {
                self.closures.push(closure);
                self.closures.len() - 1
            }
        };
        Value::reference(index, 4)
    }
    /// The object V names, private or frozen (`frozen.rs`); never an
    /// executable-image string, which `string_value` reads.
    pub(super) fn object(&self, value: Value) -> Result<&Object> {
        let index = value.index(0)?;
        if index & crate::image::IMAGE != 0 {
            return self.frozen_object(index).ok_or_else(|| "invalid frozen object reference".into());
        }
        self.objects
            .get(index.checked_sub(1).ok_or("sequence required")?)
            .filter(|object| !matches!(object, Object::Free))
            .ok_or_else(|| "invalid object reference".into())
    }
    // Non-diagnostic type discrimination, total over private and frozen
    // objects; executable-image strings are not objects.
    pub(super) fn object_if(&self, value: Value) -> Option<&Object> {
        let index = value.index_if(0)?;
        if index & crate::image::IMAGE != 0 {
            return self.frozen_object(index);
        }
        self.objects.get(index.checked_sub(1)?).filter(|object| !matches!(object, Object::Free))
    }
    /// The private object V names, for writing: a frozen or image value
    /// refuses with GNU's read-only error, left pending (`refuse_read_only`).
    pub(super) fn object_mut(&mut self, value: Value) -> Result<&mut Object> {
        let index = value.index(0)?;
        if index & crate::image::IMAGE != 0 {
            return Err(self.refuse_read_only(value));
        }
        self.objects
            .get_mut(index.checked_sub(1).ok_or("sequence required")?)
            .filter(|object| !matches!(object, Object::Free))
            .ok_or_else(|| "invalid object reference".into())
    }
    /// The private cons V names, for writing; frozen and image conses refuse
    /// as `object_mut` does.
    pub(super) fn cons_mut(&mut self, value: Value) -> Result<&mut (Value, Value)> {
        let index = value.index(3)?;
        if index & crate::image::IMAGE != 0 {
            return Err(self.refuse_read_only(value));
        }
        self.conses.get_mut(index).ok_or_else(|| "invalid cons reference".into())
    }
    pub(super) fn literal(&mut self, f: &Form) -> Result<Value> {
        match f {
            Form::Generated(id) => Ok(self.generated_symbol(*id)),
            // The identical object, as GNU's `quote' returns it.
            Form::Object(object) => object.value(self),
            Form::Int(n) => Value::integer(*n),
            Form::Float(n) => Ok(self.allocate_object(Object::Float(*n))),
            Form::Sym(s) if s == "nil" => Ok(Value::NIL),
            Form::Sym(s) if s == "t" => Ok(Value::TRUE),
            Form::Sym(s) => {
                let slot = self.slot(s);
                Ok(Value::reference(slot + 1, 7))
            }
            // Unibyte text; ASCII keeps its string slot, as the reader's.
            Form::Bytes(bytes) if bytes.is_ascii() => {
                Ok(self.allocate_object(Object::String(Box::new(LispString::from_bytes(bytes.clone(), false)))))
            }
            Form::Bytes(bytes) => Ok(self.byte_string(bytes)),
            Form::String(s) => {
                Ok(self.allocate_object(Object::String(Box::new(LispString::new(s)))))
            }
            Form::Vector(a) => {
                let values = a.iter().map(|f| self.literal(f)).collect::<Result<_>>()?;
                Ok(self.allocate_object(Object::Vector(values)))
            }
            Form::Dotted(a, tail) => {
                let mut tail = self.literal(tail)?;
                for item in a.iter().rev() {
                    let value = self.literal(item)?;
                    tail = self.alloc_cons(value, tail);
                }
                Ok(tail)
            }
            Form::List(a) => {
                let values = a
                    .iter()
                    .map(|f| self.literal(f))
                    .collect::<Result<Vec<_>>>()?;
                Ok(self.list_value(&values))
            }
        }
    }
    pub(super) fn list_value(&mut self, values: &[Value]) -> Value {
        let mut tail = Value::NIL;
        for &value in values.iter().rev() {
            tail = self.alloc_cons(value, tail);
        }
        tail
    }
    pub(super) fn list_values(&self, mut value: Value) -> Result<Vec<Value>> {
        let mut values = Vec::new();
        while value != Value::NIL {
            let pair = self.cons_pair(value)?;
            values.push(pair.0);
            value = pair.1;
            if values.len() > self.list_limit() {
                return Err("circular list".into());
            }
        }
        Ok(values)
    }
    #[inline(never)]
    /// `apply`'s spread window: (FUNCTION, OFFSET, COUNT), or Err(COUNT)
    /// when COUNT arguments do not fit the stack's reserved tail.
    pub(super) fn prepare_apply(
        &mut self,
        start: usize,
        argc: usize,
    ) -> Result<std::result::Result<(Value, usize, usize), usize>> {
        if argc < 1 {
            return Err("wrong primitive arity".into());
        }
        // GNU eval.c Fapply: FUNCTION, ARGS..., then the spread list; with a
        // single argument the list's first element is the function.
        let tail = self.list_values(self.stack[start + argc - 1])?;
        let mut values = self.stack[start..start + argc - 1].to_vec();
        values.extend(tail);
        let target = values.first().copied().unwrap_or(Value::NIL);
        let values = values.get(1..).unwrap_or_default();
        if values.len() > self.spread_reserve {
            return Ok(Err(values.len()));
        }
        let offset = self.stack.len() - self.spread_reserve;
        self.stack[offset..offset + values.len()].copy_from_slice(values);
        Ok(Ok((target, offset, values.len())))
    }
    #[inline(never)]
    pub(super) fn primitive_call(
        &mut self,
        p: Primitive,
        start: usize,
        argc: usize,
        error_out: &mut Option<String>,
        signal_out: &mut Option<(Value, Value)>,
    ) -> Option<Value> {
        if matches!(
            p,
            Primitive::DefineError | Primitive::ErrorMessage | Primitive::Get | Primitive::Put
        ) {
            return self.condition_call(p, start, argc, error_out, signal_out);
        }
        if regexp::is_regexp_primitive(p) {
            return self.regexp_call(p, start, argc, error_out, signal_out);
        }
        if hash::is_hash_primitive(p) {
            return self.hash_call(p, start, argc, error_out, signal_out);
        }
        match self.primitive(p, start, argc) {
            Ok(value) => Some(value),
            Err(error) => {
                *signal_out = self.primitive_condition(p, start, argc, &error);
                *error_out = Some(error);
                None
            }
        }
    }
    #[inline(never)]
    pub(super) fn primitive_condition(
        &mut self,
        p: Primitive,
        start: usize,
        argc: usize,
        error: &str,
    ) -> Option<(Value, Value)> {
        let x = if argc > 0 {
            self.stack[start]
        } else {
            Value::NIL
        };
        let y = if argc > 1 {
            self.stack[start + 1]
        } else {
            Value::NIL
        };
        if error == "buffer mutation refused: read-only" {
            let condition = self.intern("buffer-read-only");
            let data = self.list_value(&[self.slots[self.buffer_slot].value]);
            return Some((condition, data));
        }
        if matches!(p, Primitive::Format | Primitive::Message) && error == "floating-point overflow" {
            return Some((self.intern("overflow-error"), Value::NIL));
        }
        if p == Primitive::Featurep && error == "featurep subfeature fault" {
            let fault = self.feature_subfeature_matches(x, y).err()?;
            return Some(self.fault(fault));
        }
        // get_keymap (keymap.c): (wrong-type-argument keymapp OBJECT).
        if matches!(p, Primitive::LookupKey | Primitive::UseLocalMap) && error == "keymap required" {
            let condition = self.intern("wrong-type-argument");
            let keymapp = self.intern("keymapp");
            let data = self.list_value(&[keymapp, x]);
            return Some((condition, data));
        }
        // CHECK_CHARACTER_CDR (keymap.c:1158): the range's end is the datum.
        if p == Primitive::DefineKey && error == "character range end must be a character" {
            let Some(Object::Vector(events)) = self.object_if(y) else { return None };
            let end = events.iter().find_map(|&event| self.cons_pair(event).ok().filter(|(from, _)| from.integer_value().is_some()))?.1;
            let condition = self.intern("wrong-type-argument");
            let characterp = self.intern("characterp");
            let data = self.list_value(&[characterp, end]);
            return Some((condition, data));
        }
        // The bulk-verified core primitives re-derive GNU's exact data.
        if let Some((condition, values)) = self.core_condition(p, start, argc, error) {
            let slot = self.slot(condition);
            let data = self.list_value(&values);
            return Some((Value::reference(slot + 1, 7), data));
        }
        // A forwarded (built-in) variable's store fault: GNU's condition,
        // with the value being stored as its datum.
        if matches!(
            p,
            Primitive::Set
                | Primitive::Makunbound
                | Primitive::MakeLocalVariable
                | Primitive::MakeVariableBufferLocal
                | Primitive::DefvarSetDefault
        ) && let Some(condition) = self.forwarded_condition(error, y)
        {
            return Some(condition);
        }
        // Bootstrap Fset uses the Leaf cell writer. Preserve its complete GNU
        // condition data instead of the dispatcher's lossy text fallback.
        let (condition, values) = if error == "Marker does not point anywhere" {
            let message = self.make_lisp_string(error.as_bytes().to_vec(), true);
            ("error", vec![message])
        } else if error == "arithmetic error" {
            ("arith-error", vec![])
        } else if matches!(p, Primitive::Provide | Primitive::Featurep)
            && error == "wrong primitive arity"
        {
            ("wrong-number-of-arguments", vec![prim::legacy_value(p)?, Value::integer(argc as i64).ok()?])
        } else if p == Primitive::Provide && error == "provide subfeatures list required" {
            let predicate = self.intern("listp");
            ("wrong-type-argument", vec![predicate, y])
        } else if error.starts_with("not-unicode-string") {
            ("not-unicode-string", vec![x])
        } else if (p == Primitive::Fset && error == "fset cyclic function indirection")
            || (p == Primitive::Defalias && error == "defalias cyclic function indirection")
        {
            ("cyclic-function-indirection", vec![x])
        } else if p == Primitive::Fset && error == "fset setting constant" {
            ("setting-constant", vec![x])
        } else if p == Primitive::Fset && error == "fset symbol required" {
            let predicate = self.intern("symbolp");
            ("wrong-type-argument", vec![predicate, x])
        } else if p == Primitive::SymbolValue && error == "unbound variable" {
            ("void-variable", vec![x])
        } else if error == "setting constant"
            && matches!(
                p,
                Primitive::Set | Primitive::Makunbound | Primitive::MakeLocalVariable
            )
        {
            ("setting-constant", vec![x])
        } else if matches!(
            p,
            Primitive::Consp | Primitive::CarSafe | Primitive::Symbolp
        ) && error == "wrong primitive arity"
        {
            let target = prim::legacy_value(p)?;
            (
                "wrong-number-of-arguments",
                vec![target, Value::integer(argc as i64).ok()?],
            )
        } else if p == Primitive::Aset && error == "character required" {
            let predicate = self.intern("characterp");
            (
                "wrong-type-argument",
                vec![predicate, self.stack[start + 2]],
            )
        } else if (error == "Attempt to store non-byte value into unibyte string" || error.starts_with("Cannot convert character at index")) {
            let message = self.make_lisp_string(error.as_bytes().to_vec(), true);
            ("error", vec![message])
        } else if error.contains("index out of range") {
            ("args-out-of-range", vec![x, y])
        } else if error.contains("required") || error.contains("value type") {
            use Primitive::*;
            let (predicate, value) = match p {
                Load => ("stringp", x),
                Require | Provide | Featurep => ("symbolp", x),
                Car | Cdr => ("listp", x),
                Memq | Assq => ("listp", y),
                Nth if x.integer_value().is_none() => ("integerp", x),
                Nth => {
                    // nthcdr errors blame the original list; a final car error
                    // blames the reached atom. Rewalk only on this cold path.
                    let mut tail = y;
                    let mut remaining = x.integer_value().unwrap().max(0) as usize;
                    while remaining != 0 {
                        let Ok((_, next)) = self.cons_pair(tail) else {
                            break;
                        };
                        tail = next;
                        remaining -= 1;
                    }
                    ("listp", if remaining == 0 { tail } else { y })
                }
                Setcar | Setcdr => ("consp", x),
                Length | Nreverse | CopySequence => ("sequencep", x),
                Aref | Aset
                    if !Self::image_string(x) && self.object_if(x).is_none() =>
                {
                    ("arrayp", x)
                }
                Aref | Aset if matches!(self.object_if(x), Some(Object::Record(_))) => ("fixnump", y),
                Aref | Aset => ("integerp", y),
                CharAfter | OverlaysAt | NextOverlayChange => ("integer-or-marker-p", x),
                OverlaysIn => ("integer-or-marker-p", if self.scalar_position(x).is_ok() { y } else { x }),
                BufferSize => ("bufferp", x),
                Add | Sub | Mul | Div | Abs | Less | Equal | Inc | Dec | Min | Max => {
                    let value = self.stack[start..start + argc]
                        .iter()
                        .copied()
                        .find(|v| {
                            self.coerce_number(*v, p != Abs).is_err()
                        })
                        .unwrap_or(x);
                    (
                        if p == Abs {
                            "numberp"
                        } else {
                            "number-or-marker-p"
                        },
                        value,
                    )
                }
                _ => ("listp", x),
            };
            let slot = self.slot(predicate);
            (
                "wrong-type-argument",
                vec![Value::reference(slot + 1, 7), value],
            )
        } else {
            return None;
        };
        let slot = self.slot(condition);
        let data = self.list_value(&values);
        Some((Value::reference(slot + 1, 7), data))
    }
    pub(super) fn data_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        use Primitive::*;
        let arity = match p {
            Vector => n,
            Aset => 3,
            CopySequence => 1,
            _ => 2,
        };
        if n != arity {
            return Err("wrong primitive arity".into());
        }
        if p == Vector {
            return Ok(self.allocate_object(Object::Vector(self.stack[start..start + n].to_vec())));
        }
        let x = self.stack[start];
        let y = if n > 1 {
            self.stack[start + 1]
        } else {
            Value::NIL
        };
        match p {
            Aref | Aset => {
                let index = usize::try_from(y.as_integer()?).map_err(|_| "index out of range")?;
                if Self::image_string(x) {
                    if p == Aset {
                        return Err(self.refuse_read_only(x));
                    }
                    return Value::integer(
                        self.string_value(x)?
                            .chars()
                            .nth(index)
                            .ok_or("index out of range")? as i64,
                    );
                }
                if p == Aref && x.tag() == 4 && !self.closures[x.index(4)?].metadata.is_empty() {
                    return self.closures[x.index(4)?].metadata.get(index).copied().ok_or_else(|| "index out of range".into());
                }
                // CLOSURE_CONSTANTS of a closure made where no lexical
                // binding is visible: the dialect's empty environment, as
                // nadvice reads it (VM2-CLOSEQ1).
                // An ordinary closure's lambda list, slot 0 (VM2-CLOSEQ1).
                if p == Aref && x.tag() == 4 && index == 0 {
                    let function = self.closures[x.index(4)?].function;
                    if let Some(params) = self.functions[function].params.clone() {
                        return self.literal(&Form::List(params));
                    }
                }
                if p == Aref && x.tag() == 4 && index == 2 {
                    let function = self.closures[x.index(4)?].function;
                    if let Some(dynamic) = self.functions[function].dynamic {
                        return Ok(if dynamic { Value::NIL } else { self.list_value(&[Value::TRUE]) });
                    }
                }
                if p == Aref {
                    return match self.object(x)? {
                        Object::String(s) | Object::RawString(s) => Value::integer(
                            s.char_code(coordinates::CharOffset(index))
                                .ok_or("index out of range")? as i64,
                        ),
                        Object::BoolVector(v) => v.get(index).map(|b| if b { Value::TRUE } else { Value::NIL }).ok_or_else(|| "index out of range".into()),
                        Object::CharTable(_) => self.char_table_get(x, index),
                        Object::Vector(v) | Object::Record(v) => v
                            .get(index)
                            .copied()
                            .ok_or_else(|| "index out of range".into()),
                        Object::Bytes(v) => {
                            Value::integer(*v.get(index).ok_or("index out of range")? as i64)
                        }
                        Object::SubCharTable(_) | Object::MapKeymapCursor(_)
                        | Object::Float(_)
                        | Object::Bignum(_)
                        | Object::Buffer(_)
                        | Object::ByteBuffer(_)
                        | Object::Marker(_)
                        | Object::Overlay(_)
                        | Object::Window(_)
                        | Object::Obarray(_)
                        | Object::Free
                            | Object::Frame(_)
                            | Object::Terminal(_)
                            | Object::WindowConfiguration(_)
                            | Object::Sqlite(_)
                            | Object::Treesit(_)
                            | Object::Finalizer(_)
                            | Object::Thread
                            | Object::Process(_)
                        | Object::HashTable(_)
                        | Object::SyntaxTable(_) => Err("array required".into()),
                    };
                }
                self.object(x)?;
                let value = self.stack[start + 2];
                let table_slot = self.literal_table_slot(value);
                match self.object_mut(x)? {
                    Object::String(s) | Object::RawString(s) => {
                        if index >= s.len() { return Err("index out of range".into()); }
                        let code = value
                            .integer_value()
                            .and_then(|n| u32::try_from(n).ok())
                            .filter(|&n| n <= crate::lstring::MAX_CHAR)
                            .ok_or("character required")?;
                        s.set_code(coordinates::CharOffset(index), code)?;
                    }
                    Object::BoolVector(v) => {
                        if index >= v.len { return Err("index out of range".into()); }
                        v.set(index, value != Value::NIL);
                    }
                    Object::CharTable(t) => t.set_range_slot(index, index, table_slot)?,
                    Object::Vector(v) | Object::Record(v) => *v.get_mut(index).ok_or("index out of range")? = value,
                    Object::Bytes(v) => {
                        if index >= v.len() { return Err("index out of range".into()); }
                        let code = value
                            .integer_value()
                            .and_then(|n| u32::try_from(n).ok())
                            .filter(|&n| n <= crate::lstring::MAX_CHAR)
                            .ok_or("character required")?;
                        let octet = u8::try_from(code)
                            .map_err(|_| "Attempt to store non-byte value into unibyte string")?;
                        *v.get_mut(index).ok_or("index out of range")? = octet;
                    }
                    Object::SubCharTable(_) | Object::MapKeymapCursor(_)
                    | Object::Float(_)
                    | Object::Bignum(_)
                    | Object::Buffer(_)
                    | Object::ByteBuffer(_)
                    | Object::Marker(_)
                    | Object::Overlay(_)
                    | Object::Window(_)
                        | Object::Obarray(_)
                        | Object::Free
                        | Object::Frame(_)
                        | Object::Terminal(_)
                        | Object::WindowConfiguration(_)
                            | Object::Sqlite(_)
                            | Object::Treesit(_)
                            | Object::Finalizer(_)
                            | Object::Thread
                            | Object::Process(_)
                        | Object::HashTable(_)
                    | Object::SyntaxTable(_) => {
                        return Err("array required".into());
                    }
                }
                let slot = self.object_mut(x)?;
                let promote = matches!(slot, Object::String(s) if !s.is_utf8());
                let demote = matches!(slot, Object::RawString(s) if s.is_utf8());
                if promote || demote {
                    let old = std::mem::replace(slot, Object::Bytes(Vec::new()));
                    let (Object::String(s) | Object::RawString(s)) = old else {
                        unreachable!()
                    };
                    *slot = if promote {
                        Object::RawString(s)
                    } else {
                        Object::String(s)
                    };
                }
                Ok(value)
            }
            CopySequence => {
                if x == Value::NIL {
                    return Ok(x);
                }
                if x.tag() == 3 {
                    let values = self.list_values(x)?;
                    return Ok(self.list_value(&values));
                }
                if Self::image_string(x) {
                    let copy = LispString::new(self.string_value(x)?);
                    return Ok(self.allocate_object(Object::String(Box::new(copy))));
                }
                if matches!(
                    self.object(x)?,
                    Object::SubCharTable(_)
                        | Object::MapKeymapCursor(_)
                        | Object::Float(_)
                        | Object::Bignum(_)
                        | Object::Buffer(_)
                        | Object::ByteBuffer(_)
                        | Object::Marker(_)
                        | Object::Overlay(_)
                        | Object::Window(_)
                        | Object::Obarray(_)
                        | Object::Free
                            | Object::Frame(_)
                            | Object::Terminal(_)
                            | Object::WindowConfiguration(_)
                            | Object::Sqlite(_)
                            | Object::Treesit(_)
                            | Object::Finalizer(_)
                            | Object::Thread
                            | Object::Process(_)
                        | Object::HashTable(_)
                        | Object::SyntaxTable(_)
                ) {
                    return Err("sequence required".into());
                }
                Ok(self.allocate_object(self.object(x)?.clone()))
            }
            _ => unreachable!("data primitive dispatch"),
        }
    }
}
