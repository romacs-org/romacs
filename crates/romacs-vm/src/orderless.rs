//! Orderless's builtin matching styles, matched without a regexp.
//!
//! Each style answers what the regexp orderless.el (e271a8e) builds for it
//! answers under GNU's backtracking search: whether a candidate matches,
//! and where the regexp's groups land, which orderless highlights.  Text
//! is a candidate's characters, case-folded by the caller when the match
//! ignores case; positions are character indexes.
//!
//! - `orderless-literal` and `-literal-prefix`: the leftmost occurrence,
//!   or one at the start.
//! - `orderless-flex`, `(group c1) [^c2]* (group c2) ...`: no `[^c]*` can
//!   cross a `c`, so each character is the first one after the previous.
//! - `orderless-initialism`, `(group \<c1) .* (group \<c2) ...`, and
//!   `orderless-prefixes`, `(group \bP1) .* (group \bP2) ...`: the first
//!   group at the leftmost start that can complete, and, `.*' being greedy,
//!   every later group at the last place the rest still fits, all on the
//!   first group's line.

const NEWLINE: u32 = '\n' as u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    Literal,
    LiteralPrefix,
    Flex,
    Initialism,
    Prefixes,
}

/// A component in one style, its pieces case-folded as the text is:
/// literals have one piece, flex and initialism one per character.
#[derive(Debug)]
pub(crate) struct Component {
    style: Style,
    pieces: Vec<Vec<u32>>,
}

impl Component {
    /// STYLE over PIECES, or None where the regexp answers differently
    /// from these rules: a newline in a piece, which a `.*' gap cannot
    /// reach past, or no piece at all.
    pub(crate) fn new(style: Style, pieces: Vec<Vec<u32>>) -> Option<Self> {
        let gapped = matches!(style, Style::Initialism | Style::Prefixes);
        if pieces.is_empty() || gapped && pieces.iter().flatten().any(|&c| c == NEWLINE) {
            return None;
        }
        Some(Component { style, pieces })
    }

    /// Whether word syntax decides this component's matches.
    pub(crate) fn reads_words(&self) -> bool {
        matches!(self.style, Style::Initialism | Style::Prefixes)
    }

    /// The spans of this component's groups in TEXT, or None when it does
    /// not match.  WORD tells whether TEXT's character at an index is a
    /// word constituent.
    pub(crate) fn find(&self, text: &[u32], word: &dyn Fn(usize) -> bool) -> Option<Vec<(usize, usize)>> {
        match self.style {
            Style::Literal => literal(&self.pieces[0], text),
            Style::LiteralPrefix => text.starts_with(&self.pieces[0]).then(|| vec![(0, self.pieces[0].len())]),
            Style::Flex => flex(&self.pieces, text),
            Style::Initialism => gapped(&self.pieces, text, &|p| word_start(text, p, word)),
            Style::Prefixes => gapped(&self.pieces, text, &|p| word_boundary(text, p, word)),
        }
    }
}

fn literal(needle: &[u32], text: &[u32]) -> Option<Vec<(usize, usize)>> {
    let at = if needle.is_empty() { 0 } else { text.windows(needle.len()).position(|w| w == needle)? };
    Some(vec![(at, at + needle.len())])
}

fn flex(chars: &[Vec<u32>], text: &[u32]) -> Option<Vec<(usize, usize)>> {
    let mut from = 0;
    chars.iter().map(|c| {
        let at = from + text[from..].iter().position(|t| *t == c[0])?;
        from = at + 1;
        Some((at, at + 1))
    }).collect()
}

/// `\<`: a word constituent at P not preceded by one.
fn word_start(text: &[u32], p: usize, word: &dyn Fn(usize) -> bool) -> bool {
    p < text.len() && word(p) && (p == 0 || !word(p - 1))
}

/// `\b`: either end of the text, or a change of word syntax at P.
fn word_boundary(text: &[u32], p: usize, word: &dyn Fn(usize) -> bool) -> bool {
    p == 0 || p == text.len() || word(p - 1) != word(p)
}

