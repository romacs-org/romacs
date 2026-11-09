//! Syntax state as a monoid: what `scan_sexps_forward` does to a span of
//! text, kept small enough that a parse state crosses the span without
//! reading it, and that the summaries of two adjacent spans compose into
//! the summary of both (Mytkowicz, Musuvathi and Schulte, "Data-Parallel
//! Finite-State Machines", ASPLOS 2014; Hillis and Steele, "Data Parallel
//! Algorithms", CACM 1986).
//!
//! The parse is still `crate::syntax_scan`'s. A summary is that scanner run
//! once over the span from a symbolic entry: the entry's open parens, its
//! current level's `last' and `prev' and its string or comment start are
//! sentinels, which the scanner only ever copies, so its output reads back
//! as positions in the span or references to the entry. What the span does
//! depends on the scanner's control state at its start, the [`Key`], so a
//! span has one summary per key it is entered in.
use crate::syntax_scan::{Continuation, ParseState, Pos, R, Scanner, Text};

/// GNU's levels in scan_sexps_forward: a stack this long stops growing.
pub(crate) const LEVELS: usize = 100;
/// Entry levels a span's scan is given to close.
const SEEDS: i64 = 48;
const TOP_LAST: i64 = -(SEEDS + 2);
const TOP_PREV: i64 = -(SEEDS + 3);
const COMSTR: i64 = -(SEEDS + 4);
/// The sentinel of the entry's Jth open paren, counted from the innermost.
fn seed(j: i64) -> i64 {
    -(2 + j)
}

/// The scanner's control state between two characters: everything that
/// decides how it reads what follows (`ParseState`'s control fields and
/// the continuation's `in_symbol`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    instring: i32,
    /// -1 outside a nestable comment, else its nesting.
    incomment: i64,
    comstyle: i32,
    quoted: bool,
    prev_syntax: i32,
    in_symbol: bool,
}
romacs_snapshot::snapshot_struct!(Key { instring, incomment, comstyle, quoted, prev_syntax, in_symbol });
impl Key {
    pub(crate) fn of(state: &ParseState) -> Self {
        Self {
            instring: state.instring,
            incomment: state.incomment,
            comstyle: state.comstyle,
            quoted: state.quoted,
            prev_syntax: state.prev_syntax,
            in_symbol: state.continuation.is_some_and(|c| c.in_symbol),
        }
    }
}

/// A level value a summary keeps: a position in the span (from its start),
/// or one of the entry state's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ref {
    Unset,
    At(u32),
    /// The entry's Jth open paren from the innermost.
    Open(u32),
    TopLast,
    TopPrev,
    ComStart,
}
romacs_snapshot::snapshot_enum!(Ref {
    Ref::Unset => (),
    Ref::At(at) => (at),
    Ref::Open(j) => (j),
    Ref::TopLast => (),
    Ref::TopPrev => (),
    Ref::ComStart => (),
});

/// What a span does to a parse state entered in some key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    exit: Key,
    /// Entry levels the span closes.
    pops: u32,
    /// How far above the entry depth it goes.
    peak: u32,
    /// The starts of the levels it leaves open below its last one: the
    /// level it ends at holds them, innermost last.
    opened: Vec<u32>,
    /// The `last' and `prev' of the level it ends at.
    top: (Ref, Ref),
    comstr: Ref,
}
romacs_snapshot::snapshot_struct!(Summary { exit, pops, peak, opened, top, comstr });

/// A span's summary, or `Deep` where it cannot have one: it closes more
/// levels than its scan was given, or reaches GNU's level cap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Span {
    Exact(Summary),
    Deep,
}
romacs_snapshot::snapshot_enum!(Span { Span::Exact(summary) => (summary), Span::Deep => () });

/// The span START..END (END a character position) entered in KEY.
pub(crate) fn summarize<T: Text>(scanner: &mut Scanner<'_, T>, start: Pos, end: i64, key: Key) -> Span {
    let mut state = ParseState {
        depth: SEEDS,
        instring: key.instring,
        incomment: key.incomment,
        comstyle: key.comstyle,
        quoted: key.quoted,
        prev_syntax: key.prev_syntax,
        comstr_start: COMSTR,
        levelstarts: (0..SEEDS).rev().map(|j| Some(seed(j))).collect(),
        continuation: Some(Continuation { in_symbol: key.in_symbol, top_last: TOP_LAST, top_prev: TOP_PREV, spent: false }),
        ..ParseState::default()
    };
    match scanner.scan_sexps_forward(&mut state, start, end, i64::MIN, false, 0) {
        Ok(()) => decode(&state, start.char).map_or(Span::Deep, Span::Exact),
        Err(_) => Span::Deep,
    }
}

