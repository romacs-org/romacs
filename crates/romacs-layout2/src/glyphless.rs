//! Characters no installed font has a glyph for, shown as GNU shows them
//! on a window-system frame (xdisp.c produce_glyphless_glyph, xterm.c
//! x_draw_glyphless_glyph_string_foreground): per `glyphless-char-display''s
//! `no-font' slot, a one-pixel thin space, an empty box, or a box holding
//! the character's code (or an acronym) in two rows of small digits.
//!
//! A glyphless character is bound in its face's character table to the
//! primary font's `.notdef' at the glyph's advance, so every measurement
//! and row break reads the same width painting uses; painting draws the
//! box from here instead of a glyph.

use romacs_display_protocol::font::{ResolvedCharGlyph, ResolvedFont, ResolvedGlyphId};
use romacs_display_protocol::{Color, FaceId, FrameGlyphBuffer};
use romacs_font::InkExtents;
use romacs_layout_input::NoFont;
use std::collections::HashMap;

/// `glyphless-char''s default height on a graphical frame (faces.el).
pub(crate) const DIGIT_SCALE: f32 = 0.6;

/// The base font's metrics a glyphless glyph is laid out against: its
/// ascent and descent, and its average width (`font->average_width').
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Base {
    pub ascent: i32,
    pub descent: i32,
    pub width: i32,
}

/// One row of a box's text: what it shows, its x from the glyph's left
/// and its baseline's offset below the row's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Half {
    pub text: String,
    pub x: i32,
    pub y: i32,
}

/// A glyphless character's glyph: its advance, whether it is boxed (all
/// but a thin space), and the rows of text in the box.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub width: i32,
    pub boxed: bool,
    pub halves: Vec<Half>,
}

/// What METHOD shows for CH (lookup_glyphless_char_display for the no-font
/// case): `empty-box' when an acronym has no ASCII text to show.
pub(crate) fn text_of(method: &NoFont, ch: char) -> Option<String> {
    match method {
        NoFont::ThinSpace | NoFont::EmptyBox => None,
        NoFont::HexCode => Some(if (ch as u32) < 0x10000 { format!("{:04X}", ch as u32) } else { format!("{:06X}", ch as u32) }),
        NoFont::Acronym(text) => Some(text.chars().take_while(char::is_ascii).take(6).collect()),
    }
}

/// produce_glyphless_glyph for CH under METHOD against BASE, with EXTENTS
/// measuring a run of the box's text in `glyphless-char''s font (None when
/// it cannot: the box is then empty).
pub(crate) fn layout(method: &NoFont, ch: char, base: Base, extents: &mut dyn FnMut(&str) -> Option<InkExtents>) -> Glyph {
    let empty = Glyph { width: base.width.max(1), boxed: true, halves: vec![] };
    let text = match method {
        NoFont::ThinSpace => return Glyph { width: 1, boxed: false, halves: vec![] },
        _ => text_of(method, ch),
    };
    let Some(text) = text.filter(|text| !text.is_empty()) else { return empty };
    let upper_len = text.len().div_ceil(2);
    let (upper, lower) = text.split_at(upper_len);
    let (Some(up), Some(low)) = (extents(upper), extents(lower).or(lower.is_empty().then_some(InkExtents::default()))) else { return empty };
    let px = |value: f32| value.round() as i32;
    let (up_w, low_w) = (px(up.width), px(low.width));
    // +4: the box's two bars and a pixel's space inside each.
    let boxed = up_w.max(low_w) + 4;
    let (width, up_x, low_x) = if base.width >= boxed {
        // Upper row to the left, lower to the right.
        (base.width, 2, base.width - 2 - low_w)
    } else if up_w >= low_w {
        (boxed, 2, (boxed - low_w) / 2)
    } else {
        (boxed, (boxed - up_w) / 2, 2)
    };
    // +5: the two bars, a pixel above and below, and one between the rows.
    let height = px(up.ascent) + px(up.descent) + px(low.ascent) + px(low.descent) + 5;
    let base_height = base.ascent + base.descent;
    // Centred vertically, as C's integer division rounds.
    let descent = base.descent - (base_height - height) / 2;
    let low_y = descent - 2 - px(low.descent);
    let up_y = low_y - px(low.ascent) - 1 - px(up.descent);
    let mut halves = vec![Half { text: upper.to_owned(), x: up_x, y: up_y }];
    if !lower.is_empty() { halves.push(Half { text: lower.to_owned(), x: low_x, y: low_y }); }
    Glyph { width, boxed: true, halves }
}

