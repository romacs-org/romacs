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
//!
//! What the scanner does with levels depends on how many are open and on
//! nothing else of the entry: a close at the bottom level pops nothing, and
//! an open at GNU's cap pushes nothing. A general summary is scanned with
//! more entry levels than most spans close and holds for every count the
//! span neither exhausts nor lifts to the cap; a span that does either has
//! a summary per count it is entered with, scanned with exactly that many,
//! so that its scan is the real one, bottom and cap included.
use crate::syntax_scan::{Continuation, ParseState, Pos, R, Scanner, Text};

/// GNU's levels in scan_sexps_forward: a stack this long stops growing.
pub(crate) const LEVELS: usize = 100;
/// Entry levels a general summary's scan is given to close.
const SEEDS: u32 = 48;
/// The sentinel of the entry's Jth open paren, counted from the innermost.
fn seed(j: i64) -> i64 {
    -(2 + j)
}
/// The entry's other values, below every seed: a scan is entered with
/// fewer than LEVELS levels.
const TOP_LAST: i64 = -103;
const TOP_PREV: i64 = -104;
const COMSTR: i64 = -105;

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
    /// Entry levels gone at its end.
    pops: u32,
    /// Its end's depth above the entry's, how far below the entry's depth
    /// it goes, and how far above.
    rise: i32,
    fall: u32,
    peak: u32,
    /// The starts of the levels it leaves open below its last one: the
    /// level it ends at holds them, innermost last.
    opened: Vec<u32>,
    /// The `last' and `prev' of the level it ends at.
    top: (Ref, Ref),
    comstr: Ref,
    /// The count of entry levels it is exact for; none, it is general, for
    /// every count it `fits`.
    levels: Option<u32>,
}
romacs_snapshot::snapshot_struct!(Summary { exit, pops, rise, fall, peak, opened, top, comstr, levels });

/// A span's summary, or `Deep` where it has none: a general one whose span
/// closes more levels than its scan was given or reaches GNU's level cap,
/// and any whose span ends in a state no key resumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Span {
    Exact(Summary),
    Deep,
}
romacs_snapshot::snapshot_enum!(Span { Span::Exact(summary) => (summary), Span::Deep => () });

/// The span START..END (END a character position) entered in KEY: exact
/// for a state with LEVELS levels open, or general with no count.
pub(crate) fn summarize<T: Text>(scanner: &mut Scanner<'_, T>, start: Pos, end: i64, key: Key, levels: Option<u32>) -> Span {
    // A state holds fewer levels than the cap; so the seeds stay above the sentinels.
    debug_assert!(levels.is_none_or(|levels| (levels as usize) < LEVELS), "{levels:?} entry levels");
    let seeds = i64::from(levels.unwrap_or(SEEDS));
    let mut state = ParseState {
        depth: seeds,
        instring: key.instring,
        incomment: key.incomment,
        comstyle: key.comstyle,
        quoted: key.quoted,
        prev_syntax: key.prev_syntax,
        comstr_start: COMSTR,
        levelstarts: (0..seeds).rev().map(|j| Some(seed(j))).collect(),
        continuation: Some(Continuation { in_symbol: key.in_symbol, top_last: TOP_LAST, top_prev: TOP_PREV, spent: false }),
        ..ParseState::default()
    };
    match scanner.scan_sexps_forward(&mut state, start, end, i64::MIN, false, 0) {
        Ok(()) => decode(&state, start.char, levels).map_or(Span::Deep, Span::Exact),
        Err(_) => Span::Deep,
    }
}

