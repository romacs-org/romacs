//! Implementation-independent acceptance seam for the future VM2 text owner.
//! This crate supplies no production text representation or coordinate index.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Multibyte,
    Unibyte,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidPosition;

/// Absolute, zero-based coordinates, independent of a buffer's restriction.
/// Inputs are valid GNU internal text; UTF-8 is only a subset of that encoding.
/// Failed operations leave text and cached correspondences unchanged.
pub trait Coordinates {
    fn len_bytes(&self) -> usize;
    fn len_chars(&self) -> usize;
    fn byte_of_char(&self, char_pos: usize) -> Result<usize, InvalidPosition>;
    fn char_of_byte(&self, byte_pos: usize) -> Result<usize, InvalidPosition>;
    fn replace(&mut self, bytes: Range<usize>, inserted: &[u8]) -> Result<(), InvalidPosition>;
    /// Reinterpret unchanged bytes; they must be valid in the target mode.
    /// General unibyte-to-multibyte normalization is a separate owner operation.
    fn set_mode(&mut self, mode: Mode);
}

/// Exhaustively check all boundaries against an independent linear oracle.
/// The head-byte rule applies to valid GNU internal multibyte text as well as UTF-8.
pub fn assert_coordinates<T: Coordinates>(text: &T, bytes: &[u8], mode: Mode) {
    let boundaries: Vec<_> = (0..=bytes.len())
        .filter(|&i| i == bytes.len() || mode == Mode::Unibyte || bytes[i] & 0xc0 != 0x80)
        .collect();
    assert_eq!(text.len_bytes(), bytes.len(), "byte length");
    assert_eq!(text.len_chars(), boundaries.len() - 1, "character length");
    // Reverse and interleaved queries defeat accidental forward-cursor correctness.
    for (c, &b) in boundaries.iter().enumerate().rev() {
        assert_eq!(text.byte_of_char(c), Ok(b), "char-to-byte at {c}");
        assert_eq!(text.char_of_byte(b), Ok(c), "byte-to-char at {b}");
        assert_eq!(text.byte_of_char(0), Ok(0), "start anchor");
    }
    for b in 0..bytes.len() {
        if !boundaries.contains(&b) {
            assert_eq!(
                text.char_of_byte(b),
                Err(InvalidPosition),
                "interior byte {b}"
            );
        }
    }
    assert_eq!(text.byte_of_char(boundaries.len()), Err(InvalidPosition));
    assert_eq!(text.char_of_byte(bytes.len() + 1), Err(InvalidPosition));
}

pub fn unicode_boundaries<T: Coordinates>(make: impl Fn(&[u8], Mode) -> T) {
    for bytes in [b"".as_slice(), b"abcdef", "aλβЖ🙂z".as_bytes()] {
        assert_coordinates(&make(bytes, Mode::Multibyte), bytes, Mode::Multibyte);
    }
}

pub fn boundaries<T: Coordinates>(make: impl Fn(&[u8], Mode) -> T) {
    for bytes in [
        b"".as_slice(),
        b"abcdef",
        "aλβЖ🙂z".as_bytes(),
        &[0xc0, 0x80, b'a', 0xf8, 0x88, 0x80, 0x80, 0x80],
    ] {
        let text = make(bytes, Mode::Multibyte);
        assert_coordinates(&text, bytes, Mode::Multibyte);
    }
    let raw = [0x00, 0x7f, 0x80, 0xc0, 0xff, b'z'];
    assert_coordinates(&make(&raw, Mode::Unibyte), &raw, Mode::Unibyte);
}

