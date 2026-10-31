//! GNU src/fileio.c `insert-file-contents' (fileio.c:4057) and `write-region'
//! (fileio.c:5461): read a file into the current buffer, write buffer text or
//! a string to a file. Both are T2: GNU calls Lisp while choosing the coding
//! system and, for `insert-file-contents', after inserting
//! (`after-insert-file-set-coding', `format-decode',
//! `after-insert-file-functions'). Here are the checks, the read and the
//! write; prims/insert_coding.rs decides, decodes and inserts, and
//! prims/write_coding.rs chooses the coding system written. Both dispatch
//! file name handlers as GNU does; `write-region' refuses the annotation
//! functions.
//!
//! Supported subset (DIVERGENCES.md VM2-FILEIO1 lists the rest):
//!
//! * Decoding: GNU's decision and coding.c's detection in its priority
//!   order, through the native string decoder. Detection refuses where it
//!   would need a detector VM2 lacks (ISO-2022 escapes, UTF-16 for 8-bit text
//!   with NUL bytes, the charset, CCL and CJK categories), and text holding
//!   raw bytes cannot enter a multibyte buffer.
//! * Encoding: GNU's choose_write_coding_system, then the native string
//!   encoder (`encode-coding-string''s coding types) over the region's text,
//!   multibyte or unibyte.
//! * Regular input files only; no REPLACE, no `~user'.
use super::*;
use crate::prim::leaf::Kind;
use crate::forwarded::VisitedModtime;
use super::insert_coding;

fn metadata_modtime(meta: &std::fs::Metadata) -> VisitedModtime {
    use std::os::unix::fs::MetadataExt;
    VisitedModtime::Known { seconds: meta.mtime(), nanos: meta.mtime_nsec(), size: Some(meta.len()) }
}

fn stat_modtime(path: &std::path::Path) -> VisitedModtime {
    match crate::host::metadata(path) {
        Ok(meta) => metadata_modtime(&meta),
        Err(e) if matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) => VisitedModtime::Missing,
        Err(_) => VisitedModtime::Unknown,
    }
}

defsubr! { name = "visited-file-modtime", gnu = "fileio.c:6165", effect = Global,
fn visited_file_modtime(cx) {
    let (state, _) = cx.fileio_modtime(Value::NIL)?;
    match state {
        VisitedModtime::Unknown => Ok(Value::integer(0).map_err(Fault::Error)?),
        VisitedModtime::Missing => Ok(Value::integer(-1).map_err(Fault::Error)?),
        VisitedModtime::Known { seconds, nanos, .. } => {
            if var(cx, "current-time-list")? == Value::NIL {
                let ticks = seconds as i128 * 1_000_000_000 + nanos as i128;
                let ticks = i64::try_from(ticks).ok().and_then(|n| Value::integer(n).ok())
                    .ok_or_else(|| refuse("visited-file-modtime requiring bignum ticks"))?;
                return Ok(cx.cons(ticks, Value::integer(1_000_000_000).unwrap()));
            }
            let mut time = Value::NIL;
            for n in [nanos % 1000 * 1000, nanos / 1000, seconds.rem_euclid(65536), seconds.div_euclid(65536)] {
                let n = Value::integer(n).map_err(Fault::Error)?;
                time = cx.cons(n, time);
            }
            Ok(time)
        }
    }
}}

defsubr! { name = "verify-visited-file-modtime", gnu = "fileio.c:6121", effect = Global,
fn verify_visited_file_modtime(cx; optional buffer: Value) -> Step {
    let (state, name) = cx.fileio_modtime(buffer)?;
    let Some(name) = name else { return Ok(Step::Done(Value::TRUE)) };
    if state == VisitedModtime::Unknown { return Ok(Step::Done(Value::TRUE)) }
    if let Some(call) = handler(cx, name, "verify-visited-file-modtime", vec![buffer])? { return call; }
    let actual = stat_modtime(&file_name_path(cx, name)?);
    let equal = match (state, actual) {
        (VisitedModtime::Known { seconds: s, nanos: n, size }, VisitedModtime::Known { seconds, nanos, size: actual_size }) =>
            s == seconds && n == nanos && (size.is_none() || size == actual_size),
        _ => state == actual,
    };
    Ok(Step::Done(if equal { Value::TRUE } else { Value::NIL }))
}}

fn float_modtime_nanos(f: f64) -> std::result::Result<i128, Fault> {
    if f.is_nan() { return Err(Fault::Error("Invalid time specification".into())) }
    if !f.is_finite() || !(-9223372036854775808.0..9223372036854775808.0).contains(&f) {
        return Err(Fault::Error("Specified time is not representable".into()));
    }
    let bits = f.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let mantissa = (bits & ((1u64 << 52) - 1)) | if exponent == 0 { 0 } else { 1u64 << 52 };
    let signed = if bits >> 63 == 0 { mantissa as i128 } else { -(mantissa as i128) };
    let numerator = signed * 1_000_000_000;
    let power = if exponent == 0 { -1074 } else { exponent - 1023 - 52 };
    Ok(if power >= 0 { numerator << power }
        else if power > -128 { numerator >> -power }
        else if numerator < 0 { -1 } else { 0 })
}