fn gapped(pieces: &[Vec<u32>], text: &[u32], boundary: &dyn Fn(usize) -> bool) -> Option<Vec<(usize, usize)>> {
    let fits = |p: usize, piece: &[u32]| text[p..].starts_with(piece) && boundary(p);
    (0..=text.len()).filter(|&s| fits(s, &pieces[0])).find_map(|s| {
        let line_end = text[s..].iter().position(|&c| c == NEWLINE).map_or(text.len(), |i| s + i);
        let first = (s, s + pieces[0].len());
        let mut spans = vec![first];
        let mut limit = line_end;
        for piece in pieces[1..].iter().rev() {
            let last = limit.checked_sub(piece.len())?;
            let at = (first.1..=last).rev().find(|&p| fits(p, piece))?;
            spans.push((at, at + piece.len()));
            limit = at;
        }
        spans[1..].reverse();
        Some(spans)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(s: &str) -> Vec<u32> {
        s.chars().map(|c| c as u32).collect()
    }
    fn chars(s: &str) -> Vec<Vec<u32>> {
        s.chars().map(|c| vec![c as u32]).collect()
    }
    fn find(style: Style, pieces: Vec<Vec<u32>>, text: &str) -> Option<Vec<(usize, usize)>> {
        let text = codes(text);
        let word = |i: usize| char::from_u32(text[i]).is_some_and(char::is_alphanumeric);
        Component::new(style, pieces).unwrap().find(&text, &word)
    }

    #[test]
    fn literal_is_the_leftmost_occurrence() {
        assert_eq!(find(Style::Literal, vec![codes("ab")], "xabab"), Some(vec![(1, 3)]));
        assert_eq!(find(Style::Literal, vec![codes("ba")], "aab"), None);
        assert_eq!(find(Style::LiteralPrefix, vec![codes("fi")], "file"), Some(vec![(0, 2)]));
        assert_eq!(find(Style::LiteralPrefix, vec![codes("il")], "file"), None);
    }

    #[test]
    fn flex_takes_each_first_occurrence() {
        assert_eq!(find(Style::Flex, chars("fl1"), "file-0101.txt"), Some(vec![(0, 1), (2, 3), (6, 7)]));
        assert_eq!(find(Style::Flex, chars("aa"), "bab"), None);
        assert_eq!(find(Style::Flex, chars("ab"), "a\nb"), Some(vec![(0, 1), (2, 3)]));
    }

    #[test]
    fn initialism_is_leftmost_then_greedy() {
        // \(\<f\).*\(\<b\): f at the first word start, b at the last.
        assert_eq!(find(Style::Initialism, chars("fb"), "foo-bar-baz"), Some(vec![(0, 1), (8, 9)]));
        assert_eq!(find(Style::Initialism, chars("fb"), "afoo-bar"), None);
        assert_eq!(find(Style::Initialism, chars("ab"), "x-a\nb"), None);
        assert_eq!(find(Style::Initialism, chars("ab"), "a\nx-a-b"), Some(vec![(4, 5), (6, 7)]));
        // GNU's match data: (0 9 0 1 6 7 8 9), the middle group as late as the last allows.
        assert_eq!(find(Style::Initialism, chars("fbq"), "f-b f-b-q"), Some(vec![(0, 1), (6, 7), (8, 9)]));
    }

    #[test]
    fn prefixes_match_at_word_boundaries() {
        let pieces = vec![codes("fo"), codes("-ba"), vec![]];
        // \(\bfo\).*\(\b-ba\).*\(\b\): the empty last group at the text's end.
        assert_eq!(find(Style::Prefixes, pieces, "foo-bar"), Some(vec![(0, 2), (3, 6), (7, 7)]));
        assert_eq!(find(Style::Prefixes, vec![codes("ar")], "foo-bar"), None);
    }

    #[test]
    fn newlines_in_gapped_pieces_take_the_regexp() {
        assert!(Component::new(Style::Initialism, chars("a\n")).is_none());
        assert!(Component::new(Style::Flex, chars("a\n")).is_some());
    }
}
