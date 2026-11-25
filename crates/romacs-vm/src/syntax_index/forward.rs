//! Forward list scans folded over the index: the scanner runs one loop,
//! and at each leaf boundary it reaches it folds the largest node there
//! whose summary qualifies (`crate::syntax_summary::lists`):
//! - an excursion, where the scan's depth stays at 1 or more across it;
//! - an exact entry, where the span makes fewer count events than the
//!   scan has left and never goes below its floor.
//!
//! Otherwise the scanner reads the leaf with its live count, floor and
//! `last_good', and the loop consults the index again at the leaf's end.
//!
//! A leaf's summary is made when the scan reaches it, its read; an inner
//! node's is composed only from what its children keep. Only text wholly
//! before the accessible end and, where `syntax-table' properties are
//! honoured, wholly below `syntax-propertize--done' folds: the scanner
//! stops for `internal--syntax-propertize' where GNU's does, and after
//! the call the scan folds again up to the new frontier. The loop polls
//! for quit on entry and after every fold, a stop for a poll keeping the
//! scan where its last fold or read left it; the scanner reading a leaf
//! polls as it always does, and a summary's scan, at most a leaf or two,
//! does not.
use super::*;
use crate::syntax_scan::{Halt, Pos, Scan, Stopped};
use crate::syntax_summary::lists::Flags;

/// A forward list scan's end: its value, or its stop.
type Run = Result<Option<i64>, Stopped<Box<Scan>>>;

/// What one run of the loop did, for the stats: the spans it folded, and
/// whether it reached a leaf the propertize frontier lies inside.
#[derive(Default)]
struct Segment {
    folds: u64,
    frontier: bool,
}

/// A kind of forward list span summary, as a node keeps it.
trait ListSpan: Copy {
    type Key: Copy + PartialEq;
    /// Its kept summary for KEY in ENTRY, made under context slot CONTEXT.
    fn kept(entry: &Entry, context: u8, key: &Self::Key) -> Option<Option<Self>>;
    fn entry(context: u8, key: Self::Key, span: Option<Self>) -> Entry;
    fn build<T: Text>(scanner: &Scanner<'_, T>, start: Pos, end: i64, key: Self::Key) -> Option<Self>;
    fn exit(self) -> Self::Key;
    /// It, then NEXT, which starts OFFSET characters on.
    fn then(self, next: Self, offset: u32) -> Option<Self>;
}

impl ListSpan for Excursion {
    type Key = Mode;
    fn kept(entry: &Entry, context: u8, key: &Mode) -> Option<Option<Self>> {
        match entry {
            Entry::Excursion(owner, kept, span) if *owner == context && kept == key => Some(*span),
            _ => None,
        }
    }
    fn entry(context: u8, key: Mode, span: Option<Self>) -> Entry {
        Entry::Excursion(context, key, span)
    }
    fn build<T: Text>(scanner: &Scanner<'_, T>, start: Pos, end: i64, key: Mode) -> Option<Self> {
        list_summary::excursion(scanner, start, end, key)
    }
    fn exit(self) -> Mode {
        Excursion::exit(self)
    }
    fn then(self, next: Self, _offset: u32) -> Option<Self> {
        Some(Excursion::then(self, next))
    }
}

impl ListSpan for Exact {
    type Key = list_summary::Key;
    fn kept(entry: &Entry, context: u8, key: &Self::Key) -> Option<Option<Self>> {
        match entry {
            Entry::Exact(owner, kept, span) if *owner == context && kept == key => Some(*span),
            _ => None,
        }
    }
    fn entry(context: u8, key: Self::Key, span: Option<Self>) -> Entry {
        Entry::Exact(context, key, span)
    }
    fn build<T: Text>(scanner: &Scanner<'_, T>, start: Pos, end: i64, key: Self::Key) -> Option<Self> {
        list_summary::exact(scanner, start, end, key)
    }
    fn exit(self) -> Self::Key {
        Exact::exit(self)
    }
    fn then(self, next: Self, offset: u32) -> Option<Self> {
        Exact::then(self, next, offset)
    }
}

impl SyntaxIndex {
    /// `scan_lists` from RESUME, where a stop for Lisp or a quit poll left
    /// it, else from FROM0 with COUNT and DEPTH, in a text of CHARS
    /// characters under CONTEXT: forward, folded over the index; backward,
    /// the scanner's. Its value and stops are the scanner's.
    pub(crate) fn scan_lists<T: Text>(&mut self, scanner: &mut Scanner<'_, T>, chars: usize, context: Context,
                                      resume: Option<Scan>, (from0, count, depth): (i64, i64, i64), sexpflag: bool) -> Run {
        if scanner.text.stop_due() {
            self.stats.list_declined[Decline::Quit as usize] += 1;
            return Err(Stopped { halt: Halt::Poll, suspended: resume.map(Box::new) });
        }
        let scan = match resume {
            Some(scan) => scan,
            None => scanner.start_lists(from0, count, depth)?,
        };
        if scan.count <= 0 {
            return scanner.lists(scan, sexpflag);
        }
        // A tree that describes the text gives the leaf ends a read stops
        // at; `prepare' waits until the scan reaches one, so that a scan
        // ending in its first leaf pays nothing more.
        let slot = if self.root.as_ref().is_some_and(|root| root.chars == chars) {
            None
        } else {
            Some(self.prepare(scanner, chars, context.clone()))
        };
        let mut segment = Segment::default();
        let run = self.drive(scanner, (chars, context, slot), scan, sexpflag, &mut segment);
        let decline = if segment.folds > 0 {
            None
        } else if matches!(run, Err(Stopped { halt: Halt::Poll, .. })) {
            Some(Decline::Quit)
        } else if segment.frontier {
            Some(Decline::Properties)
        } else {
            Some(Decline::Short)
        };
        match decline {
            None => self.stats.list_served += 1,
            Some(decline) => self.stats.list_declined[decline as usize] += 1,
        }
        run
    }