/// The summary SCANNED, the output of `summarize''s scan from BASE, holds.
fn decode(scanned: &ParseState, base: i64) -> Option<Summary> {
    // Closed past every seed, or reached the level cap.
    let reached = usize::try_from(scanned.maxdepth).ok()?;
    if scanned.mindepth < 0 || reached >= LEVELS || usize::try_from(scanned.depth).ok()? != scanned.levelstarts.len() {
        return None;
    }
    let pops = SEEDS - scanned.mindepth;
    let kept = usize::try_from(SEEDS - pops).ok()?;
    let at = |v: i64| (v >= base).then(|| u32::try_from(v - base).ok()).flatten();
    let reference = |v: i64| match v {
        -1 => Some(Ref::Unset),
        TOP_LAST => Some(Ref::TopLast),
        TOP_PREV => Some(Ref::TopPrev),
        COMSTR => Some(Ref::ComStart),
        v if (seed(SEEDS - 1)..=seed(0)).contains(&v) => u32::try_from(-2 - v).ok().map(Ref::Open),
        v => at(v).map(Ref::At),
    };
    let continuation = scanned.continuation.filter(|c| !c.spent)?;
    Some(Summary {
        exit: Key::of(scanned),
        pops: u32::try_from(pops).ok()?,
        peak: u32::try_from(scanned.maxdepth - SEEDS).ok()?,
        opened: scanned.levelstarts[kept..].iter().map(|v| v.and_then(at)).collect::<Option<_>>()?,
        top: (reference(continuation.top_last)?, reference(continuation.top_prev)?),
        comstr: reference(scanned.comstr_start)?,
    })
}

/// STATE, a scanner's output in SUMMARY's entry key, carried across the
/// span SUMMARY describes, which starts at character BASE. False, with
/// STATE unchanged, where the span would close a level past STATE's bottom
/// or grow its stack to GNU's cap: the span must be scanned.
pub(crate) fn apply(state: &mut ParseState, summary: &Summary, base: i64) -> bool {
    let (len, pops) = (state.levelstarts.len(), summary.pops as usize);
    let Some(entry) = state.continuation else { return false };
    if pops > len || len + 1 + summary.peak as usize > LEVELS {
        return false;
    }
    let resolve = |r: Ref| -> Option<i64> {
        Some(match r {
            Ref::Unset => -1,
            Ref::At(at) => base + i64::from(at),
            Ref::Open(j) => (*state.levelstarts.get(len.checked_sub(1 + j as usize)?)?)?,
            Ref::TopLast => entry.top_last,
            Ref::TopPrev => entry.top_prev,
            Ref::ComStart => state.comstr_start,
        })
    };
    let (Some(top_last), Some(top_prev), Some(comstr)) = (resolve(summary.top.0), resolve(summary.top.1), resolve(summary.comstr))
    else { return false };
    let Ok(opened) = i64::try_from(summary.opened.len()) else { return false };
    let (depth, popped) = (state.depth, i64::from(summary.pops));
    state.levelstarts.truncate(len - pops);
    state.levelstarts.extend(summary.opened.iter().map(|&at| Some(base + i64::from(at))));
    state.depth = depth + opened - popped;
    state.mindepth = state.mindepth.min(depth - popped);
    state.maxdepth = state.maxdepth.max(depth + i64::from(summary.peak));
    let exit = summary.exit;
    (state.instring, state.incomment, state.comstyle, state.quoted, state.prev_syntax) =
        (exit.instring, exit.incomment, exit.comstyle, exit.quoted, exit.prev_syntax);
    state.continuation = Some(Continuation { in_symbol: exit.in_symbol, top_last, top_prev, spent: false });
    state.comstr_start = comstr;
    true
}

/// A then B, where B was entered in A's exit key and A covers A_CHARS
/// characters; `Deep' where the two cannot be one summary.
pub(crate) fn compose(a: &Span, b: &Span, a_chars: u32) -> Span {
    let (Span::Exact(a), Span::Exact(b)) = (a, b) else { return Span::Deep };
    let (opened_a, pops_a, pops_b) = (a.opened.len(), a.pops as usize, b.pops as usize);
    let shift = |at: u32| a_chars.checked_add(at);
    let through_a = |r: Ref| -> Option<Ref> {
        Some(match r {
            Ref::Unset => Ref::Unset,
            Ref::At(at) => Ref::At(shift(at)?),
            Ref::Open(j) if (j as usize) < opened_a => Ref::At(a.opened[opened_a - 1 - j as usize]),
            Ref::Open(j) => Ref::Open(u32::try_from(j as usize - opened_a + pops_a).ok()?),
            Ref::TopLast => a.top.0,
            Ref::TopPrev => a.top.1,
            Ref::ComStart => a.comstr,
        })
    };
    let summary = || -> Option<Summary> {
        let kept = &a.opened[..opened_a.saturating_sub(pops_b)];
        let opened: Vec<u32> = kept.iter().copied().map(Some).chain(b.opened.iter().map(|&at| shift(at))).collect::<Option<_>>()?;
        let peak = i64::from(a.peak).max(i64::try_from(opened_a).ok()? - i64::from(a.pops) + i64::from(b.peak));
        let peak = u32::try_from(peak).ok().filter(|&peak| (peak as usize) < LEVELS && opened.len() <= LEVELS)?;
        Some(Summary {
            exit: b.exit,
            pops: u32::try_from(pops_a + pops_b.saturating_sub(opened_a)).ok()?,
            peak,
            opened,
            top: (through_a(b.top.0)?, through_a(b.top.1)?),
            comstr: through_a(b.comstr)?,
        })
    };
    summary().map_or(Span::Deep, Span::Exact)
}

