//! A display line as a snapshot gives it: one piece of a window's
//! published text between the newlines that end rows, with everything the
//! snapshot says about it, offsets counted from its own start and faces
//! named by content. It is the key the line cache compares and the input
//! rows are wrapped from, so two lines that are equal here lay out alike
//! in one context, whichever window, snapshot or face table they came in.
use crate::faces::{Faces, Realized, Shows, Table};
use romacs_font::FontResolver;
use romacs_layout_input::cluster::{cluster_end, is_composition_joiner};
use romacs_layout_input::{InlineImage, StretchWidth, WindowSnapshot};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// The bytes START to END of a line in the face SLOT of the line names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Run {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) slot: u16,
}

/// One line. Its faces are numbered in order of first appearance, slot 0
/// the default face; a character outside every run shows in slot 0.
#[derive(Debug)]
pub(crate) struct LineText {
    /// Its characters, without the newline that ends it.
    pub(crate) text: Box<str>,
    /// Ordered and disjoint; neighbours in one slot are one run.
    pub(crate) runs: Box<[Run]>,
    pub(crate) faces: Box<[Rc<Realized>]>,
    /// The bytes of raw-byte characters.
    pub(crate) raw: Box<[usize]>,
    pub(crate) stretches: Box<[(usize, StretchWidth)]>,
    /// Each with its byte counted from the line's start.
    pub(crate) images: Box<[InlineImage]>,
    pub(crate) flips: Box<[usize]>,
    /// The line numbers' labels inside it, past its first byte: where a
    /// buffer line starts that a hidden newline joined to this one, and
    /// how many glyphs its label has. A row from there on starts after a
    /// gutter that wide.
    pub(crate) labels: Box<[(usize, u16)]>,
    /// The face its last row paints past its text in.
    pub(crate) extend: Option<u16>,
    /// The face of the newline that ends it; none for the line the text
    /// ends in, whose last row ends as the text does.
    pub(crate) newline: Option<u16>,
    /// What each slot draws, the newline's escape included.
    pub(crate) shows: Box<[Shows]>,
    /// Whether rows of it can be kept across an edit (`simple`).
    pub(crate) simple: bool,
    /// Whether a face of it has a box.
    pub(crate) boxed: bool,
    /// Whether a slot of it but the default shows no character: its face
    /// may show none in a window either, and then answers no metrics.
    pub(crate) ghostly: bool,
}

/// A character no simple line holds: one that reorders (right to left
/// scripts and the bidi controls) or one that can bind a composed sequence
/// (a joiner, a variation selector, a tag, a skin tone, a regional
/// indicator). With none of the second kind no character of a line
/// composes with another (`cluster::joins`), so each character's glyph is
/// its own and a row reads nothing behind its first byte.
fn complex(ch: char) -> bool {
    is_composition_joiner(ch) || matches!(ch as u32,
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF
        | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069
        | 0x1F3FB..=0x1F3FF | 0x1F1E6..=0x1F1FF)
}

impl LineText {
    /// The slot of the character at BYTE.
    pub(crate) fn slot_at(&self, byte: usize) -> u16 {
        let at = self.runs.partition_point(|run| run.end <= byte);
        self.runs.get(at).filter(|run| run.start <= byte).map_or(0, |run| run.slot)
    }

    pub(crate) fn is_raw(&self, byte: usize) -> bool {
        !self.raw.is_empty() && self.raw.binary_search(&byte).is_ok()
    }

    pub(crate) fn stretch_at(&self, byte: usize) -> Option<StretchWidth> {
        if self.stretches.is_empty() { return None; }
        self.stretches.binary_search_by_key(&byte, |s| s.0).ok().map(|i| self.stretches[i].1)
    }

    pub(crate) fn image_at(&self, byte: usize) -> Option<InlineImage> {
        if self.images.is_empty() { return None; }
        self.images.binary_search_by_key(&byte, |i| i.byte).ok().map(|i| self.images[i])
    }

    pub(crate) fn flipped(&self, byte: usize) -> bool {
        !self.flips.is_empty() && self.flips.binary_search(&byte).is_ok()
    }

    pub(crate) fn len(&self) -> usize { self.text.len() }

    /// The bytes this line holds in the cache, about.
    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.text.len() + self.runs.len() * std::mem::size_of::<Run>()
            + self.faces.len() * (std::mem::size_of::<usize>() + std::mem::size_of::<Shows>())
            + self.shows.iter().map(|shows| shows.wide.len() * 4).sum::<usize>()
            + (self.raw.len() + self.flips.len()) * std::mem::size_of::<usize>()
            + self.stretches.len() * std::mem::size_of::<(usize, StretchWidth)>() + self.images.len() * std::mem::size_of::<InlineImage>()
    }
}

