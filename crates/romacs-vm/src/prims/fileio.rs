//! GNU src/fileio.c `insert-file-contents' (fileio.c:4057) and `write-region'
//! (fileio.c:5461): read a file into the current buffer, write buffer text or
//! a string to a file. Both are T2: GNU calls Lisp while choosing the coding
//! system and, for `insert-file-contents', after inserting
//! (`after-insert-file-set-coding', `format-decode',
//! `after-insert-file-functions'). Here are the checks, the read and the
//! write; prims/insert_coding.rs decides, decodes and inserts, and
//! prims/write_coding.rs chooses the coding system written. Both refuse file
//! name handlers; `write-region' refuses the annotation functions.
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

fn stat_modtime(path: &str) -> VisitedModtime {
    match std::fs::metadata(path) {
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
fn verify_visited_file_modtime(cx; optional buffer: Value) {
    let (state, path) = cx.fileio_modtime(buffer)?;
    let Some(path) = path else { return Ok(Value::TRUE) };
    if state == VisitedModtime::Unknown { return Ok(Value::TRUE) }
    let name = cx.string(&path);
    refuse_handler(cx, name, "verify-visited-file-modtime")?;
    let actual = stat_modtime(&path);
    let equal = match (state, actual) {
        (VisitedModtime::Known { seconds: s, nanos: n, size }, VisitedModtime::Known { seconds, nanos, size: actual_size }) =>
            s == seconds && n == nanos && (size.is_none() || size == actual_size),
        _ => state == actual,
    };
    Ok(if equal { Value::TRUE } else { Value::NIL })
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
fn set_visited_file_modtime(cx; optional time: Value) {
    let state = if time != Value::NIL { explicit_modtime(cx, time)? } else {
        let (_, filename) = cx.fileio_modtime(Value::NIL)?;
        let Some(filename) = filename else { return Err(Fault::Wrong("stringp", Value::NIL)) };
        let name = cx.string(&filename);
        let (path, name) = expanded_name(cx, name)?;
        refuse_handler(cx, name, "set-visited-file-modtime")?;
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) if matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) => return Ok(Value::NIL),
            Err(e) => return Err(file_errno(cx, "Getting attributes", name, &e)),
        };
        metadata_modtime(&meta)
    };
    cx.fileio_set_modtime(state)?;
    Ok(Value::NIL)
}}

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

/// GNU `Fexpand_file_name (NAME, Qnil)' (prims/expand_file_name.rs) as
/// text, refusing where GNU would call a file name handler: callers that
/// cannot continue through Lisp use this.
pub(super) fn expand_file_name(cx: &mut Leaf<'_>, name: Value) -> std::result::Result<String, Fault> {
    Ok(expanded_name(cx, name)?.0)
}