/// Scan FROM..END (character positions) into STATE as one scan would that
/// had not stopped where STATE did: `mindepth' and `maxdepth' keep what
/// came before.
pub(crate) fn scan<T: Text>(scanner: &mut Scanner<'_, T>, state: &mut ParseState, from: i64, end: i64) -> R<()> {
    let (mindepth, maxdepth) = (state.mindepth, state.maxdepth);
    let start = scanner.text.pos(from);
    scanner.scan_sexps_forward(state, start, end, i64::MIN, false, 0)?;
    state.mindepth = state.mindepth.min(mindepth);
    state.maxdepth = state.maxdepth.max(maxdepth);
    Ok(())
}

impl Summary {
    pub(crate) fn exit(&self) -> Key {
        self.exit
    }
    /// Heap bytes it holds.
    pub(crate) fn opened_bytes(&self) -> usize {
        self.opened.capacity() * size_of::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax_scan::tests::{position, random_table, random_text, Plain, Rng};
    use crate::syntax_scan::{comstart_first, comstart_second, Options};
    use std::collections::BTreeMap;

    /// (A B) C and A (B C) are one summary, and a state carried across
    /// A, B and C one at a time lands where one scan does.
    #[test]
    fn summaries_compose_associatively_and_apply_as_a_scan() {
        let mut rng = Rng(0x0a55_0c1a_71e5);
        for case in 0..3000 {
            let table = random_table(&mut rng);
            let len = rng.below(90);
            let plain = Plain { text: random_text(&mut rng, len), table: |c: u8| table[usize::from(c)] };
            let ppss = BTreeMap::new();
            let options = Options { comment_end_can_be_escaped: rng.chance(50), ..Options::default() };
            let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options, ppss: &ppss, defun_start: None, modiff: 1 };
            // Three cuts, none inside a two-character comment start.
            let mut cuts: Vec<i64> = (0..3).map(|_| position(rng.below(len + 1))).collect();
            cuts.sort_unstable();
            let inside = |cut: i64| cut > 1 && cut < position(len)
                && comstart_first(plain.syntax(plain.pos(cut - 1))) && comstart_second(plain.syntax(plain.pos(cut)));
            if cuts.iter().any(|&cut| inside(cut)) { continue; }
            let end = position(len);
            let mut state = ParseState::default();
            scanner.scan_sexps_forward(&mut state, plain.pos(1), cuts[0], i64::MIN, false, 0).unwrap();
            let mut whole = state.clone();
            scan(&mut scanner, &mut whole, cuts[0], end).unwrap();
            let bounds = [cuts[0], cuts[1], cuts[2], end];
            let mut key = Key::of(&state);
            let mut spans = Vec::new();
            for pair in bounds.windows(2) {
                let span = summarize(&mut scanner, plain.pos(pair[0]), pair[1], key);
                let Span::Exact(summary) = &span else { break };
                key = summary.exit;
                spans.push(span);
            }
            if spans.len() < 3 { continue; }
            let chars = |i: usize| u32::try_from(bounds[i + 1] - bounds[i]).expect("a span");
            let left = compose(&compose(&spans[0], &spans[1], chars(0)), &spans[2], chars(0) + chars(1));
            let right = compose(&spans[0], &compose(&spans[1], &spans[2], chars(1)), chars(0));
            assert_eq!(left, right, "case {case}");
            let mut carried = state.clone();
            for (i, span) in spans.iter().enumerate() {
                let Span::Exact(summary) = span else { unreachable!() };
                if !apply(&mut carried, summary, bounds[i]) { scan(&mut scanner, &mut carried, bounds[i], bounds[i + 1]).unwrap(); }
            }
            scan(&mut scanner, &mut carried, end, end).unwrap();
            let mut composed = state.clone();
            if let Span::Exact(summary) = &left && apply(&mut composed, summary, bounds[0]) {
                scan(&mut scanner, &mut composed, end, end).unwrap();
                assert_eq!(composed, whole, "case {case}: composed");
            }
            assert_eq!(carried, whole, "case {case}: {:?} cut {bounds:?}", String::from_utf8_lossy(&plain.text));
        }
    }
}
