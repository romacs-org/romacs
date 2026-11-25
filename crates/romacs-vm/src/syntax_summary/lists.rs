//! Forward list-scan spans, using the scanner's existing suspension states.
//!
//! A span has two kinds of summary, each the list scanner run once across
//! it from a continuation:
//! - an [`Excursion`] is keyed by the lexical [`Mode`] alone and run deep
//!   enough that it never reaches depth 0: its depth change and how far it
//!   falls below its entry hold for any scan entered at a depth that stays
//!   at 1 or more throughout, which makes no count event, meets no floor
//!   and moves no `last_good';
//! - an [`Exact`] summary is keyed by the exact entry ([`Key`]: depth and
//!   floor too) and run with an unbounded count: it holds the count events
//!   the span makes and where it last read at its floor, for any scan
//!   whose count is larger.
//!
//! A span with neither (it ends where no key resumes, reaches the end of
//! the text, or, exact, goes below its floor) is read by the scanner.
use crate::syntax_scan::{Forw, ForwAt, Halt, Options, Passed, Pos, Scan, ScanAt, Scanner, Stopped, Text};

#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
struct Comment {
    at: ForwAt,
    nesting: i64,
    syntax: i32,
    code: i32,
    prev_syntax: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
struct Head {
    c: u32,
    syntax: i32,
    code: i32,
    style: i32,
    nested: bool,
}

impl Head {
    fn of(scan: &Scan) -> Self {
        Self { c: scan.c, syntax: scan.syntax, code: scan.code, style: scan.comstyle, nested: scan.comnested }
    }