/// The summary SCANNED, the output of the scan `summarize` made from BASE
/// for LEVELS, holds. None for a general scan that closed past every seed or
/// reached the level cap: what it did there depends on the count.
fn decode(scanned: &ParseState, base: i64, levels: Option<u32>) -> Option<Summary> {
    let seeds = i64::from(levels.unwrap_or(SEEDS));
    // Depth and levels part only at the bottom or at the cap.
    let capped = usize::try_from(scanned.maxdepth).is_ok_and(|reached| reached >= LEVELS);
    if levels.is_none() && (scanned.mindepth < 0 || capped) {
        return None;
    }
    let at = |v: i64| (v >= base).then(|| u32::try_from(v - base).ok()).flatten();
    let is_seed = |v: i64| (seed(seeds - 1)..=seed(0)).contains(&v);
    let reference = |v: i64| match v {
        -1 => Some(Ref::Unset),
        TOP_LAST => Some(Ref::TopLast),
        TOP_PREV => Some(Ref::TopPrev),
        COMSTR => Some(Ref::ComStart),
        v if is_seed(v) => u32::try_from(-2 - v).ok().map(Ref::Open),
        v => at(v).map(Ref::At),
    };
    // The open list is the entry levels left, then the levels the span
    // opened: a level below the top holds the open paren of the one above.
    let kept = scanned.levelstarts.iter().take_while(|v| v.is_some_and(is_seed)).count();
    let continuation = scanned.continuation.filter(|c| !c.spent)?;
    Some(Summary {
        exit: Key::of(scanned),
        pops: u32::try_from(seeds - i64::try_from(kept).ok()?).ok()?,
        rise: i32::try_from(scanned.depth - seeds).ok()?,
        fall: u32::try_from(seeds - scanned.mindepth).ok()?,
        peak: u32::try_from(scanned.maxdepth - seeds).ok()?,
        opened: scanned.levelstarts[kept..].iter().map(|v| v.and_then(at)).collect::<Option<_>>()?,
        top: (reference(continuation.top_last)?, reference(continuation.top_prev)?),
        comstr: reference(scanned.comstr_start)?,
        levels,
    })
}

