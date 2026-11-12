//! Faces realised once. A face's font, its metrics and each character
//! asked of it are found the first time a layout needs them and kept by
//! the face's content, for as long as the font rules hold; a frame then
//! publishes the faces its windows show from what is kept, binding the
//! characters their text draws, and no layout resolves a font it has
//! resolved before.
use crate::glyphless::{self, Glyphless};
use crate::{Metrics, color, font_request, is_icon, px, underline_placement, warn_once};
use romacs_display_protocol::font::{ResolvedCharGlyph, ResolvedFont, ResolvedFontId, ResolvedGlyphId};
use romacs_display_protocol::{Color, FaceId, FrameGlyphBuffer};
use romacs_font::{FontRequest, FontResolver};
use romacs_layout_input::cluster::is_composition_joiner;
use romacs_layout_input::{Face, NoFont, UnderlineRules};
use rustc_hash::FxHashMap;
use std::cell::{OnceCell, RefCell};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// A character as one face shows it.
#[derive(Clone, Debug)]
pub(crate) enum Bound {
    /// Not bound: the face's nominal advance and its own ascent and height
    /// (ASCII in a font of one cell width, a joiner, a character whose
    /// font would not open).
    Cell,
    /// A font's glyph, and that font's ascent and height.
    Font(ResolvedCharGlyph, (f32, f32)),
    /// No font has it: the primary font's `.notdef` at its box's advance,
    /// that font's ascent and height, and the box.
    Glyphless(ResolvedCharGlyph, (f32, f32), Rc<glyphless::Glyph>),
}

/// What a face's glyphless glyphs are laid out against, and the face their
/// text shows in (`glyphless-char`: the face at 0.6 of its height), when a
/// font could be had for it.
struct Boxes {
    base: glyphless::Base,
    digits: Option<Rc<Realized>>,
}

/// One face as layout uses it: the font it asked for after its fallbacks,
/// the font it got, its metrics, and what each character asked of it so
/// far shows as.
pub(crate) struct Realized {
    face: Face,
    /// The default face's family and size, tried when its own fail.
    fallback: Option<(String, u32)>,
    request: FontRequest,
    pub(crate) primary: ResolvedFont,
    pub(crate) metrics: Metrics,
    /// Its font has an advance per glyph: ASCII is bound too.
    per_glyph: bool,
    pub(crate) foreground: Color,
    pub(crate) background: Color,
    chars: RefCell<FxHashMap<char, Bound>>,
    boxes: OnceCell<Boxes>,
}

impl std::fmt::Debug for Realized {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Realized").field("face", &self.face).field("metrics", &self.metrics).finish_non_exhaustive()
    }
}

/// The ascent and height a character drawn in FONT adds to its row.
fn rise_of(font: &ResolvedFont) -> (f32, f32) {
    let ascent = font.ascent_px.round();
    (ascent, ascent + font.descent_px.round())
}

/// FACE's and DEFAULT's parts that decide what `realize` makes, hashed.
fn key_hash(face: &Face, fallback: Option<(&str, u32)>) -> u64 {
    let mut hasher = rustc_hash::FxHasher::default();
    (face.foreground, face.background, face.family.as_str(), face.pixel_size.to_bits(), face.weight, face.italic, face.width, face.decor, fallback)
        .hash(&mut hasher);
    hasher.finish()
}

/// The family and size a face falls back to: DEFAULT's, when it names both.
fn fallback_of(default: Option<&Face>) -> Option<(&str, u32)> {
    default.filter(|d| !d.family.is_empty() && d.pixel_size.is_finite() && d.pixel_size > 0.0).map(|d| (d.family.as_str(), d.pixel_size.to_bits()))
}

/// Every face realised under the font rules in force, by content, the
/// fonts their characters resolved to, and how a character no font has is
/// shown.
#[derive(Default)]
pub(crate) struct Faces {
    by_key: FxHashMap<u64, Vec<Rc<Realized>>>,
    fonts: FxHashMap<ResolvedFontId, ResolvedFont>,
    no_font: NoFont,
}

impl Faces {
    /// Adopt `NO_FONT`; true when it changed, which drops every face: their
    /// glyphless glyphs were laid out under the old one.
    pub(crate) fn set_no_font(&mut self, no_font: &NoFont) -> bool {
        if self.no_font == *no_font { return false; }
        self.no_font = no_font.clone();
        self.clear();
        true
    }

    /// How many faces are realised.
    pub(crate) fn len(&self) -> usize { self.by_key.values().map(Vec::len).sum() }

    /// Forget every face: the font rules changed, or too many were made.
    pub(crate) fn clear(&mut self) {
        self.by_key.clear();
        self.fonts.clear();
    }

