//! The fzf completion style's matching (lisp/romacs/romacs-fzf.el), each over a
//! whole candidate list in one call.  Candidates are copied into plain
//! buffers first (crate::fzf::Corpus), so scoring may use other threads.
//! QUERY is fzf's extended search syntax; CASE as `fzf_query' takes it;
//! PARALLEL, an integer or nil, the candidate count from which scoring is
//! split across threads.  A pass whose completion generation advances
//! (romacs_vm::advance_completion_generation) stops and signals
//! `romacs-completion-stale'.
use super::*;
use super::orderless::{elements, list, number};
use crate::fzf::{Corpus, Generation, Scratch};

// The CANDIDATES QUERY matches, best first: higher score, shorter, then
// in their order.  Each is matched as the corresponding element of NAMES
// when given.
defsubr! { name = "romacs--fzf-filter", gnu = "romacs:fzf", effect = Reads,
fn romacs_fzf_filter(cx, candidates: Value, query: Value, case: Value, parallel: Value; optional names: Value) {
    let candidates = elements(cx, candidates)?;
    let names = names_for(cx, &candidates, names)?;
    let order = ranked(cx, &names, query, case, parallel)?;
    let kept: Vec<Value> = order.into_iter().map(|i| candidates[i]).collect();
    Ok(list(cx, &kept))
}}

// COMPLETIONS, those QUERY matches first in their rank, then the others
// in their order: the `display-sort-function' the style composes.  Each
// is matched as the corresponding element of NAMES when given.
defsubr! { name = "romacs--fzf-sort", gnu = "romacs:fzf", effect = Reads,
fn romacs_fzf_sort(cx, completions: Value, query: Value, case: Value, parallel: Value; optional names: Value) {
    let completions = elements(cx, completions)?;
    let names = names_for(cx, &completions, names)?;
    let order = ranked(cx, &names, query, case, parallel)?;
    let mut matched = vec![false; completions.len()];
    for &i in &order { matched[i] = true; }
    let rest = (0..completions.len()).filter(|&i| !matched[i]);
    let sorted: Vec<Value> = order.iter().copied().chain(rest).map(|i| completions[i]).collect();
    Ok(list(cx, &sorted))
}}

// STRINGS with FACE added, as `add-face-text-property' does, where
// QUERY's terms match: the characters fuzzy terms align, the ranges of
// the others.  A string is matched as the corresponding element of NAMES
// when given, a subsequence of it (a file's name, unquoted), and the
// positions are carried to the string.  The strings themselves, or
// copies when COPY.
defsubr! { name = "romacs--fzf-highlight", gnu = "romacs:fzf", effect = Reads,
fn romacs_fzf_highlight(cx, strings: Value, query: Value, case: Value, face: Value; optional copy: Value, names: Value) {
    let (query, mut fold) = cx.fzf_query(query, case)?;
    let strings = elements(cx, strings)?;
    let names = names_for(cx, &strings, names)?;
    let mut scratch = Scratch::default();
    let mut out = vec![];
    for (&string, &name) in strings.iter().zip(&names) {
        let string = if copy == Value::NIL { string } else {
            let chars = cx.string_view(string)?.chars;
            cx.string_slice(string, 0, chars)?
        };
        let mut corpus = Corpus::default();
        cx.fzf_decode(&mut corpus, &mut fold, name)?;
        if let Some((_, positions)) = crate::fzf::match_item(&mut scratch, &query, corpus.item(0), true) {
            let shown: Vec<u32> = cx.string_view(string)?.chars().collect();
            let mut positions = carried(&corpus.original, &shown, positions);
            positions.sort_unstable();
            positions.dedup();
            for (start, end) in runs(&positions) {
                let edit = crate::buffer::PropertyEdit::Face { face, append: false };
                cx.property_begin(&edit, number(start), number(end), string)?;
            }
        }
        out.push(string);
    }
    Ok(list(cx, &out))
}}

/// The indexes of NAMES that QUERY matches, in rank order.
fn ranked(cx: &mut Leaf<'_>, names: &[Value], query: Value, case: Value, parallel: Value)
    -> std::result::Result<Vec<usize>, Fault> {
    let generation = Generation::current();
    let parallel = match parallel.integer_value() {
        Some(n) => Some(n.max(1) as usize),
        None if parallel == Value::NIL => None,
        None => return Err(Fault::Wrong("integerp", parallel)),
    };
    let (query, mut fold) = cx.fzf_query(query, case)?;
    let mut corpus = Corpus::default();
    for &name in names { cx.fzf_decode(&mut corpus, &mut fold, name)?; }
    crate::fzf::rank(&query, &corpus, parallel, &|| generation.stale())
        .ok_or_else(|| Fault::Signal(cx.intern("romacs-completion-stale"), Value::NIL))
}

/// NAMES as a list parallel to VALUES, VALUES themselves when nil.
fn names_for(cx: &mut Leaf<'_>, values: &[Value], names: Value) -> std::result::Result<Vec<Value>, Fault> {
    if names == Value::NIL { return Ok(values.to_vec()); }
    let names = elements(cx, names)?;
    if names.len() != values.len() { return Err(Fault::Wrong("listp", names.first().copied().unwrap_or(Value::NIL))); }
    Ok(names)
}

/// POSITIONS in NAME as positions in SHOWN, of which NAME is a
/// subsequence: each name character at the first place left in SHOWN.
fn carried(name: &[u32], shown: &[u32], positions: Vec<usize>) -> Vec<usize> {
    if name == shown { return positions; }
    let mut at = Vec::with_capacity(name.len());
    let mut j = 0;
    for &c in name {
        while j < shown.len() && shown[j] != c { j += 1; }
        at.push(j.min(shown.len().saturating_sub(1)));
        j += 1;
    }
    positions.into_iter().filter_map(|p| at.get(p).copied()).collect()
}

/// Sorted POSITIONS as maximal (START, END) runs.
fn runs(positions: &[usize]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = vec![];
    for &p in positions {
        match out.last_mut() {
            Some(run) if run.1 == p => run.1 = p + 1,
            _ => out.push((p, p + 1)),
        }
    }
    out
}
