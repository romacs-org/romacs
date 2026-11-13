//! fns.c `Fcompare_strings`' loop over strings and symbol names where they
//! lie: the characters are decoded from the bytes, a pair that differs is
//! folded through the buffer's upcase table read flat, and neither text is
//! copied.
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

/// VIEW's characters FROM..TO, a unibyte string's bytes above ASCII as raw
/// bytes.
fn codes(view: StrView<'_>, from: usize, to: usize) -> impl Iterator<Item = u32> + '_ {
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
    std::iter::from_fn(next).skip(from).take(to.saturating_sub(from))
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
    /// The text a comparison reads of VALUE: a string's, or a symbol's
    /// name (minibuf.c `Fassoc_string`).
    fn compared_text(&self, value: Value) -> Option<StrView<'_>> {
        match self.kind(value) {
            Kind::Symbol => {
                let name = if value == Value::NIL { "nil" } else if value == Value::TRUE { "t" } else {
                    self.vm.symbol_names[self.symbol_slot_of(value).ok()?].as_str()
                };
                Some(StrView { bytes: name.as_bytes(), chars: name.chars().count(), multibyte: true })
            }
            Kind::String | Kind::Bytes => self.vm.str_view(value),
            _ => None,
        }
    }

    /// Whether a comparison reads VALUE: a string or a symbol.
    pub(crate) fn compared_text_p(&self, value: Value) -> bool {
        matches!(self.kind(value), Kind::Symbol | Kind::String | Kind::Bytes)
    }

    /// A and B's parting from character SKIP of each, as FOLD folds.
    fn text_parting(&self, a: Span, b: Span, skip: usize, fold: impl Fn(u32) -> std::result::Result<u32, Fault>)
        -> std::result::Result<(Ordering, usize, bool), Fault> {
        let view = |span: Span| self.compared_text(span.text).ok_or(Fault::Wrong("stringp", span.text));
        let (order, matched, differ) = parting(
            codes(view(a)?, a.start + skip, a.end), codes(view(b)?, b.start + skip, b.end), fold)?;
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