pub fn edits<T: Coordinates>(make: impl Fn(&[u8], Mode) -> T) {
    let mut expected = "aλβЖ🙂z".as_bytes().to_vec();
    let mut text = make(&expected, Mode::Multibyte);
    // Warm *every* correspondence before each edit, then exercise after edits
    // before/at/after anchors, including equal-character-count width changes.
    for (range, inserted) in [
        (0..0, "Ω".as_bytes()),
        (2..3, b"q".as_slice()),
        (3..3, "я".as_bytes()),
        (1..2, b"".as_slice()),
        (0..1, "🙂".as_bytes()),
    ] {
        assert_coordinates(&text, &expected, Mode::Multibyte);
        let boundaries: Vec<_> = (0..=expected.len())
            .filter(|&i| i == expected.len() || expected[i] & 0xc0 != 0x80)
            .collect();
        let byte_range = boundaries[range.start]..boundaries[range.end];
        text.replace(byte_range.clone(), inserted)
            .expect("valid edit");
        expected.splice(byte_range, inserted.iter().copied());
        assert_coordinates(&text, &expected, Mode::Multibyte);
    }
    let end = text.len_bytes();
    text.replace(end..end, "λ".as_bytes()).unwrap();
    expected.extend_from_slice("λ".as_bytes());
    assert_coordinates(&text, &expected, Mode::Multibyte);
    text.replace(0..expected.len(), b"ascii").unwrap();
    assert_coordinates(&text, b"ascii", Mode::Multibyte);
    text.replace(0..5, b"").unwrap();
    assert_coordinates(&text, b"", Mode::Multibyte);
}

pub fn edit_invalidation<T: Coordinates>(make: impl Fn(&[u8], Mode) -> T) {
    for (range, inserted) in [
        (0..0, b"x".as_slice()),
        (0..1, b"".as_slice()),
        (0..1, "🙂".as_bytes()),
        (3..5, b"Q".as_slice()),
        (5..7, "ΩΩ".as_bytes()),
        (11..12, "Ω".as_bytes()),
    ] {
        let mut bytes = "aλβЖ🙂z".as_bytes().to_vec();
        let mut text = make(&bytes, Mode::Multibyte);
        assert_coordinates(&text, &bytes, Mode::Multibyte);
        text.replace(range.clone(), inserted).unwrap();
        bytes.splice(range, inserted.iter().copied());
        assert_coordinates(&text, &bytes, Mode::Multibyte);
    }
}

pub fn rejected_edits<T: Coordinates>(make: impl Fn(&[u8], Mode) -> T) {
    let bytes = "aλЖz".as_bytes();
    let mut text = make(bytes, Mode::Multibyte);
    let reversed = Range { start: 4, end: 3 };
    for range in [2..3, 1..2, reversed, 0..bytes.len() + 1] {
        assert_eq!(text.replace(range, b"x"), Err(InvalidPosition));
        assert_coordinates(&text, bytes, Mode::Multibyte);
    }
}

pub fn reinterpretation<T: Coordinates>(make: impl Fn(&[u8], Mode) -> T) {
    let bytes = "aλЖ🙂z".as_bytes();
    let mut text = make(bytes, Mode::Multibyte);
    assert_coordinates(&text, bytes, Mode::Multibyte);
    text.set_mode(Mode::Unibyte);
    assert_coordinates(&text, bytes, Mode::Unibyte);
    text.set_mode(Mode::Multibyte);
    assert_coordinates(&text, bytes, Mode::Multibyte);
}

/// Restriction belongs to the buffer owner: it must not rebase text coordinates.
/// The caller supplies the actual narrow/widen operations; no fake buffer here.
pub trait RestrictedCoordinates: Coordinates {
    fn narrow(&mut self, chars: Range<usize>) -> Result<(), InvalidPosition>;
    fn widen(&mut self);
    fn accessible_chars(&self) -> Range<usize>;
}

/// The new buffer/string owner implements this Elisp-facing seam. Character
/// values are integers; offsets passed here remain zero-based character units.
pub trait EditorCoordinates: RestrictedCoordinates {
    fn mode(&self) -> Mode;
    fn character_at(&self, char_pos: usize) -> Result<Option<i64>, InvalidPosition>;
    fn slice_chars(&self, chars: Range<usize>) -> Result<Vec<u8>, InvalidPosition>;
    fn put_property(
        &mut self,
        chars: Range<usize>,
        key: &str,
        value: i64,
    ) -> Result<(), InvalidPosition>;
    fn property_at(&self, char_pos: usize, key: &str) -> Result<Option<i64>, InvalidPosition>;
}