    /// The loop: fold at each leaf boundary, else read to the next one.
    /// The tree is prepared for CONTEXT at the first boundary, unless SLOT
    /// says it was.
    fn drive<T: Text>(&mut self, scanner: &mut Scanner<'_, T>, (chars, context, mut slot): (usize, Context, Option<u8>),
                      mut scan: Scan, sexpflag: bool, segment: &mut Segment) -> Run {
        let (done, zv) = (scanner.options.propertize_done, scanner.zv.char);
        let flags = (sexpflag, scanner.options.ignore_comments, scanner.options.multibyte_syntax_as_symbol);
        // A folded leaf ends before ZV, and below the frontier.
        let before_zv = offset(zv).saturating_sub(1);
        let limit = done.map_or(before_zv, |done| before_zv.min(usize::try_from(done - 1).unwrap_or(0)));
        loop {
            let at = offset(scan.reached().char);
            let Some(root) = self.root.as_ref() else { return scanner.lists(scan, sexpflag) };
            let (leaf_start, leaf_chars, _) = locate(root, at, 0);
            let to = locate(root, limit, 0).0;
            let next = leaf_start + leaf_chars;
            if leaf_start == at && at < to {
                let Some(context) = slot else {
                    // The cuts it checks may move: look again.
                    slot = Some(self.prepare(scanner, chars, context.clone()));
                    continue;
                };
                let (reached, polled) = self.fold_from(scanner, context, &mut scan, (at, to), flags, segment);
                // As the scanner's update where the last span ended: the
                // frontier first, then the poll.
                let here = scan.reached().char;
                if let Some(done) = done && reached > at && done <= here && done < zv {
                    return Err(Stopped { halt: Halt::Propertize(zv.min(here + 1), here), suspended: Some(Box::new(scan)) });
                }
                if polled {
                    return Err(Stopped { halt: Halt::Poll, suspended: Some(Box::new(scan)) });
                }
                if reached > at {
                    continue;
                }
            } else if leaf_start == at && limit < before_zv && next > limit {
                segment.frontier = true;
            }
            if position(next) >= zv {
                return scanner.lists(scan, sexpflag);
            }
            // Read to the leaf's end, stopping there as for propertization.
            scanner.options.propertize_done = Some(done.map_or(position(next), |done| done.min(position(next))));
            let run = scanner.lists(scan, sexpflag);
            scanner.options.propertize_done = done;
            match run {
                Err(Stopped { halt: Halt::Propertize(_, charpos), suspended: Some(suspended) })
                    if !done.is_some_and(|done| done <= charpos && done < zv) => scan = *suspended,
                run => return run,
            }
        }
    }

