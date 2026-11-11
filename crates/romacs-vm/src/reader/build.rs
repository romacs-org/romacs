//! The reader's raw work, which its grammar (`parse`) runs in leaves: each
//! takes plain data (a token, a radix, root indices) and makes or reads the
//! Values, leaving what it made in `external_roots` and returning the index.
//! Nothing here calls Lisp but a pending variable's force, which is held.
use crate::coordinates::ByteOffset;
use crate::gc::RootFrame;
use crate::{Result, Value, Vm, data::Object};

/// A leaf's datum, rooted: its index in `external_roots`; or the error's text
/// and, when GNU's condition is known, the root of its `(CONDITION . DATA)`.
pub(super) type Built = std::result::Result<usize, (String, Option<usize>)>;

impl Vm {
    /// lread.c oblookup_considering_shorthand: the name TOKEN reads as under
    /// `read-symbol-shorthands', the first (SHORTHAND . LONGHAND) pair of
    /// strings whose SHORTHAND prefixes it, or None. A name made only of
    /// `^*+-/<=>_|' (symbol_char_span) is exempt, as is `#_' syntax.
    fn shorthand_longhand(&mut self, token: &str) -> Result<Option<String>> {
        let Some(&slot) = self.names.get("read-symbol-shorthands") else { return Ok(None) };
        let shorthands = self.variable_value(slot)?;
        if matches!(shorthands, Value::NIL | Value::UNBOUND) || token.bytes().all(|b| b"^*+-/<=>_|".contains(&b)) {
            return Ok(None);
        }
        let mut tail = shorthands;
        for _ in 0..=self.list_limit() {
            let Ok((pair, next)) = self.cons_pair(tail) else { break };
            if let Ok((short, long)) = self.cons_pair(pair)
                && let (Some(short), Some(long)) = (self.str_view(short), self.str_view(long))
                && let Some(rest) = token.as_bytes().strip_prefix(short.bytes)
            {
                let mut name = long.bytes.to_vec();
                name.extend_from_slice(rest);
                return Ok(Some(String::from_utf8_lossy(&name).into_owned()));
            }
            tail = next;
        }
        Ok(None)
    }
    /// A fresh symbol outside the obarray, as `make-symbol' makes.
    pub(crate) fn uninterned_symbol(&mut self, name: &str) -> Value {
        let value = self.generated_symbol(crate::expand::generated_id());
        let slot = value.index_if(7).unwrap() - 1;
        self.symbol_names[slot] = name.to_owned();
        value
    }
    /// Interning is stable for this VM's lifetime, independent of moving GC.
    pub(crate) fn intern(&mut self, name: &str) -> Value {
        match name {
            "nil" => Value::NIL,
            "t" => Value::TRUE,
            _ => {
                let slot = self.slot(name);
                Value::reference(slot + 1, 7)
            }
        }
    }
    fn reader_buffer_location(&self, source: Value) -> Result<(Value, usize, bool)> {
        if let Some(Object::Marker(m)) = self.object_if(source) {
            let anchor = m.anchor.ok_or("Reader refuses a detached marker")?;
            self.require_live_buffer(m.buffer)?;
            let at = self.marker_anchors(m.buffer)?.resolve(anchor).ok_or("invalid marker anchor")? as usize;
            Ok((m.buffer, at, true))
        } else {
            self.require_live_buffer(source)?;
            let at = match self.object(source)? {
                Object::Buffer(b) => b.point.0,
                Object::ByteBuffer(b) => b.point.0,
                _ => unreachable!(),
            };
            Ok((source, at, false))
        }
    }
    fn reader_buffer_position(
        &mut self,
        source: Value,
        buffer: Value,
        at: usize,
        marker: bool,
    ) -> Result<()> {
        if marker {
            let Object::Marker(m) = self.object(source)? else {
                unreachable!()
            };
            let (anchor, bias) = (m.anchor.unwrap(), m.bias);
            if !self.marker_anchors_mut(buffer)?.set(anchor, at as u32, bias) {
                return Err("invalid marker anchor".into());
            }
        } else {
            match self.object_mut(buffer)? {
                Object::Buffer(b) => b.point = ByteOffset(at),
                Object::ByteBuffer(b) => b.point = ByteOffset(at),
                _ => unreachable!(),
            }
        }
        Ok(())
    }
    fn reader_buffer_get(&mut self, source: Value) -> Result<Option<u32>> {
        let (buffer, at, marker) = self.reader_buffer_location(source)?;
        let (code, width) = match self.object(buffer)? {
            Object::Buffer(b) => {
                if at >= b.accessible().end.0 {
                    return Ok(None);
                }
                let c = b.character_at(ByteOffset(at))?.ok_or("invalid buffer reader point")? as u32;
                (c, crate::lstring::char_string(c, &mut [0; 5]))
            }
            Object::ByteBuffer(b) => {
                if at >= b.restriction.end.0 {
                    return Ok(None);
                }
                let c = *b.bytes.get(at).ok_or("invalid byte buffer reader point")? as u32;
                (if c >= 128 { c + 0x3fff00 } else { c }, 1)
            }
            _ => unreachable!(),
        };
        self.reader_buffer_position(source, buffer, at + width, marker)?;
        Ok(Some(code))
    }
    fn reader_buffer_unget(&mut self, source: Value, code: u32) -> Result<()> {
        let (buffer, at, marker) = self.reader_buffer_location(source)?;
        let width = match self.object(buffer)? {
            Object::Buffer(_) => crate::lstring::char_string(code, &mut [0; 5]),
            _ => 1,
        };
        self.reader_buffer_position(
            source,
            buffer,
            at.checked_sub(width)
                .ok_or("reader unread before buffer start")?,
            marker,
        )
    }
}