    /// FACE realised, DEFAULT its fallback (the table's first face; none
    /// for a face realised alone): the one kept for that content, or made
    /// now. Nothing here stops the display: a face whose font cannot be
    /// had falls back to DEFAULT's family and then to `monospace` (GNU
    /// falls back to the frame's font), a font without a positive advance
    /// to its space or half its size.
    pub(crate) fn realize(&mut self, face: &Face, default: Option<&Face>, resolver: &mut FontResolver) -> Result<Rc<Realized>, String> {
        let fallback = fallback_of(default);
        let hash = key_hash(face, fallback);
        let same = |kept: &&Rc<Realized>| kept.face == *face && kept.fallback.as_ref().map(|(family, size)| (family.as_str(), *size)) == fallback;
        if let Some(kept) = self.by_key.get(&hash).and_then(|kept| kept.iter().find(same)) { return Ok(kept.clone()); }
        let made = Rc::new(realize(face, fallback, resolver)?);
        self.by_key.entry(hash).or_default().push(made.clone());
        Ok(made)
    }
}

/// FACE realised: its font after FALLBACK's family and `monospace`, and
/// its ascent, height, nominal advance and space width.
fn realize(face: &Face, fallback: Option<(&str, u32)>, resolver: &mut FontResolver) -> Result<Realized, String> {
    let pixel_size = if face.pixel_size.is_finite() && face.pixel_size > 0.0 { face.pixel_size }
        else { fallback.map_or(16.0, |(_, size)| f32::from_bits(size)) };
    let asked = FontRequest { pixel_size, ..font_request(face) };
    // The families tried after the face's own, built only when it fails.
    let fallbacks = [fallback.map(|(family, _)| family), Some("monospace")];
    let mut failure = None;
    let (request, primary) = 'found: {
        let mut tried: Vec<&str> = vec![];
        for family in std::iter::once(None).chain(fallbacks.into_iter().flatten().map(Some)) {
            let request = match family {
                None => asked.clone(),
                Some(family) if family == asked.family || tried.contains(&family) => continue,
                Some(family) => { tried.push(family); FontRequest { family: family.to_owned(), ..asked.clone() } }
            };
            if request.family.is_empty() { continue; }
            match resolver.resolve_primary(&request) {
                Ok(primary) => break 'found (request, primary),
                Err(error) => { failure.get_or_insert(error); }
            }
        }
        return Err(format!("no font for face {:?}: {}", face.family, failure.map_or_else(String::new, |e| e.to_string())));
    };
    if let Some(error) = failure { warn_once(format!("face family {:?} shows in {:?}: {error}", face.family, request.family)); }
    let height = (primary.ascent_px + primary.descent_px).ceil();
    let space = primary.space_advance_px;
    let per_glyph = primary.glyph_advance.cell_advance_px().is_none();
    let mut cell = primary.glyph_advance.cell_advance_px().unwrap_or(space);
    if !cell.is_finite() || cell <= 0.0 {
        warn_once(format!("font {:?} has no positive nominal advance", primary.family));
        cell = if space.is_finite() && space > 0.0 { space } else { (pixel_size / 2.0).round().max(1.0) };
    }
    // A box's edges outside the text grow the face's row by their width
    // above and below (produce_glyphs: `it->ascent += thick`).
    let thick = face.decor.boxed.map_or(0.0, |b| f32::from(b.horizontal.max(0)));
    Ok(Realized {
        fallback: fallback.map(|(family, size)| (family.to_owned(), size)),
        metrics: (primary.ascent_px + thick, height + 2.0 * thick, cell, space),
        per_glyph,
        foreground: color(face.foreground),
        background: color(face.background),
        face: face.clone(),
        request,
        primary,
        chars: RefCell::default(),
        boxes: OnceCell::new(),
    })
}

impl Realized {
    /// Its box, when it has one.
    pub(crate) fn boxed(&self) -> Option<romacs_layout_input::BoxLine> { self.face.decor.boxed }

    /// Whether its font binds ASCII too: one with an advance per glyph.
    pub(crate) fn binds_ascii(&self) -> bool { self.per_glyph }

    /// Its background as 0xRRGGBB.
    pub(crate) fn background_pixel(&self) -> u32 {
        let [r, g, b] = self.face.background;
        u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
    }