/// The glyphless glyphs one face table's layout made: each character's
/// by face index, and each such face's digit face (with its nominal
/// advance) and box-line face; END, the first face id above every face
/// they reserve.
#[derive(Default)]
pub(crate) struct Glyphless {
    glyphs: HashMap<(usize, char), Glyph>,
    faces: HashMap<usize, Faces>,
    /// The digit faces' copies made on other backgrounds, by face index.
    copies: std::cell::RefCell<HashMap<usize, Vec<([u32; 4], FaceId)>>>,
    pub end: u32,
}

/// The faces a glyphless glyph is drawn in: its text's, `glyphless-char'
/// over the character's, with its nominal advance (none when no font
/// could be had for it); its box's, whose background is the character
/// face's foreground.
/// SPARE are the ids reserved for the digit face's copies on a band's
/// background (a window's header and mode line).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Faces {
    pub digits: Option<(FaceId, f32)>,
    pub line: FaceId,
    pub spare: [FaceId; 2],
}

impl Glyphless {
    pub(crate) fn get(&self, index: usize, ch: char) -> Option<&Glyph> {
        if self.glyphs.is_empty() { return None; }
        self.glyphs.get(&(index, ch))
    }

    /// Record CH's GLYPH for face INDEX (published as ID over PRIMARY),
    /// drawn in FACES, binding CH to PRIMARY's
    /// `.notdef' at GLYPH's advance.
    pub(crate) fn bind(&mut self, frame: &mut FrameGlyphBuffer, (index, id): (usize, FaceId), primary: &ResolvedFont,
                       ch: char, glyph: Glyph, faces: Faces) {
        let binding = ResolvedCharGlyph { resolved_font_id: primary.id, glyph_id: ResolvedGlyphId::new(0), advance_px: glyph.width as f32 };
        frame.char_fonts.entry(id).or_default().insert(ch, binding);
        self.faces.insert(index, faces);
        self.glyphs.insert((index, ch), glyph);
    }