/// VALUE, rooted.
#[inline]
fn keep(vm: &mut Vm, value: Value) -> usize {
    vm.external_roots.push(value);
    vm.external_roots.len() - 1
}
/// GNU's `(CONDITION . DATA)` for CONDITION on DATA, rooted.
fn condition(vm: &mut Vm, condition: &str, data: &[Value]) -> usize {
    let (condition, data) = (vm.intern(condition), vm.list_value(data));
    let pair = vm.alloc_cons(condition, data);
    keep(vm, pair)
}
/// The error `invalid-read-syntax` on DATUM.
pub(super) fn syntax(vm: &mut Vm, datum: &str) -> (String, Option<usize>) {
    let text = vm.make_lisp_string(datum.as_bytes().to_vec(), true);
    (format!("invalid-read-syntax {datum}"), Some(condition(vm, "invalid-read-syntax", &[text])))
}
/// The error `error` with MESSAGE, its text a byte string or, when MULTIBYTE,
/// a multibyte one.
fn error(vm: &mut Vm, message: String, multibyte: bool) -> (String, Option<usize>) {
    let text = if multibyte { vm.make_lisp_string(message.as_bytes().to_vec(), true) } else { vm.byte_string(message.as_bytes()) };
    let signal = condition(vm, "error", &[text]);
    (message, Some(signal))
}
/// The escape a stream's Lisp left, as the reader records it: its
/// `(CONDITION . DATA)`, rooted, when it is a signal or an error.
pub(super) fn pending_signal(vm: &mut Vm, (): ()) -> Option<usize> {
    let (condition, data) = vm.pending_eval_escape()?;
    let pair = vm.alloc_cons(condition, data);
    Some(keep(vm, pair))
}