fn explicit_modtime(cx: &Leaf<'_>, time: Value) -> std::result::Result<VisitedModtime, Fault> {
    if let Some(flag) = time.integer_value() {
        return match flag {
            0 => Ok(VisitedModtime::Unknown), -1 => Ok(VisitedModtime::Missing),
            _ => Err(Fault::Range3(time, Value::integer(-1).unwrap(), Value::integer(0).unwrap())),
        }
    }
    let invalid = || Fault::Error("Invalid time specification".into());
    let ns: i128 = if cx.kind(time) == Kind::Float {
        float_modtime_nanos(cx.float_value(time)?)?
    } else {
        let (high, tail) = cx.pair(time).ok_or_else(invalid)?;
        let high = high.integer_value().ok_or_else(invalid)? as i128;
        if let Some(hz) = tail.integer_value() {
            if hz <= 0 { return Err(invalid()) }
            (high * 1_000_000_000).div_euclid(hz as i128)
        } else {
            let (low, tail) = cx.pair(tail).ok_or_else(invalid)?;
            let low = low.integer_value().ok_or_else(invalid)? as i128;
            let (usec, psec) = if let Some((usec, rest)) = cx.pair(tail) {
                (usec, cx.pair(rest).map_or(Value::integer(0).unwrap(), |(psec, _)| psec))
            } else {
                (if tail == Value::NIL { Value::integer(0).unwrap() } else { tail }, Value::integer(0).unwrap())
            };
            let usec = usec.integer_value().ok_or_else(invalid)? as i128;
            let psec = psec.integer_value().ok_or_else(invalid)? as i128;
            (high * 65536 + low) * 1_000_000_000 + usec * 1000 + psec.div_euclid(1000)
        }
    };
    let seconds = i64::try_from(ns.div_euclid(1_000_000_000))
        .map_err(|_| Fault::Error("Specified time is not representable".into()))?;
    Ok(VisitedModtime::Known { seconds, nanos: ns.rem_euclid(1_000_000_000) as i64, size: None })
}

defsubr! { name = "set-visited-file-modtime", gnu = "fileio.c:6177", effect = Global,
fn set_visited_file_modtime(cx; optional time: Value) -> Step {
    if time != Value::NIL {
        let state = explicit_modtime(cx, time)?;
        cx.fileio_set_modtime(state)?;
        return Ok(Step::Done(Value::NIL));
    }
    let (_, filename) = cx.fileio_modtime(Value::NIL)?;
    let Some(name) = filename else { return Err(Fault::Wrong("stringp", Value::NIL)) };
    expanding(cx, name, visited_modtime_expanded, vec![])
}}
fn visited_modtime_expanded(cx: &mut Leaf<'_>, name: Value, _: &[Value]) -> Next {
    if let Some(call) = handler(cx, name, "set-visited-file-modtime", vec![Value::NIL])? { return call; }
    let path = file_name_path(cx, name)?;
    let meta = match crate::host::metadata(path) {
        Ok(meta) => meta,
        Err(e) if matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) => return Ok(Step::Done(Value::NIL)),
        Err(e) => return Err(file_errno(cx, "Getting attributes", name, &e)),
    };
    cx.fileio_set_modtime(metadata_modtime(&meta))?;
    Ok(Step::Done(Value::NIL))
}

/// GNU `report_file_errno' (fileio.c:286) via `get_file_errno_data'
/// (fileio.c:265): EEXIST drops STRING; ENOENT, EACCES and the rest choose
/// the condition.
pub(super) fn file_errno(cx: &mut Leaf<'_>, string: &str, name: Value, error: &std::io::Error) -> Fault {
    let errno = error.raw_os_error().unwrap_or(0);
    let text = std::io::Error::from_raw_os_error(errno).to_string();
    // glibc strerror text, without Rust's " (os error N)" suffix.
    let text = text.rsplit_once(" (os error ").map_or(text.as_str(), |(t, _)| t).to_owned();
    let message = cx.string(&text);
    let tail = cx.cons(name, Value::NIL);
    let errdata = cx.cons(message, tail);
    const EEXIST: i32 = 17;
    const ENOENT: i32 = 2;
    const EACCES: i32 = 13;
    if errno == EEXIST {
        return Fault::Signal(cx.intern("file-already-exists"), errdata);
    }
    let condition = cx.intern(match errno {
        ENOENT => "file-missing",
        EACCES => "permission-denied",
        _ => "file-error",
    });
    let string = cx.string(string);
    Fault::Signal(condition, cx.cons(string, errdata))
}

/// The named refusal every unsupported path takes: `(error "VM2 ...")'.
pub(super) fn refuse(what: &str) -> Fault {
    Fault::Error(format!("VM2 {what} is unsupported"))
}

/// The value of special variable NAME, nil when void (the C value of every
/// variable read here is nil).
pub(super) fn var(cx: &mut Leaf<'_>, name: &str) -> std::result::Result<Value, Fault> {
    let symbol = cx.intern(name);
    Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL))
}

/// STRINGP: VM2 keeps a unibyte string with octets above 127 as `Bytes'.
fn stringp(cx: &Leaf<'_>, v: Value) -> bool {
    matches!(cx.kind(v), Kind::String | Kind::Bytes)
}

fn symbol_is(cx: &mut Leaf<'_>, v: Value, names: &[&str]) -> bool {
    cx.kind(v) == Kind::Symbol && cx.symbol_name(v).is_ok_and(|n| names.contains(&n.as_str()))
}

/// GNU `Fexpand_file_name (NAME, Qnil)', then THEN with the expansion:
/// native where no handler applies, through Lisp `expand-file-name' where
/// GNU would call one.
pub(super) fn expanding(cx: &mut Leaf<'_>, name: Value, then: crate::prim::Then, saved: Vec<Value>) -> Next {
    super::expand_file_name::expanding(cx, name, Value::NIL, then, saved)
}

/// ENCODE_FILE under a UTF-8 `file-name-coding-system': the host bytes of
/// the file name VALUE, a raw-byte character as its byte.
pub(super) fn file_name_bytes(cx: &Leaf<'_>, value: Value) -> std::result::Result<Vec<u8>, Fault> {
    cx.file_name_bytes(value)
}

/// VALUE as the host path: a name with raw-byte characters names the file
/// whose name has those bytes.
pub(super) fn file_name_path(cx: &Leaf<'_>, value: Value) -> std::result::Result<std::path::PathBuf, Fault> {
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(file_name_bytes(cx, value)?).into())
}

