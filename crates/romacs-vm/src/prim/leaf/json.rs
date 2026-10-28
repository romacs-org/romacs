//! json.c: JSON serialization to UTF-8 and strict parsing from GNU's internal
//! encoding. The parser keeps nesting on an explicit stack, so its 10000-level
//! limit cannot exhaust the native one.
use super::*;

#[derive(Clone, Copy, PartialEq)]
enum ObjectType { HashTable, Alist, Plist }

#[derive(Clone, Copy, PartialEq)]
enum ArrayType { Array, List }

/// struct json_configuration.
#[derive(Clone, Copy)]
struct Config { object: ObjectType, array: ArrayType, null: Value, false_: Value }

/// json.c json_plain_char: printable ASCII and DEL, except `"` and `\`.
fn plain(c: u8) -> bool {
    (0x20..=0x7f).contains(&c) && c != b'"' && c != b'\\'
}

/// json_out_string: STRING's internal BYTES from SKIP, quoted and escaped;
/// None when they are not Unicode text.
fn out_string(out: &mut Vec<u8>, bytes: &[u8], multibyte: bool, skip: usize) -> Option<()> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push(b'"');
    let mut p = skip;
    while p < bytes.len() {
        let c = bytes[p];
        if plain(c) { out.push(c); p += 1; continue; }
        if c > 0x7f {
            if !multibyte || c <= 0xc1 { return None; }
            let n = match c {
                ..=0xdf => 2,
                ..=0xef => {
                    let v = (u32::from(c & 0x0f) << 12) | (u32::from(bytes[p + 1] & 0x3f) << 6) | u32::from(bytes[p + 2] & 0x3f);
                    if (0xd800..=0xdfff).contains(&v) { return None; }
                    3
                }
                ..=0xf7 => {
                    let v = (u32::from(c & 0x07) << 18) | (u32::from(bytes[p + 1] & 0x3f) << 12)
                        | (u32::from(bytes[p + 2] & 0x3f) << 6) | u32::from(bytes[p + 3] & 0x3f);
                    if v > 0x10ffff { return None; }
                    4
                }
                _ => return None,
            };
            out.extend_from_slice(&bytes[p..p + n]);
            p += n;
            continue;
        }
        out.push(b'\\');
        match c {
            b'"' | b'\\' => out.push(c),
            0x08 => out.push(b'b'),
            b'\t' => out.push(b't'),
            b'\n' => out.push(b'n'),
            0x0c => out.push(b'f'),
            b'\r' => out.push(b'r'),
            _ => out.extend_from_slice(&[b'u', b'0', b'0', HEX[usize::from(c >> 4)], HEX[usize::from(c & 0xf)]]),
        }
        p += 1;
    }
    out.push(b'"');
    Some(())
}

/// One open container while parsing.
enum Open {
    Array(Vec<Value>),
    /// Members so far, and the key whose value is being parsed.
    Object(Vec<(Value, Value)>, Value),
}

/// struct json_parser over one contiguous input.
struct Parser<'l, 'a> {
    cx: &'l mut Leaf<'a>,
    input: Vec<u8>,
    at: usize,
    multibyte: bool,
    depth: i32,
    conf: Config,
}

type Parsed<T> = std::result::Result<T, Fault>;