/// The character at BYTE of the string stream rooted at ROOT, and its width.
#[inline]
pub(super) fn string_char(vm: &mut Vm, (root, byte): (usize, usize)) -> Result<(u32, usize)> {
    let view = vm.str_view(vm.external_roots[root]).ok_or("string stream no longer exists")?;
    let bytes = view.bytes().get(byte..).filter(|b| !b.is_empty()).ok_or("string stream changed length")?;
    super::parse::string_input_char(bytes, view.is_multibyte())
}
/// The width CODE takes in the string stream rooted at ROOT.
#[inline]
pub(super) fn string_width(vm: &mut Vm, (root, code): (usize, u32)) -> Result<usize> {
    let view = vm.str_view(vm.external_roots[root]).ok_or("string stream no longer exists")?;
    Ok(if view.is_multibyte() { crate::lstring::char_string(code, &mut [0; 5]) } else { 1 })
}
/// Inside a string literal read from the string stream rooted at ROOT, from
/// BYTE (CHARACTER of END), the characters up to the next quote or backslash,
/// pushed onto S in one pass; whether any is non-ASCII, and the byte and
/// character the run stopped at.
pub(super) fn string_run(vm: &mut Vm, (mut s, (root, mut byte, mut character, end)): (Vec<u32>, (usize, usize, usize, usize))) -> (Vec<u32>, bool, usize, usize) {
    let mut unicode = false;
    if let Some(view) = vm.str_view(vm.external_roots[root]) {
        let (bytes, multibyte) = (view.bytes(), view.is_multibyte());
        while character < end {
            let Some(rest) = bytes.get(byte..).filter(|rest| !matches!(rest.first(), None | Some(b'"' | b'\\'))) else { break };
            let Ok((c, width)) = super::parse::string_input_char(rest, multibyte) else { break };
            unicode |= (128..0x3fff80).contains(&c);
            s.push(c);
            (character, byte) = (character + 1, byte + width);
        }
    }
    (s, unicode, byte, character)
}
/// The next character of the buffer or marker stream rooted at ROOT.
#[inline]
pub(super) fn buffer_get(vm: &mut Vm, root: usize) -> Result<Option<u32>> {
    vm.reader_buffer_get(vm.external_roots[root])
}
/// CODE unread to the buffer or marker stream rooted at ROOT.
pub(super) fn buffer_unget(vm: &mut Vm, (root, code): (usize, u32)) -> Result<()> {
    vm.reader_buffer_unget(vm.external_roots[root], code)
}
/// A stream's call (`Input::Function`): the function rooted at ROOT, given
/// CODE to unread or nothing to read, in a frame of its own.
pub(super) fn stream_call(vm: &mut Vm, (root, code): (usize, Option<u32>)) -> Result<RootFrame> {
    let function = vm.external_roots[root];
    Ok(match code {
        Some(code) => { let code = Value::integer(i64::from(code))?; vm.root_frame(&[function, code]) }
        None => vm.root_frame(&[function]),
    })
}
/// `char-from-name` on NAME, ignoring case (lread.c `read_char_escape`), in a
/// frame of its own.
pub(super) fn name_call(vm: &mut Vm, name: &str) -> RootFrame {
    let function = vm.intern("char-from-name");
    let string = vm.byte_string(name.as_bytes());
    vm.root_frame(&[function, string, Value::TRUE])
}

