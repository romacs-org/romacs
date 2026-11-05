//! GNU string leaves. Representation-preserving reads never borrow heap storage.
use super::*;
use crate::lstring::{char_string, BYTE8_OFFSET, MAX_CHAR};
use crate::prim::leaf::Kind;
use std::result::Result;

fn string_chars(cx: &Leaf<'_>, value: Value) -> Result<Vec<u32>, Fault> {
    Ok(cx.string_view(value)?.chars().collect())
}

fn character(value: Value) -> Result<u32, Fault> {
    value.integer_value()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n <= MAX_CHAR)
        .ok_or(Fault::Wrong("characterp", value))
}

fn encode_chars(chars: impl IntoIterator<Item = u32>, multibyte: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    for code in chars {
        if multibyte {
            let mut encoded = [0; 5];
            let width = char_string(code, &mut encoded);
            bytes.extend_from_slice(&encoded[..width]);
        } else {
            bytes.push(code as u8);
        }
    }
    bytes
}

fn widen_string(cx: &mut Leaf<'_>, value: Value) -> Out {
    let string = cx.string_view(value)?;
    if string.is_multibyte() { return Ok(value); }
    let chars = string.chars().map(|c| if c >= 128 { c + BYTE8_OFFSET } else { c });
    let bytes = encode_chars(chars, true);
    Ok(cx.make_string(bytes, true))
}

defsubr! { name = "string-bytes", gnu = "fns.c:string-bytes", effect = Pure,
fn string_bytes(cx, string: Value) {
    Ok(Value::integer(cx.string_view(string)?.bytes().len() as i64).unwrap())
}}

