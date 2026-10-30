//! GNU fileio.c `choose_write_coding_system' (fileio.c:5348) for
//! `write-region', with the coding.c helpers it uses
//! (`coding_inherit_eol_type', coding.c:5970; `raw_text_coding_system',
//! coding.c:5932). GNU calls Lisp in the middle of the choice: a function
//! entry of `file-coding-system-alist' (through
//! `Ffind_operation_coding_system'), `select-safe-coding-system-function', and
//! the define form of a lazily defined coding system (CHECK_CODING_SYSTEM ->
//! `Fcheck_coding_system'). Each is a `Step::Call'; the choice so far waits in
//! the registers below and the walk resumes at its recorded stage.
use super::*;
use super::fileio::{var, write_chosen};

// Registers: write-region's arguments as GNU passes them on (START and END
// resolved to positions or the string, FILENAME expanded, LOCKNAME
// defaulted to VISIT-FILE), then VISIT-FILE, the choice VAL and FLAGS.
pub(super) const START: usize = 0;
pub(super) const END: usize = 1;
pub(super) const FILENAME: usize = 2;
pub(super) const APPEND: usize = 3;
pub(super) const VISIT: usize = 4;
pub(super) const LOCKNAME: usize = 5;
pub(super) const MUSTBENEW: usize = 6;
pub(super) const VISIT_FILE: usize = 7;
pub(super) const VAL: usize = 8;
pub(super) const FLAGS: usize = 9;
pub(super) const REGISTERS: usize = 10;

// FLAGS: GNU's two locals and the stage. write-region runs this inside its
// save-restriction extent (fileio.rs), widened for a nil START.
const FORCE_RAW_TEXT: i64 = 1;
const USING_DEFAULT: i64 = 2;
const STAGE: i64 = 4;

/// Where the walk resumes.
#[derive(Clone, Copy)]
enum Stage {
    /// VAL holds `find-operation-coding-system''s answer.
    Alist = 1,
    /// CHECK_CODING_SYSTEM (VAL) and the raw-text type test.
    Check,
    /// `select-safe-coding-system-function'.
    Select,
    /// VAL holds that function's answer.
    Selected,
    /// EOL inheritance from the default `buffer-file-coding-system'.
    Inherit,
    /// The final `coding_inherit_eol_type (val, Qnil)' and setup.
    Finish,
}

fn flags(r: &[Value]) -> i64 { r[FLAGS].integer_value().expect("flags register") }
fn set(r: &mut [Value], bits: i64) { r[FLAGS] = Value::integer(flags(r) | bits).unwrap(); }
fn at(r: &mut [Value], stage: Stage) {
    let rest = flags(r) % STAGE;
    r[FLAGS] = Value::integer(rest + stage as i64 * STAGE).unwrap();
}

/// Start the choice for registers R (FLAGS zero).
pub(super) fn choose(cx: &mut Leaf<'_>, mut r: Vec<Value>) -> Next {
    debug_assert_eq!(r.len(), REGISTERS);
    // fileio.c:5355: an auto-save other than into the visited file writes
    // utf-8-emacs with Unix line ends.
    if super::fileio::auto_saving(cx)?.is_some() && !super::fileio::auto_saving_into_visited(cx)? {
        r[VAL] = cx.intern("utf-8-emacs-unix");
        at(&mut r, Stage::Finish);
        return advance(cx, r);
    }
    let csw = var(cx, "coding-system-for-write")?;
    if csw != Value::NIL {
        r[VAL] = csw;
        at(&mut r, Stage::Finish);
        if var(cx, "coding-system-require-warning")? != Value::NIL {
            if let Some(f) = select_function(cx)? {
                // Confirm that VAL can surely encode the current region.
                let t_val = cx.coding_list(&[Value::TRUE, csw]);
                let args = vec![r[START], r[END], t_val, Value::NIL, r[FILENAME]];
                return Ok(Step::Call { f, args, then: answered, saved: r });
            }
        }
        return advance(cx, r);
    }
    // A local `buffer-file-coding-system' means the file was read with that
    // conversion or the user chose it: keep it even in a unibyte buffer.
    let symbol = cx.intern("buffer-file-coding-system");
    let buffer = cx.current_buffer()?;
    let mut val = var(cx, "buffer-file-coding-system")?;
    if val == Value::NIL || !cx.has_buffer_local(symbol, buffer)? {
        val = Value::NIL;
        if cx.buffer_shape()?.unibyte { set(&mut r, FORCE_RAW_TEXT); }
    }
    r[VAL] = val;
    at(&mut r, Stage::Alist);
    if val != Value::NIL {
        // The alist answer is ignored: Stage::Alist sees VAL already chosen.
        at(&mut r, Stage::Check);
        return advance(cx, r);
    }
    let operation = cx.intern("write-region");
    let args = [operation, r[START], r[END], r[FILENAME], r[APPEND], r[VISIT], r[LOCKNAME]];
    match cx.find_operation_coding(&args)? {
        Ok(pair) => { r[VAL] = pair; advance(cx, r) }
        Err(function) => {
            let list = cx.coding_list(&args);
            Ok(Step::Call { f: function, args: vec![list], then: operation_answered, saved: r })
        }
    }
}