    /// What CH shows as in this face: kept, or resolved now.
    pub(crate) fn bound(&self, ch: char, faces: &mut Faces, resolver: &mut FontResolver) -> Bound {
        // ASCII is the face's own font (face_for_char); only a font with
        // an advance per glyph binds it, and never a control character.
        if ch.is_ascii() && (!self.per_glyph || ch.is_ascii_control()) { return Bound::Cell; }
        if let Some(kept) = self.chars.borrow().get(&ch) { return kept.clone(); }
        let bound = self.resolve(ch, faces, resolver);
        self.chars.borrow_mut().insert(ch, bound.clone());
        bound
    }

    fn resolve(&self, ch: char, faces: &mut Faces, resolver: &mut FontResolver) -> Bound {
        let icon = is_icon(ch);
        if !ch.is_ascii() && !icon && is_composition_joiner(ch) { return Bound::Cell; }
        // Icons are bound from the resolver's one answer for them.
        let answer = if icon { resolver.resolve_sole(&self.request, ch).cloned() } else { resolver.resolve_char(&self.request, ch) };
        match answer {
            Ok(resolution) => {
                let rise = rise_of(&resolution.font);
                faces.fonts.entry(resolution.font.id).or_insert(resolution.font);
                Bound::Font(resolution.glyph, rise)
            }
            Err(error) if ch.is_ascii() => {
                warn_once(format!("{ch:?} in {:?}: {error}", self.request.family));
                Bound::Cell
            }
            Err(error) => {
                warn_once(format!("{}U+{:04X} shows glyphless: {error}", if icon { "icon " } else { "" }, ch as u32));
                self.glyphless(ch, faces, resolver)
            }
        }
    }

    /// GNU's no-font glyph for CH (`produce_glyphless_glyph`), its code in
    /// `glyphless-char` and its box in the face's foreground.
    fn glyphless(&self, ch: char, faces: &mut Faces, resolver: &mut FontResolver) -> Bound {
        let boxes = self.boxes(faces, resolver);
        let digit_font = boxes.digits.as_ref().map(|digits| &digits.primary);
        let mut extents = |text: &str| digit_font.and_then(|font| resolver.ink_extents(font, text));
        let glyph = glyphless::layout(&faces.no_font, ch, boxes.base, &mut extents);
        let binding = ResolvedCharGlyph { resolved_font_id: self.primary.id, glyph_id: ResolvedGlyphId::new(0), advance_px: px::signed(i64::from(glyph.width)) };
        Bound::Glyphless(binding, rise_of(&self.primary), Rc::new(glyph))
    }

    fn boxes(&self, faces: &mut Faces, resolver: &mut FontResolver) -> &Boxes {
        self.boxes.get_or_init(|| {
            let face = &self.face;
            let small = Face { pixel_size: (face.pixel_size * glyphless::DIGIT_SCALE).round().max(1.0), decor: romacs_layout_input::Decor::default(), ..face.clone() };
            let digits = faces.realize(&small, None, resolver).ok();
            let width = resolver.ink_extents(&self.primary, "x").map_or(self.metrics.2, |x| x.width);
            let width = px::whole(width);
            Boxes { base: glyphless::Base { ascent: px::whole(self.primary.ascent_px), descent: px::whole(self.primary.descent_px), width }, digits }
        })
    }

    /// The advance CH takes in this face: its glyph's, else the face's
    /// nominal one; an icon is one column in any family and size.
    pub(crate) fn advance(&self, ch: char, faces: &mut Faces, resolver: &mut FontResolver) -> f32 {
        let cell = self.metrics.2;
        if ch.is_ascii() && !self.per_glyph || is_icon(ch) { return cell; }
        match self.bound(ch, faces, resolver) {
            Bound::Cell => cell,
            Bound::Font(glyph, _) | Bound::Glyphless(glyph, ..) => glyph.advance_px,
        }
    }

    /// The ascent and height CH adds to its row (`gui_produce_glyphs`):
    /// those of the font it is shown in, which a character outside this
    /// face's font takes from its fontset; the face's otherwise, as for
    /// every ASCII character.
    pub(crate) fn rise(&self, ch: char, faces: &mut Faces, resolver: &mut FontResolver) -> (f32, f32) {
        let face = (self.metrics.0, self.metrics.1);
        if ch.is_ascii() { return face; }
        match self.bound(ch, faces, resolver) {
            Bound::Cell => face,
            Bound::Font(_, rise) | Bound::Glyphless(_, rise, _) => rise,
        }
    }

    /// CH's box, when no font has CH.
    pub(crate) fn boxed_char(&self, ch: char, faces: &mut Faces, resolver: &mut FontResolver) -> Option<Rc<glyphless::Glyph>> {
        if ch.is_ascii() { return None; }
        match self.bound(ch, faces, resolver) {
            Bound::Glyphless(_, _, glyph) => Some(glyph),
            _ => None,
        }
    }