    /// Fold from offset AT, a leaf boundary, towards TO, another: the
    /// offset reached, and whether a poll came due after a fold, which
    /// ends the folding there.
    fn fold_from<T: Text>(&mut self, scanner: &Scanner<'_, T>, context: u8, scan: &mut Scan, range: (usize, usize),
                          flags: Flags, segment: &mut Segment) -> (usize, bool) {
        let Self { root: Some(root), stats, .. } = self else { unreachable!("prepared") };
        let mut polled = false;
        let reached = Self::fold(root, 0, range, &mut |node, start| {
            if polled {
                return false;
            }
            let folded = Self::fold_node(stats, scanner, (node, start, context), scan, flags);
            if folded {
                segment.folds += 1;
                polled = scanner.text.stop_due();
            }
            folded
        });
        (reached, polled)
    }

    /// Carry SCAN across NODE, at offset START, on its excursion or its
    /// exact summary, where one qualifies: whether it did.
    fn fold_node<T: Text>(stats: &mut Stats, scanner: &Scanner<'_, T>, (node, start, context): (&mut Node, usize, u8),
                          scan: &mut Scan, flags: Flags) -> bool {
        let Some(mode) = Mode::of(scan, flags) else { return false };
        let chars = node.chars;
        let (from, end) = (scanner.text.pos(position(start)), scanner.text.pos(position(start + chars)));
        if scan.depth >= 1
            && let Some(Some(span)) = Self::list_span::<Excursion, T>(stats, scanner, (&mut *node, start, context), mode, true)
            && span.fits(scan.depth)
        {
            span.apply(scan, end);
            stats.folds[0] += 1;
            stats.skipped += u64::try_from(chars).unwrap_or(u64::MAX);
            return true;
        }
        let key = list_summary::Key::of(scan, mode);
        if let Some(Some(span)) = Self::list_span::<Exact, T>(stats, scanner, (node, start, context), key, true)
            && span.fits(scan.count)
        {
            span.apply(scan, from, end);
            stats.folds[1] += 1;
            stats.skipped += u64::try_from(chars).unwrap_or(u64::MAX);
            return true;
        }
        false
    }

    /// NODE's summary of kind S entered at KEY: a kept one, a leaf's made
    /// now where BUILD, an inner node's composed from what its children
    /// keep. Some(None) where the span has none; None where it is not
    /// known without reading a leaf.
    fn list_span<S: ListSpan, T: Text>(stats: &mut Stats, scanner: &Scanner<'_, T>, (node, start, context): (&mut Node, usize, u8),
                                       key: S::Key, build: bool) -> Option<Option<S>> {
        if let Some((at, span)) = node.spans.iter().enumerate().find_map(|(at, entry)| S::kept(entry, context, &key).map(|span| (at, span))) {
            node.hit(at);
            return Some(span);
        }
        let span = if node.is_leaf() {
            if node.unsafe_ends & (1 << context) != 0 {
                return Some(None);
            }
            if !build {
                return None;
            }
            stats.leaf_scans += 1;
            S::build(scanner, scanner.text.pos(position(start)), position(start + node.chars), key)
        } else {
            Self::compose::<S, T>(stats, scanner, (&mut *node, start, context), key)?
        };
        if let Some(kind) = node.remember(S::entry(context, key, span)) {
            stats.evictions[kind] += 1;
        }
        Some(span)
    }

    /// The summaries NODE's children keep as one, each next child entered
    /// at the key the one before leaves.
    fn compose<S: ListSpan, T: Text>(stats: &mut Stats, scanner: &Scanner<'_, T>, (node, start, context): (&mut Node, usize, u8),
                                     mut key: S::Key) -> Option<Option<S>> {
        let (mut at, mut whole) = (start, None::<S>);
        for child in &mut node.children {
            let chars = child.chars;
            let Some(next) = Self::list_span::<S, T>(stats, scanner, (child, at, context), key, false)? else { return Some(None) };
            whole = Some(match whole {
                None => next,
                Some(so_far) => {
                    stats.composed += 1;
                    let Some(both) = u32::try_from(at - start).ok().and_then(|offset| so_far.then(next, offset)) else { return Some(None) };
                    both
                }
            });
            key = next.exit();
            at += chars;
        }
        Some(whole)
    }
}
