//! Forward list-scan spans, using the scanner's existing suspension states.
use crate::syntax_scan::{Forw, ForwAt, Halt, Pos, R, Scan, ScanAt, Scanner, Text};

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
        Self {
            c: scan.c,
            syntax: scan.syntax,
            code: scan.code,
            style: scan.comstyle,
            nested: scan.comnested,
        }
    }

    fn apply(self, scan: &mut Scan) {
        (
            scan.c,
            scan.syntax,
            scan.code,
            scan.comstyle,
            scan.comnested,
        ) = (self.c, self.syntax, self.code, self.style, self.nested);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
enum Pending {
    Top,
    Stepped(Head),
    Dispatch(Head),
    Symbol,
    Comment {
        style: i32,
        nested: bool,
        body: Option<Comment>,
    },
    String {
        terminator: u32,
        code: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
pub(crate) struct Key {
    pending: Pending,
    depth: i64,
    math: bool,
    ignore_comments: bool,
    multibyte_symbol: bool,
}

#[derive(Clone, Copy, Debug, romacs_snapshot::Snapshot)]
pub(crate) struct Span {
    exit: Key,
    /// Where the scan last read at its floor, relative to the span start.
    /// None if it kept the position it entered with.
    last_good: Option<u32>,
}

impl Key {
    pub(crate) fn of(scan: &Scan, ignore_comments: bool, multibyte_symbol: bool) -> Option<Self> {
        let pending = match scan.at {
            ScanAt::Top => Pending::Top,
            ScanAt::Stepped => Pending::Stepped(Head::of(scan)),
            ScanAt::Dispatch => Pending::Dispatch(Head::of(scan)),
            ScanAt::Symbol => Pending::Symbol,
            ScanAt::Comment => Pending::Comment {
                style: scan.comstyle,
                nested: scan.comnested,
                body: scan.forw.map(|f| Comment {
                    at: f.at,
                    nesting: f.nesting,
                    syntax: f.syntax,
                    code: f.code,
                    prev_syntax: f.prev_syntax,
                }),
            },
            ScanAt::String => Pending::String {
                terminator: scan.c,
                code: scan.code,
            },
            ScanAt::Decode => return None,
        };
        Some(Self {
            pending,
            depth: scan.depth,
            math: scan.mathexit,
            ignore_comments,
            multibyte_symbol,
        })
    }

    fn scan(self, from: Pos, last_good: i64) -> Scan {
        let mut scan = Scan {
            from,
            depth: self.depth,
            mathexit: self.math,
            last_good,
            count: i64::MAX,
            ..Scan::default()
        };
        match self.pending {
            Pending::Top => {}
            Pending::Stepped(head) | Pending::Dispatch(head) => {
                scan.at = if matches!(self.pending, Pending::Stepped(_)) {
                    ScanAt::Stepped
                } else {
                    ScanAt::Dispatch
                };
                head.apply(&mut scan);
            }
            Pending::Symbol => scan.at = ScanAt::Symbol,
            Pending::Comment {
                style,
                nested,
                body,
            } => {
                (scan.at, scan.comstyle, scan.comnested) = (ScanAt::Comment, style, nested);
                scan.forw = body.map(|f| Forw {
                    at: f.at,
                    from,
                    nesting: f.nesting,
                    syntax: f.syntax,
                    code: f.code,
                    prev_syntax: f.prev_syntax,
                });
            }
            Pending::String { terminator, code } => {
                (scan.at, scan.c, scan.code) = (ScanAt::String, terminator, code)
            }
        }
        scan
    }
}

pub(crate) fn summarize<T: Text>(
    scanner: &mut Scanner<'_, T>,
    start: Pos,
    end: i64,
    key: Key,
) -> R<Option<Span>> {
    let original = scanner.options.propertize_done;
    // The caller has already supplied the syntax properties. Use the same
    // resume boundary as propertization without performing any Lisp call.
    scanner.options.propertize_done = Some(end);
    let result = scanner.lists(key.scan(start, -1), true);
    scanner.options.propertize_done = original;
    let Err(stopped) = result else {
        return Ok(None);
    };
    let scan = match stopped.halt {
        Halt::Poll => return Err(Halt::Poll),
        Halt::Propertize(_, position) if position == end => stopped.suspended,
        _ => None,
    };
    Ok(scan.and_then(|scan| span(start, key, &scan)))
}

fn span(start: Pos, key: Key, scan: &Scan) -> Option<Span> {
    let exit = Key::of(scan, key.ignore_comments, key.multibyte_symbol)?;
    let last_good = if scan.last_good == -1 {
        None
    } else {
        Some(u32::try_from(scan.last_good - start.char).ok()?)
    };
    Some(Span { exit, last_good })
}

impl Span {
    pub(crate) fn exit(self) -> Key {
        self.exit
    }

    pub(crate) fn then(self, next: Self, offset: u32) -> Option<Self> {
        let last_good = match next.last_good {
            Some(last) => Some(last.checked_add(offset)?),
            None => self.last_good,
        };
        Some(Self {
            last_good,
            exit: next.exit,
        })
    }

    pub(crate) fn apply(self, scan: &mut Scan, start: Pos, end: Pos) {
        let last_good = self
            .last_good
            .map_or(scan.last_good, |offset| start.char + i64::from(offset));
        *scan = self.exit.scan(end, last_good);
    }
}