/// DECODE_FILE under a UTF-8 `file-name-coding-system': the Lisp string
/// of the host name BYTES, invalid sequences as raw-byte characters;
/// unibyte when ASCII, as decode_coding leaves ASCII.
pub(super) fn file_name_value(cx: &mut Leaf<'_>, bytes: &[u8]) -> Value {
    if bytes.is_ascii() {
        return cx.make_string(bytes.to_vec(), false);
    }
    cx.make_string(crate::text::octets_as_multibyte(bytes).0, true)
}

/// GNU `auto_saving': the mode bits an auto-save file is created with while
/// `do-auto-save' writes (prims/autosave.rs), else None.
pub(super) fn auto_saving(cx: &mut Leaf<'_>) -> std::result::Result<Option<u32>, Fault> {
    Ok(var(cx, super::autosave::AUTO_SAVING)?.integer_value().map(|bits| bits as u32))
}

/// Whether the current buffer auto-saves into its visited file:
/// Fstring_equal of its file name and its auto-save file name.
pub(super) fn auto_saving_into_visited(cx: &mut Leaf<'_>) -> std::result::Result<bool, Fault> {
    let (file, auto) = (var(cx, "buffer-file-name")?, var(cx, "buffer-auto-save-file-name")?);
    Ok(stringp(cx, file) && stringp(cx, auto) && cx.string_bytes(file)? == cx.string_bytes(auto)?)
}

pub(super) fn returned(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(value))
}

/// GNU's `handler = Ffind_file_name_handler (NAME, OPERATION); if
/// (!NILP (handler)) return calln (handler, OPERATION, ARGS...)': the
/// call when a handler is selected, whose value the primitive returns.
pub(super) fn handler(cx: &mut Leaf<'_>, name: Value, operation: &str, args: Vec<Value>)
    -> std::result::Result<Option<Next>, Fault> {
    let operation = cx.intern(operation);
    let f = find_file_name_handler(cx, name, operation)?;
    if f == Value::NIL { return Ok(None); }
    let args = std::iter::once(operation).chain(args).collect();
    Ok(Some(Ok(Step::Call { f, args, then: returned, saved: vec![] })))
}

/// GNU `file_offset' (fileio.c:3949): a non-negative integer, or a float
/// with an exact non-negative integral value.
fn file_offset(cx: &Leaf<'_>, v: Value) -> std::result::Result<u64, Fault> {
    if let Some(n) = v.integer_value() {
        if n >= 0 {
            return Ok(n as u64);
        }
    } else if cx.kind(v) == Kind::Float {
        let f = cx.float_value(v)?;
        if (0.0..9.223_372_036_854_776e18).contains(&f) && f.fract() == 0.0 {
            return Ok(f as u64);
        }
    }
    Err(Fault::Wrong("file-offset", v))
}

/// GNU Finsert_file_contents (fileio.c:4057) up to the coding decision:
/// the read-only and visit checks, the handler dispatch, the open (a
/// visited missing file goes on to notfound with no bytes), the
/// regular-file refusal, the read of BEG..END and the lock
/// checks. Every refusal precedes the first change to the buffer. GNU
/// decides the coding system before reading when the buffer has text and
/// after reading otherwise; insert_coding.rs follows both orders, so this
/// reads first in either case and records where its
/// `set-auto-coding-function' tail sample would start.
/// REPLACE of a non-empty accessible region is insert_coding.rs's.
defsubr! { name = "insert-file-contents", gnu = "fileio.c:4057", effect = Global,
fn insert_file_contents(cx, filename: Value; optional visit: Value, beg: Value, end: Value,
                        replace: Value) -> Step {
    // fileio.c:4143: VM2 has no indirect buffers.
    super::buffer_state::barf_if_read_only(cx, Value::NIL)?;
    let shape = cx.buffer_shape()?;
    // fileio.c:4126 empty_undo_list_p.
    let empty_undo = visit != Value::NIL && var(cx, "buffer-undo-list")? == Value::NIL && shape.empty;
    let empty_undo = if empty_undo { Value::TRUE } else { Value::NIL };
    // fileio.c:4153-4155: CHECK_STRING inside Fexpand_file_name.
    if !stringp(cx, filename) { return Err(Fault::Wrong("stringp", filename)); }
    expanding(cx, filename, insert_expanded, vec![visit, beg, end, replace, empty_undo])
}}

/// fileio.c:4160-4167: a handler for the expanded ORIG inserts, and GNU's
/// tail from `handled:' follows (insert_coding.rs `handled').
fn insert_expanded(cx: &mut Leaf<'_>, orig: Value, saved: &[Value]) -> Next {
    let [visit, beg, end, replace, empty_undo] = saved[..5] else { unreachable!() };
    let operation = cx.intern("insert-file-contents");
    let f = find_file_name_handler(cx, orig, operation)?;
    if f != Value::NIL {
        let args = vec![operation, orig, visit, beg, end, replace];
        return Ok(Step::Call { f, args, then: insert_coding::handled, saved: vec![orig, visit, beg, end, replace, empty_undo, f] });
    }
    let path = file_name_path(cx, orig)?;
    let shape = cx.buffer_shape()?;
    let empty_undo = empty_undo != Value::NIL;
    let visiting = visit != Value::NIL;
    if visiting {
        if beg != Value::NIL || end != Value::NIL {
            return Err(Fault::Error("Attempt to visit less than an entire file".into()));
        }
        if !shape.empty && replace == Value::NIL {
            return Err(Fault::Error("Cannot do file visiting in a non-empty buffer".into()));
        }
    }
    let beg_offset = if beg != Value::NIL { file_offset(cx, beg)? } else { 0 };
    let end_offset = if end != Value::NIL { file_offset(cx, end)? } else { u64::MAX }.max(beg_offset);
    let read = |bytes, modtime, errno, tail| insert_coding::Read {
        orig, args: [visit, beg, end, replace], bytes, modtime, errno, empty_undo,
        had_text: !shape.empty, modifying: !visiting && end_offset > beg_offset, tail,
    };
    let file = match open_for_reading(&path) {
        Ok(file) => file,
        Err(error) => {
            if !visiting {
                return Err(file_errno(cx, "Opening input file", orig, &error));
            }
            if replace != Value::NIL && !shape.empty {
                return Err(refuse("insert-file-contents REPLACE"));
            }
            // notfound (fileio.c:5016): visiting completes, then the error.
            let modtime = if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) {
                VisitedModtime::Missing
            } else { VisitedModtime::Unknown };
            return insert_coding::start(cx, read(None, modtime, error.raw_os_error().unwrap_or(libc::EIO), None));
        }
    };
    let meta = file.stat().map_err(|e| file_errno(cx, "Input file status", orig, &e))?;
    if meta.file_type().is_dir() {
        let error = std::io::Error::from_raw_os_error(21); // EISDIR, fileio.c:4234
        return Err(file_errno(cx, "Read error", orig, &error));
    }
    if !meta.file_type().is_file() {
        return Err(refuse("insert-file-contents of a non-regular file"));
    }
    let inhibit = var(cx, "inhibit-read-only")?; if !visiting && end_offset > beg_offset && replace == Value::NIL { cx.fileio_check_insertion(inhibit)?; } // REPLACE checks the ranges it changes.
    // BEG..END through the boundary, which sees the bytes consumed; a seek
    // or read error discards what was read.
    let bytes = {
        let mut file = file;
        file.read_range(beg_offset, end_offset - beg_offset)
            .map_err(|e| file_errno(cx, "Read error", orig, &e))?
    };
    // fileio.c:4344-4373: when the first 4 KiB read stops short of END,
    // set-auto-coding-function sees 1 KiB of head and the tail from 3 KiB
    // before the end of the file (or of END), never overlapping that read;
    // a file ending at exactly 4 KiB gives an empty tail.
    let size = meta.len();
    let tail = (bytes.len() >= 4 * 1024 && beg_offset + 4 * 1024 < end_offset).then(|| {
        if size.saturating_sub(3 * 1024) <= beg_offset + 4 * 1024 { 4 * 1024 }
        else { size.saturating_sub(3 * 1024).min(end_offset.saturating_sub(3 * 1024)).saturating_sub(beg_offset) }
    });
    insert_coding::start(cx, read(Some(bytes), metadata_modtime(&meta), 0, tail.map(|t| t as usize)))
}