/// A line read off a snapshot, not yet owned: what `LineText` holds, in
/// buffers a pass reuses, and INDEXES, the table index each slot has in
/// the snapshot it was read from.
#[derive(Default)]
pub(crate) struct LineView {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) runs: Vec<Run>,
    pub(crate) faces: Vec<Rc<Realized>>,
    pub(crate) indexes: Vec<u32>,
    pub(crate) raw: Vec<usize>,
    pub(crate) stretches: Vec<(usize, StretchWidth)>,
    pub(crate) images: Vec<InlineImage>,
    pub(crate) flips: Vec<usize>,
    pub(crate) labels: Vec<(usize, u16)>,
    pub(crate) extend: Option<u16>,
    pub(crate) newline: Option<u16>,
}

impl LineView {
    /// The slot table index INDEX has in this line, given one if new.
    fn slot(&mut self, index: u32, table: &mut Table, faces: &mut Faces, resolver: &mut FontResolver) -> Result<u16, String> {
        if let Some(slot) = self.indexes.iter().position(|&known| known == index) { return u16::try_from(slot).map_err(|_| "too many faces in one line".into()); }
        let slot = u16::try_from(self.indexes.len()).map_err(|_| "too many faces in one line")?;
        self.faces.push(table.get(index as usize, faces, resolver)?);
        self.indexes.push(index);
        Ok(slot)
    }

    /// What tells this line from every line that lays out otherwise,
    /// hashed with TEXT, its characters.
    pub(crate) fn hash(&self, text: &str) -> u64 {
        let mut hasher = rustc_hash::FxHasher::default();
        hasher.write(text.as_bytes());
        self.runs.hash(&mut hasher);
        for face in &self.faces { hasher.write_usize(Rc::as_ptr(face) as usize); }
        (self.extend, self.newline, self.raw.len(), self.stretches.len(), self.images.len(), self.flips.len()).hash(&mut hasher);
        for byte in self.raw.iter().chain(&self.flips) { hasher.write_usize(*byte); }
        for (byte, _) in &self.stretches { hasher.write_usize(*byte); }
        self.labels.hash(&mut hasher);
        for image in &self.images { hasher.write_usize(image.byte); }
        hasher.finish()
    }

    /// Whether LINE is this line, its characters TEXT.
    pub(crate) fn is(&self, text: &str, line: &LineText) -> bool {
        *line.text == *text && *line.runs == *self.runs && (line.extend, line.newline) == (self.extend, self.newline)
            && line.faces.len() == self.faces.len() && line.faces.iter().zip(&self.faces).all(|(a, b)| Rc::ptr_eq(a, b))
            && *line.raw == *self.raw && *line.stretches == *self.stretches && *line.images == *self.images && *line.flips == *self.flips && *line.labels == *self.labels
    }

    /// This line, owned, its characters TEXT.
    pub(crate) fn own(&self, text: &str) -> LineText {
        let mut line = LineText {
            text: text.into(), runs: self.runs.as_slice().into(), faces: self.faces.as_slice().into(), raw: self.raw.as_slice().into(),
            stretches: self.stretches.as_slice().into(), images: self.images.as_slice().into(), flips: self.flips.as_slice().into(), labels: self.labels.as_slice().into(),
            extend: self.extend, newline: self.newline, shows: Box::default(), simple: true, boxed: self.faces.iter().any(|face| face.boxed().is_some()),
            ghostly: false,
        };
        (line.shows, line.simple) = shows(&line);
        line.ghostly = line.shows.iter().skip(1).any(|shows| !shows.used);
        line
    }
}