impl Parser<'_, '_> {
    /// json_signal_error: (ERROR LINE nil POSITION) at the byte just read.
    fn fail(&mut self, condition: &str) -> Fault {
        let before = &self.input[..self.at];
        let line = before.iter().filter(|&&c| c == b'\n').count() as i64 + 1;
        let position = if self.multibyte { before.iter().filter(|&&c| c & 0xc0 != 0x80).count() } else { before.len() } as i64;
        let condition = self.cx.intern(condition);
        let data = self.cx.cons(Value::integer(position).expect("json position"), Value::NIL);
        let data = self.cx.cons(Value::NIL, data);
        let data = self.cx.cons(Value::integer(line).expect("json line"), data);
        Fault::Signal(condition, data)
    }
    fn at_eof(&self) -> bool {
        self.at >= self.input.len()
    }
    /// json_input_get: end of input signals json-end-of-file.
    fn get(&mut self) -> Parsed<u8> {
        let Some(&c) = self.input.get(self.at) else { return Err(self.fail("json-end-of-file")) };
        self.at += 1;
        Ok(c)
    }
    fn get_if_possible(&mut self) -> Option<u8> {
        let c = *self.input.get(self.at)?;
        self.at += 1;
        Some(c)
    }
    fn skip_whitespace(&mut self) -> Parsed<u8> {
        loop {
            let c = self.get()?;
            if !matches!(c, b' ' | b'\t' | b'\r' | b'\n') { return Ok(c); }
        }
    }
    fn skip_whitespace_if_possible(&mut self) -> Option<u8> {
        loop {
            let c = self.get_if_possible()?;
            if !matches!(c, b' ' | b'\t' | b'\r' | b'\n') { return Some(c); }
        }
    }
    /// json_parse_unicode: the CCCC of 쳌.
    fn unicode(&mut self) -> Parsed<u32> {
        let mut v = 0;
        for _ in 0..4 {
            let digit = (self.get()? as char).to_digit(16);
            v = v << 4 | digit.ok_or_else(|| self.fail("json-escape-sequence-error"))?;
        }
        Ok(v)
    }
    /// json_parse_string, after the opening quote: the UTF-8 contents.
    fn string(&mut self) -> Parsed<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let c = self.get()?;
            if plain(c) { out.push(c); continue; }
            if c == b'"' { return Ok(out); }
            if c & 0x80 != 0 {
                // Strict UTF-8, whether the input is unibyte or multibyte.
                out.push(c);
                let c1 = self.get()?;
                if c1 & 0xc0 != 0x80 { return Err(self.fail("json-utf8-decode-error")); }
                out.push(c1);
                match c {
                    ..=0xc1 => return Err(self.fail("json-utf8-decode-error")),
                    ..=0xdf => {}
                    ..=0xef => {
                        let c2 = self.get()?;
                        if c2 & 0xc0 != 0x80 { return Err(self.fail("json-utf8-decode-error")); }
                        let v = (u32::from(c & 0x0f) << 12) + (u32::from(c1 & 0x3f) << 6) + u32::from(c2 & 0x3f);
                        if v < 0x800 || (0xd800..=0xdfff).contains(&v) { return Err(self.fail("json-utf8-decode-error")); }
                        out.push(c2);
                    }
                    ..=0xf7 => {
                        let (c2, c3) = (self.get()?, self.get()?);
                        if c2 & 0xc0 != 0x80 || c3 & 0xc0 != 0x80 { return Err(self.fail("json-utf8-decode-error")); }
                        let v = (u32::from(c & 0x07) << 18) + (u32::from(c1 & 0x3f) << 12)
                            + (u32::from(c2 & 0x3f) << 6) + u32::from(c3 & 0x3f);
                        if !(0x10000..=0x10ffff).contains(&v) { return Err(self.fail("json-utf8-decode-error")); }
                        out.extend_from_slice(&[c2, c3]);
                    }
                    _ => return Err(self.fail("json-utf8-decode-error")),
                }
            } else if c == b'\\' {
                let escaped = match self.get()? {
                    c @ (b'"' | b'\\' | b'/') => c,
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'u' => {
                        let mut num = self.unicode()?;
                        if (0xd800..0xdc00).contains(&num) {
                            if self.get()? != b'\\' || self.get()? != b'u' { return Err(self.fail("json-invalid-surrogate-error")); }
                            let low = self.unicode()?;
                            if !(0xdc00..0xe000).contains(&low) { return Err(self.fail("json-invalid-surrogate-error")); }
                            num = 0x10000 + ((num - 0xd800) << 10 | (low - 0xdc00));
                        } else if (0xdc00..0xe000).contains(&num) {
                            return Err(self.fail("json-invalid-surrogate-error"));
                        }
                        let mut encoded = [0; 4];
                        out.extend_from_slice(char::from_u32(num).expect("scalar value").encode_utf8(&mut encoded).as_bytes());
                        continue;
                    }
                    _ => return Err(self.fail("json-escape-sequence-error")),
                };
                out.push(escaped);
            } else {
                return Err(self.fail("json-parse-error"));
            }
        }
    }
    /// The digits after C, extending TEXT, until a non-digit (returned) or
    /// the end of input (None).
    fn digits(&mut self, text: &mut Vec<u8>) -> Parsed<Option<u8>> {
        loop {
            if self.at_eof() { return Ok(None); }
            let c = self.get()?;
            if !c.is_ascii_digit() { return Ok(Some(c)); }
            text.push(c);
        }
    }
    fn integer(&mut self, text: &[u8]) -> Value {
        let n: num_bigint::BigInt = std::str::from_utf8(text).expect("ASCII digits").parse().expect("JSON integer");
        self.cx.bigint(n)
    }
    /// json_create_float: strtod's overflow is json-number-out-of-range-error.
    fn float(&mut self, text: &[u8]) -> Parsed<Value> {
        let value: f64 = std::str::from_utf8(text).expect("ASCII number").parse().expect("JSON float");
        if value.is_infinite() { return Err(self.fail("json-number-out-of-range-error")); }
        Ok(self.cx.float(value))
    }
    /// json_parse_number from its first character C.
    fn number(&mut self, mut c: u8) -> Parsed<Value> {
        let mut text = vec![c];
        if c == b'-' {
            c = self.get()?;
            text.push(c);
        }
        if !c.is_ascii_digit() { return Err(self.fail("json-parse-error")); }
        if c == b'0' {
            if self.at_eof() { return Ok(Value::integer(0).expect("zero")); }
            c = self.get()?;
        } else {
            match self.digits(&mut text)? {
                None => return Ok(self.integer(&text)),
                Some(next) => c = next,
            }
        }
        let mut float = false;
        if c == b'.' {
            float = true;
            text.push(c);
            c = self.get()?;
            text.push(c);
            if !c.is_ascii_digit() { return Err(self.fail("json-parse-error")); }
            match self.digits(&mut text)? {
                None => return self.float(&text),
                Some(next) => c = next,
            }
        }
        if c == b'e' || c == b'E' {
            float = true;
            text.push(c);
            c = self.get()?;
            text.push(c);
            if c == b'-' || c == b'+' {
                c = self.get()?;
                text.push(c);
            }
            if !c.is_ascii_digit() { return Err(self.fail("json-parse-error")); }
            match self.digits(&mut text)? {
                None => return self.float(&text),
                Some(_) => {}
            }
        }
        // The character after the number is not part of it.
        self.at -= 1;
        if float { self.float(&text) } else { Ok(self.integer(&text)) }
    }
    /// The literal LITERAL after its first letter, as RESULT. A following
    /// token character makes it a parse error.
    fn literal(&mut self, rest: &[u8], result: Value) -> Option<Value> {
        for &expected in rest {
            if self.get_if_possible() != Some(expected) { return None; }
        }
        match self.get_if_possible() {
            Some(c) if c.is_ascii_alphanumeric() || c == b'-' => None,
            Some(_) => { self.at -= 1; Some(result) }
            None => Some(result),
        }
    }
    /// An object key, after its opening quote, as the configured type wants.
    fn key(&mut self) -> Parsed<Value> {
        let bytes = self.string()?;
        let name = std::str::from_utf8(&bytes).expect("validated UTF-8");
        Ok(match self.conf.object {
            ObjectType::HashTable => self.cx.make_string(bytes.clone(), true),
            ObjectType::Alist => self.cx.intern(name),
            ObjectType::Plist => self.cx.intern(&format!(":{name}")),
        })
    }
    /// json_parse_object_member_value up to the value's first character.
    fn member_value(&mut self) -> Parsed<u8> {
        if self.skip_whitespace()? != b':' { return Err(self.fail("json-parse-error")); }
        self.skip_whitespace()
    }
    fn enter(&mut self) -> Parsed<()> {
        self.depth -= 1;
        if self.depth < 0 { return Err(self.fail("json-object-too-deep")); }
        Ok(())
    }
    fn array(&mut self, items: Vec<Value>) -> Value {
        match self.conf.array {
            ArrayType::Array => self.cx.vector(&items),
            ArrayType::List => items.into_iter().rev().fold(Value::NIL, |tail, item| self.cx.cons(item, tail)),
        }
    }
    fn object(&mut self, members: Vec<(Value, Value)>) -> Parsed<Value> {
        Ok(match self.conf.object {
            ObjectType::HashTable => {
                let table = crate::hash::HashTable::new(crate::hash::Test::Equal, crate::hash::Weakness::None, members.len());
                let table = self.cx.vm.allocate_object(data::Object::HashTable(Box::new(table)));
                // A repeated key keeps its first slot and its last value.
                for (key, value) in members {
                    self.cx.vm.declaration_hash_put(table, key, value).map_err(Fault::Error)?;
                }
                table
            }
            ObjectType::Alist => members.into_iter().rev().fold(Value::NIL, |tail, (key, value)| {
                let pair = self.cx.cons(key, value);
                self.cx.cons(pair, tail)
            }),
            ObjectType::Plist => members.into_iter().rev().fold(Value::NIL, |tail, (key, value)| {
                let tail = self.cx.cons(value, tail);
                self.cx.cons(key, tail)
            }),
        })
    }
    /// json_parse: one value from the next non-whitespace character.
    fn parse(&mut self) -> Parsed<Value> {
        let mut open: Vec<Open> = Vec::new();
        let mut c = self.skip_whitespace()?;
        loop {
            // A complete value, or None after opening a container.
            let value = match c {
                b'{' => {
                    c = self.skip_whitespace()?;
                    if c == b'}' {
                        Some(self.object(Vec::new())?)
                    } else {
                        self.enter()?;
                        if c != b'"' { return Err(self.fail("json-parse-error")); }
                        let key = self.key()?;
                        open.push(Open::Object(Vec::new(), key));
                        c = self.member_value()?;
                        None
                    }
                }
                b'[' => {
                    c = self.skip_whitespace()?;
                    if c == b']' {
                        Some(self.array(Vec::new()))
                    } else {
                        self.enter()?;
                        open.push(Open::Array(Vec::new()));
                        None
                    }
                }
                b'"' => {
                    let bytes = self.string()?;
                    Some(self.cx.make_string(bytes, true))
                }
                b'0'..=b'9' | b'-' => Some(self.number(c)?),
                b't' => Some(self.literal(b"rue", Value::TRUE).ok_or_else(|| self.fail("json-parse-error"))?),
                b'f' => { let v = self.conf.false_; Some(self.literal(b"alse", v).ok_or_else(|| self.fail("json-parse-error"))?) }
                b'n' => { let v = self.conf.null; Some(self.literal(b"ull", v).ok_or_else(|| self.fail("json-parse-error"))?) }
                _ => return Err(self.fail("json-parse-error")),
            };
            let Some(mut value) = value else { continue };
            // Hand the value to its container; close every container it ends.
            loop {
                let Some(top) = open.last_mut() else { return Ok(value) };
                match top {
                    Open::Array(items) => items.push(value),
                    Open::Object(members, key) => members.push((*key, value)),
                }
                c = self.skip_whitespace()?;
                let close = match open.last() { Some(Open::Array(_)) => b']', _ => b'}' };
                if c == close {
                    self.depth += 1;
                    value = match open.pop() {
                        Some(Open::Array(items)) => self.array(items),
                        Some(Open::Object(members, _)) => self.object(members)?,
                        None => unreachable!("open container"),
                    };
                    continue;
                }
                if c != b',' { return Err(self.fail("json-parse-error")); }
                c = self.skip_whitespace()?;
                if matches!(open.last(), Some(Open::Object(..))) {
                    if c != b'"' { return Err(self.fail("json-parse-error")); }
                    let next = self.key()?;
                    if let Some(Open::Object(_, key)) = open.last_mut() { *key = next; }
                    c = self.member_value()?;
                }
                break;
            }
        }
    }
}