/// STATE, a scanner's output in SUMMARY's entry key, carried across the
/// span SUMMARY describes, which starts at character BASE. False, with
/// STATE unchanged, where SUMMARY does not fit STATE's levels.
pub(crate) fn apply(state: &mut ParseState, summary: &Summary, base: i64) -> bool {
    let (len, pops) = (state.levelstarts.len(), summary.pops as usize);
    let Some(entry) = state.continuation else { return false };
    if !summary.fits(len) {
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
    let depth = state.depth;
    state.levelstarts.truncate(len - pops);
    state.levelstarts.extend(summary.opened.iter().map(|&at| Some(base + i64::from(at))));
    state.depth = depth + i64::from(summary.rise);
    state.mindepth = state.mindepth.min(depth - i64::from(summary.fall));
    state.maxdepth = state.maxdepth.max(depth + i64::from(summary.peak));
    let exit = summary.exit;
    (state.instring, state.incomment, state.comstyle, state.quoted, state.prev_syntax) =
        (exit.instring, exit.incomment, exit.comstyle, exit.quoted, exit.prev_syntax);
    state.continuation = Some(Continuation { in_symbol: exit.in_symbol, top_last, top_prev, spent: false });
    state.comstr_start = comstr;
    true
}

/// A then B, where B was entered in A's exit key and with the levels A
/// leaves, and A covers A_CHARS characters. General where both are; else
/// exact for a count the caller knows (`Summary::exact_for`). None where
/// the two cannot be one summary: more open levels than a state holds.
pub(crate) fn compose(a: &Summary, b: &Summary, a_chars: u32) -> Option<Summary> {
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
    let kept = &a.opened[..opened_a.saturating_sub(pops_b)];
    let opened: Vec<u32> = kept.iter().copied().map(Some).chain(b.opened.iter().map(|&at| shift(at))).collect::<Option<_>>()?;
    let rise = i64::from(a.rise);
    Some(Summary {
        exit: b.exit,
        pops: u32::try_from(pops_a + pops_b.saturating_sub(opened_a)).ok()?,
        rise: a.rise.checked_add(b.rise)?,
        fall: u32::try_from(i64::from(a.fall).max(i64::from(b.fall) - rise)).ok()?,
        peak: u32::try_from(i64::from(a.peak).max(rise + i64::from(b.peak))).ok()?,
        opened: (opened.len() < LEVELS).then_some(opened)?,
        top: (through_a(b.top.0)?, through_a(b.top.1)?),
        comstr: through_a(b.comstr)?,
        levels: None,
    })
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
    /// Whether it carries a state with OPEN levels across its span: its
    /// own count, or for a general one any count the span neither closes
    /// past nor lifts to GNU's cap.
    pub(crate) fn fits(&self, open: usize) -> bool {
        match self.levels {
            Some(levels) => levels as usize == open,
            None => self.pops as usize <= open && open + 1 + self.peak as usize <= LEVELS,
        }
    }
    /// The levels a state it `fits`, entered with OPEN, has open after it.
    pub(crate) fn levels_after(&self, open: u32) -> Option<u32> {
        open.checked_sub(self.pops)?.checked_add(u32::try_from(self.opened.len()).ok()?)
    }
    /// It, as exact for LEVELS entry levels: what a composition of
    /// summaries entered with that count is.
    pub(crate) fn exact_for(self, levels: u32) -> Self {
        Self { levels: Some(levels), ..self }
    }
    /// Heap bytes it holds.
    pub(crate) fn opened_bytes(&self) -> usize {
        self.opened.capacity() * size_of::<u32>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax_scan::tests::{nested_text, position, random_table, random_text, with_parens, Plain, Rng};
    use crate::syntax_scan::{comstart_first, comstart_second, Options, SCLOSE, SOPEN, SSTRING, SWHITESPACE, SWORD};
    use std::collections::BTreeMap;

    /// The summary that carries CARRIED across START..END: the general one
    /// where it fits CARRIED's levels, else the one exact for them. One of
    /// the two there is, unless the span ends in a comment no key resumes.
    fn fitting<T: Text>(scanner: &mut Scanner<'_, T>, carried: &ParseState, start: i64, end: i64) -> Option<Summary> {
        let (key, open) = (Key::of(carried), carried.levelstarts.len());
        let from = scanner.text.pos(start);
        if let Span::Exact(general) = summarize(scanner, from, end, key, None) && general.fits(open) {
            return Some(general);
        }
        let exact = summarize(scanner, from, end, key, Some(u32::try_from(open).expect("levels under the cap")));
        if let Span::Exact(exact) = exact {
            return Some(exact);
        }
        let mut plain = carried.clone();
        scan(scanner, &mut plain, start, end).unwrap();
        assert!(plain.continuation.is_some_and(|c| c.spent), "{start}..{end} has no summary for {carried:?}");
        None
    }

    /// (A B) C and A (B C) are one summary, general and exact ones mixed,
    /// and a state carried across A, B and C one at a time, or across
    /// their composition, lands where one scan does: no span is left to
    /// the scanner for closing past the bottom level or reaching the cap.
    #[test]
    fn summaries_compose_associatively_and_apply_as_a_scan() {
        let mut rng = Rng(0x0a55_0c1a_71e5);
        let (mut exact, mut mixed, mut bottomed, mut capped) = (0, 0, 0, 0);
        for case in 0..3000 {
            let nested = case % 3 == 0;
            let table = if nested { with_parens(random_table(&mut rng)) } else { random_table(&mut rng) };
            let len = rng.below(if nested { 400 } else { 90 });
            let text = if nested { nested_text(&mut rng, len) } else { random_text(&mut rng, len) };
            let plain = Plain { text, table: |c: u8| table[usize::from(c)] };
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
            let mut carried = state.clone();
            let mut spans = Vec::new();
            for pair in bounds.windows(2) {
                let Some(summary) = fitting(&mut scanner, &carried, pair[0], pair[1]) else { break };
                // A span closes past the bottom where its depth falls below
                // the levels it pops, and reaches the cap where its peak
                // over its entry's levels does.
                bottomed += usize::from(summary.fall > summary.pops);
                capped += usize::from(carried.levelstarts.len() + 1 + summary.peak as usize > LEVELS);
                assert!(apply(&mut carried, &summary, pair[0]), "case {case}: {summary:?} refuses {carried:?}");
                spans.push(summary);
            }
            if spans.len() < 3 { continue; }
            scan(&mut scanner, &mut carried, end, end).unwrap();
            assert_eq!(carried, whole, "case {case}: {:?} cut {bounds:?}", String::from_utf8_lossy(&plain.text));
            let chars = |i: usize| u32::try_from(bounds[i + 1] - bounds[i]).expect("a span");
            let pair = |a: &Summary, b: &Summary, chars: u32| compose(a, b, chars).expect("spans of one scan compose");
            let left = pair(&pair(&spans[0], &spans[1], chars(0)), &spans[2], chars(0) + chars(1));
            let right = pair(&spans[0], &pair(&spans[1], &spans[2], chars(1)), chars(0));
            assert_eq!(left, right, "case {case}");
            // Exact for the entry's count where a part is exact for its own.
            let exacts = spans.iter().filter(|summary| summary.levels.is_some()).count();
            let open = u32::try_from(state.levelstarts.len()).expect("levels under the cap");
            let left = if exacts > 0 { left.exact_for(open) } else { left };
            let mut composed = state.clone();
            assert!(apply(&mut composed, &left, bounds[0]), "case {case}: {left:?} refuses {state:?}");
            scan(&mut scanner, &mut composed, end, end).unwrap();
            assert_eq!(composed, whole, "case {case}: composed");
            exact += usize::from(exacts > 0);
            mixed += usize::from(exacts > 0 && exacts < 3);
        }
        assert!(exact > 300 && mixed > 150 && bottomed > 300 && capped > 150, "exact {exact}, mixed {mixed}, spans bottomed {bottomed}, capped {capped}");
    }

    /// A span entered with more levels open than a general summary's scan
    /// is given, 60 and GNU's most, 99: the entry's other values are told
    /// from every level's, and its closes down to the bottom level and
    /// past it, and its opens at the cap, are the scanner's.
    #[test]
    fn a_span_entered_deep_is_carried_as_scanned() {
        let table = |c: u8| match c { b'(' => SOPEN, b')' => SCLOSE, b'"' => SSTRING, b' ' => SWHITESPACE, _ => SWORD };
        for open in [60, 99] {
            for closes in [0, 1, 47, 48, 49, 50, 51, open, open + 2] {
                // The span starts after LEAD: at top level or inside a string.
                for (lead, before, after) in [("a ", "", ""), ("a ", "", " b (c ((d"), ("\"s", "t\" ", ""), ("\"s", "t\" ", " e (f) \"g")] {
                    let text = format!("{}{lead}{before}{}{after}", "(".repeat(open), ")".repeat(closes));
                    let plain = Plain { text: text.clone().into_bytes(), table };
                    let ppss = BTreeMap::new();
                    let mut scanner = Scanner { begv: plain.pos(1), zv: plain.pos(i64::MAX), text: &plain, options: Options::default(), ppss: &ppss, defun_start: None, modiff: 1 };
                    let (cut, end) = (position(open + lead.len()), position(text.len()));
                    let mut state = ParseState::default();
                    scanner.scan_sexps_forward(&mut state, plain.pos(1), cut, i64::MIN, false, 0).unwrap();
                    assert_eq!(state.levelstarts.len(), open);
                    let mut whole = state.clone();
                    scan(&mut scanner, &mut whole, cut, end).unwrap();
                    let summary = fitting(&mut scanner, &state, cut, end).expect("no comment");
                    let mut carried = state.clone();
                    assert!(apply(&mut carried, &summary, cut), "{text:?}: {summary:?} refuses {state:?}");
                    scan(&mut scanner, &mut carried, end, end).unwrap();
                    assert_eq!(carried, whole, "{text:?}: {summary:?}");
                }
            }
        }
    }
}