/// What each slot of LINE draws, as `face_text` collected it for a whole
/// window: each character as it shows (a TAB, a stretch or an image as a
/// space, a control character as its escape's glyphs, a composed
/// sequence's characters all in its first one's face), and the newline
/// that ends the line as the escape it would show as; and whether the
/// line is simple.
fn shows(line: &LineText) -> (Box<[Shows]>, bool) {
    let mut shows = vec![Shows::default(); line.faces.len()];
    let mut simple = true;
    let text = &*line.text;
    let escape = |shows: &mut Shows, ch: char, raw: bool| {
        match ch {
            '\t' => shows.add_char(' '),
            '\u{0}'..='\u{1f}' | '\u{7f}' if !raw => { shows.add_char('^'); shows.add_char(char::from((ch as u8) ^ 0x40)); }
            _ => {
                shows.add_char('\\');
                for shift in [6, 3, 0] { shows.add_char(char::from_digit((ch as u32 >> shift) & 7, 8).unwrap_or('0')); }
            }
        }
    };
    // Runs in order, with the default face between them.
    let mut at = 0;
    let mut inside = 0usize;
    let mut pieces = line.runs.iter().copied().chain(std::iter::once(Run { start: line.len(), end: line.len(), slot: 0 }));
    let mut piece = pieces.next();
    while at < line.len() {
        let (end, slot) = match piece {
            Some(run) if run.start <= at => { piece = pieces.next(); (run.end, run.slot) }
            Some(run) => (run.start, 0),
            None => (line.len(), 0),
        };
        if end <= at { continue; }
        let (from, to) = (at, end);
        at = end;
        let slice = &text.as_bytes()[from..to];
        let target = &mut shows[slot as usize];
        // Plain ASCII of a face whose font has one advance draws nothing
        // that is bound: it only uses the face.
        let plain = slice.is_ascii() && line.raw.is_empty();
        if plain && !line.faces[slot as usize].binds_ascii() {
            target.used = true;
            continue;
        }
        for (offset, ch) in text[from..to].char_indices() {
            let byte = from + offset;
            if byte < inside { continue; }
            let raw = line.is_raw(byte);
            if ch.is_ascii() {
                if raw || ch.is_ascii_control() { escape(target, ch, raw); } else { target.add_char(ch); }
                continue;
            }
            simple &= !complex(ch);
            if !raw && let Some(cluster) = cluster_end(text, byte) {
                target.add_text(&text[byte..cluster]);
                inside = cluster;
                simple = false;
            } else if raw || ch.is_control() {
                escape(target, ch, true);
            } else {
                target.add_char(ch);
            }
        }
    }
    if let Some(slot) = line.newline { escape(&mut shows[slot as usize], '\n', false); }
    for shows in &mut shows {
        shows.wide.sort_unstable();
        shows.wide.dedup();
    }
    (shows.into(), simple)
}

/// A window's published text cut into lines, each read with what the
/// snapshot's sorted lists say inside it: the lists are walked once, in
/// step with the text.
pub(crate) struct Lines<'a> {
    snapshot: &'a WindowSnapshot,
    at: usize,
    run: usize,
    raw: usize,
    stretch: usize,
    image: usize,
    flip: usize,
    label: usize,
    extend: usize,
    done: bool,
}

impl<'a> Lines<'a> {
    /// The first byte of the line `next` reads.
    pub(crate) fn at(&self) -> usize { self.at }

    /// The lines of SNAPSHOT's text from the one starting at FROM, a line start.
    pub(crate) fn from(snapshot: &'a WindowSnapshot, from: usize) -> Self {
        let text = &snapshot.text;
        Lines { snapshot, at: from, done: false,
            run: text.face_runs.partition_point(|run| run.bytes.end <= from),
            raw: text.raw.partition_point(|&byte| (byte as usize) < from),
            stretch: snapshot.stretches.partition_point(|s| s.byte < from),
            image: snapshot.images.partition_point(|i| i.byte < from),
            flip: snapshot.wrap_flips.partition_point(|&byte| (byte as usize) < from),
            label: snapshot.line_numbers.as_ref().map_or(0, |numbers| numbers.labels.partition_point(|label| label.0 <= from)),
            extend: text.extends.partition_point(|e| (e.byte as usize) < from) }
    }