/// `Fexpand_file_name (NAME, Qnil)' as text and as the Lisp string GNU
/// returns: NAME itself when the expansion leaves it unchanged.
fn expanded_name(cx: &mut Leaf<'_>, name: Value) -> std::result::Result<(String, Value), Fault> {
    if !stringp(cx, name) {
        return Err(Fault::Wrong("stringp", name));
    }
    let value = super::expand_file_name::expand(cx, name, Value::NIL)?
        .ok_or_else(|| refuse("expand-file-name matching file name handler"))?;
    Ok((cx.string_text(value)?, value))
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

/// Select with GNU's regexp, operations and inhibition rules. A leaf may
/// proceed locally only when no callback is selected for this name/operation.
pub(super) fn refuse_handler(cx: &mut Leaf<'_>, filename: Value, operation: &str) -> std::result::Result<(), Fault> {
    let symbol = cx.intern(operation);
    if find_file_name_handler(cx, filename, symbol)? != Value::NIL {
        return Err(refuse(&format!("{operation} matching file name handler")));
    }
    Ok(())
}

/// GNU Finsert_file_contents (fileio.c:4057) up to the coding decision:
/// the read-only and visit checks, the handler refusal, the open (a
/// visited missing file goes on to notfound with no bytes), the
/// regular-file refusal, the read of BEG..END and the lock
/// checks. Every refusal precedes the first change to the buffer. GNU
/// decides the coding system before reading when the buffer has text and
/// after reading otherwise; insert_coding.rs follows both orders, so this
/// reads first in either case and records where its
/// `set-auto-coding-function' tail sample would start.
///
/// REPLACE stays refused: none of the daily-driver consumers pass it, and
/// GNU's replacement keeps unchanged head and tail text and their markers,
/// which needs its own transcription.
defsubr! { name = "insert-file-contents", gnu = "fileio.c:4057", effect = Global,
fn insert_file_contents(cx, filename: Value; optional visit: Value, beg: Value, end: Value,
                        replace: Value) -> Step {
    // fileio.c:4143: VM2 has no indirect buffers.
    super::buffer_state::barf_if_read_only(cx, Value::NIL)?;
    let shape = cx.buffer_shape()?;
    // fileio.c:4126 empty_undo_list_p.
    let empty_undo = visit != Value::NIL && var(cx, "buffer-undo-list")? == Value::NIL && shape.empty;
    // fileio.c:4153-4155: CHECK_STRING inside Fexpand_file_name.
    let (path, orig) = expanded_name(cx, filename)?;
    refuse_handler(cx, orig, "insert-file-contents")?;
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
            let truename = var(cx, "buffer-file-truename")?;
            if truename != Value::NIL { refuse_handler(cx, truename, "unlock-file")?; }
            refuse_handler(cx, orig, "unlock-file")?;
            let modtime = if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) {
                VisitedModtime::Missing
            } else { VisitedModtime::Unknown };
            return insert_coding::start(cx, read(None, modtime, error.raw_os_error().unwrap_or(libc::EIO), None));
        }
    };
    let meta = file.metadata().map_err(|e| file_errno(cx, "Input file status", orig, &e))?;
    if meta.is_dir() {
        let error = std::io::Error::from_raw_os_error(21); // EISDIR, fileio.c:4234
        return Err(file_errno(cx, "Read error", orig, &error));
    }
    if !meta.is_file() {
        return Err(refuse("insert-file-contents of a non-regular file"));
    }
    if replace != Value::NIL {
        return Err(refuse("insert-file-contents REPLACE"));
    }
    let inhibit = var(cx, "inhibit-read-only")?; if !visiting && end_offset > beg_offset { cx.fileio_check_insertion(inhibit)?; }
    let mut bytes = vec![];
    {
        use std::io::{Read as _, Seek, SeekFrom};
        let mut file = file;
        file.seek(SeekFrom::Start(beg_offset)).map_err(|e| file_errno(cx, "Read error", orig, &e))?;
        file.take(end_offset - beg_offset).read_to_end(&mut bytes).map_err(|e| file_errno(cx, "Read error", orig, &e))?;
    }
    // Every refusal below precedes the first change to the buffer.
    let truename = var(cx, "buffer-file-truename")?;
    if !bytes.is_empty() && truename != Value::NIL
        && var(cx, "buffer-file-name")? != Value::NIL
        && { let state = cx.buffer_modification(Value::NIL)?; state.save_modiff >= state.modiff }
        && var(cx, "inhibit-modification-hooks")? == Value::NIL {
        refuse_handler(cx, truename, "lock-file")?;
    }
    if visiting {
        if truename != Value::NIL { refuse_handler(cx, truename, "unlock-file")?; }
        refuse_handler(cx, orig, "unlock-file")?;
    }
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
}}

/// Open PATH for `insert-file-contents' with O_NONBLOCK, so a FIFO without a
/// writer (or a device) opens at once and reaches the non-regular refusal
/// instead of blocking in open(2); the flag does not affect the regular files
/// that are read.
fn open_for_reading(path: &str) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)
}

