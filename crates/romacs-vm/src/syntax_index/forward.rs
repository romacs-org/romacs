//! Forward list scans folded over the index: the scanner runs one loop,
//! and at each leaf boundary it reaches it folds the largest node there
//! whose summary qualifies (`crate::syntax_summary::lists`):
//! - an excursion, where the scan's depth stays at 1 or more across it;
//! - an exact entry, where the span makes fewer count events than the
//!   scan has left and never goes below its floor. An excursion that fits
//!   the entry's depth is one too, so exact entries compose over both.
//!
//! Otherwise the scanner reads the leaf with its live count, floor and
//! `last_good', and the loop consults the index again at the leaf's end.
//!
//! A leaf's summaries are made when the scan reaches it, by one scan that
//! is its read; an inner node's are composed only from what its children
//! keep. Only text wholly before the accessible end and, where
//! `syntax-table' properties are honoured, wholly below
//! `syntax-propertize--done' folds: the scanner stops for
//! `internal--syntax-propertize' where GNU's does, and after the call the
//! scan folds again up to the new frontier.
//!
//! The loop asks the quit probe on entry and after every fold; the scanner
//! reading a leaf polls as it always does, and a summary's scan, at most a
//! leaf or two, does not. Every probe is a commit point: no mutation of
//! the index is under way when one is asked, so what a run built before a
//! stop for a poll stays, a derived cache made from the real text, and the
//! scan goes on, if nothing quits, where its last fold or read left it.
use super::*;
use crate::syntax_scan::{Halt, Pos, Scan, Stopped};
use crate::syntax_summary::lists::{Flags, Key as ExactKey};

/// A forward list scan's end: its value, or its stop.
type Run = Result<Option<i64>, Stopped<Box<Scan>>>;

/// What one run of the loop did, for the stats: the spans it folded, and
/// whether it reached a leaf the propertize frontier lies inside.
#[derive(Default)]
struct Segment {
    folds: u64,
    frontier: bool,
}

/// Leaves a scan makes ready at once from where it is: what it costs the
/// partition stays within a few leaves of what it reads.
const READY: usize = 16;

/// How list entries are looked up and made in one run: its context slot.
#[derive(Clone, Copy)]
struct Look {
    context: u8,
}