    /// Read the next line into VIEW, its faces realised through TABLE;
    /// false after the line the text ends in.
    pub(crate) fn next(&mut self, view: &mut LineView, table: &mut Table, faces: &mut Faces, resolver: &mut FontResolver) -> Result<bool, String> {
        if self.done { return Ok(false); }
        let snapshot = self.snapshot;
        let text = &snapshot.text;
        let (start, bytes) = (self.at, text.utf8.as_bytes());
        let newline = text.utf8[start..].find('\n').map(|at| start + at);
        let end = newline.unwrap_or(bytes.len());
        // The newline is the line's: its face draws its escape.
        let reach = newline.map_or(end, |at| at + 1);
        view.start = start;
        view.end = end;
        for list in [&mut view.raw, &mut view.flips] { list.clear(); }
        view.labels.clear();
        if let Some(numbers) = &snapshot.line_numbers {
            while let Some(label) = numbers.labels.get(self.label).filter(|label| label.0 <= end) {
                if label.0 > start { view.labels.push((label.0 - start, u16::try_from(label.1.len()).unwrap_or(u16::MAX))); }
                self.label += 1;
            }
        }
        view.runs.clear();
        view.faces.clear();
        view.indexes.clear();
        view.stretches.clear();
        view.images.clear();
        view.slot(0, table, faces, resolver)?;
        view.newline = newline.map(|_| 0);
        while let Some(run) = text.face_runs.get(self.run).filter(|run| run.bytes.start < reach) {
            let (from, to) = (run.bytes.start.max(start), run.bytes.end.min(reach));
            if from < to {
                let slot = view.slot(run.face.0, table, faces, resolver)?;
                if newline.is_some_and(|at| to > at) { view.newline = Some(slot); }
                let (from, to) = (from - start, to.min(end) - start);
                match view.runs.last_mut() {
                    _ if from == to => {}
                    Some(last) if last.slot == slot && last.end == from => last.end = to,
                    _ if slot == 0 => {}
                    _ => view.runs.push(Run { start: from, end: to, slot }),
                }
            }
            if run.bytes.end > reach { break; }
            self.run += 1;
        }
        let relative = |byte: usize| byte - start;
        while let Some(&byte) = text.raw.get(self.raw).filter(|&&byte| (byte as usize) < end) { view.raw.push(relative(byte as usize)); self.raw += 1; }
        while let Some(stretch) = snapshot.stretches.get(self.stretch).filter(|s| s.byte < end) { view.stretches.push((relative(stretch.byte), stretch.width)); self.stretch += 1; }
        while let Some(image) = snapshot.images.get(self.image).filter(|i| i.byte < end) { view.images.push(InlineImage { byte: image.byte - start, ..*image }); self.image += 1; }
        while let Some(&byte) = snapshot.wrap_flips.get(self.flip).filter(|&&byte| (byte as usize) < end) { view.flips.push(relative(byte as usize)); self.flip += 1; }
        // xdisp.c extend_face_to_end_of_line: the newline's entry; the row
        // ending at point-max takes the last one, when it is there.
        while text.extends.get(self.extend).is_some_and(|e| (e.byte as usize) < end) { self.extend += 1; }
        let extend = match newline {
            Some(at) => text.extends.get(self.extend).filter(|e| e.byte as usize == at),
            None => text.extends.last().filter(|e| e.byte as usize == bytes.len()),
        };
        view.extend = match extend {
            Some(extend) => Some(view.slot(extend.face.0, table, faces, resolver)?),
            None => None,
        };
        // Past the newline, the lists' entries on it belong to no line.
        if let Some(at) = newline {
            while text.raw.get(self.raw).is_some_and(|&byte| byte as usize <= at) { self.raw += 1; }
            while snapshot.stretches.get(self.stretch).is_some_and(|s| s.byte <= at) { self.stretch += 1; }
            while snapshot.images.get(self.image).is_some_and(|i| i.byte <= at) { self.image += 1; }
            while snapshot.wrap_flips.get(self.flip).is_some_and(|&byte| byte as usize <= at) { self.flip += 1; }
            self.at = at + 1;
        } else {
            self.done = true;
        }
        Ok(true)
    }
}

/// Add what SLICE, a band's text in DEFAULT where no run names a face,
/// shows in each face to SHOWN: each character, a composed sequence's
/// all in its first one's face.
pub(crate) fn slice_shows(slice: &romacs_layout_input::TextSlice, default: usize, shown: &mut [Shows]) {
    let mut inside = 0;
    for (byte, ch) in slice.utf8.char_indices() {
        if byte < inside { continue; }
        let at = slice.face_runs.partition_point(|run| run.bytes.end <= byte);
        let face = slice.face_runs.get(at).filter(|run| run.bytes.contains(&byte)).map_or(default, |run| run.face.0 as usize);
        let raw = slice.is_raw(byte);
        if !ch.is_ascii() && !raw && let Some(end) = cluster_end(&slice.utf8, byte) {
            shown[face].add_text(&slice.utf8[byte..end]);
            inside = end;
        } else if raw || ch.is_control() {
            shown[face].add_char('\\');
            for shift in [6, 3, 0] { shown[face].add_char(char::from_digit((ch as u32 >> shift) & 7, 8).unwrap_or('0')); }
        } else {
            shown[face].add_char(ch);
        }
    }
}