// File name handler selection, shared by every primitive here: VM2 calls
// no handler, so each operation first asks whether GNU would dispatch to
// one and refuses if so (`refuse_handler'). The selection itself is GNU's,
// including `inhibit-file-name-operation' and each handler's `operations'
// property.

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
    // fileio.c:5539-5540 validate_region (editfns.c:2504).
    let shape = cx.buffer_shape()?;
    let mut region = None;
    if start != Value::NIL && !stringp(cx, start) {
        let (b, e) = (position(cx, start)?, position(cx, end)?);
        let (b, e) = (b.min(e), b.max(e));
        if !(shape.begv <= b && e <= shape.zv) {
            let buffer = cx.current_buffer()?;
            return Err(Fault::Range3(buffer, start, end));
        }
        region = Some((b, e));
    }
    let (path, name) = expanded_name(cx, filename)?;
    let excl = symbol_is(cx, mustbenew, &["excl"]);
    if mustbenew != Value::NIL && !excl && std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
        // barf_or_query_if_file_exists checks directory before confirmation.
        let message = cx.string("File is a directory");
        let tail = cx.cons(name, Value::NIL);
        let data = cx.cons(message, tail);
        return Err(Fault::Signal(cx.intern("file-error"), data));
    }
    if mustbenew != Value::NIL && !excl && std::fs::symlink_metadata(&path).is_ok() {
        // barf_or_query_if_file_exists (fileio.c:5547) asks the user.
        return Err(refuse("write-region MUSTBENEW confirmation"));
    }
    let visit_file = if stringp(cx, visit) { expanded_name(cx, visit)?.1 } else { name };
    refuse_handler(cx, name, "write-region")?;
    // GNU fileio.c:5564 tests the original VISIT, not its expanded name.
    if stringp(cx, visit) { refuse_handler(cx, visit, "write-region")?; }
    // GNU invokes lock/unlock handlers even when create-lockfiles is nil.
    let lock = if lockname != Value::NIL { lockname } else { visit_file };
    refuse_handler(cx, lock, "lock-file")?;
    if !(visit == Value::TRUE || stringp(cx, visit)) { refuse_handler(cx, lock, "unlock-file")?; }
    // build_annotations (fileio.c:5604) and the post-annotation call (5811).
    if var(cx, "write-region-annotate-functions")? != Value::NIL
        || var(cx, "write-region-post-annotation-function")? != Value::NIL
        || (!stringp(cx, start) && var(cx, "buffer-file-format")? != Value::NIL) {
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
}}
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
    let (start, end, name, append, visit) = (r[w::START], r[w::END], r[w::FILENAME], r[w::APPEND], r[w::VISIT]);
    let (lockname, mustbenew, visit_file, coding) = (r[w::LOCKNAME], r[w::MUSTBENEW], r[w::VISIT_FILE], r[w::VAL]);
    let visiting = visit == Value::TRUE || stringp(cx, visit);
    let lcsu = cx.intern("last-coding-system-used");
    cx.variable_set(lcsu, coding)?;
    // fileio.c:5629 Flock_file: VM2 creates no lock files (VM2-FILEIO1),
    // but the name is still checked as Flock_file's expand-file-name does.
    expand_file_name(cx, lockname)?;
    let (path, _) = expanded_name(cx, name)?;
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
    let bytes = cx.string_view(encoded)?.bytes().to_vec();
    cx.variable_set(lcsu, coding)?;
    let excl = symbol_is(cx, mustbenew, &["excl"]);
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    // NUMBERP (append): an offset to seek to (fileio.c:5638).
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
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options.open(&path).map_err(|e| file_errno(cx, "Opening output file", name, &e))?;
    {
        use std::io::{Seek, SeekFrom};
        if let Some(offset) = offset {
            file.seek(SeekFrom::Start(offset)).map_err(|e| file_errno(cx, "Setting file position", name, &e))?;
        }
        cx.fileio_write_output(&mut file, &bytes).map_err(|e| file_errno(cx, "Write error", name, &e))?;
        // fileio.c:5699: inhibit defaults to t. Retry EINTR; EINVAL means
        // this descriptor does not support fsync (e.g. devices and pipes).
        if var(cx, "write-region-inhibit-fsync")? == Value::NIL {
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
        Some(metadata_modtime(&file.metadata().map_err(|e| file_errno(cx, "Output file status", name, &e))?))
    } else { None };
    cx.fileio_close_output(file).map_err(|e| file_errno(cx, "Write error", name, &e))?;
    if visiting {
        cx.bset("buffer-file-name", visit_file)?;
        cx.fileio_set_modtime(written_modtime.unwrap())?;
        cx.set_buffer_modified(Value::NIL, true)?;
    }
    Ok(Step::Done(Value::NIL))
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
    // absolute filename. A selected handler can run Lisp: still refuse.
    // A natural-number predicate tests access(2) without handlers.
    if access_mode.is_none() {
        refuse_locate_handler(cx, candidate_value, "file-exists-p")?;
    }
    let found = match access_mode {
        None => std::path::Path::new(&candidate).is_file() && open_for_reading(&candidate).is_ok(),
        Some(mode) => i32::try_from(mode).is_ok_and(|mode| cx.filesave_access(&candidate, mode).is_ok())
            && !std::path::Path::new(&candidate).is_dir(),
    };
    if found { return Ok(Step::Done(candidate_value)); }
    locate_suffix(cx, &saved)
}

/// openp's verdict on a function PREDICATE's RESULT for the candidate last
/// in SAVED: nil rejects it, and a directory needs `dir-ok'.
fn locate_predicate_result(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    let candidate = saved[7];
    if result != Value::NIL
        && (result == cx.intern("dir-ok") || !std::path::Path::new(&cx.string_text(candidate)?).is_dir())
    {
        return Ok(Step::Done(candidate));
    }
    locate_suffix(cx, &saved[..7])
}

/// Native file lookup still refuses dispatch when a handler is selected.
fn refuse_locate_handler(cx: &mut Leaf<'_>, filename: Value, operation: &str) -> std::result::Result<(), Fault> {
    let operation = cx.intern(operation);
    if find_file_name_handler(cx, filename, operation)? != Value::NIL {
        return Err(refuse("locate-file-internal matching file name handler"));
    }
    Ok(())
}

// Romacs has no toolkit file dialog, so it answers as a GNU built without
// one (no GTK, Motif, NS, NTGUI or Haiku) does.
defsubr! { name = "next-read-file-uses-dialog-p", gnu = "fileio.c:6556", effect = Pure,
fn next_read_file_uses_dialog_p(cx) {
    let _ = cx;
    Ok(Value::NIL)
}}