/// Open PATH for `insert-file-contents' with O_NONBLOCK, so a FIFO without a
/// writer (or a device) opens at once and reaches the non-regular refusal
/// instead of blocking in open(2); the flag does not affect the regular files
/// that are read.
fn open_for_reading(path: &std::path::Path) -> std::io::Result<crate::host::Input> {
    let flags = libc::O_NONBLOCK;
    crate::host::open_input(path, flags)
}

// File name handler selection, shared by every file primitive: GNU's,
// including `inhibit-file-name-operation' and each handler's `operations'
// property. A selected handler is called (`handler').

// GNU Fmemq validates a dotted tail only when search reaches it.
fn handler_member(cx: &Leaf<'_>, value: Value, list: Value) -> std::result::Result<bool, Fault> {
    let mut tail = list;
    let mut tortoise = list;
    let mut steps = 0u64;
    while let Some((head, rest)) = cx.pair(tail) {
        if cx.eq(head, value) { return Ok(true); }
        tail = rest;
        if tail == tortoise { return Err(Fault::Circular(tail)); }
        steps += 1;
        if steps & 4095 == 0 && steps.is_power_of_two() { tortoise = tail; }
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", list)); }
    Ok(false)
}

/// GNU fileio.c:370: greatest match position wins, first entry wins ties.
/// Matching does not change match data or honor case-fold-search.
pub(super) fn find_file_name_handler(cx: &mut Leaf<'_>, filename: Value, operation: Value) -> Out {
    if !stringp(cx, filename) { return Err(Fault::Wrong("stringp", filename)); }
    let operations_key = cx.intern("operations");
    let inhibited = if var(cx, "inhibit-file-name-operation")? == operation {
        var(cx, "inhibit-file-name-handlers")?
    } else { Value::NIL };
    let mut chain = var(cx, "file-name-handler-alist")?;
    let mut selected = Value::NIL;
    let mut position = None;
    let mut tortoise = chain;
    let mut steps = 0u64;
    while let Some((entry, tail)) = cx.pair(chain) {
        chain = tail;
        if chain == tortoise { return Err(refuse("cyclic file-name-handler-alist")); }
        steps += 1;
        if steps & 4095 == 0 && steps.is_power_of_two() { tortoise = chain; }
        let Some((pattern, handler)) = cx.pair(entry) else { continue };
        let mut operations = Value::NIL;
        if handler == Value::NIL || handler == Value::TRUE || cx.kind(handler) == Kind::Symbol {
            let mut plist = cx.symbol_plist(handler)?;
            for _ in 0..=cx.list_limit() {
                let Some((key, rest)) = cx.pair(plist) else { break };
                let Some((value, rest)) = cx.pair(rest) else { break };
                if key == operations_key { operations = value; break; }
                plist = rest;
            }
        }
        if !stringp(cx, pattern) { continue; }
        let Some(at) = cx.file_name_match(pattern, filename)? else { continue };
        if position.is_some_and(|old| at <= old) { continue; }
        if operations != Value::NIL && !handler_member(cx, operation, operations)? { continue; }
        if handler_member(cx, handler, inhibited)? { continue; }
        selected = handler;
        position = Some(at);
    }
    Ok(selected)
}

defsubr! { name = "find-file-name-handler", gnu = "fileio.c:370", effect = Global,
fn find_file_name_handler_primitive(cx, filename: Value, operation: Value) {
    find_file_name_handler(cx, filename, operation)
}}

/// GNU `fix_position' (editfns.c) for a region bound.
fn position(cx: &Leaf<'_>, v: Value) -> std::result::Result<i64, Fault> {
    if let Some(n) = v.integer_value() {
        return Ok(n);
    }
    match cx.marker_position(v) {
        Some(Some(n)) => Ok(n),
        Some(None) => Err(Fault::Error("Marker does not point anywhere".into())),
        None => Err(Fault::Wrong("integer-or-marker-p", v)),
    }
}
defsubr! { name = "write-region", gnu = "fileio.c:5461", effect = Global,
fn write_region(cx, start: Value, end: Value, filename: Value; optional append: Value,
                visit: Value, lockname: Value, mustbenew: Value) -> Step {
    // fileio.c:5539-5540 validate_region (editfns.c:2504): START and END
    // become ordered positions, once in range, before the name is expanded
    // or a handler sees them.
    let shape = cx.buffer_shape()?;
    let (start, end) = if start != Value::NIL && !stringp(cx, start) {
        let (b, e) = (position(cx, start)?, position(cx, end)?);
        let (b, e) = (Value::integer(b.min(e)).map_err(Fault::Error)?, Value::integer(b.max(e)).map_err(Fault::Error)?);
        if !(shape.begv <= b.integer_value().unwrap() && e.integer_value().unwrap() <= shape.zv) {
            let buffer = cx.current_buffer()?;
            return Err(Fault::Range3(buffer, start, end));
        }
        (b, e)
    } else { (start, end) };
    expanding(cx, filename, write_expanded, vec![start, end, append, visit, lockname, mustbenew])
}}

// write-region's SAVED: START END APPEND VISIT LOCKNAME MUSTBENEW, then the
// expanded FILENAME.
fn write_expanded(cx: &mut Leaf<'_>, name: Value, saved: &[Value]) -> Next {
    let [_, _, _, visit, _, mustbenew] = saved[..6] else { unreachable!() };
    let path = file_name_path(cx, name)?;
    let excl = symbol_is(cx, mustbenew, &["excl"]);
    if mustbenew != Value::NIL && !excl && crate::host::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_dir()) {
        // barf_or_query_if_file_exists checks directory before confirmation.
        let message = cx.string("File is a directory");
        let tail = cx.cons(name, Value::NIL);
        let data = cx.cons(message, tail);
        return Err(Fault::Signal(cx.intern("file-error"), data));
    }
    if mustbenew != Value::NIL && !excl && crate::host::symlink_metadata(&path).is_ok() {
        // barf_or_query_if_file_exists (fileio.c:5547) asks the user.
        return Err(refuse("write-region MUSTBENEW confirmation"));
    }
    let mut saved = saved.to_vec();
    saved.push(name);
    if stringp(cx, visit) { return expanding(cx, visit, write_visit_expanded, saved); }
    write_visit_expanded(cx, name, &saved)
}