pub fn unicode_editor_semantics<T: EditorCoordinates>(make: impl Fn(&[u8], Mode) -> T) {
    let mut text = make("aλβЖ🙂z".as_bytes(), Mode::Multibyte);
    assert_eq!(text.mode(), Mode::Multibyte);
    for (i, ch) in "aλβЖ🙂z".chars().enumerate() {
        let integer: i64 = text.character_at(i).unwrap().unwrap();
        assert_eq!(
            integer,
            i64::from(u32::from(ch)),
            "integer character at {i}"
        );
        assert_eq!(integer + 1 - 1, integer, "character arithmetic");
    }
    assert_eq!(text.character_at(6), Ok(None));
    assert_eq!(text.character_at(7), Err(InvalidPosition));
    assert_eq!(text.slice_chars(1..4), Ok("λβЖ".as_bytes().to_vec()));
    assert_eq!(text.slice_chars(2..2), Ok(vec![]));
    assert_eq!(text.slice_chars(0..7), Err(InvalidPosition));
    text.put_property(1..4, "probe", 7).unwrap();
    // Plain insertion has no inherited properties; surviving characters shift.
    text.replace(3..3, "Ω".as_bytes()).unwrap();
    assert_coordinates(&text, "aλΩβЖ🙂z".as_bytes(), Mode::Multibyte);
    for (c, value) in [None, Some(7), None, Some(7), Some(7), None, None]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            text.property_at(c, "probe"),
            Ok(value),
            "property after insert at {c}"
        );
    }
    text.replace(3..5, b"").unwrap();
    assert_coordinates(&text, "aλβЖ🙂z".as_bytes(), Mode::Multibyte);
    for (c, value) in [None, Some(7), Some(7), Some(7), None, None]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            text.property_at(c, "probe"),
            Ok(value),
            "property after delete at {c}"
        );
    }
    text.replace(3..5, "я".as_bytes()).unwrap();
    assert_coordinates(&text, "aλяЖ🙂z".as_bytes(), Mode::Multibyte);
    assert_eq!(text.property_at(2, "probe"), Ok(None));
    assert_eq!(text.property_at(3, "probe"), Ok(Some(7)));
}

pub fn editor_semantics<T: EditorCoordinates>(make: impl Fn(&[u8], Mode) -> T) {
    unicode_editor_semantics(&make);
    let raw = [b'a', 0xe9, 0xff];
    let text = make(&raw, Mode::Unibyte);
    assert_eq!(text.mode(), Mode::Unibyte);
    assert_eq!(text.character_at(1), Ok(Some(233)));
    assert_eq!(text.character_at(2), Ok(Some(255)));
    assert_eq!(text.slice_chars(1..3), Ok(vec![0xe9, 0xff]));
}

/// Test-only work accounting supplied by the text owner, not by a benchmark's
/// estimate. Counters are monotonic and exclude the independent test oracle.
#[derive(Clone, Copy, Debug, Default)]
pub struct CoordinateWork {
    pub scanned_bytes: usize,
    pub tree_steps: usize,
}

pub trait MeasuredCoordinates: Coordinates {
    fn coordinate_work(&self) -> CoordinateWork;
}

