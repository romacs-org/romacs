//! fns.c `Fcompare_strings`' loop over strings and symbol names where they
//! lie: the characters are decoded from the bytes, a pair that differs is
//! folded through the buffer's upcase table read flat, and neither text is
//! copied but an escaped symbol name's decoding (`Leaf::read_symbol_name`).
use super::{Fault, Kind, Leaf, Value};
use crate::lstring::{BYTE8_OFFSET, StrView};
use std::cmp::Ordering;

/// The characters START..END of a string or of a symbol's name, END the
/// text's end when past it.
#[derive(Clone, Copy)]
pub(crate) struct Span {
    pub(crate) text: Value,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl Span {
    pub(crate) fn whole(text: Value) -> Self {
        Self { text, start: 0, end: usize::MAX }
    }
}

/// COUNT of VIEW's characters after its first SKIPPED.
fn after(view: StrView<'_>, skipped: usize, count: usize) -> impl Iterator<Item = u32> + '_ {
    codes(view, skipped.saturating_add(count)).skip(skipped)
}

/// VIEW's first COUNT characters, a unibyte string's bytes above ASCII as
/// raw bytes.
fn codes(view: StrView<'_>, count: usize) -> impl Iterator<Item = u32> + '_ {
    let (mut rest, multibyte) = (view.bytes, view.multibyte);
    let next = move || {
        let (&lead, tail) = rest.split_first()?;
        let (code, width) = match lead {
            0..0x80 => (u32::from(lead), 1),
            _ if multibyte => crate::lstring::string_char(rest),
            _ => (u32::from(lead) + BYTE8_OFFSET, 1),
        };
        rest = if width == 1 { tail } else { &rest[width..] };
        Some(code)
    };
    std::iter::from_fn(next).take(count)
}

/// How the character sequences A and B order, how many characters match
/// before they part, and whether they part at two characters rather than
/// at the end of one. Characters that differ are compared as FOLD gives
/// each.
fn parting(a: impl Iterator<Item = u32>, b: impl Iterator<Item = u32>, fold: impl Fn(u32) -> std::result::Result<u32, Fault>)
    -> std::result::Result<(Ordering, usize, bool), Fault> {
    let (mut a, mut b, mut matched) = (a, b, 0);
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) if x == y => {}
            (Some(x), Some(y)) => {
                let (x, y) = (fold(x)?, fold(y)?);
                if x != y { return Ok((x.cmp(&y), matched, true)); }
            }
            (x, y) => return Ok((x.is_some().cmp(&y.is_some()), matched, false)),
        }
        matched += 1;
    }
}

impl Leaf<'_> {
    /// F of the text a comparison reads of VALUE, with how many of its
    /// characters to skip to reach character FROM: a string's from FROM, or
    /// a symbol's whole name as Lisp reads it (minibuf.c `Fassoc_string`).
    fn with_compared<R>(&self, value: Value, from: usize, f: impl FnOnce(StrView<'_>, usize) -> R)
        -> std::result::Result<R, Fault> {
        match self.symbol_key(value) {
            Some(key) => Ok(self.read_symbol_name(value, key, |name| f(name, from))),
            None => Ok(f(self.vm.str_view_from(value, from).ok_or(Fault::Wrong("stringp", value))?, 0)),
        }
    }

    /// Whether a comparison reads VALUE: a string or a symbol.
    pub(crate) fn compared_text_p(&self, value: Value) -> bool {
        matches!(self.kind(value), Kind::Symbol | Kind::String | Kind::Bytes)
    }

    /// A and B's parting from character SKIP of each, as FOLD folds.
    fn text_parting(&self, a: Span, b: Span, skip: usize, fold: impl Fn(u32) -> std::result::Result<u32, Fault>)
        -> std::result::Result<(Ordering, usize, bool), Fault> {
        let (from_a, from_b) = (a.start + skip, b.start + skip);
        let (count_a, count_b) = (a.end.saturating_sub(from_a), b.end.saturating_sub(from_b));
        let (order, matched, differ) = self.with_compared(a.text, from_a, |va, sa| {
            self.with_compared(b.text, from_b, |vb, sb| parting(after(va, sa, count_a), after(vb, sb, count_b), &fold))
        })???;
        Ok((order, skip + matched, differ))
    }

    /// `Fcompare_strings` over A and B: their order, and how many characters
    /// match before the one that decides it. With FOLD, two characters
    /// that differ are compared as `upcase' gives each in the current
    /// buffer; its table is looked for at the first such pair, not before.
    pub(crate) fn compare_text(&mut self, a: Span, b: Span, fold: bool) -> std::result::Result<(Ordering, usize), Fault> {
        let (order, matched, differ) = self.text_parting(a, b, 0, Ok)?;
        if !(fold && differ) { return Ok((order, matched)); }
        let (table, multibyte) = (self.case_table(false)?, self.buffer_multibyte()?);
        let upcase = table.reader(self.vm);
        let (order, matched, _) = self.text_parting(a, b, matched, |code| upcase.character(code, multibyte))?;
        Ok((order, matched))
    }
}
