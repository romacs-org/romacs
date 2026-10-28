//! GNU bidi.c's paragraph direction for `current-bidi-paragraph-direction'
//! (xdisp.c Fcurrent_bidi_paragraph_direction): UAX#9 P2 and P3 from the
//! paragraph start GNU's `bidi-paragraph-start-re' finds, skipping isolates,
//! and, where a paragraph has no strong character, the paragraphs before it
//! (HL1), else left to right. A pure function of the accessible text, the
//! starting byte and a regexp matcher; character classes are Unicode's
//! (`unicode-bidi'). Text a display property covers counts as its own
//! characters, where GNU counts one neutral (DIVERGENCES.md VM2-BIDI1).
use unicode_bidi::{BidiClass, bidi_class};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Direction { LeftToRight, RightToLeft }

/// The regexps GNU matches with fast_looking_at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Re {
    /// `bidi-paragraph-start-re', else "^\\(\f\\|[ \t]*\\)$".
    Start,
    /// `bidi-paragraph-separate-re', else "^[ \t\f]*$".
    Separate,
    /// "[\f\t ]*\n".
    TrailingBlank,
}

/// The default regexps, GNU bidi.c bidi_initialize.
pub(crate) const START_RE: &str = "^\\(\x0c\\|[ \t]*\\)$";
pub(crate) const SEPARATE_RE: &str = "^[ \t\x0c]*$";
pub(crate) const TRAILING_BLANK_RE: &str = "[\x0c\t ]*\n";

/// The length of the match of RE at a byte of the text, if it matches.
pub(crate) type Looking<'a> = dyn FnMut(Re, usize) -> Option<usize> + 'a;

const MAX_PARAGRAPH_SEARCH: usize = 7500;
const MAX_STRONG_CHAR_SEARCH: usize = 100_000;

/// The direction of the paragraph at, or before, POINT (a byte of TEXT,
/// the accessible text): Fcurrent_bidi_paragraph_direction's search from
/// the previous non-blank line, bidi_paragraph_init with NO_DEFAULT_P.
pub(crate) fn paragraph_direction(text: &str, point: usize, looks: &mut Looking) -> Direction {
    let bytes = text.as_bytes();
    let zv = text.len();
    let mut pos = point.min(zv);
    if pos >= zv && pos > 0 { pos = previous(text, pos); }
    if looks(Re::TrailingBlank, pos).is_some_and(|n| n > 0) {
        while pos < zv && matches!(bytes[pos], b'\n' | b' ' | b'\t' | 0x0c) {
            if pos == 0 { break; }
            pos -= 1;
        }
        while !text.is_char_boundary(pos) { pos -= 1; }
    }
    // An empty buffer.
    if zv == 0 { return Direction::LeftToRight; }
    // On a newline, where the next paragraph might start.
    if pos > 0 && bytes[pos] == b'\n' { pos += 1; }
    let mut start = paragraph_start(text, pos, looks);
    loop {
        match first_strong(text, start, looks) {
            BidiClass::R | BidiClass::AL => return Direction::RightToLeft,
            BidiClass::L => return Direction::LeftToRight,
            _ if start == 0 => return Direction::LeftToRight,
            _ => {}
        }
        // The previous paragraph's start.
        let (mut previous_start, mut at) = (start, start);
        while at > 0 && previous_start >= start {
            at = previous(text, at);
            previous_start = paragraph_start(text, at, looks);
        }
        start = previous_start;
    }
}

/// bidi_find_paragraph_start: back over lines from POS until
/// `bidi-paragraph-start-re' matches at one's start, else the beginning.
fn paragraph_start(text: &str, mut pos: usize, looks: &mut Looking) -> usize {
    let mut searched = 0;
    while pos > 0 {
        searched += 1;
        if searched > MAX_PARAGRAPH_SEARCH { return 0; }
        if looks(Re::Start, pos).is_some() { break; }
        pos = line_start(text, previous(text, pos));
    }
    if searched >= MAX_PARAGRAPH_SEARCH { 0 } else { pos }
}

