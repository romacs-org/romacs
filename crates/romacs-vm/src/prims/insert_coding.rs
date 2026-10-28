//! GNU fileio.c `insert-file-contents' (fileio.c:4057) from the coding
//! decision on: `coding-system-for-read', `set-auto-coding-function' (on a
//! work buffer when the buffer has text, fileio.c:4323; on the file's bytes
//! made the buffer's unibyte text otherwise, fileio.c:5044, undone by
//! decide_coding_unwind), `file-coding-system-alist' through
//! `find-operation-coding-system', CHECK_CODING_SYSTEM and
//! raw_text_coding_system; then decode_coding_gap (coding.c detect_coding
//! and the native string decoder), the visit bookkeeping, and the Lisp tail:
//! `after-insert-file-set-coding' (fileio.c:5174), `format-decode' and
//! `after-insert-file-functions' (5186-5306).
//!
//! Every Lisp call is a `Step::Call'; the state waits in the registers
//! below. GNU's unwind and binding extents are `lambda' forms, evaluated by
//! a `Step::Call' of `eval' and then called with the live objects
//! (`closure_then'): `unwind-protect' around the empty-buffer decision,
//! `save-current-buffer' around the work buffer, and `let' of the two
//! inhibit variables around `format-decode'.
use super::*;
use super::fileio::{file_errno, refuse, var};
use super::write_coding::{need, raw_text_coding_system};
use crate::forwarded::VisitedModtime;

// Registers. ORIG is the expanded file name; VISIT..REPLACE the arguments;
// BYTES the file's bytes as a unibyte string (nil when it was not found);
// CODING the decision; INSERTED a fixnum (bytes read, then characters
// inserted); MODTIME the visited modtime (see `modtime_value'); ERRNO the
// open error of a visited missing file, else 0; DATA decide_coding_unwind's
// cons; FLAGS the bits and stage below; PREPARED nil until a modifying read
// prepares its change, then where the read resumes (RESUME_*); BEHIND,
// IN_FRONT and OVERLAYS the hooks the change found for its after-change.
const ORIG: usize = 0;
const VISIT: usize = 1;
const BEG: usize = 2;
const END: usize = 3;
const REPLACE: usize = 4;
const BYTES: usize = 5;
const CODING: usize = 6;
const INSERTED: usize = 7;
const MODTIME: usize = 8;
const ERRNO: usize = 9;
const DATA: usize = 10;
const FLAGS: usize = 11;
/// Where the 3 KiB tail `set-auto-coding-function' sees starts in BYTES.
const TAIL: usize = 12;
const PREPARED: usize = 13;
const BEHIND: usize = 14;
const IN_FRONT: usize = 15;
const OVERLAYS: usize = 16;
/// REPLACE: (MARKER OLDPOS OLDEND), point as a marker (restore_point_unwind)
/// restored at the end, where point was and where the replaced text ended.
const POINT: usize = 17;
/// REPLACE: (FROM . TEXT), the decoded middle that replaces the buffer's
/// differing text at FROM.
const MIDDLE: usize = 18;
const REGISTERS: usize = 19;

/// A read of an empty buffer prepares before its coding decision.
const RESUME_UNDECIDED: i64 = 1;
/// A read whose decision came first prepares after it.
const RESUME_DECIDED: i64 = 2;
/// A REPLACE prepares the middle's insertion after deleting the old one.
const RESUME_REPLACE: i64 = 3;

/// empty_undo_list_p (fileio.c:4126).
const EMPTY_UNDO: i64 = 1;
/// BEG < Z when the call began: the decision precedes the read.
const HAD_TEXT: i64 = 2;
/// The insertion is visible to change hooks: !VISIT and a non-empty range.
const MODIFYING: i64 = 4;
/// The decision preceded the read, so `deactivate-mark' is set after it.
const DECIDED_FIRST: i64 = 8;
const STAGE: i64 = 16;

#[derive(Clone, Copy)]
enum Stage {
    /// CODING holds a candidate (nil: undecided); CHECK_CODING_SYSTEM it.
    Check = 1,
    /// raw_text_coding_system for a unibyte buffer.
    Unibyte,
    /// The decision is made: decode and insert.
    Decided,
    /// A visited missing file: CHECK_CODING_SYSTEM (coding-system-for-read).
    MissingCheck,
}

fn flags(r: &[Value]) -> i64 { r[FLAGS].integer_value().expect("flags register") }
fn has(r: &[Value], bit: i64) -> bool { flags(r) & bit != 0 }
fn at(r: &mut [Value], stage: Stage) {
    let bits = flags(r) % STAGE;
    r[FLAGS] = Value::integer(bits + stage as i64 * STAGE).unwrap();
}
fn fixnum(n: i64) -> std::result::Result<Value, Fault> { Value::integer(n).map_err(Fault::Error) }
fn count(r: &[Value]) -> i64 { r[INSERTED].integer_value().expect("inserted register") }

/// What `insert-file-contents' read before the decision.
pub(super) struct Read {
    pub orig: Value,
    pub args: [Value; 4],
    /// The file's bytes; None for a visited file that could not be opened.
    pub bytes: Option<Vec<u8>>,
    pub modtime: VisitedModtime,
    pub errno: i32,
    pub empty_undo: bool,
    pub had_text: bool,
    pub modifying: bool,
    /// fileio.c:4344-4373's tail start, relative to the first byte read;
    /// None when GNU reads no tail (the first 4 KiB read reached END or EOF).
    pub tail: Option<usize>,
}