impl Leaf<'_> {
    /// json_parse_args: keyword/value pairs, the first occurrence winning.
    fn json_config(&mut self, args: &[Value], object_types: bool) -> std::result::Result<Config, Fault> {
        let mut conf = Config { object: ObjectType::HashTable, array: ArrayType::Array, null: self.intern(":null"), false_: self.intern(":false") };
        if args.len() % 2 != 0 {
            let list = args.iter().rev().fold(Value::NIL, |tail, &v| self.cons(v, tail));
            return Err(Fault::Wrong("plistp", list));
        }
        let choice = |cx: &mut Self, message: &str, value: Value| {
            let error = cx.intern("error");
            let message = cx.string(message);
            let data = cx.cons(value, Value::NIL);
            Fault::Signal(error, cx.cons(message, data))
        };
        for pair in args.chunks(2).rev() {
            let (key, value) = (pair[0], pair[1]);
            if object_types && key == self.intern(":object-type") {
                conf.object = if value == self.intern("hash-table") { ObjectType::HashTable }
                    else if value == self.intern("alist") { ObjectType::Alist }
                    else if value == self.intern("plist") { ObjectType::Plist }
                    else { return Err(choice(self, "One of hash-table, alist or plist should be specified", value)) };
            } else if object_types && key == self.intern(":array-type") {
                conf.array = if value == self.intern("array") { ArrayType::Array }
                    else if value == self.intern("list") { ArrayType::List }
                    else { return Err(choice(self, "One of array or list should be specified", value)) };
            } else if key == self.intern(":null-object") {
                conf.null = value;
            } else if key == self.intern(":false-object") {
                conf.false_ = value;
            } else if object_types {
                return Err(choice(self, "One of :object-type, :array-type, :null-object or :false-object should be specified", value));
            } else {
                return Err(choice(self, "One of :null-object or :false-object should be specified", value));
            }
        }
        Ok(conf)
    }

    /// json_out_nest: at most 50 open containers.
    fn json_nest(depth: &mut i32) -> std::result::Result<(), Fault> {
        *depth -= 1;
        if *depth < 0 { return Err(Fault::Error("Maximum JSON serialization depth exceeded".into())); }
        Ok(())
    }

    fn json_out_string(&self, out: &mut Vec<u8>, string: Value, skip: usize) -> std::result::Result<(), Fault> {
        let view = self.vm.str_view(string).ok_or(Fault::Wrong("stringp", string))?;
        out_string(out, view.bytes(), view.is_multibyte(), skip).ok_or(Fault::Wrong("json-value-p", string))
    }

    /// json_out_something.
    fn json_out(&mut self, out: &mut Vec<u8>, depth: &mut i32, conf: &Config, v: Value) -> std::result::Result<(), Fault> {
        if v == conf.null { out.extend_from_slice(b"null"); return Ok(()); }
        if v == conf.false_ { out.extend_from_slice(b"false"); return Ok(()); }
        if v == Value::TRUE { out.extend_from_slice(b"true"); return Ok(()); }
        if v == Value::NIL { out.extend_from_slice(b"{}"); return Ok(()); }
        match self.kind(v) {
            Kind::Integer | Kind::Bignum => out.extend_from_slice(self.bigint_value(v).expect("integer").to_string().as_bytes()),
            Kind::String | Kind::Bytes => self.json_out_string(out, v, 0)?,
            Kind::Cons => self.json_out_cons(out, depth, conf, v)?,
            Kind::Float => {
                let Some(data::Object::Float(x)) = self.vm.object_if(v) else { unreachable!("float") };
                let x = *x;
                if !x.is_finite() {
                    let error = self.intern("error");
                    let message = self.string("JSON does not allow Inf or NaN");
                    let data = self.cons(v, Value::NIL);
                    return Err(Fault::Signal(error, self.cons(message, data)));
                }
                out.extend_from_slice(crate::numbers::print_float(x).as_bytes());
            }
            Kind::HashTable => {
                Self::json_nest(depth)?;
                out.push(b'{');
                let Some(data::Object::HashTable(table)) = self.vm.object_if(v) else { unreachable!("hash table") };
                let entries: Vec<(Value, Value)> = table.entries().map(|(_, k, v)| (k, v)).collect();
                for (i, (key, value)) in entries.into_iter().enumerate() {
                    if i > 0 { out.push(b','); }
                    // Keys are the caller's to keep unique.
                    self.json_out_string(out, key, 0)?;
                    out.push(b':');
                    self.json_out(out, depth, conf, value)?;
                }
                out.push(b'}');
                *depth += 1;
            }
            Kind::Vector => {
                Self::json_nest(depth)?;
                out.push(b'[');
                for (i, item) in self.vector_values(v)?.into_iter().enumerate() {
                    if i > 0 { out.push(b','); }
                    self.json_out(out, depth, conf, item)?;
                }
                out.push(b']');
                *depth += 1;
            }
            _ => return Err(Fault::Wrong("json-value-p", v)),
        }
        Ok(())
    }

    /// json_out_object_cons: an alist or plist with symbol keys; for a
    /// repeated key the first value counts.
    fn json_out_cons(&mut self, out: &mut Vec<u8>, depth: &mut i32, conf: &Config, object: Value) -> std::result::Result<(), Fault> {
        Self::json_nest(depth)?;
        out.push(b'{');
        let alist = self.pair(object).is_some_and(|(car, _)| self.pair(car).is_some());
        let mut seen = std::collections::HashSet::new();
        let mut walk = crate::annotations::TailWalk::new(object);
        let mut tail = object;
        while let Some((car, cdr)) = self.pair(tail) {
            let (key, value, next) = if alist {
                let (key, value) = self.pair(car).ok_or(Fault::Wrong("consp", car))?;
                (key, value, cdr)
            } else {
                let (value, next) = self.pair(cdr).ok_or(Fault::Wrong("consp", cdr))?;
                (car, value, next)
            };
            if self.kind(key) != Kind::Symbol { return Err(Fault::Wrong("symbolp", key)); }
            if seen.insert(key.0) {
                if seen.len() > 1 { out.push(b','); }
                let name = self.symbol_name(key)?;
                // A plist key's leading colon is elided.
                let skip = usize::from(!alist && name.len() > 1 && name.starts_with(':'));
                out_string(out, name.as_bytes(), true, skip).ok_or(Fault::Wrong("json-value-p", key))?;
                out.push(b':');
                self.json_out(out, depth, conf, value)?;
            }
            tail = next;
            if walk.cycle(tail) { return Err(Fault::Circular(tail)); }
        }
        if tail != Value::NIL { return Err(Fault::Wrong("listp", object)); }
        out.push(b'}');
        *depth += 1;
        Ok(())
    }

    /// json_serialize: OBJECT as UTF-8 bytes.
    pub fn json_serialize_bytes(&mut self, object: Value, args: &[Value]) -> std::result::Result<Vec<u8>, Fault> {
        let conf = self.json_config(args, false)?;
        let mut out = Vec::new();
        self.json_out(&mut out, &mut 50, &conf, object)?;
        Ok(out)
    }

    /// Parse INPUT, GNU internal bytes; the value and the bytes it used.
    fn json_parse(&mut self, input: Vec<u8>, multibyte: bool, args: &[Value], whole: bool) -> std::result::Result<(Value, usize), Fault> {
        let conf = self.json_config(args, true)?;
        let mut parser = Parser { cx: self, input, at: 0, multibyte, depth: 10000, conf };
        let value = parser.parse()?;
        if whole && parser.skip_whitespace_if_possible().is_some() {
            return Err(parser.fail("json-trailing-content"));
        }
        Ok((value, parser.at))
    }

    pub fn json_parse_string(&mut self, string: Value, args: &[Value]) -> super::super::Out {
        let view = self.vm.str_view(string).ok_or(Fault::Wrong("stringp", string))?;
        let (input, multibyte) = (view.bytes().to_vec(), view.is_multibyte());
        Ok(self.json_parse(input, multibyte, args, true)?.0)
    }

    /// Fjson_parse_buffer: from point to the end of the accessible region;
    /// point moves past the value only on success.
    pub fn json_parse_buffer(&mut self, args: &[Value]) -> super::super::Out {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let (input, point, multibyte) = match &self.vm.objects[index] {
            data::Object::Buffer(b) => (b.bytes()[b.point.0..b.accessible().end.0].to_vec(), b.point.0, true),
            data::Object::ByteBuffer(b) => (b.bytes[b.point.0..b.restriction.end.0].to_vec(), b.point.0, false),
            _ => unreachable!("buffer_index validates"),
        };
        let (value, used) = self.json_parse(input, multibyte, args, false)?;
        self.set_point_byte(point + used)?;
        Ok(value)
    }
}