/// fileio.c:5550-5577 with VISIT-FILE expanded: the handler for FILENAME,
/// else for the original VISIT, writes; a visit is then recorded.
fn write_visit_expanded(cx: &mut Leaf<'_>, visit_file: Value, saved: &[Value]) -> Next {
    let [start, end, append, visit, lockname, mustbenew, name] = saved[..7] else { unreachable!() };
    let lock = if lockname != Value::NIL { lockname } else { visit_file };
    let operation = cx.intern("write-region");
    let mut f = find_file_name_handler(cx, name, operation)?;
    if f == Value::NIL && stringp(cx, visit) { f = find_file_name_handler(cx, visit, operation)?; }
    if f != Value::NIL {
        let args = vec![operation, start, end, name, append, visit, lock, mustbenew];
        return Ok(Step::Call { f, args, then: write_handled, saved: vec![visit, visit_file] });
    }
    // A nil START widens and means BEGV..ZV; else the validated region.
    let shape = cx.buffer_shape()?;
    let region = if start != Value::NIL && !stringp(cx, start) {
        let (b, e) = (position(cx, start)?, position(cx, end)?);
        Some((b.min(e), b.max(e)))
    } else { None };
    // build_annotations (fileio.c:5604) and the post-annotation call (5811);
    // an auto-save uses `buffer-auto-save-file-format' unless it is t.
    let auto_format = var(cx, "buffer-auto-save-file-format")?;
    let format = if auto_saving(cx)?.is_some() && auto_format != Value::TRUE { auto_format } else { var(cx, "buffer-file-format")? };
    if var(cx, "write-region-annotate-functions")? != Value::NIL
        || var(cx, "write-region-post-annotation-function")? != Value::NIL
        || (!stringp(cx, start) && format != Value::NIL) {
        return Err(refuse("write-region annotations"));
    }
    // choose_write_coding_system's CODING_MODE_SELECTIVE_DISPLAY (fileio.c:5455).
    if !stringp(cx, start) && var(cx, "selective-display")? == Value::TRUE {
        return Err(refuse("write-region with selective-display t"));
    }
    // A nil START widens (fileio.c:5593) and then means BEGV..ZV.
    let (b, e) = match region { Some(r) => r, None => (1, shape.z) };
    let fixnum = |n: i64| Value::integer(n).map_err(Fault::Error);
    let widen = start == Value::NIL;
    let (start, end) = if stringp(cx, start) { (start, end) } else { (fixnum(b)?, fixnum(e)?) };
    // fileio.c:5583 save_restriction_save holds from here through the coding
    // choice and the write, across its Lisp calls and non-local exits. The
    // arguments reach the body live, not quoted: `eval' copies constants.
    let source = format!("(lambda (romacs--s romacs--e romacs--n romacs--a romacs--v romacs--l romacs--m romacs--f)
                            (save-restriction {}
                              (romacs--write-region-body romacs--s romacs--e romacs--n romacs--a
                                                         romacs--v romacs--l romacs--m romacs--f)))",
                         if widen { "(widen)" } else { "" });
    let args = vec![start, end, name, append, visit, lock, mustbenew, visit_file];
    insert_coding::closure_then(cx, &source, write_closure, args)
}