fn operation_answered(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let mut r = saved.to_vec();
    r[VAL] = cx.operation_coding_pair(value)?;
    advance(cx, r)
}

fn answered(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let mut r = saved.to_vec();
    r[VAL] = value;
    advance(cx, r)
}

fn realized(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    advance(cx, saved.to_vec())
}

/// Run the walk from the stage in FLAGS until it calls Lisp or finishes.
fn advance(cx: &mut Leaf<'_>, mut r: Vec<Value>) -> Next {
    loop {
        let stage = flags(&r) / STAGE;
        let bits = flags(&r) % STAGE;
        let force_raw = bits & FORCE_RAW_TEXT != 0;
        match stage {
            s if s == Stage::Alist as i64 => {
                let pair = r[VAL];
                r[VAL] = match cx.pair(pair) { Some((_, cdr)) => cdr, None => Value::NIL };
                if r[VAL] == Value::NIL {
                    // Not decided: the current buffer's value.
                    r[VAL] = var(cx, "buffer-file-coding-system")?;
                    set(&mut r, USING_DEFAULT);
                }
                at(&mut r, Stage::Check);
            }
            s if s == Stage::Check as i64 => {
                if r[VAL] != Value::NIL && !force_raw {
                    let spec = match need(cx, r[VAL])? { Ok(spec) => spec, Err(name) => return realize(cx, name, r) };
                    if coding_type(cx, spec)? == cx.intern("raw-text") { set(&mut r, FORCE_RAW_TEXT); }
                }
                at(&mut r, Stage::Select);
            }
            s if s == Stage::Select as i64 => {
                at(&mut r, Stage::Inherit);
                if !force_raw {
                    if let Some(f) = select_function(cx)? {
                        at(&mut r, Stage::Selected);
                        let args = vec![r[START], r[END], r[VAL], Value::NIL, r[FILENAME]];
                        return Ok(Step::Call { f, args, then: answered, saved: r });
                    }
                }
            }
            s if s == Stage::Selected as i64 => {
                // The function is out of our control: refuse a bogus value.
                if r[VAL] != Value::NIL {
                    if let Err(name) = need(cx, r[VAL])? { return realize(cx, name, r); }
                }
                at(&mut r, Stage::Inherit);
            }
            s if s == Stage::Inherit as i64 => {
                if bits & USING_DEFAULT == 0 {
                    let symbol = cx.intern("buffer-file-coding-system");
                    let dflt = super::vars::default_value(cx, symbol)?.unwrap_or(Value::NIL);
                    if dflt != Value::NIL {
                        match inherit_eol(cx, r[VAL], dflt)? { Ok(val) => r[VAL] = val, Err(name) => return realize(cx, name, r) }
                    }
                }
                // Not encoding: raw-text or the subsidiary with VAL's EOL.
                if force_raw {
                    match raw_text_coding_system(cx, r[VAL])? { Ok(val) => r[VAL] = val, Err(name) => return realize(cx, name, r) }
                }
                at(&mut r, Stage::Finish);
            }
            s if s == Stage::Finish as i64 => {
                let val = match inherit_eol(cx, r[VAL], Value::NIL)? { Ok(val) => val, Err(name) => return realize(cx, name, r) };
                // setup_coding_system: CHECK_CODING_SYSTEM_GET_ID.
                if let Err(name) = need(cx, val)? { return realize(cx, name, r); }
                r[VAL] = val;
                return write_chosen(cx, &r);
            }
            _ => unreachable!("write-region coding stage {stage}"),
        }
    }
}

/// `Ffboundp (Vselect_safe_coding_system_function)': the function to call,
/// if any. Ffboundp signals for a non-symbol value.
fn select_function(cx: &mut Leaf<'_>) -> std::result::Result<Option<Value>, Fault> {
    let f = var(cx, "select-safe-coding-system-function")?;
    if cx.kind(f) != Kind::Symbol { return Err(Fault::Wrong("symbolp", f)); }
    Ok((f != Value::NIL && cx.fboundp(f)).then_some(f))
}

/// Realize NAME's define form (Fcheck_coding_system), then rerun the stage.
fn realize(cx: &mut Leaf<'_>, name: Value, r: Vec<Value>) -> Next {
    super::coding::realize(cx, name, r, realized)
}

