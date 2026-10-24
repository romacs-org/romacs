//! Baseline-only adapter: no changes to the inherited text implementation.
use romacs_chars::{BytePos, CharPos};
use romacs_coordinate_contract::{self as contract, Coordinates, InvalidPosition, Mode};
use romacs_text::Text;
use std::ops::Range;

struct Legacy(Text);
impl Coordinates for Legacy {
    fn len_bytes(&self) -> usize {
        self.0.len_bytes()
    }
    fn len_chars(&self) -> usize {
        self.0.len_chars()
    }
    fn byte_of_char(&self, c: usize) -> Result<usize, InvalidPosition> {
        if c > self.len_chars() {
            return Err(InvalidPosition);
        }
        Ok(self.0.byte_of_char(CharPos(c)).0)
    }
    fn char_of_byte(&self, b: usize) -> Result<usize, InvalidPosition> {
        if b > self.len_bytes() || !self.0.is_char_boundary(BytePos(b)) {
            return Err(InvalidPosition);
        }
        Ok(self.0.char_of_byte(BytePos(b)).0)
    }
    fn replace(&mut self, range: Range<usize>, inserted: &[u8]) -> Result<(), InvalidPosition> {
        if range.start > range.end {
            return Err(InvalidPosition);
        }
        self.char_of_byte(range.start)?;
        self.char_of_byte(range.end)?;
        self.0
            .replace(BytePos(range.start), BytePos(range.end), inserted);
        Ok(())
    }
    fn set_mode(&mut self, mode: Mode) {
        self.0.set_multibyte(mode == Mode::Multibyte);
    }
}
fn make(bytes: &[u8], mode: Mode) -> Legacy {
    Legacy(Text::from_bytes(bytes, mode == Mode::Multibyte))
}
#[test]
fn boundaries() {
    contract::boundaries(make);
}
#[test]
fn edits() {
    contract::edits(make);
}
#[test]
fn edit_invalidation() {
    contract::edit_invalidation(make);
}
#[test]
fn rejected_edits() {
    contract::rejected_edits(make);
}
#[test]
fn reinterpretation() {
    contract::reinterpretation(make);
}

// Named controlled fault: cache one answer forever across edits. This is test
// instrumentation, not an implementation offered to VM2. The unchanged suite
// must catch it at a coordinate assertion, not merely fail to compile.
struct Stale {
    inner: Legacy,
    anchor: std::cell::Cell<Option<(usize, usize)>>,
}
impl Coordinates for Stale {
    fn len_bytes(&self) -> usize {
        self.inner.len_bytes()
    }
    fn len_chars(&self) -> usize {
        self.inner.len_chars()
    }
    fn byte_of_char(&self, c: usize) -> Result<usize, InvalidPosition> {
        if let Some((cached, b)) = self.anchor.get()
            && cached == c
        {
            return Ok(b);
        }
        let b = self.inner.byte_of_char(c)?;
        if c == 3 && self.anchor.get().is_none() {
            self.anchor.set(Some((c, b)));
        }
        Ok(b)
    }
    fn char_of_byte(&self, b: usize) -> Result<usize, InvalidPosition> {
        self.inner.char_of_byte(b)
    }
    fn replace(&mut self, r: Range<usize>, b: &[u8]) -> Result<(), InvalidPosition> {
        self.inner.replace(r, b)
    }
    fn set_mode(&mut self, mode: Mode) {
        self.inner.set_mode(mode);
    }
}
fn stale(bytes: &[u8], mode: Mode) -> Stale {
    Stale {
        inner: make(bytes, mode),
        anchor: std::cell::Cell::new(None),
    }
}
#[test]
#[should_panic(expected = "char-to-byte at 3")]
fn stale_edit_anchor_is_detected() {
    contract::edits(stale);
}
#[test]
#[should_panic(expected = "char-to-byte at 3")]
fn stale_reinterpretation_anchor_is_detected() {
    contract::reinterpretation(stale);
}

#[test]
#[ignore = "intentional red probe; run with --ignored to see the target assertion fail"]
fn red_stale_edit_anchor() {
    contract::edits(stale);
}
#[test]
#[ignore = "intentional red probe; run with --ignored to see the target assertion fail"]
fn red_stale_mode_anchor() {
    contract::reinterpretation(stale);
}