/// fileio.c:5567-5574: a handler wrote; a visit (SAVED: VISIT VISIT-FILE)
/// is recorded, and the handler's value returned.
fn write_handled(cx: &mut Leaf<'_>, val: Value, saved: &[Value]) -> Next {
    let [visit, visit_file] = saved[..2] else { unreachable!() };
    if visit == Value::TRUE || stringp(cx, visit) {
        cx.set_buffer_modified(Value::NIL, true)?;
        cx.bset("buffer-file-name", visit_file)?;
    }
    Ok(Step::Done(val))
}
fn write_closure(_: &mut Leaf<'_>, closure: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: closure, args: saved.to_vec(), then: written, saved: vec![] })
}
fn written(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next { Ok(Step::Done(value)) }

// Internal: write-region's tail inside its restriction extent, from the
// coding choice on. Its arguments are write-region's, resolved as above.
defsubr! { name = "romacs--write-region-body", gnu = "fileio.c:5617", effect = Global,
fn write_region_body(cx, start: Value, end: Value, filename: Value, append: Value, visit: Value,
                     lockname: Value, mustbenew: Value, visit_file: Value) -> Step {
    use super::write_coding as w;
    if !stringp(cx, start) {
        for v in [start, end] { if v.integer_value().is_none() { return Err(Fault::Wrong("fixnump", v)); } }
    }
    let mut r = vec![Value::NIL; w::REGISTERS];
    r[..w::VAL].copy_from_slice(&[start, end, filename, append, visit, lockname, mustbenew, visit_file]);
    r[w::FLAGS] = Value::integer(0).unwrap();
    w::choose(cx, r)
}}

/// e_write encodes through encode_coding_object (coding.c:8336), which runs
/// CODING's pre-write conversion and translation tables even on the ASCII
/// text `encode-coding-string' passes through unchanged. Neither is
/// implemented: refuse before the file is opened.
fn refuse_write_conversion(cx: &mut Leaf<'_>, coding: Value) -> std::result::Result<(), Fault> {
    let spec = cx.coding_spec(coding)?;
    let attrs = cx.vector_get(spec, Value::integer(0).unwrap())?;
    if cx.vector_get(attrs, Value::integer(10).unwrap())? != Value::NIL {
        return Err(refuse("write-region with a pre-write-conversion coding system"));
    }
    if !cx.coding_encode_translation(attrs)?.is_empty() {
        return Err(refuse("write-region with an encoding translation table"));
    }
    Ok(())
}

/// The rest of write_region once the coding system R[VAL] is chosen
/// (fileio.c:5623 onwards): encode, open, write, sync and visit.
pub(super) fn write_chosen(cx: &mut Leaf<'_>, r: &[Value]) -> Next {
    use super::write_coding as w;
    let (start, end, append, coding) = (r[w::START], r[w::END], r[w::APPEND], r[w::VAL]);
    let lcsu = cx.intern("last-coding-system-used");
    cx.variable_set(lcsu, coding)?;
    let text = if stringp(cx, start) { start } else {
        // A Lisp call during the choice may have edited the buffer.
        let (b, e) = (start.integer_value().unwrap(), end.integer_value().unwrap());
        let shape = cx.buffer_shape()?;
        if !(1 <= b && b <= e && e <= shape.z) { return Err(Fault::Range(start, end)); }
        let bytes = cx.buffer_text_bytes(b, e)?;
        cx.make_string(bytes, !shape.unibyte)
    };
    refuse_write_conversion(cx, coding)?;
    let encoded = cx.coding_convert_string(text, coding, Value::TRUE, Value::NIL, true)?;
    cx.variable_set(lcsu, coding)?;
    // NUMBERP (append): an offset to seek to (fileio.c:5638).
    if append.integer_value().is_some() || cx.kind(append) == Kind::Float { file_offset(cx, append)?; }
    // fileio.c:5629: Flock_file (LOCKNAME) before the file is opened, but
    // for an auto-save.
    let mut saved = r.to_vec();
    saved.push(encoded);
    if auto_saving(cx)?.is_some() { return write_locked(cx, Value::NIL, &saved); }
    Ok(Step::Call { f: cx.intern("lock-file"), args: vec![r[w::LOCKNAME]], then: write_locked, saved })
}

/// The open, write, sync and close, then Funlock_file (LOCKNAME), which
/// GNU calls after the write and before reporting its error (fileio.c:5652,
/// 5667, 5821). SAVED: the registers, then the encoded text.
fn write_locked(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    use super::write_coding as w;
    let (r, encoded) = (&saved[..w::REGISTERS], saved[w::REGISTERS]);
    let mut saved = saved.to_vec();
    match write_file(cx, r, encoded) {
        Ok(modtime) => saved.extend([Value::NIL, modtime]),
        Err(Fault::Signal(condition, data)) => saved.extend([condition, data]),
        Err(fault) => return Err(fault),
    }
    if auto_saving(cx)?.is_some() { return write_unlocked(cx, Value::NIL, &saved); }
    Ok(Step::Call { f: cx.intern("unlock-file"), args: vec![r[w::LOCKNAME]], then: write_unlocked, saved })
}

/// After the unlock: the write's error, else the visit's bookkeeping.
/// SAVED: the registers, the encoded text, then the error's condition (nil
/// when none) and its data, or the visited file's modtime.
fn write_unlocked(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    use super::write_coding as w;
    let (condition, data) = (saved[w::REGISTERS + 1], saved[w::REGISTERS + 2]);
    if condition != Value::NIL { return Err(Fault::Signal(condition, data)); }
    let (visit, visit_file) = (saved[w::VISIT], saved[w::VISIT_FILE]);
    // fileio.c:5835-5857: an auto-save into the visited file unlocks it.
    let into_visited = auto_saving(cx)?.is_some() && auto_saving_into_visited(cx)?;
    if visit == Value::TRUE || stringp(cx, visit) {
        let modtime = insert_coding::modtime_of(cx, data);
        cx.fileio_set_modtime(modtime)?;
        cx.set_buffer_modified(Value::NIL, true)?;
        cx.bset("buffer-file-name", visit_file)?;
    } else if visit != Value::NIL && into_visited {
        cx.set_buffer_modified(Value::NIL, true)?;
    }
    if visit != Value::NIL && into_visited {
        return Ok(Step::Call { f: cx.intern("unlock-file"), args: vec![saved[w::LOCKNAME]], then: written_nil, saved: vec![] });
    }
    report_written(cx, saved[w::APPEND], visit, visit_file)
}

/// fileio.c:5859: an interactive write that is neither quiet (a VISIT
/// other than nil, t or a string) nor an auto-save names the file it
/// wrote, as `message' does.
fn report_written(cx: &mut Leaf<'_>, append: Value, visit: Value, visit_file: Value) -> Next {
    let quietly = visit != Value::NIL && visit != Value::TRUE && !stringp(cx, visit);
    if quietly || auto_saving(cx)?.is_some() || var(cx, "noninteractive")? != Value::NIL {
        return Ok(Step::Done(Value::NIL));
    }
    let format = if append.integer_value().is_some() || cx.kind(append) == Kind::Float { "Updated %s" }
        else if append != Value::NIL { "Added to %s" } else { "Wrote %s" };
    let format = cx.string(format);
    Ok(Step::Call { f: cx.intern("message"), args: vec![format, visit_file], then: written_nil, saved: vec![] })
}

fn written_nil(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next { Ok(Step::Done(Value::NIL)) }

/// Write the ENCODED text to the registers' file: its modtime as register
/// data when visiting, else nil.
fn write_file(cx: &mut Leaf<'_>, r: &[Value], encoded: Value) -> Out {
    use super::write_coding as w;
    let (name, append, visit, mustbenew) = (r[w::FILENAME], r[w::APPEND], r[w::VISIT], r[w::MUSTBENEW]);
    let visiting = visit == Value::TRUE || stringp(cx, visit);
    let path = file_name_path(cx, name)?;
    let bytes = cx.string_view(encoded)?.bytes().to_vec();
    let excl = symbol_is(cx, mustbenew, &["excl"]);
    let mut options = crate::host::OpenOptions::new();
    options.write(true);
    let offset = if append.integer_value().is_some() || cx.kind(append) == Kind::Float {
        Some(file_offset(cx, append)?)
    } else {
        None
    };
    if excl {
        options.create_new(true);
    } else {
        options.create(true);
    }
    options.truncate(append == Value::NIL);
    if append != Value::NIL && offset.is_none() {
        options.append(true);
    }
    let auto_save_bits = auto_saving(cx)?;
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
        // fileio.c:5645: an auto-save file is created with its mode bits.
        if let Some(bits) = auto_save_bits { options.mode(bits); }
    }
    let mut file = crate::host::open_output(&path, &options).map_err(|e| file_errno(cx, "Opening output file", name, &e))?;
    {
        use std::io::{Seek, SeekFrom};
        if let Some(offset) = offset {
            file.seek(SeekFrom::Start(offset)).map_err(|e| file_errno(cx, "Setting file position", name, &e))?;
        }
        cx.fileio_write_output(&mut file, &bytes).map_err(|e| file_errno(cx, "Write error", name, &e))?;
        // fileio.c:5699: inhibit defaults to t. Retry EINTR; EINVAL means
        // this descriptor does not support fsync (e.g. devices and pipes).
        if auto_save_bits.is_none() && var(cx, "write-region-inhibit-fsync")? == Value::NIL {
            loop {
                match cx.fileio_sync_output(&file) {
                    Ok(()) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) if e.raw_os_error() == Some(libc::EINVAL) => break,
                    Err(e) => return Err(file_errno(cx, "Write error", name, &e)),
                }
            }
        }
    }
    let written_modtime = if visiting {
        Some(metadata_modtime(&crate::host::file_metadata(&file).map_err(|e| file_errno(cx, "Output file status", name, &e))?))
    } else { None };
    cx.fileio_close_output(file).map_err(|e| file_errno(cx, "Write error", name, &e))?;
    match written_modtime {
        Some(modtime) => insert_coding::modtime_value(cx, modtime),
        None => Ok(Value::NIL),
    }
}

