use romacs_chars::*;

#[test]
fn every_char_round_trips() {
    let mut buf = [0; MAX_MULTIBYTE_LENGTH];
    for c in 0..=MAX_CHAR {
        let len = char_string(c, &mut buf);
        let form = &buf[..len];
        assert_eq!(len, char_bytes(c), "{c:#x}");
        assert_eq!(string_char(form), (c, len), "{c:#x}");
        assert_eq!(multibyte_length(form, true), len, "{c:#x}");
        let strict = if char_byte8_p(c) { 0 } else { len };
        assert_eq!(multibyte_length(form, false), strict, "{c:#x}");
        assert_eq!(multibyte_length(&form[..len - 1], true), 0, "{c:#x}");
        assert_eq!(bytes_by_char_head(form[0]), len);
        assert_eq!(char_byte8_head_p(form[0]), char_byte8_p(c));
        assert!(char_head_p(form[0]) && form[1..].iter().all(|&b| trailing_code_p(b)));
        assert_eq!(prev_char_len(form), len);
        assert_eq!(char_to_byte_safe(c).is_some(), c < 0x80 || char_byte8_p(c));
    }
}

#[test]
fn every_byte_round_trips() {
    let all: Vec<u8> = (0..=255).collect();
    let multi = str_to_multibyte(&all);
    assert_eq!(multi.len(), count_size_as_multibyte(&all));
    assert_eq!(multibyte_chars_in_text(&multi), 256);
    let decoded: Vec<u32> = chars(&multi).collect();
    assert_eq!(
        decoded,
        all.iter().map(|&b| unibyte_to_char(b)).collect::<Vec<_>>()
    );
    assert!(
        decoded
            .iter()
            .zip(&all)
            .all(|(&c, &b)| char_to_byte8(c) == b)
    );
    assert_eq!(str_as_unibyte(&multi), all);
    assert_eq!(str_to_unibyte(&multi), Ok(all.clone()));
    assert_eq!(str_as_multibyte(&all), (multi, 256));
    assert_eq!(str_to_unibyte("aλ".as_bytes()), Err(1));
}

#[test]
fn invalid_forms_and_boundaries() {
    for form in [
        &b"\x80"[..],
        &b"\xff"[..],
        &b"\xc2A"[..],
        &b"\xe0\x80\x80"[..],
        &b"\xf0\x80\x80\x80"[..],
        &b"\xf8\x87\xbf\xbf\xbf"[..],
        &b"\xf8\x8f\xbf\xbe\x80"[..],
        &b"\xf9\x88\x80\x80\x80"[..],
    ] {
        assert_eq!(multibyte_length(form, true), 0, "{form:x?}");
    }
    for c in [
        0, 0x7f, 0x80, 0x7ff, 0x800, 0xd800, 0xffff, 0x10000, 0x1fffff, 0x200000, 0x3fff7f,
        0x3fff80, 0x3fffff,
    ] {
        assert!(char_valid_p(c));
    }
    assert!(!char_valid_p(-1));
    assert!(!char_valid_p(0x400000));
    assert_eq!(
        str_as_multibyte(b"a\xc2\x80\xff"),
        (b"a\xc2\x80\xc1\xbf".to_vec(), 3)
    );
    assert_eq!(multibyte_chars_in_text(b""), 0);
    assert_eq!(chars(b"").next(), None);
}

#[test]
#[should_panic(expected = "Lisp position overflow")]
fn lisp_position_cannot_wrap_to_zero() {
    let _ = LispPos::from(CharPos(usize::MAX));
}

#[test]
fn position_arithmetic_cannot_wrap() {
    let mut failures = Vec::new();
    macro_rules! boundaries {
        ($pos:ident) => {
            failures.push(std::panic::catch_unwind(|| $pos(usize::MAX) + 1).is_err());
            failures.push(std::panic::catch_unwind(|| $pos(0) - 1).is_err());
            failures.push(std::panic::catch_unwind(|| $pos(0) - $pos(1)).is_err());
            assert_eq!($pos(usize::MAX) + 0, $pos(usize::MAX));
            assert_eq!($pos(usize::MAX) - usize::MAX, $pos(0));
            assert_eq!($pos(usize::MAX) - $pos(usize::MAX), 0);
        };
    }
    boundaries!(BytePos);
    boundaries!(CharPos);
    assert_eq!(failures, [true; 6]);
}

#[test]
fn ascii_blocks_preserve_fold_order_and_remainders() {
    for prefix in 0..24 {
        for c in [0x80, 0x800, 0x10000, 0x200000, 0x3fff80, 0x3fffff] {
            let mut buf = [0; MAX_MULTIBYTE_LENGTH];
            let width = char_string(c, &mut buf);
            let mut bytes: Vec<_> = (0..prefix).map(|i| i as u8).collect();
            bytes.extend_from_slice(&buf[..width]);
            bytes.extend_from_slice(b"abcdefghijklmnopq");
            let expected: Vec<_> = chars(&bytes).collect();
            let folded = chars(&bytes).fold(Vec::new(), |mut out, c| {
                out.push(c);
                out
            });
            assert_eq!(folded, expected);
            assert_eq!(
                chars(&bytes).map(u64::from).sum::<u64>(),
                expected.iter().map(|&c| c as u64).sum::<u64>()
            );
            let mut partial = chars(&bytes);
            assert_eq!(partial.next(), expected.first().copied());
            assert_eq!(
                partial.fold(Vec::new(), |mut out, c| {
                    out.push(c);
                    out
                }),
                expected[1..]
            );
        }
    }
}

#[cfg(debug_assertions)]
#[test]
fn trusted_boundaries_reject_malformed_forms() {
    for form in [
        &b"\xc2A"[..],
        &b"\xe0\x80\x80"[..],
        &b"\xc0"[..],
        &b"\xc0A"[..],
        &b"\xe2\x82"[..],
        &b"\x82\xac"[..],
    ] {
        assert!(std::panic::catch_unwind(|| string_char(form)).is_err());
        assert!(std::panic::catch_unwind(|| chars(form).collect::<Vec<_>>()).is_err());
        assert!(std::panic::catch_unwind(|| prev_char_len(form)).is_err());
        assert!(std::panic::catch_unwind(|| str_as_unibyte(form)).is_err());
        assert!(std::panic::catch_unwind(|| str_to_unibyte(form)).is_err());
    }
    assert!(std::panic::catch_unwind(|| prev_char_len(b"")).is_err());
    assert!(std::panic::catch_unwind(|| byte8_to_char(0x7f)).is_err());
    assert_eq!(chars(b"").count(), 0);
    assert_eq!(str_as_unibyte(b""), b"");
    assert_eq!(str_to_unibyte(b""), Ok(Vec::new()));
}

#[test]
fn positions() {
    let pos = CharPos(4);
    assert_eq!(LispPos::from(pos).get(), 5);
    assert_eq!(CharPos::from(LispPos::from(pos)), pos);
    assert_eq!(LispPos::new(0), None);
    assert_eq!(LispPos::MIN, LispPos::from(CharPos(0)));
    assert_eq!(BytePos(3) + 2 - BytePos(1), 4);
}
