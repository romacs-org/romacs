//! Spans flattened into runs: maximal stretches covered by the same
//! kinds, each run's kinds innermost first, as a face list orders them.
use super::{Kind, Span};

/// KINDS over START..END (bytes), innermost first, none repeated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub start: usize,
    pub end: usize,
    pub kinds: Vec<Kind>,
}

/// SPANS as runs, ascending and disjoint. Text no span covers is in no
/// run. Costs O(n log n) in the spans and their nesting depth per run.
pub(crate) fn runs(spans: &[Span]) -> Vec<Run> {
    let mut edges: Vec<(usize, bool, usize)> = Vec::with_capacity(spans.len() * 2);
    for (i, span) in spans.iter().enumerate().filter(|(_, s)| s.start < s.end) {
        edges.push((span.start, true, i));
        edges.push((span.end, false, i));
    }
    // At one position, closes before opens.
    edges.sort_unstable_by_key(|&(at, opens, i)| (at, opens, i));
    let mut active: Vec<usize> = Vec::new();
    let mut out: Vec<Run> = Vec::new();
    let mut at = 0;
    for (position, opens, i) in edges {
        if position > at && !active.is_empty() {
            push_run(&mut out, at, position, kinds(spans, &active));
        }
        at = position;
        if opens {
            active.push(i);
        } else if let Some(k) = active.iter().position(|&a| a == i) {
            active.swap_remove(k);
        }
    }
    out
}

/// The kinds of the ACTIVE spans, deepest first, each once.
fn kinds(spans: &[Span], active: &[usize]) -> Vec<Kind> {
    let mut by_depth: Vec<(u32, Kind)> = active.iter().map(|&i| (spans[i].depth, spans[i].kind)).collect();
    by_depth.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut kinds: Vec<Kind> = Vec::with_capacity(by_depth.len());
    for (_, kind) in by_depth {
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds
}

/// Add a run, joining it to the last when they meet with the same kinds.
fn push_run(out: &mut Vec<Run>, start: usize, end: usize, kinds: Vec<Kind>) {
    match out.last_mut() {
        Some(last) if last.end == start && last.kinds == kinds => last.end = end,
        _ => out.push(Run { start, end, kinds }),
    }
}