/// The symbol NAME interned, or made uninterned.
pub(super) fn symbol(vm: &mut Vm, (name, interned): (&str, bool)) -> usize {
    let symbol = if interned { vm.intern(name) } else { vm.uninterned_symbol(name) };
    keep(vm, symbol)
}
/// The fixnum N.
pub(super) fn integer(vm: &mut Vm, n: i64) -> Result<usize> {
    Ok(keep(vm, Value::integer(n)?))
}
/// TOKEN's integer in RADIX (lread.c `read_integer`).
pub(super) fn radix_integer(vm: &mut Vm, (token, radix): (&str, u32)) -> Result<usize> {
    let value = vm.integer_from_radix(token, radix)?;
    Ok(keep(vm, value))
}
/// A token read as a number when it spells one, else as a symbol under
/// `read-symbol-shorthands`; ESCAPED, a symbol whatever it spells.
pub(super) fn atom(vm: &mut Vm, (token, escaped): (&str, bool)) -> Result<usize> {
    if !escaped {
        // GNU also accepts decimal integers with a trailing dot.
        let integer = token.strip_suffix('.').unwrap_or(token);
        let digits = integer.strip_prefix(['+', '-']).unwrap_or(integer);
        if !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit()) {
            // A fixnum when it fits, else a bignum.
            let n: num_bigint::BigInt = integer.parse().map_err(|_| "invalid integer")?;
            let value = vm.bigint(n);
            return Ok(keep(vm, value));
        }
        // A lone `.' token here was followed by `)` or `]`, which GNU's dot
        // test excludes (`dot_follows`): it is the symbol `.`, so `(a .)` is a
        // list of `a` and `.`. Numeric-looking floats/bignums are not interned
        // as symbols. Rust also parses "nan" and "inf" as floats; GNU interns
        // those names. Only numeric-leading tokens can be numbers.
        let unsigned = token.strip_prefix(['+', '-']).unwrap_or(token);
        let numeric_leading = unsigned.starts_with(|c: char| c.is_ascii_digit())
            || unsigned.strip_prefix('.').is_some_and(|s| s.starts_with(|c: char| c.is_ascii_digit()));
        if numeric_leading && (token.contains('.') || token.contains(['e', 'E'])) {
            let special = match token {
                "1.0e+INF" | "+1.0e+INF" => Some(f64::INFINITY),
                "-1.0e+INF" => Some(f64::NEG_INFINITY),
                "0.0e+NaN" | "+0.0e+NaN" => Some(f64::NAN),
                "-0.0e+NaN" => Some(-f64::NAN),
                _ => None,
            };
            if let Some(n) = special.or_else(|| token.parse::<f64>().ok()) {
                let value = vm.allocate_object(Object::Float(n));
                return Ok(keep(vm, value));
            }
        }
    }
    let longhand = vm.shorthand_longhand(token)?;
    let symbol = vm.intern(longhand.as_deref().unwrap_or(token));
    Ok(keep(vm, symbol))
}
/// The string literal of the characters S: multibyte when UNICODE (GNU
/// `read_string_literal`: only for a character above 127 that is not a raw byte).
pub(super) fn string_literal(vm: &mut Vm, (s, unicode): (Vec<u32>, bool)) -> usize {
    let value = if unicode {
        let mut bytes = Vec::new();
        for c in s {
            let mut encoded = [0; 5];
            let n = crate::lstring::char_string(c, &mut encoded);
            bytes.extend_from_slice(&encoded[..n]);
        }
        vm.make_lisp_string(bytes, true)
    } else {
        let bytes = s.into_iter().map(|c| if c >= 0x3fff80 { (c - 0x3fff00) as u8 } else { c as u8 }).collect::<Vec<_>>();
        // A unibyte literal with bytes above 127 ("\351") is a byte string;
        // ASCII keeps its string slot.
        if bytes.is_ascii() {
            vm.allocate_object(Object::String(Box::new(crate::data::LispString::from_bytes(bytes, false))))
        } else {
            vm.make_lisp_string(bytes, false)
        }
    };
    keep(vm, value)
}
/// The list or, when VECTOR, the vector of the values rooted at VALUES, a
/// dotted list's tail rooted at TAIL.
pub(super) fn sequence(vm: &mut Vm, (vector, values, tail): (bool, Vec<usize>, Option<usize>)) -> usize {
    let value = if vector {
        let values = values.into_iter().map(|i| vm.external_roots[i]).collect();
        vm.allocate_object(Object::Vector(values))
    } else {
        let mut list = tail.map_or(Value::NIL, |t| vm.external_roots[t]);
        for index in values.into_iter().rev() {
            let value = vm.external_roots[index];
            list = vm.alloc_cons(value, list);
        }
        list
    };
    keep(vm, value)
}
/// `(SYMBOL VALUE)', each rooted: a quote, backquote, comma or `#''.
pub(super) fn prefixed(vm: &mut Vm, (symbol, value): (usize, usize)) -> usize {
    let list = vm.list_value(&[vm.external_roots[symbol], vm.external_roots[value]]);
    keep(vm, list)
}
/// GNU lread.c `read_bool_vector`'s object: LENGTH bits from the string rooted
/// at STRING, of exactly ceil(LENGTH/8) bytes (or one more, as Emacs 19 printed it).
pub(super) fn bool_vector(vm: &mut Vm, (length, string): (usize, usize)) -> Built {
    let view = vm.str_view(vm.external_roots[string]).ok_or(("bool-vector literal string".to_owned(), None))?;
    // Host-source literals keep ASCII text multibyte; its bytes are the same.
    let (multibyte, bytes) = (view.is_multibyte() && !view.bytes().is_ascii(), view.bytes().to_vec());
    let size = length.div_ceil(8);
    if multibyte || !(bytes.len() == size || length == bytes.len().saturating_sub(1) * 8) {
        return Err(syntax(vm, "#&..."));
    }
    let mut vector = crate::bool_vector::BoolVector::new(length, false).map_err(|e| (e, None))?;
    vector.bytes.copy_from_slice(&bytes[..size]);
    vector.mask_padding();
    let value = vm.allocate_object(Object::BoolVector(Box::new(vector)));
    Ok(keep(vm, value))
}
/// `#^[...]', or `#^^[...]' when SUB: the char-table of the values rooted at ROOTS.
pub(super) fn char_table(vm: &mut Vm, (sub, roots): (bool, Vec<usize>)) -> Built {
    let values = roots.iter().map(|&i| vm.external_roots[i]).collect::<Vec<_>>();
    let table = if sub { vm.sub_char_table_from_literal(&values) } else { vm.char_table_from_literal(&values) };
    match table {
        Ok(table) => Ok(keep(vm, table)),
        Err(e) if !sub && e == "Invalid size char-table" => Err(syntax(vm, &e)),
        Err(e) => Err(error(vm, e, false)),
    }
}
/// `#("STRING" START END PLIST ...)`: the string rooted first in ROOTS, given
/// the properties of each triple rooted after it.
pub(super) fn property_string(vm: &mut Vm, roots: &[usize]) -> Built {
    let Some(&root) = roots.first() else { return Err(syntax(vm, "#")) };
    let value = vm.external_roots[root];
    if vm.str_view(value).is_none() {
        return Err(syntax(vm, "#"));
    }
    if !(roots.len() - 1).is_multiple_of(3) {
        return Err(syntax(vm, "Invalid string property list"));
    }
    let plain = |e: String| (e, None);
    for triple in roots[1..].chunks_exact(3) {
        let start = vm.external_roots[triple[0]];
        let end = vm.external_roots[triple[1]];
        let plist = vm.external_roots[triple[2]];
        let integerp = vm.intern("integer-or-marker-p");
        let Some(a) = start.integer_value() else {
            return Err(("invalid property start".into(), Some(condition(vm, "wrong-type-argument", &[integerp, start]))));
        };
        let Some(b) = end.integer_value() else {
            return Err(("invalid property end".into(), Some(condition(vm, "wrong-type-argument", &[integerp, end]))));
        };
        let length = vm.str_view(value).unwrap().len();
        if a < 0 || b < 0 || a as usize > length || b as usize > length {
            return Err(("property range".into(), Some(condition(vm, "args-out-of-range", &[start, end]))));
        }
        let (a, b) = if a <= b { (a as usize, b as usize) } else { (b as usize, a as usize) };
        let view = vm.str_view(value).unwrap();
        let width = |code| crate::lstring::char_string(code, &mut [0; 5]);
        let offset = |n| if view.is_multibyte() { view.chars().take(n).map(width).sum() } else { n };
        let range = ByteOffset(offset(a))..ByteOffset(offset(b));
        let mut pairs = Vec::new();
        let mut cursor = plist;
        while cursor != Value::NIL {
            let (key, tail) = vm.cons_pair(cursor).map_err(plain)?;
            if tail == Value::NIL {
                return Err(error(vm, "Odd length text property list".into(), false));
            }
            let (val, tail) = vm.cons_pair(tail).map_err(plain)?;
            let name = if key == Value::NIL {
                "nil".to_owned()
            } else if key == Value::TRUE {
                "t".to_owned()
            } else if key.tag() == 7 {
                vm.symbol_names[key.index(7).map_err(plain)? - 1].clone()
            } else {
                return Err(plain("reader properties require interned symbol keys".into()));
            };
            if pairs.iter().any(|(previous, _)| previous == &name) {
                return Err(error(vm, "Reader cannot retain duplicate text property keys".into(), false));
            }
            pairs.push((name, val));
            cursor = tail;
            if pairs.len() > vm.list_limit() {
                return Err(plain("circular property list".into()));
            }
        }
        // The new literal may be an unibyte Bytes object: promote while preserving representation.
        let view = vm.str_view(value).unwrap();
        let mut string = crate::data::LispString::from_bytes(view.bytes().to_vec(), view.is_multibyte());
        if let Object::String(s) | Object::RawString(s) = vm.object(value).map_err(plain)? {
            string.properties = s.properties.clone();
        }
        for key in string.properties.properties_keys() {
            string.properties.remove(range.clone(), &key);
        }
        // Reverse insertion preserves the plist's printed key order.
        for (key, val) in pairs.into_iter().rev() {
            string.properties.put(range.clone(), &key, val);
        }
        *vm.objects.entry_mut(value.index(0).map_err(plain)? - 1) = if string.is_utf8() {
            Object::String(Box::new(string))
        } else {
            Object::RawString(Box::new(string))
        };
    }
    Ok(root)
}
/// `#s(...)` of the values rooted at ROOTS: a record, or, when the first is
/// the symbol `hash-table`, the table `hash_table_from_plist` builds.
pub(super) fn record(vm: &mut Vm, roots: &[usize]) -> Built {
    let values: Vec<Value> = roots.iter().map(|&i| vm.external_roots[i]).collect();
    let Some(&head) = values.first() else { return Err(syntax(vm, "#s")) };
    if head == vm.intern("hash-table") {
        return match vm.hash_table_from_plist(&values[1..]) {
            Ok(table) => Ok(keep(vm, table)),
            Err(fault) => match vm.hash_fault_signal(fault) {
                Ok((_, condition, data)) => {
                    let pair = vm.alloc_cons(condition, data);
                    Err(("hash table read error".into(), Some(keep(vm, pair))))
                }
                Err(message) => Err((message, None)),
            },
        };
    }
    match vm.allocate_record(values) {
        Ok(record) => Ok(keep(vm, record)),
        Err(message) => Err(error(vm, message, true)),
    }
}
/// lread.c `read0` `#N=`: a placeholder cons, rooted, that stands for the
/// object while it is read, so the object may refer to itself.
pub(super) fn label(vm: &mut Vm, (): ()) -> usize {
    let placeholder = vm.list_value(&[Value::NIL]);
    keep(vm, placeholder)
}
/// `#N=` done: the object rooted at VALUE takes the place of the placeholder
/// rooted at SLOT. A cons becomes the placeholder; anything else replaces it
/// within itself (lread.c `substitute_object_recurse`).
pub(super) fn labelled(vm: &mut Vm, (slot, value): (usize, usize)) -> Built {
    let (placeholder, object) = (vm.external_roots[slot], vm.external_roots[value]);
    if object == placeholder {
        return Err(syntax(vm, "nonsensical self-reference"));
    }
    match object.index_if(3).filter(|_| !Vm::image_value(object)) {
        // A cons: the placeholder becomes it, so references to the
        // placeholder are references to the object.
        Some(cons) => {
            let pair = vm.conses[cons];
            let at = placeholder.payload() as usize;
            vm.conses.set_car(at, pair.0);
            vm.conses.set_cdr(at, pair.1);
            Ok(slot)
        }
        None => {
            vm.external_roots[slot] = object;
            substitute(vm, object, placeholder);
            Ok(value)
        }
    }
}
/// `#N#`: the object labelled at SLOT, rooted again.
pub(super) fn reference(vm: &mut Vm, slot: usize) -> usize {
    let object = vm.external_roots[slot];
    keep(vm, object)
}
/// lread.c `substitute_object_recurse`: replace PLACEHOLDER by OBJECT
/// within OBJECT's conses, vectors and records, visiting each once.
fn substitute(vm: &mut Vm, object: Value, placeholder: Value) {
    let mut seen = std::collections::HashSet::new();
    let mut pending = vec![object];
    while let Some(v) = pending.pop() {
        if Vm::image_value(v) || !seen.insert(v.0) {
            continue;
        }
        let mut fix = |x: &mut Value| {
            if *x == placeholder {
                *x = object;
            } else {
                pending.push(*x);
            }
        };
        if let Some(i) = v.index_if(3) {
            let (mut car, mut cdr) = vm.conses[i];
            fix(&mut car);
            fix(&mut cdr);
            vm.conses.set_car(i, car);
            vm.conses.set_cdr(i, cdr);
        } else if let Some(i) = v.index_if(0).and_then(|i| i.checked_sub(1))
            && let Some(Object::Vector(items) | Object::Record(items)) = vm.objects.get_entry_mut(i)
        {
            items.iter_mut().for_each(fix);
        }
    }
}