impl SyntaxIndex {
    /// `scan_lists` from RESUME, where a stop for Lisp or a quit poll left
    /// it, else from FROM0 with COUNT and DEPTH, in a text of CHARS
    /// characters under CONTEXT: forward, folded over the index; backward,
    /// the scanner's. Its value and stops are the scanner's.
    pub(crate) fn scan_lists<T: Text>(&mut self, scanner: &mut Scanner<'_, T>, chars: usize, context: Context,
                                      resume: Option<Scan>, (from0, count, depth): (i64, i64, i64), sexpflag: bool) -> Run {
        // GNU's maybe_quit before the scan's setup, and the probe again
        // where a stop left it.
        if scanner.text.stop_uncounted() {
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
        let mut segment = Segment::default();
        let run = self.drive(scanner, (chars, context), scan, sexpflag, &mut segment);
        if matches!(run, Err(Stopped { halt: Halt::Poll, .. })) {
            self.stats.list_declined[Decline::Quit as usize] += 1;
        } else if segment.folds > 0 {
            self.stats.list_served += 1;
        } else {
            let decline = if segment.frontier { Decline::Properties } else { Decline::Short };
            self.stats.list_declined[decline as usize] += 1;
        }
        run
    }

    /// The loop: fold at each leaf boundary, else read to the next one.
    /// The tree is prepared for CONTEXT, and the leaves made ready, as the
    /// scan reaches them.
    fn drive<T: Text>(&mut self, scanner: &mut Scanner<'_, T>, (chars, context): (usize, Context),
                      mut scan: Scan, sexpflag: bool, segment: &mut Segment) -> Run {
        let (done, zv) = (scanner.options.propertize_done, scanner.zv.char);
        let flags = (sexpflag, scanner.options.ignore_comments, scanner.options.multibyte_syntax_as_symbol);
        // A folded leaf ends before ZV, and below the frontier.
        let before_zv = offset(zv).saturating_sub(1);
        let limit = done.map_or(before_zv, |done| before_zv.min(usize::try_from(done - 1).unwrap_or(0)));
        let mut slot = None;
        loop {
            let at = offset(scan.reached().char);
            // No tree yet is one raw leaf: `prepare' makes it.
            let (leaf_start, leaf_chars, raw) = if self.root.is_some() { self.leaf_at(at) } else { (at, chars.saturating_sub(at), true) };
            let next = leaf_start + leaf_chars;
            if position(leaf_start) < zv && (raw || (leaf_start == at && at < limit)) {
                let Some(context) = slot else {
                    slot = Some(self.prepare(chars, context.clone()));
                    continue;
                };
                // Cut, or checked for CONTEXT, where the scan is and a few
                // leaves on: look again.
                if (raw || self.pending(context, at).is_some_and(|pending| pending.start <= at))
                    && self.ready(scanner, context, (at, at + READY * self.leaf))
                {
                    continue;
                }
                let to = self.fold_end(context, at, limit);
                if !raw && at < to {
                    let (reached, polled) = self.fold_from(scanner, context, &mut scan, (at, to), flags, segment);
                    // As the scanner's update where the last span ended: the
                    // frontier first, then the poll.
                    if reached > at && let Err(halt) = scanner.update_forward(scan.reached().char) {
                        return Err(Stopped { halt, suspended: Some(Box::new(scan)) });
                    }
                    if polled {
                        return Err(Stopped { halt: Halt::Poll, suspended: Some(Box::new(scan)) });
                    }
                    if reached > at {
                        continue;
                    }
                }
            }
            if leaf_start == at && limit < before_zv && next > limit {
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
                    if scanner.update_forward(charpos).is_ok() => scan = *suspended,
                run => return run,
            }
        }
    }

    /// The leaf holding offset AT: its start, characters, and whether it
    /// is raw.
    fn leaf_at(&self, at: usize) -> (usize, usize, bool) {
        let Some(mut node) = self.root.as_ref() else { return (at, 0, false) };
        let mut start = 0;
        while !node.is_leaf() {
            let last = node.children.len() - 1;
            let mut i = 0;
            while i < last && at >= start + node.children[i].chars {
                start += node.children[i].chars;
                i += 1;
            }
            node = &node.children[i];
        }
        (start, node.chars, node.raw)
    }

    /// Where a fold from offset AT stops for CONTEXT: the start of the
    /// leaf LIMIT is in, or of the first leaf after AT not ready for it.
    fn fold_end(&self, context: u8, at: usize, limit: usize) -> usize {
        let to = self.leaf_at(limit).0;
        self.pending(context, at).map_or(to, |pending| to.min(pending.start))
    }

    /// Fold from offset AT, a leaf boundary, towards TO, another: the
    /// offset reached, and whether the quit probe asked to stop after a
    /// fold, which ends the folding there. A fold onto the propertize
    /// frontier ends it too, the probe not asked: the stop for Lisp comes
    /// first, as at the scanner's update.
    fn fold_from<T: Text>(&mut self, scanner: &mut Scanner<'_, T>, context: u8, scan: &mut Scan, range: (usize, usize),
                          flags: Flags, segment: &mut Segment) -> (usize, bool) {
        let Self { root: Some(root), stats, .. } = self else { unreachable!("prepared") };
        let look = Look { context };
        let (mut polled, mut ended) = (false, false);
        let reached = Self::fold(root, 0, range, &mut |node, start| {
            if ended {
                return false;
            }
            let folded = Self::fold_node(stats, look, &mut *scanner, (node, start), scan, flags);
            if folded {
                segment.folds += 1;
                ended = scanner.update_forward(scan.reached().char).is_err() || {
                    polled = scanner.text.stop_uncounted();
                    polled
                };
            }
            folded
        });
        (reached, polled)
    }

    /// Carry SCAN across NODE, at offset START, on its excursion or its
    /// exact summary, where one qualifies, a leaf's made now where it has
    /// none: whether it did.
    fn fold_node<T: Text>(stats: &mut Stats, look: Look, scanner: &mut Scanner<'_, T>, (node, start): (&mut Node, usize),
                          scan: &mut Scan, flags: Flags) -> bool {
        let Some(mode) = Mode::of(scan, flags) else { return false };
        let key = ExactKey::of(scan, mode);
        let chars = node.chars;
        let (from, end) = (scan.reached(), scanner.text.pos(position(start + chars)));
        let mut built = false;
        loop {
            if scan.depth >= 1
                && let Some(Some(span)) = Self::excursion(stats, look, &mut *node, mode)
                && span.fits(scan.depth)
            {
                span.apply(scan, end);
                stats.folds[0] += 1;
                break;
            }
            match Self::exact(stats, look, (&mut *node, start), key) {
                Some(Some(span)) if span.fits(scan.count) => {
                    span.apply(scan, from, end);
                    stats.folds[1] += 1;
                    break;
                }
                None if !built && node.is_leaf() => {
                    if !Self::build_leaf(stats, look, scanner, (&mut *node, from, end), key) {
                        return false;
                    }
                    built = true;
                }
                _ => return false,
            }
        }
        let chars = u64::try_from(chars).unwrap_or(u64::MAX);
        stats.skipped += chars;
        if built {
            stats.built += chars;
        }
        true
    }

    /// Make the summaries at KEY of a leaf from FROM to END by one scan: its
    /// exact one, and the excursion of KEY's mode where the scan is one and
    /// the leaf keeps none for that mode. False, nothing kept, where a quit
    /// came due during the scan.
    fn build_leaf<T: Text>(stats: &mut Stats, look: Look, scanner: &mut Scanner<'_, T>, (node, from, end): (&mut Node, Pos, Pos),
                           key: ExactKey) -> bool {
        let Some(made) = list_summary::summarize(scanner, from, end.char, key) else { return false };
        stats.leaf_scans += 1;
        let (exact, excursion) = made.map_or((None, None), |(exact, excursion)| (Some(exact), excursion));
        if let Some(excursion) = excursion
            && Self::excursion(stats, look, &mut *node, key.mode()).is_none()
        {
            Self::remember(stats, node, Entry::Excursion(look.context, key.mode(), Some(excursion)));
        }
        Self::remember(stats, node, Entry::Exact(look.context, key, exact));
        true
    }

    fn remember(stats: &mut Stats, node: &mut Node, entry: Entry) {
        if let Some(kind) = node.remember(entry) {
            stats.evictions[kind] += 1;
        }
    }

    /// NODE's excursion in MODE: a kept one, or an inner node's composed
    /// from what its children keep. Some(None) where the span has none;
    /// None where it is not known without reading a leaf.
    fn excursion(stats: &mut Stats, look: Look, node: &mut Node, mode: Mode) -> Option<Option<Excursion>> {
        let kept = node.spans.iter().position(|entry| {
            matches!(entry, Entry::Excursion(owner, kept, _) if *owner == look.context && *kept == mode)
        });
        if let Some(at) = kept {
            let at = node.hit(at);
            let Entry::Excursion(.., span) = &node.spans[at] else { unreachable!("an excursion") };
            return Some(*span);
        }
        if node.is_leaf() {
            return (node.unsafe_ends & (1 << look.context) != 0 && !node.raw).then_some(None);
        }
        let (mut entry, mut whole) = (mode, None::<Excursion>);
        for child in &mut node.children {
            let Some(next) = Self::excursion(stats, look, child, entry)? else { return Some(None) };
            whole = Some(whole.map_or(next, |so_far| {
                stats.composed += 1;
                so_far.then(next)
            }));
            entry = next.exit();
        }
        Self::remember(stats, node, Entry::Excursion(look.context, mode, whole));
        let Entry::Excursion(.., span) = &node.spans[0] else { unreachable!("remembered") };
        Some(*span)
    }

    /// NODE's exact summary at KEY: a kept one, its excursion in KEY's
    /// mode where that fits KEY's depth, or an inner node's composed from
    /// its children's. Some(None) where the span has none; None where it is
    /// not known without reading a leaf.
    fn exact(stats: &mut Stats, look: Look, (node, start): (&mut Node, usize), key: ExactKey) -> Option<Option<Exact>> {
        let kept = node.spans.iter().position(|entry| {
            matches!(entry, Entry::Exact(owner, kept, _) if *owner == look.context && *kept == key)
        });
        if let Some(at) = kept {
            let at = node.hit(at);
            let Entry::Exact(.., span) = &node.spans[at] else { unreachable!("an exact entry") };
            return Some(*span);
        }
        if let Some(Some(excursion)) = Self::kept_excursion(look, node, key.mode())
            && let Some(span) = excursion.at(key)
        {
            return Some(Some(span));
        }
        if node.is_leaf() {
            return (node.unsafe_ends & (1 << look.context) != 0 && !node.raw).then_some(None);
        }
        let (mut at, mut entry, mut whole) = (start, key, None::<Exact>);
        for child in &mut node.children {
            let chars = child.chars;
            let Some(next) = Self::exact(stats, look, (child, at), entry)? else { return Some(None) };
            whole = match whole {
                None => Some(next),
                Some(so_far) => {
                    stats.composed += 1;
                    u32::try_from(at - start).ok().and_then(|offset| so_far.then(next, offset))
                }
            };
            if whole.is_none() {
                return Some(None);
            }
            entry = next.exit();
            at += chars;
        }
        Self::remember(stats, node, Entry::Exact(look.context, key, whole));
        Some(whole)
    }

    /// NODE's kept excursion in MODE, looked at without moving it.
    fn kept_excursion(look: Look, node: &Node, mode: Mode) -> Option<Option<Excursion>> {
        node.spans.iter().find_map(|entry| match entry {
            Entry::Excursion(owner, kept, span) if *owner == look.context && *kept == mode => Some(*span),
            _ => None,
        })
    }
}