/// VisitedModtime as register data: 0 unknown, -1 missing, else
/// (SECONDS NANOS SIZE).
fn modtime_value(cx: &mut Leaf<'_>, modtime: VisitedModtime) -> std::result::Result<Value, Fault> {
    Ok(match modtime {
        VisitedModtime::Unknown => fixnum(0)?,
        VisitedModtime::Missing => fixnum(-1)?,
        VisitedModtime::Known { seconds, nanos, size } => {
            let size = fixnum(size.map_or(-1, |s| s as i64))?;
            let items = [fixnum(seconds)?, fixnum(nanos)?, size];
            cx.coding_list(&items)
        }
    })
}

fn modtime_of(cx: &Leaf<'_>, value: Value) -> VisitedModtime {
    match value.integer_value() {
        Some(0) => return VisitedModtime::Unknown,
        Some(_) => return VisitedModtime::Missing,
        None => {}
    }
    let (seconds, rest) = cx.pair(value).expect("modtime register");
    let (nanos, rest) = cx.pair(rest).expect("modtime register");
    let (size, _) = cx.pair(rest).expect("modtime register");
    let size = size.integer_value().filter(|&s| s >= 0).map(|s| s as u64);
    VisitedModtime::Known { seconds: seconds.integer_value().unwrap(), nanos: nanos.integer_value().unwrap(), size }
}

/// Start the decision for what `insert-file-contents' read.
pub(super) fn start(cx: &mut Leaf<'_>, read: Read) -> Next {
    let mut r = vec![Value::NIL; REGISTERS];
    r[ORIG] = read.orig;
    r[VISIT..=REPLACE].copy_from_slice(&read.args);
    let inserted = read.bytes.as_ref().map_or(0, Vec::len);
    if let Some(bytes) = read.bytes { r[BYTES] = cx.make_string(bytes, false); }
    r[INSERTED] = fixnum(inserted as i64)?;
    r[MODTIME] = modtime_value(cx, read.modtime)?;
    r[ERRNO] = fixnum(read.errno.into())?;
    r[TAIL] = match read.tail { Some(tail) => fixnum(tail as i64)?, None => Value::NIL };
    let bits = [(read.empty_undo, EMPTY_UNDO), (read.had_text, HAD_TEXT), (read.modifying, MODIFYING)]
        .iter().filter(|(on, _)| *on).map(|(_, bit)| bit).sum::<i64>();
    r[FLAGS] = fixnum(bits)?;
    let csr = var(cx, "coding-system-for-read")?;
    if r[BYTES] == Value::NIL {
        // fileio.c:4201: a visited missing file keeps coding-system-for-read
        // in buffer-file-coding-system, then decides at notfound.
        if csr != Value::NIL {
            r[CODING] = csr;
            at(&mut r, Stage::MissingCheck);
            return advance(cx, r);
        }
        return decide_empty(cx, r);
    }
    if csr == cx.intern("auto-save-coding") {
        // coding_inherit_eol_type (Qutf_8_emacs, Qunix).
        r[CODING] = cx.intern("utf-8-emacs-unix");
        r[FLAGS] = fixnum(flags(&r) | DECIDED_FIRST)?;
        at(&mut r, Stage::Check);
        return advance(cx, r);
    }
    if has(&r, HAD_TEXT) {
        r[FLAGS] = fixnum(flags(&r) | DECIDED_FIRST)?;
        if csr != Value::NIL {
            r[CODING] = csr;
            at(&mut r, Stage::Check);
            return advance(cx, r);
        }
        if var(cx, "set-auto-coding-function")? != Value::NIL {
            return auto_coding_work(cx, r);
        }
        return alist(cx, r, Value::NIL);
    }
    if has(&r, MODIFYING) && inserted > 0 { return prepare(cx, r, RESUME_UNDECIDED); }
    read_undecided(cx, r)
}

fn read_undecided(cx: &mut Leaf<'_>, r: Vec<Value>) -> Next {
    // fileio.c:5008, after the read and before this decision.
    if count(&r) > 0 {
        let deactivate = cx.intern("deactivate-mark");
        cx.variable_set(deactivate, Value::TRUE)?;
    }
    decide_empty(cx, r)
}

/// fileio.c:4889: prepare_to_modify_buffer (PT, PT) for a modifying read,
/// after a decision made first and before one made after the read. The
/// read-only checks, then the before-change Lisp when any can run.
fn prepare(cx: &mut Leaf<'_>, mut r: Vec<Value>, resume: i64) -> Next {
    r[PREPARED] = fixnum(resume)?;
    let point = cx.point_char()?;
    let prepared = cx.prepare_change(point, point)?;
    (r[BEHIND], r[IN_FRONT]) = (prepared.behind, prepared.in_front);
    let insert_hooks = prepared.behind != Value::NIL || prepared.in_front != Value::NIL;
    if (prepared.before || cx.after_change_runs(false, insert_hooks)?) && super::insdel::half_missing(cx)? {
        return super::insdel::install_half(cx, reprepare, r);
    }
    if prepared.before {
        let undoable = if prepared.undoable { Value::TRUE } else { Value::NIL };
        let args = vec![cx.current_buffer()?, fixnum(point)?, fixnum(point)?, Value::NIL, undoable];
        return Ok(Step::Call { f: cx.intern("romacs--before-change"), args, then: before_changed, saved: r });
    }
    resume_read(cx, r)
}

fn reprepare(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let resume = saved[PREPARED].integer_value().expect("resume register");
    prepare(cx, saved.to_vec(), resume)
}

fn before_changed(cx: &mut Leaf<'_>, before: Value, saved: &[Value]) -> Next {
    let mut r = saved.to_vec();
    r[OVERLAYS] = super::insdel::found_overlays(cx, before);
    resume_read(cx, r)
}