defsubr! { name = "characterp", gnu = "character.c:characterp", effect = Pure,
fn characterp(_cx, object: Value; optional ignored: Value) {
    let _ = ignored;
    Ok(if character(object).is_ok() { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "max-char", gnu = "character.c:max-char", effect = Pure,
fn max_char(_cx; optional unicode: Value) {
    Ok(Value::integer(if unicode == Value::NIL { i64::from(MAX_CHAR) } else { 0x10ffff }).unwrap())
}}

defsubr! { name = "char-to-string", gnu = "editfns.c:char-to-string", effect = Pure,
fn char_to_string(cx, char: Value) {
    let code=character(char)?;
    let multibyte=code>=128;
    let bytes = encode_chars([code], multibyte);
    Ok(cx.make_string(bytes, multibyte))
}}

defsubr! { name = "string-to-multibyte", gnu = "fns.c:string-to-multibyte", effect = Pure,
fn string_to_multibyte(cx, string: Value) {
    widen_string(cx, string)
}}

defsubr! { name = "string-make-multibyte", gnu = "fns.c:string-make-multibyte", effect = Pure,
fn string_make_multibyte(cx, string: Value) {
    if cx.string_view(string)?.bytes().is_ascii() { return Ok(string); }
    widen_string(cx, string)
}}

defsubr! { name = "string-make-unibyte", gnu = "fns.c:string-make-unibyte", effect = Pure,
fn string_make_unibyte(cx, string: Value) {
    let view = cx.string_view(string)?;
    if !view.is_multibyte() { return Ok(string); }
    let bytes = encode_chars(view.chars(), false);
    Ok(cx.make_string(bytes, false))
}}

defsubr! { name = "unibyte-char-to-multibyte", gnu = "character.c:unibyte-char-to-multibyte", effect = Pure,
fn unibyte_char_to_multibyte(_cx, char: Value) {
    let code = character(char)?;
    if code > 255 { return Err(Fault::Error(format!("Not a unibyte character: {code}"))); }
    Ok(Value::integer(i64::from(if code >= 128 { code + BYTE8_OFFSET } else { code })).unwrap())
}}

defsubr! { name = "multibyte-char-to-unibyte", gnu = "character.c:multibyte-char-to-unibyte", effect = Pure,
fn multibyte_char_to_unibyte(_cx, char: Value) {
    let code = character(char)?;
    let byte = if code < 256 { i64::from(code) }
        else if code >= BYTE8_OFFSET + 128 { i64::from(code - BYTE8_OFFSET) }
        else { -1 };
    Ok(Value::integer(byte).unwrap())
}}

defsubr! { name = "string", gnu = "alloc.c:string", effect = Pure,
fn string(cx; rest args) {
    let chars = args.iter().map(|&value| character(value)).collect::<Result<Vec<_>, _>>()?;
    let multibyte = chars.iter().any(|&c|c>=128);
    let bytes = encode_chars(chars, multibyte);
    Ok(cx.make_string(bytes, multibyte))
}}

defsubr! { name = "make-string", gnu = "alloc.c:make-string", effect = Pure,
fn make_string(cx, length: Value, initial: Value; optional multibyte: Value) {
    let count = length.integer_value().and_then(|n| usize::try_from(n).ok())
        .ok_or(Fault::Wrong("wholenump", length))?;
    let code = character(initial)?;
    let multibyte = multibyte != Value::NIL || code >= 128;
    let unit = encode_chars([code], multibyte);
    let size = count.checked_mul(unit.len())
        .ok_or_else(|| Fault::Error("String overflow".into()))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| Fault::Error("String overflow".into()))?;
    for _ in 0..count { bytes.extend_from_slice(&unit); }
    Ok(cx.make_string(bytes, multibyte))
}}

defsubr! { name = "char-equal", gnu = "editfns.c:char-equal", effect = Reads,
fn char_equal(cx, first: Value, second: Value) {
    let first = character(first)?;
    let second = character(second)?;
    let fold = match cx.obarray_lookup("case-fold-search") {
        Some(symbol) => cx.effective_cell(symbol)? != Value::NIL,
        None => false,
    };
    let equal = if first == second { true } else if fold {
        let table = romacs_regex_engine::CaseTranslation::standard();
        table.translate(first) == table.translate(second)
    } else { false };
    Ok(if equal { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "substring-no-properties", gnu = "fns.c:1619", effect = Pure,
fn substring_no_properties(cx, string: Value; optional from: Value, to: Value) {
    let view = cx.string_view(string)?;
    let chars: Vec<_> = view.chars().collect();
    let size = chars.len() as i64;
    let index = |value: Value, default| -> Result<i64, Fault> {
        if value == Value::NIL { return Ok(default); }
        let n = value.integer_value().ok_or(Fault::Wrong("integerp", value))?;
        Ok(if n < 0 { n + size } else { n })
    };
    let start = index(from, 0)?;
    let end = index(to, size)?;
    if !(0 <= start && start <= end && end <= size) {
        return Err(Fault::Range3(string, from, to));
    }
    // GNU's zero-length string allocator canonicalizes empty strings.
    // Preserve that identity when the input is already empty.
    if chars.is_empty() { return Ok(string); }
    let part = &chars[start as usize..end as usize];
    let bytes = encode_chars(part.iter().copied(), view.is_multibyte());
    Ok(cx.make_string(bytes, view.is_multibyte()))
}}

defsubr! { name = "string-equal", gnu = "fns.c:339", effect = Pure,
fn string_equal(cx, first: Value, second: Value) {
    let shape = |value| -> Result<(Vec<u8>, usize), Fault> {
        if let Ok(name) = cx.symbol_name(value) {
            let chars=name.chars().count(); Ok((name.into_bytes(),chars))
        } else {
            let view=cx.string_view(value)?; Ok((view.bytes,view.chars))
        }
    };
    let a=shape(first)?; let b=shape(second)?;
    Ok(if a == b { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "string-lessp", gnu = "fns.c:559", effect = Pure,
fn string_lessp(cx, first: Value, second: Value) {
    Ok(if cx.string_order(first, second)?.is_lt() { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "string-to-char", gnu = "editfns.c:176", effect = Pure,
fn string_to_char(cx, string: Value) {
    let chars = string_chars(cx, string)?;
    Ok(Value::integer(i64::from(chars.first().copied().unwrap_or(0))).unwrap())
}}

// GNU length_internal takes an int even though its callers hold EMACS_INT.
// Dotted list tails are deliberately not checked (fns.c:181-193).
fn length_internal(cx: &Leaf<'_>, mut tail: Value, bound: i64) -> Result<i64, Fault> {
    let mut remaining = bound as i32;
    let mut tortoise = tail;
    let mut traversed = 0u64;
    while let Some((_, cdr)) = cx.pair(tail) {
        remaining = remaining.wrapping_sub(1);
        if remaining <= 0 { return Ok(-1); }
        tail = cdr;
        if bound as i32 >= 0xffff {
            if tail == tortoise { return Err(Fault::Circular(tail)); }
            traversed += 1;
            if traversed & 4095 == 0 && traversed.is_power_of_two() { tortoise = tail; }
        }
    }
    Ok(i64::from(remaining))
}

fn sequence_length(cx: &Leaf<'_>, value: Value) -> Result<i64, Fault> {
    if value == Value::NIL { return Ok(0); }
    match cx.kind(value) {
        Kind::String | Kind::Bytes => Ok(cx.string_view(value)?.len() as i64),
        Kind::Vector => Ok(cx.vector_len(value)? as i64),
        _ => Err(Fault::Wrong("sequencep", value)),
    }
}

defsubr! { name = "length<", gnu = "fns.c:196", effect = Pure,
fn length_less(cx, sequence: Value, length: Value) {
    let len = length.integer_value().ok_or(Fault::Wrong("fixnump", length))?;
    let result = if cx.pair(sequence).is_some() { length_internal(cx, sequence, len)? != -1 }
        else { sequence_length(cx, sequence)? < len };
    Ok(if result { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "length>", gnu = "fns.c:211", effect = Pure,
fn length_greater(cx, sequence: Value, length: Value) {
    let len = length.integer_value().ok_or(Fault::Wrong("fixnump", length))?;
    let result = if cx.pair(sequence).is_some() { length_internal(cx, sequence, len + 1)? == -1 }
        else { sequence_length(cx, sequence)? > len };
    Ok(if result { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "length=", gnu = "fns.c:226", effect = Pure,
fn length_equal(cx, sequence: Value, length: Value) {
    let len = length.integer_value().ok_or(Fault::Wrong("fixnump", length))?;
    if len < 0 { return Ok(Value::NIL); }
    let result = if cx.pair(sequence).is_some() { length_internal(cx, sequence, len + 1)? == 1 }
        else { sequence_length(cx, sequence)? == len };
    Ok(if result { Value::TRUE } else { Value::NIL })
}}

#[cfg(test)]
mod tests {
    use super::*;
    fn printed(source: &str) -> String {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        // GNU supplies ignore; the bare Vm test fixture does not preload subr.el.
        vm.eval_source_raw("(defun ignore (&rest ignored) nil)").unwrap();
        let value = vm.eval_source_raw(&format!("(prin1-to-string (condition-case e {source} (error e)))")).expect("evaluation");
        Leaf::new(&mut vm).string_text(value).unwrap_or_else(|_| panic!("printer string"))
    }
    // GNU lane_emacs: ((2 6 1114112 4194303) (t nil) (nil t 255) (t t)).

    // GNU lane_emacs: ((ignore) (yank-handler (ignore)) "é").
    #[test] fn review_fill_properties() {
        // GNU lane_emacs: (help-echo "tip" face bold); fill preserves plist ordering.
        assert_eq!(printed("(let ((s (copy-sequence \"éé\"))) (put-text-property 0 2 'face 'bold s) (put-text-property 0 2 'help-echo \"tip\" s) (fillarray s 233) (text-properties-at 1 s))"), "(help-echo \"tip\" face bold)");
        let mut uniform_vm = Vm::new();
        let uniform = uniform_vm.eval_source_raw("(let ((s (copy-sequence \"éé\"))) (put-text-property 0 2 'face 'bold s) (fillarray s 233))").unwrap();
        let data::Object::String(uniform) = uniform_vm.object(uniform).unwrap() else { panic!("UTF8 string"); };
        assert!(uniform.properties.same_run_state(crate::coordinates::ByteOffset(0), crate::coordinates::ByteOffset(2)), "fillarray must preserve uniform property runs, not split stamps per character");
        // GNU lane_emacs: (4 nil nil). clear-string drops properties as it demotes.
        assert_eq!(printed("(let ((s (copy-sequence \"a€\"))) (put-text-property 1 2 'face 'bold s) (clear-string s) (list (length s) (text-properties-at 1 s) (text-properties-at 2 s)))"), "(4 nil nil)");
        let mut vm = Vm::new();
        let value = vm.eval_source_raw("(let ((s (concat \"a\" \"€\"))) (put-text-property 1 2 'yank-handler '(ignore) s) (fillarray s 233))").unwrap();
        let data::Object::String(s) = vm.object(value).unwrap() else { panic!("UTF8 string"); };
        assert!(s.properties.properties_at(crate::coordinates::ByteOffset(1)).is_empty(), "fillarray property boundary must move from byte 1 to byte 2");
        assert_eq!(printed("(let ((s (concat \"a\" \"€\"))) (put-text-property 1 2 'yank-handler '(ignore) s) (fillarray s 233) (list (get-text-property 1 'yank-handler s) (text-properties-at 1 s)))"), "((ignore) (yank-handler (ignore)))");
        assert_eq!(printed("(let ((s (concat \"a\" \"€\"))) (put-text-property 1 2 'yank-handler '(ignore) s) (fillarray s 233) (with-temp-buffer (insert-for-yank s) (buffer-string)))"), "\"é\"");
    }
    // GNU lane_emacs: (nil t).
    #[test] fn review_local_case_fold() {
        assert_eq!(printed("(list (with-temp-buffer (make-local-variable 'case-fold-search) (setq case-fold-search nil) (char-equal 97 65)) (char-equal 97 65))"), "(nil t)");
    }
    // GNU lane_emacs: ((wrong-type-argument arrayp nil) (wrong-type-argument arrayp foo)).
    #[test] fn review_fill_predicate() {
        assert_eq!(printed("(list (condition-case e (fillarray nil 97) (error e)) (condition-case e (fillarray 'foo 'bar) (error e)))"), "((wrong-type-argument arrayp nil) (wrong-type-argument arrayp foo))");
    }
    // GNU lane_emacs: hash of a buffer remains equal after garbage-collect => t.
    #[test] fn review_public_hash_gc() {
        let mut vm = Vm::new();
        let b = vm.eval_source_raw("(progn (let ((i 0)) (while (< i 1000) (make-string 100 97) (setq i (1+ i)))) (with-temp-buffer (current-buffer)))").unwrap();
        vm.declare_special_raw("review-buffer", b);
        let shape = "(list (sxhash-equal review-buffer) (sxhash-equal (list review-buffer)) (sxhash-equal (vector review-buffer)))";
        let before = vm.eval_source_raw(shape).unwrap();
        vm.declare_special_raw("review-before", before);
        vm.set_collection_floor(1);
        vm.collect(&mut []);
        let result = vm.eval_source_raw(&format!("(equal review-before {shape})")).unwrap();
        assert_eq!(result, Value::TRUE, "public hash must survive compaction, including opaque leaves");
    }
    #[test] fn str_extended_substring() {
        assert_eq!(printed("(let ((s (substring-no-properties (string 97 1114112 4194303) 1))) (list (length s) (string-bytes s) (aref s 0) (aref s 1)))"), "(2 6 1114112 4194303)");
    }
    #[test] fn str_case_fold_search() {
        assert_eq!(printed("(list (char-equal 65 97) (let ((case-fold-search nil)) (char-equal 65 97)))"), "(t nil)");
    }
    #[test] fn str_make_string_kind() {
        assert_eq!(printed("(list (multibyte-string-p (make-string 1 97)) (multibyte-string-p (make-string 1 97 t)) (aref (make-string 1 255) 0))"), "(nil t 255)");
    }
    #[test] fn str_characterp_ignored_argument() {
        assert_eq!(printed("(list (characterp 256 t) (characterp 4194303 t))"), "(t t)");
    }
    #[test] fn str_substring() {
        assert_eq!(printed("(substring-no-properties \"aλ雪z\" -3 -1)"), "\"λ雪\"");
        assert_eq!(printed("(substring-no-properties \"abc\" -4 nil)"), "(args-out-of-range \"abc\" -4 nil)");
    }
    #[test] fn str_equal() {
        assert_eq!(printed("(string-equal 'hello \"hello\")"), "t");
        assert_eq!(printed("(string-equal nil \"nil\")"), "t");
        assert_eq!(printed("(string-equal \"λ\" \"雪\")"), "nil");
    }
    #[test] fn str_less() {
        assert_eq!(printed("(string-lessp 'abc \"abd\")"), "t");
        assert_eq!(printed("(string-lessp \"雪\" \"λ\")"), "nil");
    }
    #[test] fn str_first() {
        assert_eq!(printed("(string-to-char \"λx\")"), "955");
        assert_eq!(printed("(string-to-char \"\")"), "0");
    }
    #[test] fn str_length_less() {
        assert_eq!(printed("(length< '(1 2) 3)"), "t");
        assert_eq!(printed("(length< '(1 . tail) 2)"), "t");
    }
    #[test] fn str_length_greater() {
        assert_eq!(printed("(length> [1 2] 1)"), "t");
        assert_eq!(printed("(length> \"λ雪\" 2)"), "nil");
    }
    #[test] fn str_length_equal() {
        assert_eq!(printed("(length= \"λ雪\" 2)"), "t");
        assert_eq!(printed("(length= 'bad -1)"), "nil");
    }
}

defsubr! { name = "sxhash", gnu = "fns.c:4166", effect = Pure,
fn sxhash(cx, value: Value) { Ok(cx.sxhash(value)) }}
defsubr! { name = "sxhash-equal", gnu = "fns.c:4166", effect = Pure,
fn sxhash_equal(cx, value: Value) { Ok(cx.sxhash(value)) }}

defsubr! { name = "sxhash-eq", gnu = "fns.c:5617", effect = Reads,
fn sxhash_eq(cx, value: Value) { cx.sxhash_eq(value) }}
// sxhash_eql: numbers `eql' compares by value hash by value, the rest by
// identity.
defsubr! { name = "sxhash-eql", gnu = "fns.c:5627", effect = Reads,
fn sxhash_eql(cx, value: Value) {
    if matches!(cx.kind(value), Kind::Float | Kind::Bignum) { Ok(cx.sxhash(value)) } else { cx.sxhash_eq(value) }
}}
// The contract is that `equal-including-properties' objects hash alike.
// GNU also mixes each string interval's plist into the hash, which breaks
// that contract for equal plists in another order or split; the `equal'
// hash keeps it, and hash values are never comparable across Emacsen.
defsubr! { name = "sxhash-equal-including-properties", gnu = "fns.c:5649", effect = Pure,
fn sxhash_equal_including_properties(cx, value: Value) { Ok(cx.sxhash(value)) }}
defsubr! { name = "fillarray", gnu = "fns.c:3308", effect = MutatesArgs,
fn fillarray(cx, array: Value, item: Value) {
    if cx.kind(array) == Kind::Vector {
        let n=cx.vector_len(array)?;
        for i in 0..n { cx.vector_set(array,Value::integer(i as i64).unwrap(),item)?; }
        Ok(array)
    } else if cx.kind(array) == Kind::BoolVector {
        let mut bits = cx.bool_vector(array)?;
        bits.bytes.fill(if item == Value::NIL { 0 } else { u8::MAX });
        bits.mask_padding();
        cx.bool_vector_store(array, bits)?;
        Ok(array)
    } else if matches!(cx.kind(array), Kind::String | Kind::Bytes) {
        cx.fill_string(array,character(item)?)
    } else if cx.kind(array) == Kind::CharTable {
        // Every character and the default, as set-char-table-range's t and
        // nil ranges write them; GNU's own loop leaves its ASCII cache
        // stale (VM2-FILLARRAY1).
        cx.set_char_table_range(array, Value::TRUE, item)?;
        cx.set_char_table_range(array, Value::NIL, item)?;
        Ok(array)
    } else { Err(Fault::Wrong("arrayp", array)) }
}}

defsubr! { name = "clear-string", gnu = "fns.c:3364", effect = MutatesArgs,
fn clear_string(cx, string: Value) { cx.clear_string(string) }}

defsubr! { name = "byte-to-string", gnu = "editfns.c:164", effect = Pure,
fn byte_to_string(cx, byte: Value) {
    let b = byte.integer_value().ok_or(Fault::Wrong("fixnump", byte))?;
    let b = u8::try_from(b).map_err(|_| Fault::Error("Invalid byte".into()))?;
    Ok(cx.make_string(vec![b], false))
}}

/// get-byte's value of C, a character of multibyte text: its byte if it
/// is ASCII or a raw byte, else GNU's error.
fn byte_of_char(c: i64) -> Out {
    let raw = i64::from(crate::lstring::BYTE8_OFFSET);
    if (0..0x80).contains(&c) { Ok(Value::integer(c).unwrap()) }
    else if (raw + 0x80..=raw + 0xff).contains(&c) { Ok(Value::integer(c - raw).unwrap()) }
    else { Err(Fault::Error(format!("Not an ASCII nor an 8-bit character: {c}"))) }
}

fn args_out_of_range(cx: &mut Leaf<'_>, items: &[Value]) -> Fault {
    let data = items.iter().rev().fold(Value::NIL, |list, &v| cx.cons(v, list));
    Fault::Signal(cx.intern("args-out-of-range"), data)
}

// character.c:898: the byte of the character at POSITION (point) of the
// current buffer, or at index POSITION (0) of STRING. Unibyte text gives
// its byte; multibyte text an ASCII or raw-byte character's.
defsubr! { name = "get-byte", gnu = "character.c:898", effect = Reads,
fn get_byte(cx; optional position: Value, string: Value) -> Step {
    if string != Value::NIL {
        let view = cx.string_view(string)?;
        let index = if position == Value::NIL { 0 } else {
            let n = position.integer_value().filter(|n| *n >= 0).ok_or(Fault::Wrong("wholenump", position))?;
            if n as usize >= view.len() { return Err(args_out_of_range(cx, &[string, position])); }
            n as usize
        };
        // GNU reads the terminating NUL of an empty string.
        let Some(c) = view.chars().nth(index) else { return Ok(Step::Done(Value::integer(0).unwrap())) };
        if !view.is_multibyte() { return Ok(Step::Done(Value::integer(i64::from(view.bytes()[index])).unwrap())); }
        return byte_of_char(i64::from(c)).map(Step::Done);
    }
    let (point, begv, zv) = cx.point_extent()?;
    if position == Value::NIL && point == zv {
        // PT_ADDR past a narrowing reads the text after it, at the true end
        // the terminating NUL.
        let buffer = cx.current_buffer()?;
        let Ok(next) = cx.buffer_region_text(buffer, point, point + 1) else { return Ok(Step::Done(Value::integer(0).unwrap())) };
        let view = cx.string_view(next)?;
        let byte = match (view.is_multibyte(), view.chars().next()) {
            (_, None) => Ok(Value::integer(0).unwrap()),
            (false, Some(_)) => Ok(Value::integer(i64::from(view.bytes()[0])).unwrap()),
            (true, Some(c)) => byte_of_char(i64::from(c)),
        };
        return byte.map(Step::Done);
    }
    let at = if position == Value::NIL { point } else {
        let n = match (position.integer_value(), cx.marker_position(position)) {
            (Some(n), _) => n,
            (None, Some(Some(n))) => n,
            (None, Some(None)) => return Err(Fault::Error("Marker does not point anywhere".into())),
            (None, None) => match cx.bigint_value(position) {
                // fix_position clips a bignum to the fixnum range.
                Some(n) if n.sign() == num_bigint::Sign::Minus => i64::MIN,
                Some(_) => i64::MAX,
                None => return Err(Fault::Wrong("integer-or-marker-p", position)),
            },
        };
        if !(begv <= n && n < zv) { return Err(args_out_of_range(cx, &[position, Value::integer(begv).unwrap(), Value::integer(zv).unwrap()])); }
        n
    };
    let f = cx.intern("char-after");
    Ok(Step::Call { f, args: vec![Value::integer(at).unwrap()], then: got_byte, saved: vec![] })
}}

fn got_byte(cx: &mut Leaf<'_>, c: Value, _: &[Value]) -> Next {
    let Some(c) = c.integer_value() else { return Ok(Step::Done(Value::integer(0).unwrap())) };
    if !cx.buffer_multibyte()? { return Ok(Step::Done(Value::integer(c).unwrap())); }
    byte_of_char(c).map(Step::Done)
}
