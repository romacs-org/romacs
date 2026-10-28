//! Strand-owned literal and sequence storage. Tag zero with a nonzero index
//! denotes an object; nil remains zero. Strings use GNU internal encoding,
//! indexed by characters rather than bytes. No global mutator or table.
use super::*;
pub(crate) use crate::bool_vector::BoolVector;
#[cfg(test)]
#[path = "string_cursor_tests.rs"]
mod string_cursor_tests;

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
    Process(Box<crate::subprocess::Process>),
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
}
const _: [(); 32] = [(); size_of::<Object>()];
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
const MULTIBYTE: u8 = 1;
const ASCII: u8 = 2;
const UTF8: u8 = 4;
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
        if Self::image_value(value) {
            let text = self.string_value(value).ok()?;
            return Some(crate::lstring::StrView {
                bytes: text.as_bytes(),
                chars: text.chars().count(),
                multibyte: true,
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
            return self.allocate_object(Object::Bytes(bytes));
        }
        let string = Box::new(LispString::from_bytes(bytes, multibyte));
        let object = if string.is_utf8() {
            Object::String(string)
        } else {
            Object::RawString(string)
        };
        self.allocate_object(object)
    }
    pub(super) fn allocate_object(&mut self, object: Object) -> Value {
        self.objects.push(object);
        Value::reference(self.objects.len(), 0)
    }
    pub(super) fn object(&self, value: Value) -> Result<&Object> {
        let index = value.index(0)?.checked_sub(1).ok_or("sequence required")?;
        self.objects
            .get(index)
            .ok_or_else(|| "invalid object reference".into())
    }
    // Non-diagnostic type discrimination. Image references remain outside the
    // private object arena; callers retain explicit mapped-object handling.
    pub(super) fn object_if(&self, value: Value) -> Option<&Object> {
        let index = value.index_if(0)?.checked_sub(1)?;
        self.objects.get(index)
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
                    self.conses.push((value, tail));
                    tail = Value::reference(self.conses.len() - 1, 3);
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
            self.conses.push((value, tail));
            tail = Value::reference(self.conses.len() - 1, 3);
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
    pub(super) fn prepare_apply(
        &mut self,
        start: usize,
        argc: usize,
    ) -> Result<(Value, usize, usize)> {
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
        if values.len() > 4096 {
            return Err("apply argument limit exceeded".into());
        }
        let offset = self.stack.len() - 4096;
        self.stack[offset..offset + values.len()].copy_from_slice(values);
        Ok((target, offset, values.len()))
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
        } else if p == Primitive::Defalias && error == "list function descriptors are not yet supported" {
            // GNU stores a `(lambda ...)' list as the function; VM2 cannot call
            // one yet. Say so in the condition rather than an anonymous error.
            let message = "defalias: a list function descriptor such as (lambda ...) is not supported in VM2 yet";
            let message = self.make_lisp_string(message.as_bytes().to_vec(), true);
            ("error", vec![message])
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
                    if !(Self::image_value(x) && x.tag() == 0) && self.object_if(x).is_none() =>
                {
                    ("arrayp", x)
                }
                Aref | Aset if matches!(self.object_if(x), Some(Object::Record(_))) => ("fixnump", y),
                Aref | Aset => ("integerp", y),
                CharAfter => ("integer-or-marker-p", x),
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
                if Self::image_value(x) && x.tag() == 0 {
                    if p == Aset {
                        return Err("cannot mutate immutable image".into());
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
                            | Object::Frame(_)
                            | Object::Terminal(_)
                            | Object::WindowConfiguration(_)
                            | Object::Sqlite(_)
                            | Object::Process(_)
                        | Object::HashTable(_)
                        | Object::SyntaxTable(_) => Err("array required".into()),
                    };
                }
                self.object(x)?;
                let value = self.stack[start + 2];
                let table_slot = self.literal_table_slot(value);
                match &mut self.objects[x.index(0)? - 1] {
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
                        | Object::Frame(_)
                        | Object::Terminal(_)
                        | Object::WindowConfiguration(_)
                            | Object::Sqlite(_)
                            | Object::Process(_)
                        | Object::HashTable(_)
                    | Object::SyntaxTable(_) => {
                        return Err("array required".into());
                    }
                }
                let slot = &mut self.objects[x.index(0)? - 1];
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
                if Self::image_value(x) {
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
                            | Object::Frame(_)
                            | Object::Terminal(_)
                            | Object::WindowConfiguration(_)
                            | Object::Sqlite(_)
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