fn resume_read(cx: &mut Leaf<'_>, r: Vec<Value>) -> Next {
    match r[PREPARED].integer_value() {
        Some(RESUME_UNDECIDED) => read_undecided(cx, r),
        Some(RESUME_REPLACE) => {
            let (_, middle) = cx.pair(r[MIDDLE]).expect("replace middle register");
            let name = r[CODING];
            insert_decoded(cx, r, Some((middle, name)))
        }
        _ => decided(cx, r),
    }
}

/// The name `set-auto-coding-function' receives: ENCODE_FILE (filename),
/// coding.c encode_file_name_1. A multibyte name is encoded by
/// `file-name-coding-system', else `default-file-name-coding-system'
/// (convert_string_nocopy: `last-coding-system-used' is left as it was).
fn encoded_name(cx: &mut Leaf<'_>, name: Value) -> std::result::Result<Value, Fault> {
    if !cx.string_view(name)?.is_multibyte() { return Ok(name); }
    let mut coding = var(cx, "file-name-coding-system")?;
    if coding == Value::NIL { coding = var(cx, "default-file-name-coding-system")?; }
    if coding == Value::NIL { return Ok(name); }
    let last = cx.intern("last-coding-system-used");
    let recorded = cx.variable_value(last)?.unwrap_or(Value::NIL);
    let encoded = cx.coding_convert_string(name, coding, Value::TRUE, Value::NIL, true)?;
    cx.variable_set(last, recorded)?;
    Ok(encoded)
}

/// Evaluate the `lambda' form SOURCE into a closure, then (in THEN) call
/// it with the live objects as arguments. Romacs's `eval' copies quoted
/// constants, so data whose identity matters is never quoted into a form.
pub(super) fn closure_then(cx: &mut Leaf<'_>, source: &str, then: crate::prim::Then, saved: Vec<Value>) -> Next {
    let text = cx.string(source);
    let read = cx.read_from_string(text, Value::NIL, Value::NIL)?;
    let (form, _) = cx.pair(read).expect("read result");
    Ok(Step::Call { f: cx.intern("eval"), args: vec![form, Value::TRUE], then, saved })
}

