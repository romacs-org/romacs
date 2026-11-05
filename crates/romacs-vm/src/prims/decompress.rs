//! GNU decompress.c: zlib-available-p and zlib-decompress-region. zlib's
//! inflate with MAX_WBITS + 32 (gzip or zlib framing, detected) is the
//! framing parsed here around flate2's raw DEFLATE, so consumption and
//! partial output follow zlib's: a truncated stream keeps what it decoded
//! and leaves no input unread; a bad header stops where zlib stops.
use super::*;

defsubr! { name = "zlib-available-p", gnu = "decompress.c:202", effect = Pure,
fn zlib_available_p(_cx) { Ok(Value::TRUE) }}

// decompress.c:223. The decompressed bytes go in after the region and the
// region is then deleted, one change for the hooks (romacs--decompressed in
// rlisp/vm/insdel.el); point returns to where it was, clipped to the new
// end, however the hooks exit. On failure the region stays and the value
// is nil, or with ALLOW-PARTIAL the decoded part replaces it and the value
// is the count of unread bytes.
defsubr! { name = "zlib-decompress-region", gnu = "decompress.c:223", effect = Global,
fn zlib_decompress_region(cx, start: Value, end: Value; optional allow_partial: Value) -> Step {
    let (from, to) = cx.validate_region(start, end)?;
    if cx.buffer_multibyte()? {
        return Err(Fault::Error("This function can be called only in unibyte buffers".into()));
    }
    let text = cx.region_text(from, to)?;
    let input = cx.string_view(text)?.bytes().to_vec();
    let inflated = inflate(&input);
    let (result, replacement) = if inflated.complete { (Value::TRUE, cx.make_string(inflated.out, false)) }
        else if allow_partial != Value::NIL {
            (Value::integer((input.len() - inflated.consumed) as i64).unwrap(), cx.make_string(inflated.out, false))
        } else { (Value::NIL, Value::NIL) };
    let saved = vec![Value::integer(from).unwrap(), Value::integer(to).unwrap(), replacement, result];
    if super::insdel::half_missing(cx)? { return super::insdel::install_half(cx, decompressed, saved); }
    decompressed(cx, Value::NIL, &saved)
}}

fn decompressed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern("romacs--decompressed"), args: saved.to_vec(), then: returned, saved: vec![] })
}

fn returned(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next { Ok(Step::Done(value)) }

// zlib-decompress-region's modify_text of FROM..TO: the before hooks, the
// deactivated mark, then the modification bookkeeping, which GNU does even
// when decompression fails. The value is (INHIBITED . OVERLAYS) for
// `romacs--decompress-change'.
defsubr! { name = "romacs--decompress-prepare", gnu = "decompress.c:263", effect = Global,
fn decompress_prepare(cx, from: Value, to: Value) -> Step {
    let position = |v: Value| v.integer_value().ok_or(Fault::Wrong("fixnump", v));
    let (start, end) = (position(from)?, position(to)?);
    let prepared = cx.prepare_change(start, end)?;
    let saved = vec![from, to, if prepared.inhibited { Value::TRUE } else { Value::NIL }];
    if !prepared.before { return modified(cx, Value::NIL, &saved); }
    let buffer = cx.current_buffer()?;
    let hooks = prepared.modification_hooks.iter().rev().fold(Value::NIL, |tail, &v| cx.cons(v, tail));
    let undoable = if prepared.undoable { Value::TRUE } else { Value::NIL };
    let args = vec![buffer, from, to, hooks, undoable, from];
    Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: modified, saved })
}}

fn modified(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let overlays = super::insdel::found_overlays(cx, before);
    cx.change_deactivates_mark(saved[2] != Value::NIL)?;
    cx.modify_text(saved[0].integer_value().expect("from"), saved[1].integer_value().expect("to"))?;
    Ok(Step::Done(cx.cons(saved[2], overlays)))
}

// Then, with point at TO, the bytes TEXT inserted there (markers at TO
// stay before them unless they advance) and FROM..TO deleted, and
// signal_after_change. A nil TEXT changes nothing, but the after hooks
// still balance the before hooks, as GNU's unwind's do. PREPARED is
// `romacs--decompress-prepare''s value.
defsubr! { name = "romacs--decompress-change", gnu = "decompress.c:263", effect = Global,
fn decompress_change(cx, from: Value, to: Value, text: Value, prepared: Value) -> Step {
    let position = |v: Value| v.integer_value().ok_or(Fault::Wrong("fixnump", v));
    let (from, to) = (position(from)?, position(to)?);
    let overlays = cx.pair(prepared).map_or(Value::NIL, |(_, overlays)| overlays);
    cx.goto_char_position(to)?;
    let changed_end = if text == Value::NIL { to } else {
        let inserted = if cx.string_chars(text) == Some(0) { 0 } else { cx.insert_change(text, false, false)? };
        cx.goto_char_position(to)?;
        cx.delete_change(from, to, false)?;
        from + inserted
    };
    if !cx.after_change_runs(overlays != Value::NIL, false)? { return Ok(Step::Done(Value::NIL)); }
    let buffer = cx.current_buffer()?;
    let position = |n: i64| Value::integer(n).unwrap();
    let args = vec![buffer, position(from), position(changed_end), position(to - from), Value::NIL, Value::NIL, overlays];
    Ok(Step::Call { f: cx.intern("romacs--after-change"), args, then: returned, saved: vec![] })
}}

