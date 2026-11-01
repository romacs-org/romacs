//! Orderless matching (rlisp/romacs-orderless.el) and minibuffer.el's
//! candidate sorts, each over a whole candidate list in one call.
use super::*;
use crate::orderless::Style;
use crate::prim::leaf::{OrderlessPass, OrderlessSpec};
use std::collections::HashMap;

// The CANDIDATES that match every one of COMPONENTS, matched as the
// corresponding element of NAMES when given.  COMPONENTS: per orderless
// component, its regexp, or (STYLE TEXT . REGEXP) when a native style
// answers for the regexp: STYLE is `literal', `literal-prefix', `flex' or
// `initialism' with TEXT a string, or `prefixes' with TEXT its pieces.
defsubr! { name = "romacs--orderless-filter", gnu = "romacs:orderless", effect = Reads,
fn romacs_orderless_filter(cx, candidates: Value, components: Value, ignore_case: Value; optional names: Value) {
    let mut pass = pass(cx, components, ignore_case)?;
    let candidates = elements(cx, candidates)?;
    let names = if names == Value::NIL { candidates.clone() } else { elements(cx, names)? };
    if names.len() != candidates.len() { return Err(Fault::Wrong("listp", names.first().copied().unwrap_or(Value::NIL))); }
    let mut kept = vec![];
    for (&candidate, &name) in candidates.iter().zip(&names) {
        if cx.orderless_find(&mut pass, name, true)?.iter().all(Option::is_some) {
            kept.push(candidate);
        }
    }
    Ok(list(cx, &kept))
}}

// orderless--highlight over STRINGS: each component's spans get face N mod
// (length FACES), N its index, added as `add-face-text-property' does.  The
// strings themselves, or copies when COPY.
defsubr! { name = "romacs--orderless-highlight", gnu = "romacs:orderless", effect = Reads,
fn romacs_orderless_highlight(cx, components: Value, strings: Value, ignore_case: Value, faces: Value; optional copy: Value) {
    let mut pass = pass(cx, components, ignore_case)?;
    let faces = cx.vector_values(faces)?;
    let mut out = vec![];
    for string in elements(cx, strings)? {
        let string = if copy == Value::NIL { string } else {
            let chars = cx.string_view(string)?.chars;
            cx.string_slice(string, 0, chars)?
        };
        for (i, spans) in cx.orderless_find(&mut pass, string, false)?.into_iter().enumerate() {
            let Some(&face) = faces.get(i % faces.len().max(1)) else { break };
            for (start, end) in spans.into_iter().flatten() {
                let edit = crate::buffer::PropertyEdit::Face { face, append: false };
                cx.property_begin(&edit, number(start), number(end), string)?;
            }
        }
        out.push(string);
    }
    Ok(list(cx, &out))
}}

// minibuffer--sort-by-length-alpha: shorter first, then `string<'; stable.
defsubr! { name = "romacs--sort-by-length-alpha", gnu = "minibuffer.el:1862", effect = Pure,
fn romacs_sort_by_length_alpha(cx, elems: Value) {
    let mut keyed = Vec::new();
    for elem in elements(cx, elems)? {
        keyed.push((cx.string_view(elem)?.chars, elem));
    }
    let mut failed = None;
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| cx.string_order(a.1, b.1).unwrap_or_else(|fault| {
        failed.get_or_insert(fault);
        std::cmp::Ordering::Equal
    })));
    if let Some(fault) = failed { return Err(fault); }
    let sorted: Vec<Value> = keyed.into_iter().map(|(_, elem)| elem).collect();
    Ok(list(cx, &sorted))
}}

// minibuffer--sort-by-position: ELEMS by the first position of an `equal'
// string in HIST, those absent last; stable.
defsubr! { name = "romacs--sort-by-history", gnu = "minibuffer.el:1850", effect = Pure,
fn romacs_sort_by_history(cx, hist: Value, elems: Value) {
    let mut positions = HashMap::new();
    for (i, entry) in elements(cx, hist)?.into_iter().enumerate() {
        if let Some(key) = text_key(cx, entry) { positions.entry(key).or_insert(i); }
    }
    let mut keyed: Vec<(usize, Value)> = elements(cx, elems)?.into_iter()
        .map(|elem| (text_key(cx, elem).and_then(|key| positions.get(&key).copied()).unwrap_or(usize::MAX), elem))
        .collect();
    keyed.sort_by_key(|&(position, _)| position);
    let sorted: Vec<Value> = keyed.into_iter().map(|(_, elem)| elem).collect();
    Ok(list(cx, &sorted))
}}

pub(super) fn number(n: usize) -> Value {
    Value::integer(n as i64).unwrap()
}

pub(super) fn list(cx: &mut Leaf<'_>, values: &[Value]) -> Value {
    values.iter().rev().fold(Value::NIL, |tail, &value| cx.cons(value, tail))
}

/// LIST's elements, through FOR_EACH_TAIL.
pub(super) fn elements(cx: &mut Leaf<'_>, list: Value) -> std::result::Result<Vec<Value>, Fault> {
    let (mut tail, mut slow, mut steps, mut out) = (list, list, 0, vec![]);
    while let Some((element, _)) = cx.pair(tail) {
        out.push(element);
        (tail, slow, steps) = super::fns_equal::for_each_tail_step(cx, tail, slow, steps)?;
    }
    if tail != Value::NIL { return Err(Fault::Wrong("listp", list)); }
    Ok(out)
}

/// What `equal' compares of a string: its characters.  Non-strings have
/// none (they never take a history position).
fn text_key(cx: &mut Leaf<'_>, value: Value) -> Option<(Vec<u8>, bool)> {
    let view = cx.string_view(value).ok()?;
    let raw = !view.multibyte && !view.bytes.is_ascii();
    Some((view.bytes, raw))
}

fn pass(cx: &mut Leaf<'_>, components: Value, ignore_case: Value) -> std::result::Result<OrderlessPass, Fault> {
    let mut specs = vec![];
    for component in elements(cx, components)? {
        specs.push(match cx.pair(component) {
            None => OrderlessSpec { regexp: component, native: None },
            Some((style, more)) => {
                let (text, regexp) = cx.pair(more).ok_or(Fault::Wrong("consp", more))?;
                OrderlessSpec { regexp, native: native(cx, style, text)? }
            }
        });
    }
    cx.orderless_pass(specs, ignore_case != Value::NIL)
}

fn native(cx: &mut Leaf<'_>, style: Value, text: Value) -> std::result::Result<Option<(Style, Vec<Vec<u32>>)>, Fault> {
    let style = match cx.symbol_name(style)?.as_str() {
        "literal" => Style::Literal,
        "literal-prefix" => Style::LiteralPrefix,
        "flex" => Style::Flex,
        "initialism" => Style::Initialism,
        "prefixes" => Style::Prefixes,
        _ => return Ok(None),
    };
    let pieces = if style == Style::Prefixes { elements(cx, text)? } else { vec![text] };
    let mut codes = vec![];
    for piece in pieces {
        let view = cx.string_view(piece)?;
        // A unibyte string's non-ASCII bytes are raw bytes to the regexp.
        if !view.multibyte && !view.bytes.is_ascii() { return Ok(None); }
        codes.push(view.chars().collect::<Vec<u32>>());
    }
    Ok(Some(match style {
        Style::Flex | Style::Initialism => (style, codes[0].iter().map(|&c| vec![c]).collect()),
        _ => (style, codes),
    }))
}