/// fileio.c:4323-4406: with text in the buffer, `set-auto-coding-function'
/// sees the first 1 KiB and the last 3 KiB of the range (as GNU reads them
/// into a 16 KiB buffer) as the unibyte text of " *code-converting-work*".
fn auto_coding_work(cx: &mut Leaf<'_>, r: Vec<Value>) -> Next {
    let bytes = cx.string_view(r[BYTES])?.bytes().to_vec();
    if bytes.is_empty() { return alist(cx, r, Value::NIL); }
    let sample = match r[TAIL].integer_value() {
        Some(tail) => {
            let tail = &bytes[tail as usize..bytes.len().min(tail as usize + 16 * 1024 - 1024)];
            [&bytes[..1024], tail].concat()
        }
        None => bytes,
    };
    let mut r = r;
    r[DATA] = cx.make_string(sample, false);
    closure_then(cx, "(lambda (romacs--name romacs--n romacs--directory romacs--sample)
                        (save-current-buffer
                          (set-buffer (get-buffer-create \" *code-converting-work*\" t))
                          (set (make-local-variable 'default-directory) romacs--directory)
                          (romacs--code-converting-work romacs--sample)
                          (funcall set-auto-coding-function romacs--name romacs--n)))",
                 auto_closure, r)
}

fn auto_closure(cx: &mut Leaf<'_>, closure: Value, saved: &[Value]) -> Next {
    let mut r = saved.to_vec();
    let sample = r[DATA];
    r[DATA] = Value::NIL;
    let n = fixnum(cx.string_view(sample)?.bytes().len() as i64)?;
    let directory = var(cx, "default-directory")?;
    let name = encoded_name(cx, r[ORIG])?;
    let args = vec![name, n, directory, sample];
    Ok(Step::Call { f: closure, args, then: auto_answered, saved: r })
}

fn auto_answered(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    alist(cx, saved.to_vec(), value)
}

/// If VALUE (the auto-coding answer) is nil, `file-coding-system-alist'
/// through Ffind_operation_coding_system with REPLACE as given.
fn alist(cx: &mut Leaf<'_>, mut r: Vec<Value>, value: Value) -> Next {
    r[CODING] = value;
    at(&mut r, Stage::Check);
    if value != Value::NIL { return advance(cx, r); }
    let operation = cx.intern("insert-file-contents");
    let args = [operation, r[ORIG], r[VISIT], r[BEG], r[END], r[REPLACE]];
    match cx.find_operation_coding(&args)? {
        Ok(pair) => alist_answered(cx, pair, &r),
        Err(function) => {
            let list = cx.coding_list(&args);
            Ok(Step::Call { f: function, args: vec![list], then: alist_function_answered, saved: r })
        }
    }
}

fn alist_function_answered(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let pair = cx.operation_coding_pair(value)?;
    alist_answered(cx, pair, saved)
}

fn alist_answered(cx: &mut Leaf<'_>, pair: Value, saved: &[Value]) -> Next {
    let mut r = saved.to_vec();
    r[CODING] = cx.pair(pair).map_or(pair, |(car, _)| car);
    advance(cx, r)
}

/// fileio.c:5016-5085, an empty buffer: unless `coding-system-for-read'
/// decides, the read bytes become the buffer's unibyte text with undo off
/// while `set-auto-coding-function' and the alist decide, and
/// decide_coding_unwind takes them back on every exit (`decide_closure').
fn decide_empty(cx: &mut Leaf<'_>, mut r: Vec<Value>) -> Next {
    let csr = var(cx, "coding-system-for-read")?;
    if csr != Value::NIL {
        r[CODING] = csr;
        at(&mut r, Stage::Check);
        return advance(cx, r);
    }
    // The name the function sees, encoded before the buffer changes.
    r[DATA] = r[ORIG];
    if r[INSERTED] != fixnum(0)? && var(cx, "set-auto-coding-function")? != Value::NIL {
        r[DATA] = encoded_name(cx, r[ORIG])?;
    }
    closure_then(cx, "(lambda (romacs--name romacs--n romacs--orig romacs--visit romacs--beg romacs--end romacs--buffer romacs--data)
                        (unwind-protect
                            (or (and (> romacs--n 0) set-auto-coding-function
                                     (funcall set-auto-coding-function romacs--name romacs--n))
                                (car-safe (find-operation-coding-system 'insert-file-contents romacs--orig
                                                                        romacs--visit romacs--beg romacs--end nil)))
                          (set-buffer romacs--buffer)
                          (romacs--insert-file-contents-undecide romacs--data)))",
                 decide_closure, r)
}

/// Enter the decision's extent: DATA = (MULTIBYTE UNDO . BUFFER), then the
/// bytes as unibyte text with undo off, then the protected call.
fn decide_closure(cx: &mut Leaf<'_>, closure: Value, saved: &[Value]) -> Next {
    let mut r = saved.to_vec();
    let name = r[DATA];
    let buffer = cx.current_buffer()?;
    let multibyte = if cx.buffer_shape()?.unibyte { Value::NIL } else { Value::TRUE };
    let undo = var(cx, "buffer-undo-list")?;
    let tail = cx.cons(undo, buffer);
    r[DATA] = cx.cons(multibyte, tail);
    let bytes = if r[BYTES] == Value::NIL { vec![] } else { cx.string_view(r[BYTES])?.bytes().to_vec() };
    cx.fileio_set_multibyte_flag(false)?;
    cx.bset("buffer-undo-list", Value::TRUE)?;
    cx.fileio_fill_raw(&bytes)?;
    let args = vec![name, r[INSERTED], r[ORIG], r[VISIT], r[BEG], r[END], buffer, r[DATA]];
    Ok(Step::Call { f: closure, args, then: empty_decided, saved: r })
}

fn empty_decided(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let mut r = saved.to_vec();
    // decide_coding_unwind left the bytes, possibly edited, in DATA's car.
    let (bytes, _) = cx.pair(r[DATA]).expect("decide data");
    let n = cx.string_view(bytes)?.bytes().len();
    if r[BYTES] != Value::NIL { r[BYTES] = bytes; }
    r[INSERTED] = fixnum(n as i64)?;
    r[DATA] = Value::NIL;
    r[CODING] = value;
    at(&mut r, Stage::Check);
    advance(cx, r)
}

/// decide_coding_unwind (fileio.c:3857) for DATA = (MULTIBYTE UNDO . BUFFER)
/// in the current buffer: take its bytes back, restore its multibyteness
/// and undo list, and leave the bytes in DATA's car.
fn undecide_now(cx: &mut Leaf<'_>, data: Value) -> std::result::Result<(), Fault> {
    let (multibyte, rest) = cx.pair(data).ok_or(Fault::Wrong("consp", data))?;
    let (undo, _) = cx.pair(rest).ok_or(Fault::Wrong("consp", rest))?;
    let bytes = cx.fileio_take_raw()?;
    let unibyte = cx.buffer_shape()?.unibyte;
    // Text Lisp left multibyte goes back as its internal bytes.
    let bytes = cx.make_string(bytes, !unibyte);
    cx.fileio_set_multibyte_flag(multibyte != Value::NIL)?;
    cx.bset("buffer-undo-list", undo)?;
    cx.setcar(data, bytes)?;
    Ok(())
}

// Internal: decide_coding_unwind's work, called from the unwind forms of
// `decide_empty' after (set-buffer BUFFER).
defsubr! { name = "romacs--insert-file-contents-undecide", gnu = "fileio.c:3857", effect = Global,
fn insert_file_contents_undecide(cx, data: Value) {
    undecide_now(cx, data)?;
    Ok(Value::NIL)
}}

// Internal: fileio.c:4378-4400, the work buffer made current (its
// directory set by the caller): writable, no file, no undo, and SAMPLE as
// its unibyte text with point at its beginning. GNU deletes the buffer's
// overlays; VM2 refuses a work buffer holding any (fileio_set_multibyte_flag).
defsubr! { name = "romacs--code-converting-work", gnu = "fileio.c:4378", effect = Global,
fn code_converting_work(cx, sample: Value) {
    cx.bset("buffer-read-only", Value::NIL)?;
    cx.bset("buffer-file-name", Value::NIL)?;
    cx.bset("buffer-undo-list", Value::TRUE)?;
    cx.fileio_take_raw()?;
    cx.fileio_set_multibyte_flag(false)?;
    let bytes = cx.string_view(sample)?.bytes().to_vec();
    cx.fileio_fill_raw(&bytes)?;
    Ok(Value::NIL)
}}

fn realized(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    advance(cx, saved.to_vec())
}

fn realize(cx: &mut Leaf<'_>, name: Value, r: Vec<Value>) -> Next {
    super::coding::realize(cx, name, r, realized)
}

/// Run the coding checks from the stage in FLAGS.
fn advance(cx: &mut Leaf<'_>, mut r: Vec<Value>) -> Next {
    loop {
        let stage = flags(&r) / STAGE;
        match stage {
            s if s == Stage::MissingCheck as i64 => {
                if let Err(name) = need(cx, r[CODING])? { return realize(cx, name, r); }
                // Fset (Qbuffer_file_coding_system, Vcoding_system_for_read).
                let symbol = cx.intern("buffer-file-coding-system");
                cx.variable_set(symbol, r[CODING])?;
                r[CODING] = Value::NIL;
                return decide_empty(cx, r);
            }
            s if s == Stage::Check as i64 => {
                if r[CODING] == Value::NIL {
                    r[CODING] = cx.intern("undecided");
                } else if let Err(name) = need(cx, r[CODING])? {
                    return realize(cx, name, r);
                }
                at(&mut r, Stage::Unibyte);
            }
            s if s == Stage::Unibyte as i64 => {
                if cx.buffer_shape()?.unibyte {
                    // Suppress all conversion but end-of-line conversion.
                    match raw_text_coding_system(cx, r[CODING])? {
                        Ok(coding) => r[CODING] = coding,
                        Err(name) => return realize(cx, name, r),
                    }
                }
                at(&mut r, Stage::Decided);
            }
            s if s == Stage::Decided as i64 => return decided(cx, r),
            _ => unreachable!("insert-file-contents coding stage {stage}"),
        }
    }
}

/// CODING_FOR_UNIBYTE: the coding system's :for-unibyte attribute.
fn for_unibyte(cx: &mut Leaf<'_>, coding: Value) -> std::result::Result<bool, Fault> {
    let spec = cx.coding_spec(coding)?;
    let attrs = cx.vector_get(spec, fixnum(0)?)?;
    Ok(cx.vector_get(attrs, fixnum(12)?)? != Value::NIL)
}

/// fileio.c:5086-5101: visiting by a for-unibyte coding makes the buffer
/// unibyte, through Fset_buffer_multibyte when nothing was read.
fn decided(cx: &mut Leaf<'_>, r: Vec<Value>) -> Next {
    if r[REPLACE] != Value::NIL && r[PREPARED] == Value::NIL && { let (_, begv, zv) = cx.point_extent()?; begv < zv } {
        return replace(cx, r);
    }
    if has(&r, DECIDED_FIRST) && has(&r, MODIFYING) && count(&r) > 0 && r[PREPARED] == Value::NIL {
        return prepare(cx, r, RESUME_DECIDED);
    }
    // fileio.c:5008, after the read that followed this decision.
    if has(&r, DECIDED_FIRST) && count(&r) > 0 {
        let deactivate = cx.intern("deactivate-mark");
        cx.variable_set(deactivate, Value::TRUE)?;
    }
    if r[VISIT] != Value::NIL && r[REPLACE] == Value::NIL && for_unibyte(cx, r[CODING])? && !cx.buffer_shape()?.unibyte {
        if count(&r) == 0 {
            let f = cx.intern("set-buffer-multibyte");
            return Ok(Step::Call { f, args: vec![Value::NIL], then: made_unibyte, saved: r });
        }
        let decoded = decode(cx, &r)?;
        cx.fileio_set_multibyte_flag(false)?;
        return insert_decoded(cx, r, decoded);
    }
    let decoded = decode(cx, &r)?;
    insert_decoded(cx, r, decoded)
}

/// fileio.c:4684-4880, REPLACE of a non-empty accessible region: the
/// whole file decoded, then only the text between its longest common
/// head and tail with the buffer (on character boundaries) is replaced,
/// so markers in the unchanged parts stay. The deletion and the middle's
/// insertion are ordinary changes with their hooks; point is preserved as
/// a marker and restored at the end (restore_point_unwind, not on a
/// non-local exit).
fn replace(cx: &mut Leaf<'_>, mut r: Vec<Value>) -> Next {
    let decoded = decode(cx, &r)?;
    let (text, name) = match decoded {
        Some(pair) => pair,
        None => (cx.byte_string(&[]), r[CODING]),
    };
    let (_, begv, zv) = cx.point_extent()?;
    let old = cx.region_text(begv, zv)?;
    let multibyte = !cx.buffer_shape()?.unibyte;
    let (old, new) = (cx.string_view(old)?.bytes().to_vec(), cx.string_view(text)?.bytes().to_vec());
    if multibyte && !cx.string_view(text)?.is_multibyte() && new.iter().any(|&b| b >= 0x80) {
        return Err(refuse("insert-file-contents of raw bytes into a multibyte buffer"));
    }
    let (start, end, middle) = common_ends(&old, &new, multibyte);
    let chars = |bytes: &[u8]| if multibyte { bytes.iter().filter(|&&b| b & 0xc0 != 0x80).count() } else { bytes.len() } as i64;
    let (from, to) = (begv + chars(&old[..start]), begv + chars(&old[..end]));
    let marker = cx.editor_point_marker()?;
    let point = fixnum(cx.point_char()?)?;
    r[POINT] = cx.coding_list(&[marker, point, fixnum(to)?]);
    let middle = cx.make_string(new[middle].to_vec(), multibyte && cx.string_view(text)?.is_multibyte());
    r[MIDDLE] = cx.cons(fixnum(from)?, middle);
    r[CODING] = name;
    r[PREPARED] = fixnum(RESUME_REPLACE)?;
    let args = vec![fixnum(from)?, fixnum(to)?];
    Ok(Step::Call { f: cx.intern("delete-region"), args, then: replace_deleted, saved: r })
}

fn replace_deleted(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let r = saved.to_vec();
    let (from, middle) = cx.pair(r[MIDDLE]).expect("replace middle register");
    if cx.string_chars(middle) == Some(0) { return insert_decoded(cx, r, None); }
    // The middle goes where the old one was (SET_PT_BOTH (temp, ...)).
    cx.goto_char_position(from.integer_value().expect("replace position"))?;
    prepare(cx, r, RESUME_REPLACE)
}

/// The byte ranges of OLD and NEW that differ, as fileio.c:4758-4805
/// computes them: the common head, the common tail not overlapping it in
/// either text, both widened to character boundaries in a multibyte
/// buffer. Returns OLD's START..END and NEW's middle.
fn common_ends(old: &[u8], new: &[u8], multibyte: bool) -> (usize, usize, std::ops::Range<usize>) {
    let head = |b: u8| !multibyte || b & 0xc0 != 0x80;
    let mut start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    if start == new.len() { return (start, old.len(), start..start); }
    while start > 0 && start < old.len() && !head(old[start]) { start -= 1; }
    let limit = start.max(new.len().saturating_sub(old.len() - start));
    let (mut end, mut at) = (old.len(), new.len());
    while limit < at && old[end - 1] == new[at - 1] { end -= 1; at -= 1; }
    while end < old.len() && !head(old[end]) { end += 1; }
    (start, end, start..new.len() - (old.len() - end))
}

fn made_unibyte(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let r = saved.to_vec();
    let decoded = decode(cx, &r)?;
    insert_decoded(cx, r, decoded)
}

/// decode_coding_gap's result for the read bytes: the decoded string and
/// the coding system that decoded it, or None when nothing was read.
fn decode(cx: &mut Leaf<'_>, r: &[Value]) -> std::result::Result<Option<(Value, Value)>, Fault> {
    if r[BYTES] == Value::NIL || count(r) == 0 { return Ok(None); }
    // The string decoder reports through last-coding-system-used, which
    // insert-file-contents sets only once the text is in (fileio.c:5171).
    let lcsu = cx.intern("last-coding-system-used");
    let old = var(cx, "last-coding-system-used")?;
    let decoded = decode_bytes(cx, r);
    cx.variable_set(lcsu, old)?;
    decoded
}

fn decode_bytes(cx: &mut Leaf<'_>, r: &[Value]) -> std::result::Result<Option<(Value, Value)>, Fault> {
    let (bytes, coding) = (r[BYTES], r[CODING]);
    let spec = cx.coding_spec(coding)?;
    let attrs = cx.vector_get(spec, fixnum(0)?)?;
    if cx.vector_get(attrs, fixnum(9)?)? != Value::NIL {
        return Err(refuse("insert-file-contents with a post-read-conversion coding system"));
    }
    if cx.vector_get(attrs, fixnum(6)?)? != Value::NIL {
        return Err(refuse("insert-file-contents with a decoding translation table"));
    }
    let octets = cx.string_view(bytes)?.bytes().to_vec();
    let (found, eol_seen) = cx.coding_detect(&octets, attrs)?;
    let eols = cx.vector_get(spec, fixnum(2)?)?;
    let vector = cx.kind(eols) == crate::prim::leaf::Kind::Vector;
    let Some(found) = found else {
        if cx.vector_get(attrs, fixnum(3)?)? != cx.intern("undecided") {
            let mut coding = coding;
            if let Some(index) = cx.coding_ascii_eol(&octets, attrs)?.filter(|_| vector) {
                coding = cx.vector_get(eols, fixnum(index as i64)?)?;
            }
            let decoded = cx.coding_convert_string(bytes, coding, Value::TRUE, Value::NIL, false)?;
            let name = var(cx, "last-coding-system-used")?;
            return Ok(Some((decoded, name)));
        }
        // Nothing detected: the raw decoder, the EOL the scan saw.
        let mut name = coding;
        if let Some(index) = eol_seen.filter(|_| vector) {
            name = cx.vector_get(eols, fixnum(index as i64)?)?;
        }
        let spec = cx.coding_spec(name)?;
        let eol = cx.vector_get(spec, fixnum(2)?)?;
        let mut out = Vec::with_capacity(octets.len());
        let (mac, dos) = (eol == cx.intern("mac"), eol == cx.intern("dos"));
        let inhibit = var(cx, "inhibit-eol-conversion")? != Value::NIL;
        for (i, &byte) in octets.iter().enumerate() {
            if !inhibit && byte == b'\r' && (mac || (dos && octets.get(i + 1) == Some(&b'\n'))) {
                if mac { out.push(b'\n'); }
                continue;
            }
            out.push(byte);
        }
        let decoded = cx.make_string(out, false);
        return Ok(Some((decoded, name)));
    };
    // setup_coding_system (found), keeping an EOL CODING itself specified.
    let mut found = found;
    if !vector {
        let found_spec = cx.coding_spec(found)?;
        let found_eols = cx.vector_get(found_spec, fixnum(2)?)?;
        if cx.kind(found_eols) == crate::prim::leaf::Kind::Vector {
            let index = ["unix", "dos", "mac"].iter().position(|e| eols == cx.intern(e)).unwrap_or(0);
            found = cx.vector_get(found_eols, fixnum(index as i64)?)?;
        }
    }
    let decoded = cx.coding_convert_string(bytes, found, Value::TRUE, Value::NIL, false)?;
    let name = var(cx, "last-coding-system-used")?;
    Ok(Some((decoded, name)))
}

/// Insert DECODED at point (fileio.c:5105-5137) and signal the change of
/// a modifying read, then the visit bookkeeping (5141-5169) and
/// `last-coding-system-used'.
fn insert_decoded(cx: &mut Leaf<'_>, mut r: Vec<Value>, decoded: Option<(Value, Value)>) -> Next {
    let mut inserted = 0;
    if let Some((text, name)) = decoded {
        let view = cx.string_view(text)?;
        let (bytes, multibyte) = (view.bytes().to_vec(), view.is_multibyte());
        inserted = cx.insert_before_point(&bytes, multibyte)?;
        if multibyte { cx.fileio_copy_string_properties(text)?; }
        r[CODING] = name;
    }
    r[INSERTED] = fixnum(inserted as i64)?;
    // fileio.c:5132: signal_after_change (PT, 0, INSERTED), point before the
    // text; a REPLACE's insert_from_buffer (4859) signals with point after
    // it, then puts point back before it.
    let insert_hooks = r[BEHIND] != Value::NIL || r[IN_FRONT] != Value::NIL;
    let replacing = replacing(&r);
    if replacing && inserted > 0 { let point = cx.point_char()?; cx.goto_char_position(point + inserted as i64)?; }
    if (has(&r, MODIFYING) || replacing) && inserted > 0 && cx.after_change_runs(r[OVERLAYS] != Value::NIL, insert_hooks)? {
        if super::insdel::half_missing(cx)? { return super::insdel::install_half(cx, signal_inserted, r); }
        return signal_inserted(cx, Value::NIL, &r);
    }
    signalled(cx, Value::NIL, &r)
}

fn replacing(r: &[Value]) -> bool { r[PREPARED].integer_value() == Some(RESUME_REPLACE) }

fn signal_inserted(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let point = cx.point_char()?;
    let start = if replacing(saved) { point - count(saved) } else { point };
    let args = vec![cx.current_buffer()?, fixnum(start)?, fixnum(start + count(saved))?, fixnum(0)?,
                    saved[BEHIND], saved[IN_FRONT], saved[OVERLAYS]];
    Ok(Step::Call { f: cx.intern("romacs--after-change"), args, then: signalled, saved: saved.to_vec() })
}

fn signalled(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    if replacing(saved) && count(saved) > 0 && let Some((from, _)) = cx.pair(saved[MIDDLE]) {
        cx.goto_char_position(from.integer_value().expect("replace position"))?;
    }
    visited(cx, saved.to_vec())
}

/// restore_window_points (fileio.c:4005) for point, after a REPLACE that
/// inserted text: a point that was inside the replaced text keeps its
/// relative distance into the new text (bug#19161).
fn spread_point(cx: &mut Leaf<'_>, r: &mut [Value]) -> std::result::Result<(), Fault> {
    let (Some((marker, tail)), Some((from, middle))) = (cx.pair(r[POINT]), cx.pair(r[MIDDLE])) else { return Ok(()) };
    let inserted = cx.string_chars(middle).unwrap_or(0) as i64;
    let (oldpos, rest) = cx.pair(tail).expect("point register");
    let (oldend, _) = cx.pair(rest).expect("point register");
    let (from, oldpos, oldend) = (from.integer_value().unwrap(), oldpos.integer_value().unwrap(), oldend.integer_value().unwrap());
    if inserted == 0 || oldpos <= from || oldpos > oldend { return Ok(()); }
    let growth = inserted as f64 / (oldend - from) as f64;
    let newpos = from + (growth * (oldpos - from) as f64) as i64;
    cx.release_watch(marker)?;
    let marker = cx.editor_position_marker(newpos)?;
    r[POINT] = cx.cons(marker, tail);
    Ok(())
}

fn visited(cx: &mut Leaf<'_>, mut r: Vec<Value>) -> Next {
    spread_point(cx, &mut r)?;
    if r[VISIT] != Value::NIL {
        if has(&r, EMPTY_UNDO) { cx.bset("buffer-undo-list", Value::NIL)?; }
        cx.bset("buffer-file-name", r[ORIG])?;
        let modtime = modtime_of(cx, r[MODTIME]);
        cx.fileio_set_modtime(modtime)?;
        cx.set_buffer_modified(Value::NIL, true)?;
    }
    let lcsu = cx.intern("last-coding-system-used");
    cx.variable_set(lcsu, r[CODING])?;
    let set_coding = cx.intern("after-insert-file-set-coding");
    if cx.fboundp(set_coding) {
        let args = vec![r[INSERTED], r[VISIT]];
        return Ok(Step::Call { f: set_coding, args, then: coding_set, saved: r });
    }
    format(cx, r)
}

/// RANGED_FIXNUMP (0, INSVAL, ZV - PT), else (wrong-type-argument
/// inserted-chars INSVAL).
fn inserted_chars(cx: &mut Leaf<'_>, insval: Value) -> std::result::Result<Value, Fault> {
    let shape = cx.buffer_shape()?;
    let point = cx.point()?;
    match insval.integer_value() {
        Some(n) if (0..=shape.zv - point).contains(&n) => Ok(insval),
        _ => Err(Fault::Wrong("inserted-chars", insval)),
    }
}

fn coding_set(cx: &mut Leaf<'_>, insval: Value, saved: &[Value]) -> Next {
    let mut r = saved.to_vec();
    if insval != Value::NIL { r[INSERTED] = inserted_chars(cx, insval)?; }
    format(cx, r)
}

/// fileio.c:5189: `format-decode' and `after-insert-file-functions' with
/// point motion and modification hooks off:
///   (let ((inhibit-point-motion-hooks t) (inhibit-modification-hooks t))
///     (romacs--insert-file-contents-format 'INSERTED 'VISIT 'EMPTY-UNDO))
fn format(cx: &mut Leaf<'_>, r: Vec<Value>) -> Next {
    let format_decode = cx.intern("format-decode");
    if count(&r) == 0 || !cx.fboundp(format_decode) { return finish(cx, &r); }
    closure_then(cx, "(lambda (romacs--inserted romacs--visit romacs--empty-undo)
                        (let ((inhibit-point-motion-hooks t) (inhibit-modification-hooks t))
                          (romacs--insert-file-contents-format romacs--inserted romacs--visit romacs--empty-undo)))",
                 format_closure, r)
}

fn format_closure(cx: &mut Leaf<'_>, closure: Value, saved: &[Value]) -> Next {
    let r = saved.to_vec();
    let empty_undo = if has(&r, EMPTY_UNDO) { Value::TRUE } else { Value::NIL };
    let args = vec![r[INSERTED], r[VISIT], empty_undo];
    Ok(Step::Call { f: closure, args, then: formatted, saved: r })
}

fn formatted(cx: &mut Leaf<'_>, inserted: Value, saved: &[Value]) -> Next {
    let mut r = saved.to_vec();
    r[INSERTED] = inserted;
    finish(cx, &r)
}

/// fileio.c:5308-5333: a visited missing file signals now; else
/// (ORIG INSERTED).
fn finish(cx: &mut Leaf<'_>, r: &[Value]) -> Next {
    if let Some((marker, _)) = cx.pair(r[POINT]) {
        if let Some(Some(point)) = cx.marker_position(marker) { cx.goto_char_position(point)?; }
        cx.release_watch(marker)?;
    }
    let errno = r[ERRNO].integer_value().unwrap_or(0) as i32;
    if errno != 0 {
        let error = std::io::Error::from_raw_os_error(errno);
        return Err(file_errno(cx, "Opening input file", r[ORIG], &error));
    }
    let tail = cx.cons(r[INSERTED], Value::NIL);
    Ok(Step::Done(cx.cons(r[ORIG], tail)))
}

// The format-decode extent's registers: the arguments, the undo list it
// saved, the count before decoding, and FOR_EACH_TAIL's walk over
// `after-insert-file-functions' (lisp.h FOR_EACH_TAIL_INTERNAL: the tail,
// the tortoise, MAX, N and Q).
const F_INSERTED: usize = 0;
const F_EMPTY_UNDO: usize = 2;
const F_OLD_UNDO: usize = 3;
const F_OLD_INSERTED: usize = 4;
const F_TAIL: usize = 5;
const F_TORTOISE: usize = 6;
const F_MAX: usize = 7;
const F_N: usize = 8;
const F_Q: usize = 9;
const F_REGISTERS: usize = 10;

// Internal: the body of fileio.c:5189-5306 inside its bindings.
defsubr! { name = "romacs--insert-file-contents-format", gnu = "fileio.c:5189", effect = Global,
fn insert_file_contents_format(cx, inserted: Value, visit: Value, empty_undo: Value) -> Step {
    let mut f = vec![Value::NIL; F_REGISTERS];
    f[..3].copy_from_slice(&[inserted, visit, empty_undo]);
    // Save the old undo list and record no undo for decoding.
    f[F_OLD_UNDO] = var(cx, "buffer-undo-list")?;
    f[F_OLD_INSERTED] = inserted;
    cx.bset("buffer-undo-list", Value::TRUE)?;
    let format_decode = cx.intern("format-decode");
    Ok(Step::Call { f: format_decode, args: vec![Value::NIL, inserted, visit], then: format_decoded, saved: f })
}}

fn format_decoded(cx: &mut Leaf<'_>, insval: Value, saved: &[Value]) -> Next {
    let mut f = saved.to_vec();
    f[F_INSERTED] = inserted_chars(cx, insval)?;
    let tail = var(cx, "after-insert-file-functions")?;
    f[F_TAIL] = tail;
    f[F_TORTOISE] = tail;
    f[F_MAX] = fixnum(2)?;
    f[F_N] = fixnum(0)?;
    f[F_Q] = fixnum(2)?;
    walk(cx, f)
}

fn walk(cx: &mut Leaf<'_>, f: Vec<Value>) -> Next {
    match cx.pair(f[F_TAIL]) {
        Some((function, _)) => {
            let args = vec![f[F_INSERTED]];
            Ok(Step::Call { f: function, args, then: hooked, saved: f })
        }
        None => format_done(cx, &f),
    }
}

fn hooked(cx: &mut Leaf<'_>, insval: Value, saved: &[Value]) -> Next {
    let mut f = saved.to_vec();
    if insval != Value::NIL { f[F_INSERTED] = inserted_chars(cx, insval)?; }
    // FOR_EACH_TAIL reads the cdr after the call, then checks for a cycle.
    let (_, tail) = cx.pair(f[F_TAIL]).expect("walked a cons");
    f[F_TAIL] = tail;
    let get = |v: Value| v.integer_value().unwrap();
    let (mut max, mut n, mut q) = (get(f[F_MAX]), get(f[F_N]), get(f[F_Q]));
    q -= 1;
    let check = if q != 0 { true } else {
        n -= 1;
        if 0 < n { true } else {
            max <<= 1;
            q = max;
            n = max >> 16;
            f[F_TORTOISE] = tail;
            false
        }
    };
    (f[F_MAX], f[F_N], f[F_Q]) = (fixnum(max)?, fixnum(n)?, fixnum(q)?);
    if check && cx.pair(tail).is_some() && tail == f[F_TORTOISE] {
        return Err(Fault::Circular(tail));
    }
    walk(cx, f)
}

/// fileio.c:5286-5303: restore the undo list, adjusting its last record
/// for the size change, or start it empty after a first visit.
fn format_done(cx: &mut Leaf<'_>, f: &[Value]) -> Next {
    let old_undo = f[F_OLD_UNDO];
    if f[F_EMPTY_UNDO] == Value::NIL {
        cx.bset("buffer-undo-list", old_undo)?;
        let (inserted, old_inserted) = (f[F_INSERTED].integer_value().unwrap(), f[F_OLD_INSERTED].integer_value().unwrap());
        if inserted != old_inserted && let Some((record, _)) = cx.pair(old_undo) {
            let point = cx.point()?;
            if let Some((beg, end)) = cx.pair(record)
                && beg.integer_value().is_some()
                && end.integer_value() == Some(point + old_inserted) {
                let end = fixnum(point + inserted)?;
                cx.setcdr(record, end)?;
            }
        }
    } else {
        cx.bset("buffer-undo-list", if old_undo == Value::TRUE { Value::TRUE } else { Value::NIL })?;
    }
    Ok(Step::Done(f[F_INSERTED]))
}