/// inflate's work on one input: the bytes it produced, the input bytes it
/// read, and whether it reached Z_STREAM_END.
struct Inflated { out: Vec<u8>, consumed: usize, complete: bool }

/// Where a framing header leaves inflate.
enum Header {
    /// The DEFLATE data starts here.
    Body(usize),
    /// The input ends inside the header: zlib read it all and waits.
    Short,
    /// zlib refuses the header after reading this many bytes.
    Bad(usize),
}

fn inflate(input: &[u8]) -> Inflated {
    let gzip = input.starts_with(&[0x1f, 0x8b]);
    let stop = |out: Vec<u8>, consumed: usize| Inflated { out, consumed, complete: false };
    let start = match if gzip { gzip_header(input) } else { zlib_header(input) } {
        Header::Body(at) => at,
        Header::Short => return stop(Vec::new(), input.len()),
        Header::Bad(at) => return stop(Vec::new(), at),
    };
    let mut raw = flate2::Decompress::new(false);
    let mut out = Vec::new();
    let body = &input[start..];
    loop {
        out.reserve(16 * 1024);
        let (read, wrote) = (raw.total_in(), raw.total_out());
        match raw.decompress_vec(&body[read as usize..], &mut out, flate2::FlushDecompress::None) {
            Ok(flate2::Status::StreamEnd) => break,
            Ok(_) if raw.total_in() == read && raw.total_out() == wrote => return stop(out, input.len()),
            Ok(_) => {}
            Err(_) => return stop(out, start + raw.total_in() as usize),
        }
    }
    // The trailer: gzip's CRC-32 and length, zlib's Adler-32, each checked
    // once all of it is read.
    let at = start + raw.total_in() as usize;
    let trailer = &input[at..];
    let checks: Vec<[u8; 4]> = if gzip {
        let mut crc = flate2::Crc::new();
        crc.update(&out);
        vec![crc.sum().to_le_bytes(), (out.len() as u32).to_le_bytes()]
    } else {
        vec![adler32(&out).to_be_bytes()]
    };
    for (i, check) in checks.iter().enumerate() {
        let Some(field) = trailer.get(i * 4..i * 4 + 4) else { return stop(out, input.len()) };
        if field != check { return stop(out, at + i * 4 + 4); }
    }
    Inflated { out, consumed: at + checks.len() * 4, complete: true }
}

/// RFC 1950's two header bytes, then a preset dictionary's id, which zlib
/// asks for (Z_NEED_DICT) instead of inflating.
fn zlib_header(input: &[u8]) -> Header {
    let [cmf, flg, ..] = *input else { return Header::Short };
    if (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0 || cmf & 0x0f != 8 || (cmf >> 4) + 8 > 15 {
        return Header::Bad(2);
    }
    if flg & 0x20 == 0 { return Header::Body(2); }
    if input.len() < 6 { Header::Short } else { Header::Bad(6) }
}

/// RFC 1952's member header: magic, method and flags, then the optional
/// extra field, name, comment and header CRC.
fn gzip_header(input: &[u8]) -> Header {
    let [_, _, method, flags, ..] = *input else { return Header::Short };
    if method != 8 || flags & 0xe0 != 0 { return Header::Bad(4); }
    let mut at = 10;
    if input.len() < at { return Header::Short; }
    if flags & 0x04 != 0 {
        let Some(len) = input.get(at..at + 2) else { return Header::Short };
        at += 2 + usize::from(u16::from_le_bytes([len[0], len[1]]));
        if input.len() < at { return Header::Short; }
    }
    for flag in [0x08, 0x10] {
        if flags & flag != 0 {
            let Some(nul) = input[at..].iter().position(|&b| b == 0) else { return Header::Short };
            at += nul + 1;
        }
    }
    if flags & 0x02 != 0 {
        let Some(field) = input.get(at..at + 2) else { return Header::Short };
        let mut crc = flate2::Crc::new();
        crc.update(&input[..at]);
        if u16::from_le_bytes([field[0], field[1]]) != (crc.sum() & 0xffff) as u16 { return Header::Bad(at + 2); }
        at += 2;
    }
    Header::Body(at)
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for &byte in chunk { a += u32::from(byte); b += a; }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}