    /// The face a glyphless glyph's text shows in and its nominal advance.
    pub(crate) fn digits(&self, faces: &mut Faces, resolver: &mut FontResolver) -> Option<Rc<Realized>> {
        self.boxes(faces, resolver).digits.clone()
    }

    /// Publish this face as ID in FRAME: its colours, lines and box, its
    /// font, and where RULES put its underline.
    fn publish(&self, frame: &mut FrameGlyphBuffer, id: FaceId, rules: UnderlineRules) {
        let face = &self.face;
        let line = |line: Option<romacs_layout_input::Line>| line.map_or((0, None), |l| (l.style, l.color.map(color)));
        let ((underline, underline_color), (strike, strike_color), (overline, overline_color)) =
            (line(face.decor.underline), line(face.decor.strike), line(face.decor.overline));
        frame.set_face_with_font(id, self.foreground, Some(self.background), &self.request.family, face.weight, face.italic, self.request.pixel_size,
            underline, underline_color, strike, strike_color, overline, overline_color, false);
        frame.fonts.entry(self.primary.id).or_insert_with(|| self.primary.clone());
        let placement = underline_placement(face, &self.primary, rules);
        let Some(published) = frame.faces.get_mut(&id) else { return };
        published.default_resolved_font_id = Some(self.primary.id);
        (published.font_ascent, published.font_descent) = (px::whole(self.primary.ascent_px), px::whole(self.primary.descent_px));
        (published.underline_placement, published.underline_thickness) = placement;
        if let Some(boxed) = face.decor.boxed {
            use romacs_display_protocol::face::{BoxLineWidth, BoxType};
            published.box_type = BoxType::from_gnu_code(boxed.style).unwrap_or(BoxType::Line);
            published.box_line_width = BoxLineWidth::from_gnu(i32::from(boxed.horizontal));
            published.box_vertical_line_width = Some(BoxLineWidth::from_gnu(i32::from(boxed.vertical.unsigned_abs())));
            published.box_color = boxed.color.map(color);
        }
    }

    /// Bind CH in ID of FRAME as this face shows it; false when no font has it.
    fn bind(&self, frame: &mut FrameGlyphBuffer, id: FaceId, ch: char, faces: &mut Faces, resolver: &mut FontResolver) -> bool {
        match self.bound(ch, faces, resolver) {
            Bound::Cell => true,
            Bound::Glyphless(..) => false,
            Bound::Font(glyph, _) => {
                if let Some(font) = faces.fonts.get(&glyph.resolved_font_id) {
                    frame.fonts.entry(font.id).or_insert_with(|| font.clone());
                }
                frame.char_fonts.entry(id).or_default().insert(ch, glyph);
                true
            }
        }
    }
}

/// What a window's published text shows in one face of its table: whether
/// any character does (USED), the ASCII characters drawn (a bit each; kept
/// only for a face whose font binds them), the characters beyond ASCII
/// that take a glyph (WIDE: no joiner), and whether an emoji presentation
/// selector is among them (the font of such a sequence is the emoji
/// font's, found through U+1F600).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shows {
    pub(crate) used: bool,
    pub(crate) ascii: u128,
    pub(crate) wide: Vec<char>,
    pub(crate) selector: bool,
}

impl Shows {
    /// OTHER's characters too.
    pub(crate) fn add(&mut self, other: &Shows) {
        self.used |= other.used;
        self.ascii |= other.ascii;
        self.selector |= other.selector;
        if !other.wide.is_empty() { self.wide.extend_from_slice(&other.wide); }
    }

    /// TEXT's characters too, each drawn as itself.
    pub(crate) fn add_text(&mut self, text: &str) {
        for ch in text.chars() { self.add_char(ch); }
    }

    /// CH too, drawn as itself.
    pub(crate) fn add_char(&mut self, ch: char) {
        self.used = true;
        if ch.is_ascii() { self.ascii |= 1 << (ch as u32); }
        else if ch == '\u{FE0F}' { self.selector = true; }
        else if !is_composition_joiner(ch) { self.wide.push(ch); }
    }
}

/// A window's face table, each face realised when first asked for.
pub(crate) struct Table<'a> {
    faces: &'a [Face],
    realized: Vec<Option<Rc<Realized>>>,
}

impl<'a> Table<'a> {
    pub(crate) fn new(faces: &'a [Face]) -> Self {
        Table { faces, realized: vec![None; faces.len()] }
    }

    pub(crate) fn len(&self) -> usize { self.faces.len() }