/// find_first_strong_char from POS: the class of the first strong
/// character, past isolates, or the neutral it stopped at (the end of the
/// paragraph or of the text, or a search too long).
fn first_strong(text: &str, pos: usize, looks: &mut Looking) -> BidiClass {
    let (mut class, mut pos) = fetch_skipping_isolates(text, pos);
    let mut examined = 1;
    while !matches!(class, BidiClass::L | BidiClass::R | BidiClass::AL) && examined < MAX_STRONG_CHAR_SEARCH {
        if pos >= text.len() { return BidiClass::B; }
        if class == BidiClass::B && at_paragraph_end(pos, looks) { break; }
        let (next_class, next) = fetch_skipping_isolates(text, pos);
        examined += text[pos..next].chars().count();
        (class, pos) = (next_class, next);
    }
    class
}

/// bidi_at_paragraph_end's answer >= -1: a separator or a new paragraph
/// starts at POS.
fn at_paragraph_end(pos: usize, looks: &mut Looking) -> bool {
    looks(Re::Separate, pos).is_some() || looks(Re::Start, pos).is_some()
}

/// bidi_fetch_char_skip_isolates: the class of the character at POS, or
/// after an isolate initiator, of the one that closes it or ends its
/// paragraph; and the byte after it. The end of the text is a paragraph
/// separator.
fn fetch_skipping_isolates(text: &str, pos: usize) -> (BidiClass, usize) {
    let (mut class, mut next) = fetch(text, pos);
    if matches!(class, BidiClass::LRI | BidiClass::RLI | BidiClass::FSI) {
        let mut level = 1;
        while level > 0 && class != BidiClass::B {
            (class, next) = fetch(text, next);
            match class {
                BidiClass::LRI | BidiClass::RLI | BidiClass::FSI => level += 1,
                BidiClass::PDI => level -= 1,
                _ => {}
            }
        }
    }
    (class, next)
}

fn fetch(text: &str, pos: usize) -> (BidiClass, usize) {
    match text[pos.min(text.len())..].chars().next() {
        Some(c) => (bidi_class(c), pos + c.len_utf8()),
        None => (BidiClass::B, pos),
    }
}

fn previous(text: &str, pos: usize) -> usize {
    text[..pos].char_indices().next_back().map_or(0, |(at, _)| at)
}

/// The start of the line holding the character at POS.
fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |at| at + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Line-anchored defaults, as the regexp engine answers them.
    fn looks(text: &str) -> impl FnMut(Re, usize) -> Option<usize> + '_ {
        move |re, at| {
            let line_start = at == 0 || text.as_bytes()[at - 1] == b'\n';
            let rest = &text[at..];
            let line = &rest[..rest.find('\n').unwrap_or(rest.len())];
            match re {
                Re::Start => (line_start && (line.starts_with('\x0c') && line.len() == 1 || line.bytes().all(|b| b == b' ' || b == b'\t')))
                    .then_some(line.len()),
                Re::Separate => (line_start && line.bytes().all(|b| matches!(b, b' ' | b'\t' | 0x0c))).then_some(line.len()),
                Re::TrailingBlank => (line.bytes().all(|b| matches!(b, b' ' | b'\t' | 0x0c)) && rest.len() > line.len()).then_some(line.len() + 1),
            }
        }
    }

    fn direction(text: &str, point: usize) -> Direction {
        paragraph_direction(text, point, &mut looks(text))
    }

    #[test]
    fn the_first_strong_character_of_the_paragraph_decides() {
        use Direction::*;
        assert_eq!(direction("", 0), LeftToRight);
        assert_eq!(direction("hello", 2), LeftToRight);
        assert_eq!(direction("שלום abc", 4), RightToLeft);
        assert_eq!(direction("123 שלום abc", 0), RightToLeft);
        // Each paragraph has its own direction; a blank line separates them.
        let text = "abc\n\nשלום\n";
        assert_eq!(direction(text, 0), LeftToRight);
        assert_eq!(direction(text, 5), RightToLeft);
        // On a blank line or at the end, the paragraph before.
        assert_eq!(direction(text, 4), LeftToRight);
        assert_eq!(direction(text, text.len()), RightToLeft);
        // No strong character: the paragraphs before, else left to right.
        assert_eq!(direction("שלום\n\n123\n", 11), RightToLeft);
        assert_eq!(direction("123 !", 1), LeftToRight);
        // An isolate's contents do not count.
        assert_eq!(direction("\u{2067}abc\u{2069}שלום", 0), RightToLeft);
    }
}
