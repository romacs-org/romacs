//! Which characters display as one composed glyph: the emoji and
//! variation sequences GNU's `composition-function-table' composes on a
//! graphical frame (composite.el's VS-16 and variation-selector rules,
//! emoji-zwj.el's ZWJ sequences, flags and tag sequences).
//!
//! The evaluator keeps a sequence's joiners as characters (not glyphless
//! glyphs) and the layout draws and measures the sequence as one glyph, so
//! both decide by these rules.

/// A zero-width character that binds a composition together rather than
/// showing on its own: ZWNJ and ZWJ, variation selectors (and their
/// supplement), and tag characters. Bidi controls, spaces and separators
/// are not joiners: they stay glyphless.
pub fn is_composition_joiner(ch: char) -> bool {
    matches!(ch as u32, 0x200C | 0x200D | 0xFE00..=0xFE0F | 0xE0001 | 0xE0020..=0xE007F | 0xE0100..=0xE01EF)
}

fn is_variation_selector(ch: char) -> bool {
    matches!(ch as u32, 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
}

fn is_regional_indicator(ch: char) -> bool {
    matches!(ch as u32, 0x1F1E6..=0x1F1FF)
}

/// The blocks emoji (and the symbols that take an emoji presentation)
/// come from: what a ZWJ joins, and what takes a skin-tone modifier or a
/// tag sequence.
fn is_emoji(ch: char) -> bool {
    matches!(ch as u32, 0xA9 | 0xAE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA | 0x231A..=0x23FF
        | 0x24C2 | 0x25AA..=0x25FE | 0x2600..=0x27BF | 0x2934 | 0x2935 | 0x2B05..=0x2B55 | 0x3030 | 0x303D
        | 0x3297 | 0x3299 | 0x1F000..=0x1FAFF)
}

/// Whether CH continues a composed sequence whose last character is PREV.
pub fn joins(prev: char, ch: char) -> bool {
    let base = |c: char| !c.is_ascii() && !is_composition_joiner(c) && !c.is_control();
    match ch as u32 {
        // A variation selector after any base character (VS-16 asks for
        // its emoji presentation, VS-15 for its text one).
        _ if is_variation_selector(ch) => base(prev),
        // A skin tone after an emoji, or after its presentation selector.
        0x1F3FB..=0x1F3FF => is_emoji(prev) || prev == '\u{FE0F}',
        // A ZWJ after an emoji (or its selector, modifier or tags) ...
        0x200D => is_emoji(prev) || matches!(prev as u32, 0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F),
        // ... a tag sequence's tags and its cancel tag after an emoji.
        0xE0020..=0xE007F => is_emoji(prev) || matches!(prev as u32, 0xE0020..=0xE007E),
        _ => prev == '\u{200D}' && is_emoji(ch),
    }
}

/// Where the composed sequence starting at BYTE of TEXT ends, or None
/// when the character there composes with nothing after it: each next
/// character that `joins' the one before, or a second regional
/// indicator after a first (a flag).
pub fn cluster_end(text: &str, byte: usize) -> Option<usize> {
    let mut chars = text.get(byte..)?.char_indices();
    let (_, first) = chars.next()?;
    if first.is_ascii() || is_composition_joiner(first) { return None; }
    let mut end = byte + first.len_utf8();
    let mut prev = first;
    for (at, ch) in chars {
        let flag = is_regional_indicator(first) && is_regional_indicator(ch) && prev == first;
        if !flag && !joins(prev, ch) { break; }
        end = byte + at + ch.len_utf8();
        prev = ch;
        if flag { break; }
    }
    // A sequence ending in a ZWJ joins nothing: the ZWJ stays its own.
    if prev == '\u{200D}' { end -= '\u{200D}'.len_utf8(); }
    (end > byte + first.len_utf8()).then_some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(text: &str) -> Option<usize> { cluster_end(text, 0) }

    #[test]
    fn a_family_zwj_sequence_is_one_cluster() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(end(&format!("{family}x")), Some(family.len()));
    }

    #[test]
    fn vs16_and_a_skin_tone_join_their_base() {
        assert_eq!(end("\u{2764}\u{FE0F} love"), Some("\u{2764}\u{FE0F}".len()));
        assert_eq!(end("\u{1F44D}\u{1F3FD}!"), Some("\u{1F44D}\u{1F3FD}".len()));
        assert_eq!(end("\u{6F22}\u{E0100}"), Some("\u{6F22}\u{E0100}".len()));
    }

    #[test]
    fn a_flag_is_two_regional_indicators() {
        let flags = "\u{1F1EC}\u{1F1F7}\u{1F1EF}\u{1F1F5}";
        assert_eq!(end(flags), Some(8));
        assert_eq!(cluster_end(flags, 8), Some(16));
        assert_eq!(end("\u{1F1EC}x"), None);
    }

    #[test]
    fn plain_text_and_a_trailing_zwj_compose_nothing() {
        assert_eq!(end("\u{2764} heart"), None);
        assert_eq!(end("\u{1F600}\u{1F600}"), None);
        assert_eq!(end("ab\u{FE0F}"), None, "an ASCII base is not composed (emoji-zwj.el's FIXME)");
        assert_eq!(end("\u{1F468}\u{200D}x"), None);
        assert_eq!(end("\u{05D0}\u{200D}\u{05D1}"), None, "a ZWJ joins emoji only");
    }
}
