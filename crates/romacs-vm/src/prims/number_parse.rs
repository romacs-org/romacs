//! GNU `string-to-number`: data.c:3092 and lread.c:4500.
//! Integer results outside VM2's fixnum range require the absent bignum substrate.
use crate::prim::Fault;
use crate::Value;

fn digit(byte: u8, base: u32) -> Option<u32> {
    let n = match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'f' => u32::from(byte - b'a') + 10,
        b'A'..=b'F' => u32::from(byte - b'A') + 10,
        _ => return None,
    };
    (n < base).then_some(n)
}

defsubr! { name = "string-to-number", gnu = "data.c:3092", effect = Pure,
    fn string_to_number(cx, string: Value; optional base: Value) {
        // CHECK_STRING precedes CHECK_FIXNUM and the radix range check in GNU.
        let bytes = cx.string_bytes(string)?;
        let radix = if base == Value::NIL { 10 } else {
            let n = base.integer_value().ok_or(Fault::Wrong("fixnump", base))?;
            if !(2..=16).contains(&n) { return Err(Fault::Range1(base)); }
            n as u32
        };
        let mut p = 0;
        while matches!(bytes.get(p), Some(b' ' | b'\t')) { p += 1; }
        let negative = bytes.get(p) == Some(&b'-');
        if matches!(bytes.get(p), Some(b'+' | b'-')) { p += 1; }
        let start = p;
        // Keep the low 64 bits for GNU's NaN payload even if integer overflowed.
        let mut magnitude = 0u64;
        let mut overflow = false;
        while let Some(n) = bytes.get(p).and_then(|&b| digit(b, radix)) {
            overflow |= magnitude.checked_mul(u64::from(radix))
                .and_then(|x| x.checked_add(u64::from(n))).is_none();
            magnitude = magnitude.wrapping_mul(u64::from(radix)).wrapping_add(u64::from(n));
            p += 1;
        }
        let leading = p > start;
        if radix == 10 {
            if bytes.get(p) == Some(&b'.') { p += 1; }
            let fraction_start = p;
            while bytes.get(p).is_some_and(u8::is_ascii_digit) { p += 1; }
            let trailing = p > fraction_start;
            let mut exponent = false;
            let mut special = None;
            if matches!(bytes.get(p), Some(b'e' | b'E')) {
                let exponent_start = p;
                p += 1;
                let plus = bytes.get(p) == Some(&b'+');
                if matches!(bytes.get(p), Some(b'+' | b'-')) { p += 1; }
                let digits_start = p;
                while bytes.get(p).is_some_and(u8::is_ascii_digit) { p += 1; }
                if p > digits_start { exponent = true; }
                else if plus && bytes.get(p..p + 3) == Some(b"INF") {
                    special = Some(f64::INFINITY);
                    exponent = true;
                    p += 3;
                } else if plus && bytes.get(p..p + 3) == Some(b"NaN") {
                    // GNU initializes n from digit_to_number, whose non-digit
                    // sentinel is -2; a dot-leading float retains that payload.
                    let payload = if leading { magnitude } else { u64::MAX - 1 };
                    special = Some(f64::from_bits(0x7ff8_0000_0000_0000 | (payload & 0x0007_ffff_ffff_ffff)));
                    exponent = true;
                    p += 3;
                } else { p = exponent_start; }
            }
            if trailing || (leading && exponent) {
                // Only ASCII numeric syntax reaches this slice; no locale or C-string conversion.
                let value = if let Some(value) = special { value } else {
                    std::str::from_utf8(&bytes[start..p]).unwrap().parse::<f64>()
                        .map_err(|_| Fault::Error("Invalid floating-point syntax".into()))?
                };
                return Ok(cx.float(if negative { -value } else { value }));
            }
        }
        if !leading { return Ok(Value::integer(0).unwrap()); }
        let limit = if negative { Value::MIN_FIXNUM.unsigned_abs() } else { Value::MAX_FIXNUM as u64 };
        if overflow || magnitude > limit {
            return Err(Fault::Error("string-to-number: bignum results are not supported".into()));
        }
        let n = if negative { -(magnitude as i64) } else { magnitude as i64 };
        Ok(Value::integer(n).unwrap())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn string_to_number_preserves_nan_payload_bits_before_printing() {
        for (source, bits) in [
            ("(string-to-number \"1.0e+NaN\")", 0x7ff8_0000_0000_0001),
            ("(string-to-number \"-1.0e+NaN\")", 0xfff8_0000_0000_0001),
            ("(string-to-number \".5e+NaN\")", 0x7fff_ffff_ffff_fffe),
            ("(string-to-number \"-.5e+NaN\")", 0xffff_ffff_ffff_fffe),
            ("(string-to-number \"18446744073709551617.0e+NaN\")", 0x7ff8_0000_0000_0001),
        ] {
            let mut vm = crate::Vm::new();
            let value = vm.eval_source_raw(source).unwrap();
            assert_eq!(vm.number(value).unwrap().to_bits(), bits);
        }
    }
}