    /// The face at INDEX, realised.
    pub(crate) fn get(&mut self, index: usize, faces: &mut Faces, resolver: &mut FontResolver) -> Result<Rc<Realized>, String> {
        if let Some(kept) = self.realized.get(index).and_then(Option::as_ref) { return Ok(kept.clone()); }
        let face = self.faces.get(index).ok_or("missing face")?;
        let made = faces.realize(face, self.faces.first(), resolver)?;
        self.realized[index] = Some(made.clone());
        Ok(made)
    }
}

/// Publish TABLE's faces from `FACE_BASE` on in FRAME, as a window that
/// shows SHOWN in each needs them: each face some character uses, the
/// default and those KEEP accepts, with the characters of SHOWN bound, and
/// the glyphless glyphs of the characters no font has (`glyphless`), in
/// faces published above every copy of the table a window's bands make.
/// Each face's ascent, height, nominal advance and space width; a face
/// not published answers zeros.
#[allow(clippy::too_many_arguments, reason = "one window's publication: its table, what it shows, and where it goes")]
pub(crate) fn publish(table: &mut Table, shown: &[Shows], keep: impl Fn(usize) -> bool, frame: &mut FrameGlyphBuffer, face_base: u32,
                      rules: UnderlineRules, faces: &mut Faces, resolver: &mut FontResolver) -> Result<(Vec<Metrics>, Glyphless), String> {
    let count = table.len();
    let mut metrics = Vec::with_capacity(count);
    let mut glyphless = Glyphless::default();
    let len = u32::try_from(count).map_err(|e| e.to_string())?;
    // Above a window's band copies (face_base + 1 and 2 tables): each
    // face's digit and box-line faces, then two copies of its digit face.
    let derived = len.checked_mul(3).and_then(|n| n.checked_add(face_base)).ok_or("too many faces")?;
    glyphless.end = len.checked_mul(4).and_then(|n| n.checked_add(derived)).ok_or("too many faces")?;
    let id_at = |base: u32, index: usize| u32::try_from(index).ok().and_then(|i| base.checked_add(i)).map(FaceId::new).ok_or("too many faces");
    for (index, shows) in shown.iter().enumerate().take(count) {
        if index != 0 && !keep(index) && !shows.used {
            metrics.push((0.0, 0.0, 0.0, 0.0));
            continue;
        }
        let id = id_at(face_base, index)?;
        let real = table.get(index, faces, resolver)?;
        real.publish(frame, id, rules);
        metrics.push(real.metrics);
        let mut wide = shows.wide.clone();
        if shows.selector { wide.push('\u{1F600}'); }
        wide.sort_unstable();
        wide.dedup();
        let unresolved: Vec<char> = wide.into_iter().filter(|&ch| !real.bind(frame, id, ch, faces, resolver)).collect();
        if real.per_glyph { bind_ascii(&real, frame, id, shows.ascii, faces, resolver); }
        if unresolved.is_empty() { continue; }
        let (digits, line) = (id_at(derived, 2 * index)?, id_at(derived, 2 * index + 1)?);
        let spare = [id_at(derived, 2 * count + 2 * index)?, id_at(derived, 2 * count + 2 * index + 1)?];
        let digit_face = real.digits(faces, resolver);
        if let Some(digit_face) = &digit_face {
            digit_face.publish(frame, digits, UnderlineRules::default());
            if digit_face.per_glyph {
                let mut text = Shows::default();
                for &ch in &unresolved { if let Some(code) = glyphless::text_of(&faces.no_font, ch) { text.add_text(&code); } }
                bind_ascii(digit_face, frame, digits, text.ascii, faces, resolver);
            }
        }
        let face = &real.face;
        frame.set_face_with_font(line, real.foreground, Some(real.foreground), &face.family, face.weight, face.italic, face.pixel_size, 0, None, 0, None, 0, None, false);
        let shown_in = glyphless::Faces { digits: digit_face.as_ref().map(|d| (digits, d.metrics.2)), line, spare };
        for ch in unresolved {
            if let Bound::Glyphless(_, _, glyph) = real.bound(ch, faces, resolver) {
                glyphless.bind(frame, (index, id), &real.primary, ch, (*glyph).clone(), shown_in);
            }
        }
    }
    Ok((metrics, glyphless))
}

/// Bind the printable ASCII characters of ASCII, a bit each, in ID.
fn bind_ascii(real: &Realized, frame: &mut FrameGlyphBuffer, id: FaceId, ascii: u128, faces: &mut Faces, resolver: &mut FontResolver) {
    for code in (0x20u8..0x7f).filter(|&code| ascii >> code & 1 == 1) {
        real.bind(frame, id, char::from(code), faces, resolver);
    }
}