/// CHECK_CODING_SYSTEM (NAME): Ok(spec), or Err(NAME) when its define form
/// must be evaluated first. Otherwise Fcheck_coding_system's signals.
pub(super) fn need(cx: &mut Leaf<'_>, name: Value) -> std::result::Result<std::result::Result<Value, Value>, Fault> {
    if cx.kind(name) != Kind::Symbol { return Err(Fault::Wrong("symbolp", name)); }
    let spec = cx.coding_spec(name)?;
    if spec != Value::NIL { return Ok(Ok(spec)); }
    if name != Value::NIL && cx.coding_define_form(name, false)? != Value::NIL { return Ok(Err(name)); }
    let condition = cx.intern("coding-system-error");
    let data = cx.cons(name, Value::NIL);
    Err(Fault::Signal(condition, data))
}

fn coding_type(cx: &mut Leaf<'_>, spec: Value) -> std::result::Result<Value, Fault> {
    let attrs = cx.vector_get(spec, Value::integer(0).unwrap())?;
    cx.vector_get(attrs, Value::integer(3).unwrap())
}

fn eol_of(cx: &mut Leaf<'_>, spec: Value) -> std::result::Result<Value, Fault> {
    cx.vector_get(spec, Value::integer(2).unwrap())
}

/// The subsidiary of SPEC's EOL vector for EOL `unix', `dos' or `mac'.
fn subsidiary(cx: &mut Leaf<'_>, eols: Value, eol: Value) -> std::result::Result<Option<Value>, Fault> {
    let index = ["unix", "dos", "mac"].iter().position(|name| eol == cx.intern(name));
    index.map(|i| cx.vector_get(eols, Value::integer(i as i64).unwrap())).transpose()
}

/// `Qraw_text'. Bootstrap regime: before mule-conf.el defines `raw-text', the
/// registry holds only syms_of_coding's systems, and `no-conversion' (type
/// raw-text, EOL unix) stands in for it: it encodes exactly as raw-text-unix.
fn raw_text(cx: &mut Leaf<'_>) -> std::result::Result<Value, Fault> {
    let raw = cx.intern("raw-text");
    if cx.coding_p(raw)? != Value::NIL { return Ok(raw); }
    Ok(cx.intern("no-conversion"))
}

/// coding.c coding_inherit_eol_type: VAL (nil meaning raw-text) when it
/// specifies its EOL, else its subsidiary for PARENT's EOL (unix when
/// PARENT is nil or undecided).
fn inherit_eol(cx: &mut Leaf<'_>, val: Value, parent: Value) -> std::result::Result<std::result::Result<Value, Value>, Fault> {
    let val = if val == Value::NIL { raw_text(cx)? } else { val };
    let spec = match need(cx, val)? { Ok(spec) => spec, Err(name) => return Ok(Err(name)) };
    let eols = eol_of(cx, spec)?;
    if cx.kind(eols) != Kind::Vector { return Ok(Ok(val)); }
    let mut eol = cx.intern("unix");
    if parent != Value::NIL {
        let parent_spec = match need(cx, parent)? { Ok(spec) => spec, Err(name) => return Ok(Err(name)) };
        let parent_eol = eol_of(cx, parent_spec)?;
        if cx.kind(parent_eol) != Kind::Vector { eol = parent_eol; }
    }
    Ok(Ok(subsidiary(cx, eols, eol)?.unwrap_or(val)))
}

/// coding.c raw_text_coding_system: VAL when it is of type raw-text, else
/// the raw-text subsidiary with VAL's EOL (raw-text itself when VAL's is
/// undecided). GNU reads VAL's spec without CHECK_CODING_SYSTEM; a lazy one
/// is realized rather than read as missing.
pub(super) fn raw_text_coding_system(cx: &mut Leaf<'_>, val: Value) -> std::result::Result<std::result::Result<Value, Value>, Fault> {
    let raw = raw_text(cx)?;
    if val == Value::NIL { return Ok(Ok(raw)); }
    let spec = match need(cx, val)? { Ok(spec) => spec, Err(name) => return Ok(Err(name)) };
    if coding_type(cx, spec)? == cx.intern("raw-text") { return Ok(Ok(val)); }
    let eol = eol_of(cx, spec)?;
    if cx.kind(eol) == Kind::Vector { return Ok(Ok(raw)); }
    let raw_spec = match need(cx, raw)? { Ok(spec) => spec, Err(name) => return Ok(Err(name)) };
    let raw_eols = eol_of(cx, raw_spec)?;
    if cx.kind(raw_eols) != Kind::Vector { return Ok(Ok(raw)); }
    // EQ (eol_type, Qunix) / Qdos / otherwise mac.
    let eol = if eol == cx.intern("unix") || eol == cx.intern("dos") { eol } else { cx.intern("mac") };
    Ok(Ok(subsidiary(cx, raw_eols, eol)?.unwrap_or(raw)))
}