    fn apply(self, scan: &mut Scan) {
        (scan.c, scan.syntax, scan.code, scan.comstyle, scan.comnested) = (self.c, self.syntax, self.code, self.style, self.nested);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
enum Pending {
    Top,
    Stepped(Head),
    Dispatch(Head),
    Symbol,
    Comment { style: i32, nested: bool, body: Option<Comment> },
    String { terminator: u32, code: i32 },
}

/// What decides how a forward list scan reads on, besides its depth, its
/// floor and its count: the step pending, the math phase, and the scan's
/// options (`sexpflag', `parse-sexp-ignore-comments',
/// `multibyte-syntax-as-symbol').
#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
pub(crate) struct Mode {
    pending: Pending,
    math: bool,
    sexpflag: bool,
    ignore_comments: bool,
    multibyte_symbol: bool,
}

/// A scan's options, as a mode carries them: `sexpflag',
/// `parse-sexp-ignore-comments', `multibyte-syntax-as-symbol'.
pub(crate) type Flags = (bool, bool, bool);

impl Mode {
    /// The mode of SCAN, under FLAGS; none inside `back_comment`'s decode.
    pub(crate) fn of(scan: &Scan, (sexpflag, ignore_comments, multibyte_symbol): Flags) -> Option<Self> {
        let pending = match scan.at {
            ScanAt::Top => Pending::Top,
            ScanAt::Stepped => Pending::Stepped(Head::of(scan)),
            ScanAt::Dispatch => Pending::Dispatch(Head::of(scan)),
            ScanAt::Symbol => Pending::Symbol,
            ScanAt::Comment => Pending::Comment {
                style: scan.comstyle,
                nested: scan.comnested,
                body: scan.forw.map(|f| Comment { at: f.at, nesting: f.nesting, syntax: f.syntax, code: f.code, prev_syntax: f.prev_syntax }),
            },
            ScanAt::String => Pending::String { terminator: scan.c, code: scan.code },
            ScanAt::Decode => return None,
        };
        Some(Self { pending, math: scan.mathexit, sexpflag, ignore_comments, multibyte_symbol })
    }

    /// A scan in this mode at FROM, its other fields DEPTH, FLOOR,
    /// LAST_GOOD and COUNT.
    fn scan(self, from: Pos, (depth, floor, last_good, count): (i64, i64, i64, i64)) -> Scan {
        let mut scan = Scan { from, depth, min_depth: floor, mathexit: self.math, last_good, count, low: depth, ..Scan::default() };
        match self.pending {
            Pending::Top => {}
            Pending::Stepped(head) => {
                scan.at = ScanAt::Stepped;
                head.apply(&mut scan);
            }
            Pending::Dispatch(head) => {
                scan.at = ScanAt::Dispatch;
                head.apply(&mut scan);
            }
            Pending::Symbol => scan.at = ScanAt::Symbol,
            Pending::Comment { style, nested, body } => {
                (scan.at, scan.comstyle, scan.comnested) = (ScanAt::Comment, style, nested);
                scan.forw = body.map(|f| Forw { at: f.at, from, nesting: f.nesting, syntax: f.syntax, code: f.code, prev_syntax: f.prev_syntax });
            }
            Pending::String { terminator, code } => (scan.at, scan.c, scan.code) = (ScanAt::String, terminator, code),
        }
        scan
    }
}

/// An exact entry: the mode, the depth and the floor (`min_depth').
#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
pub(crate) struct Key {
    mode: Mode,
    depth: i64,
    floor: i64,
}
impl Key {
    pub(crate) fn of(scan: &Scan, mode: Mode) -> Self {
        Self { mode, depth: scan.depth, floor: scan.min_depth }
    }
}

/// The depth an excursion's scan is entered at: no span comes back to 0.
const DEEP: i64 = 1 << 40;

/// A span entered in a mode at a depth that stays at 1 or more: the mode
/// it leaves, its depth change, and how far below its entry depth it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
pub(crate) struct Excursion {
    exit: Mode,
    rise: i64,
    fall: i64,
}

/// A span entered at an exact key: the key it leaves, the count events it
/// makes, and where it last read at its floor, from its start (none: it
/// keeps the scan's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
pub(crate) struct Exact {
    exit: Key,
    events: i64,
    last_good: Option<u32>,
}

/// A text whose scans never stop for a quit poll: a summary's scan, at
/// most a leaf or two, whose caller polls before and after it. A poll
/// inside it would have the caller make it again, and polls more frequent
/// than a summary's scan would never let one finish.
struct Unpolled<'a, T>(&'a T);
impl<T: Text> Text for Unpolled<'_, T> {
    fn char_at(&self, at: Pos) -> u32 { self.0.char_at(at) }
    fn inc(&self, at: Pos) -> Pos { self.0.inc(at) }
    fn dec(&self, at: Pos) -> Pos { self.0.dec(at) }
    fn pos(&self, charpos: i64) -> Pos { self.0.pos(charpos) }
    fn syntax(&self, at: Pos) -> i32 { self.0.syntax(at) }
    fn table_syntax(&self, c: u32) -> i32 { self.0.table_syntax(c) }
    fn pass(&self, at: Pos, limit: i64, plain: impl Fn(u8, i32) -> bool) -> Passed { self.0.pass(at, limit, plain) }
}

/// The scan SCAN from START to END (a character position, short of the
/// accessible end) where it stops there, the text past END unread; none
/// where it ends or overshoots first.
fn across<T: Text>(scanner: &Scanner<'_, T>, scan: Scan, end: i64, sexpflag: bool) -> Option<Scan> {
    // The caller has the text propertized past END: a stop for
    // propertization at END marks where the span ends, with no Lisp call.
    let options = Options { propertize_done: Some(end), ..scanner.options };
    let text = Unpolled(scanner.text);
    let mut quiet = Scanner { text: &text, begv: scanner.begv, zv: scanner.zv, options, ppss: scanner.ppss,
                              defun_start: scanner.defun_start, modiff: scanner.modiff };
    match quiet.lists(scan, sexpflag) {
        Err(Stopped { halt: Halt::Propertize(_, position), suspended }) if position == end => suspended.map(|scan| *scan),
        _ => None,
    }
}

/// The excursion of START..END entered in MODE.
pub(crate) fn excursion<T: Text>(scanner: &Scanner<'_, T>, start: Pos, end: i64, mode: Mode) -> Option<Excursion> {
    let scan = across(scanner, mode.scan(start, (DEEP, -DEEP, -1, i64::MAX)), end, mode.sexpflag)?;
    let flags = (mode.sexpflag, mode.ignore_comments, mode.multibyte_symbol);
    Mode::of(&scan, flags).map(|exit| Excursion { exit, rise: scan.depth - DEEP, fall: DEEP - scan.low })
}

/// The exact summary of START..END entered at KEY.
pub(crate) fn exact<T: Text>(scanner: &Scanner<'_, T>, start: Pos, end: i64, key: Key) -> Option<Exact> {
    let scan = across(scanner, key.mode.scan(start, (key.depth, key.floor, -1, i64::MAX)), end, key.mode.sexpflag)?;
    let flags = (key.mode.sexpflag, key.mode.ignore_comments, key.mode.multibyte_symbol);
    let mode = Mode::of(&scan, flags)?;
    let last_good = if scan.last_good == -1 { None } else { Some(u32::try_from(scan.last_good - start.char).ok()?) };
    Some(Exact { exit: Key::of(&scan, mode), events: i64::MAX - scan.count, last_good })
}

impl Excursion {
    pub(crate) fn exit(self) -> Mode {
        self.exit
    }
    /// Whether a scan at DEPTH crosses it at depth 1 or more throughout.
    pub(crate) fn fits(self, depth: i64) -> bool {
        depth >= 1 && depth - self.fall >= 1
    }
    /// It, then NEXT, entered in its exit mode.
    pub(crate) fn then(self, next: Self) -> Self {
        Self { exit: next.exit, rise: self.rise + next.rise, fall: self.fall.max(next.fall - self.rise) }
    }
    /// SCAN, which it `fits`, carried across it to END.
    pub(crate) fn apply(self, scan: &mut Scan, end: Pos) {
        let rest = (scan.depth + self.rise, scan.min_depth, scan.last_good, scan.count);
        *scan = self.exit.scan(end, rest);
    }
}

impl Exact {
    pub(crate) fn exit(self) -> Key {
        self.exit
    }
    /// Whether a scan with COUNT sexps left crosses it.
    pub(crate) fn fits(self, count: i64) -> bool {
        self.events < count
    }
    /// It, then NEXT, entered at its exit key OFFSET characters on.
    pub(crate) fn then(self, next: Self, offset: u32) -> Option<Self> {
        let last_good = match next.last_good {
            Some(last) => Some(last.checked_add(offset)?),
            None => self.last_good,
        };
        Some(Self { exit: next.exit, events: self.events.checked_add(next.events)?, last_good })
    }
    /// SCAN, which it `fits`, carried across it from START to END.
    pub(crate) fn apply(self, scan: &mut Scan, start: Pos, end: Pos) {
        let last_good = self.last_good.map_or(scan.last_good, |offset| start.char + i64::from(offset));
        let Key { mode, depth, floor } = self.exit;
        *scan = mode.scan(end, (depth, floor, last_good, scan.count - self.events));
    }
}