pub fn bounded_mixed_conversion_work<T: MeasuredCoordinates>(make: impl Fn(&[u8], Mode) -> T) {
    let fixture = "aλβЖ🙂z".repeat(4096);
    let text = make(fixture.as_bytes(), Mode::Multibyte);
    let mut offsets: Vec<_> = fixture.char_indices().map(|(b, _)| b).collect();
    offsets.push(fixture.len());
    let at = 1234;
    assert_eq!(text.byte_of_char(at), Ok(offsets[at]));
    let before = text.coordinate_work();
    for _ in 0..64 {
        assert_eq!(text.byte_of_char(at), Ok(offsets[at]));
        assert_eq!(text.char_of_byte(offsets[at]), Ok(at));
    }
    assert_eq!(
        text.coordinate_work().scanned_bytes - before.scanned_bytes,
        0,
        "exact anchor rescanned text"
    );
    let before = text.coordinate_work();
    for (c, &byte) in offsets.iter().enumerate().skip(at + 1).take(64) {
        assert_eq!(text.byte_of_char(c), Ok(byte));
    }
    assert!(
        text.coordinate_work().scanned_bytes - before.scanned_bytes <= 64 * 5,
        "adjacent mixed-script conversion rescanned a prefix"
    );
    for i in 1..65 {
        let c = (i * 997) % text.len_chars();
        let before = text.coordinate_work();
        assert_eq!(text.byte_of_char(c), Ok(offsets[c]));
        assert!(
            text.coordinate_work().scanned_bytes - before.scanned_bytes <= 512,
            "random conversion exceeded its summary window"
        );
    }
}

/// Legacy counted-ASCII contract, retained unchanged for its original owner.
pub fn bounded_conversion_work<T: MeasuredCoordinates>(make: impl Fn(&[u8], Mode) -> T) {
    bounded_mixed_conversion_work(&make);
    let ascii = vec![b'a'; 65536];
    let text = make(&ascii, Mode::Multibyte);
    let before = text.coordinate_work();
    for c in [0, 16383, 32768, 65536, 1, 65535] {
        assert_eq!(text.byte_of_char(c), Ok(c));
        assert_eq!(text.char_of_byte(c), Ok(c));
    }
    let after = text.coordinate_work();
    assert_eq!(
        after.scanned_bytes, before.scanned_bytes,
        "ASCII conversion scanned text"
    );
    assert_eq!(
        after.tree_steps, before.tree_steps,
        "ASCII conversion descended summaries"
    );
}

/// Uniform directory: ASCII has the same zero-scan logarithmic bound as Unicode.
pub fn bounded_unicode_conversion_work<T: MeasuredCoordinates>(make: impl Fn(&[u8], Mode) -> T) {
    bounded_mixed_conversion_work(&make);
    let text = make(&vec![b'a'; 65536], Mode::Multibyte);
    for c in [0, 16383, 32768, 65536, 1, 65535] {
        let before = text.coordinate_work();
        assert_eq!(text.byte_of_char(c), Ok(c));
        assert_eq!(text.char_of_byte(c), Ok(c));
        let after = text.coordinate_work();
        assert_eq!(
            after.scanned_bytes, before.scanned_bytes,
            "ASCII conversion scanned text"
        );
        assert!(
            after.tree_steps - before.tree_steps <= 2 * 16,
            "ASCII conversion exceeded logarithmic directory bound"
        );
    }
}

pub fn narrowing<T: RestrictedCoordinates>(make: impl Fn(&[u8], Mode) -> T) {
    let bytes = "aλβЖ🙂z".as_bytes();
    let mut text = make(bytes, Mode::Multibyte);
    assert_coordinates(&text, bytes, Mode::Multibyte);
    text.narrow(1..5).unwrap();
    assert_eq!(text.accessible_chars(), 1..5);
    assert_coordinates(&text, bytes, Mode::Multibyte);
    text.narrow(2..4).unwrap();
    assert_eq!(text.accessible_chars(), 2..4);
    // GNU narrow-to-region may expand beyond the current restriction, provided
    // the requested bounds are within the full text.
    text.narrow(0..6).unwrap();
    assert_eq!(text.accessible_chars(), 0..6);
    assert_eq!(text.narrow(0..7), Err(InvalidPosition));
    assert_eq!(text.accessible_chars(), 0..6);
    text.widen();
    assert_eq!(text.accessible_chars(), 0..6);
    assert_coordinates(&text, bytes, Mode::Multibyte);
}