    /// Paint GLYPH, CH's in face INDEX, at X on a row from Y, HEIGHT tall
    /// with its baseline ASCENT below Y, over the background of FACE: the
    /// background, the box in the face's foreground, and inside it the
    /// box's text in the digit face on that background. In a band whose
    /// clicks find characters by their glyphs (the tab and status bars,
    /// ONE_GLYPH), the box is empty and a blank character glyph inside it
    /// stands for CH.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint(&self, frame: &mut FrameGlyphBuffer, index: usize, glyph: &Glyph, x: f32, y: f32,
                        (ascent, height): (f32, f32), face: FaceId, one_glyph: bool) -> Result<(), String> {
        let width = glyph.width as f32;
        let bg = frame.faces.get(&face).map(|f| f.background).unwrap_or_default();
        let fill = |frame: &mut FrameGlyphBuffer, face: FaceId, (x, y, w, h): (f32, f32, f32, f32)| {
            let bg = frame.faces.get(&face).map(|f| f.background).unwrap_or_default();
            frame.add_stretch(x, y, w, h, bg, face, false);
        };
        fill(frame, face, (x, y, width, height));
        // The text's cells stay inside the bars, which their backgrounds
        // would otherwise cover.
        let (top, inside) = if glyph.boxed { (y + 1.0, (height - 2.0).max(0.0)) } else { (y, height) };
        if one_glyph {
            let left = if glyph.boxed { 1.0 } else { 0.0 };
            crate::glyph_of(frame, ' ', x + left, top, (width - 2.0 * left).max(0.0), (ascent - (top - y), inside), face)?;
        }
        let Some(&Faces { digits, line, spare }) = self.faces.get(&index) else { return Ok(()) };
        if glyph.boxed && width >= 1.0 && height >= 1.0 {
            for rect in [(x, y, width, 1.0), (x, y + height - 1.0, width, 1.0), (x, y, 1.0, height), (x + width - 1.0, y, 1.0, height)] {
                fill(frame, line, rect);
            }
        }
        let Some((digits, cell)) = digits.filter(|_| !one_glyph) else { return Ok(()) };
        let digits = self.on_background(frame, (index, digits), bg, spare);
        for half in &glyph.halves {
            let mut at = x + half.x as f32;
            for c in half.text.chars() {
                let advance = crate::advance(frame, digits, c, cell);
                crate::glyph_of(frame, c, at, top, advance, (ascent + half.y as f32 - (top - y), inside), digits)?;
                at += advance;
            }
        }
        Ok(())
    }

    /// DIGITS, face INDEX's, or its copy on BG (a band's copy of the
    /// character's face paints on the band's background) as one of SPARE;
    /// beyond those, DIGITS itself.
    fn on_background(&self, frame: &mut FrameGlyphBuffer, (index, digits): (usize, FaceId), bg: Color, spare: [FaceId; 2]) -> FaceId {
        let Some(face) = frame.faces.get(&digits) else { return digits };
        if face.background == bg { return digits; }
        let key = [bg.r.to_bits(), bg.g.to_bits(), bg.b.to_bits(), bg.a.to_bits()];
        let mut copies = self.copies.borrow_mut();
        let made = copies.entry(index).or_default();
        if let Some(&(_, id)) = made.iter().find(|(made, _)| *made == key) { return id; }
        let Some(&id) = spare.get(made.len()) else { return digits };
        let mut copy = face.clone();
        (copy.id, copy.background) = (id, bg);
        frame.faces.insert(id, copy);
        if let Some(chars) = frame.char_fonts.get(&digits).cloned() { frame.char_fonts.insert(id, chars); }
        made.push((key, id));
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed-pitch digits: 5 pixels each, 7 above the baseline.
    fn digits(text: &str) -> Option<InkExtents> {
        Some(InkExtents { width: 5.0 * text.len() as f32, ascent: 7.0, descent: 0.0 })
    }

    const BASE: Base = Base { ascent: 15, descent: 4, width: 10 };

    #[test]
    fn hex_code_is_two_rows_of_the_code_in_a_box() {
        let glyph = layout(&NoFont::HexCode, '\u{2fff}', BASE, &mut digits);
        // Rows "2F" and "FF", 10 wide: the box is 14, wider than the base.
        assert_eq!(glyph.width, 14);
        assert!(glyph.boxed);
        assert_eq!(glyph.halves, vec![Half { text: "2F".into(), x: 2, y: -6 }, Half { text: "FF".into(), x: 2, y: 2 }]);
    }

    #[test]
    fn a_code_beyond_the_bmp_shows_six_digits() {
        let glyph = layout(&NoFont::HexCode, '\u{10ffff}', BASE, &mut digits);
        assert_eq!(glyph.width, 19);
        assert_eq!(glyph.halves.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["10F", "FFF"]);
    }

    #[test]
    fn a_narrow_code_aligns_its_rows_to_the_base_width() {
        let narrow = |text: &str| Some(InkExtents { width: 2.0 * text.len() as f32, ascent: 7.0, descent: 0.0 });
        let glyph = layout(&NoFont::HexCode, '\u{e000}', BASE, &mut { narrow });
        assert_eq!(glyph.width, 10);
        assert_eq!((glyph.halves[0].x, glyph.halves[1].x), (2, 4));
    }

    #[test]
    fn thin_space_is_one_pixel_and_unboxed() {
        assert_eq!(layout(&NoFont::ThinSpace, 'x', BASE, &mut digits), Glyph { width: 1, boxed: false, halves: vec![] });
    }

    #[test]
    fn empty_box_is_the_base_width() {
        assert_eq!(layout(&NoFont::EmptyBox, 'x', BASE, &mut digits), Glyph { width: 10, boxed: true, halves: vec![] });
    }

    #[test]
    fn an_acronym_shows_its_ascii_halves() {
        let glyph = layout(&NoFont::Acronym("ZWSP".into()), '\u{200b}', BASE, &mut digits);
        assert_eq!(glyph.halves.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), ["ZW", "SP"]);
        assert_eq!(layout(&NoFont::Acronym("é".into()), 'x', BASE, &mut digits).halves, vec![]);
    }

    #[test]
    fn unmeasurable_text_leaves_an_empty_box() {
        let glyph = layout(&NoFont::HexCode, '\u{2fff}', BASE, &mut |_| None);
        assert_eq!(glyph, Glyph { width: 10, boxed: true, halves: vec![] });
    }
}