// GNU defines `locate-file' in Lisp (lisp/files.el:1113) and only
// `locate-file-internal' in C (src/lread.c:1590), so only the C entry point
// belongs in the native substrate: GNU's own Lisp must supply the wrapper once
// files.el loads. A native `locate-file' is a stand-in for GNU Lisp.
defsubr! { name = "locate-file-internal", gnu = "lread.c:1590", effect = Global,
fn locate_file_internal(cx, filename: Value, path: Value; optional suffixes: Value, predicate: Value) -> Step {
    cx.string_view(filename)?;
    // openp: nil or t opens the file for reading; a natural number is an
    // access(2) mode tested with AT_EACCESS; anything else is a function
    // called on each candidate.
    let mut roots = safe_elements(cx, path);
    let nil_path = if path == Value::NIL { Value::TRUE } else { Value::NIL };
    if path == Value::NIL { roots.push(Value::NIL); }
    let roots = fresh_list(cx, roots);
    let suffixes = if suffixes == Value::NIL {
        let empty = cx.make_string(Vec::new(), cx.string_view(filename)?.is_multibyte());
        cx.cons(empty, Value::NIL)
    } else {
        let suffixes = safe_elements(cx, suffixes);
        fresh_list(cx, suffixes)
    };
    locate_next(cx, &[filename, roots, suffixes, predicate, nil_path])
}}

/// FOR_EACH_TAIL_SAFE: LIST's elements up to a cycle or dotted end.
fn safe_elements(cx: &mut Leaf<'_>, list: Value) -> Vec<Value> {
    let mut elements = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut tail = list;
    while let Some((element, rest)) = cx.pair(tail) {
        if !seen.insert(tail.0) { break; }
        elements.push(element);
        tail = rest;
    }
    elements
}

/// A fresh proper list of ELEMENTS, which the continuations can carry.
fn fresh_list(cx: &mut Leaf<'_>, elements: Vec<Value>) -> Value {
    elements.into_iter().rev().fold(Value::NIL, |list, element| cx.cons(element, list))
}

// Continuations share SAVED = [FILENAME ROOTS SUFFIXES PREDICATE NIL-PATH],
// ROOTS being the elements not yet tried after the current one, and while
// one root's suffixes are tried, [... BASE SUFFIXES-LEFT] after them.

/// openp's next PATH element: NAME itself for a nil PATH, else
/// (expand-file-name NAME ROOT).
fn locate_next(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    let Some((root, rest)) = cx.pair(saved[1]) else { return Ok(Step::Done(Value::NIL)) };
    let mut saved = saved.to_vec();
    saved[1] = rest;
    if saved[4] != Value::NIL { return locate_expanded(cx, saved[0], &saved); }
    super::expand_file_name::expanding(cx, saved[0], root, locate_expanded, saved)
}

/// A result still relative expands against the buffer's directory.
fn locate_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    if cx.string_bytes(expanded)?.first() == Some(&b'/') { return locate_absolute(cx, expanded, saved); }
    let directory = var(cx, "default-directory")?;
    super::expand_file_name::expanding(cx, expanded, directory, locate_absolute, saved.to_vec())
}

/// Try each suffix on an absolute EXPANDED; skip an element left relative.
fn locate_absolute(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    if cx.string_bytes(expanded)?.first() != Some(&b'/') { return locate_next(cx, saved); }
    // openp drops a leading `/:' before trying the suffixes.
    let text = cx.string_text(expanded)?;
    let base = match text.strip_prefix("/:") { Some(rest) if text.len() > 2 => rest.to_owned(), _ => text };
    let base = cx.make_string(base.into_bytes(), true);
    let mut saved = saved[..5].to_vec();
    saved.extend([base, saved[2]]);
    locate_suffix(cx, &saved)
}

/// The next suffix of SAVED's root, or the next root once none is left.
fn locate_suffix(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    let Some((suffix, rest)) = cx.pair(saved[6]) else { return locate_next(cx, &saved[..5]) };
    let mut saved = saved.to_vec();
    saved[6] = rest;
    let (filename, predicate) = (saved[0], saved[3]);
    let multibyte = cx.string_view(filename)?.is_multibyte();
    let candidate = format!("{}{}", cx.string_text(saved[5])?, cx.string_text(suffix)?);
    let candidate_value = cx.make_string(candidate.as_bytes().to_vec(), multibyte || !candidate.is_ascii());
    let access_mode = predicate.integer_value().filter(|&n| n >= 0);
    if predicate != Value::NIL && predicate != Value::TRUE && access_mode.is_none() {
        saved.push(candidate_value);
        return Ok(Step::Call { f: predicate, args: vec![candidate_value], then: locate_predicate_result, saved });
    }
    // lread.c openp selects after suffix concatenation, even for an
    // absolute filename; with a handler, `file-readable-p' decides. A
    // natural-number predicate tests access(2) without handlers.
    if access_mode.is_none() {
        let exists = cx.intern("file-exists-p");
        if find_file_name_handler(cx, candidate_value, exists)? != Value::NIL {
            saved.push(candidate_value);
            return Ok(Step::Call { f: cx.intern("file-readable-p"), args: vec![candidate_value], then: locate_verdict, saved });
        }
    }
    let found = match access_mode {
        None => crate::host::probe(crate::host::Probe::IsFile, &candidate) && open_for_reading(std::path::Path::new(&candidate)).is_ok(),
        Some(mode) => i32::try_from(mode).is_ok_and(|mode| cx.filesave_access(std::ffi::OsStr::new(&candidate), mode).is_ok())
            && !crate::host::probe(crate::host::Probe::IsDir, &candidate),
    };
    if found { return Ok(Step::Done(candidate_value)); }
    locate_suffix(cx, &saved)
}

/// openp's verdict on a function PREDICATE's RESULT for the candidate last
/// in SAVED: nil rejects it, and a directory (`file-directory-p') needs
/// `dir-ok'.
fn locate_predicate_result(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    if result == Value::NIL { return locate_suffix(cx, &saved[..7]); }
    if result == cx.intern("dir-ok") { return Ok(Step::Done(saved[7])); }
    let directoryp = cx.intern("file-directory-p");
    Ok(Step::Call { f: directoryp, args: vec![saved[7]], then: locate_not_directory, saved: saved.to_vec() })
}

fn locate_not_directory(cx: &mut Leaf<'_>, directory: Value, saved: &[Value]) -> Next {
    locate_verdict(cx, if directory == Value::NIL { Value::TRUE } else { Value::NIL }, saved)
}

/// The candidate last in SAVED is found when VERDICT is non-nil.
fn locate_verdict(cx: &mut Leaf<'_>, verdict: Value, saved: &[Value]) -> Next {
    if verdict != Value::NIL { return Ok(Step::Done(saved[7])); }
    locate_suffix(cx, &saved[..7])
}

// Romacs has no toolkit file dialog, so it answers as a GNU built without
// one (no GTK, Motif, NS, NTGUI or Haiku) does.
defsubr! { name = "next-read-file-uses-dialog-p", gnu = "fileio.c:6556", effect = Pure,
fn next_read_file_uses_dialog_p(cx) {
    let _ = cx;
    Ok(Value::NIL)
}}
