//! Text animations: a block of a window's character cells that the render
//! thread redraws frame after frame, over the glyphs the evaluator laid
//! out, with no evaluator work per frame.  Lisp registers one with its
//! parameters (rlisp/romacs-animate.el) and changes it only when they
//! change; the render thread keeps the clock, the motion time and the
//! frames.
use crate::face::Face;
use crate::frame_glyphs::{DisplaySlotId, FrameGlyph, FrameGlyphBuffer, GlyphRowRole};
use crate::types::{Color, FaceId};

/// One animation over window WINDOW's text: its ROWS by COLS cells,
/// whose first row is the window's visual row TOP (negative when scrolled
/// above it), starting at column 0.
#[derive(Clone, Debug, PartialEq)]
pub struct TextAnimation {
    /// The window, as its glyphs name it.
    pub window: u64,
    pub top: i32,
    pub cols: u32,
    pub rows: u32,
    pub fps: u32,
    /// Whether it moves; else it shows the frame it reached.
    pub moving: bool,
    /// Motion is off (`romacs-animation' reduced or nil): it shows its
    /// still frame, not the frame it reached.
    pub still: bool,
    pub kind: TextAnimationKind,
}

/// What draws the cells.
#[derive(Clone, Debug, PartialEq)]
pub enum TextAnimationKind {
    /// The Romacs mark and wordmark (romacs-logo) in cells CELL_ASPECT as
    /// wide as high, a sweep of light crossing them.  A new EPOCH starts
    /// the motion over.
    Logo { cell_aspect: f64, epoch: u64 },
}

impl TextAnimation {
    /// Whether NEXT continues this one's motion rather than restarting it.
    pub fn continues(&self, next: &Self) -> bool {
        match (&self.kind, &next.kind) {
            (TextAnimationKind::Logo { epoch: a, .. }, TextAnimationKind::Logo { epoch: b, .. }) => a == b,
        }
    }

    /// The cells after MOVED seconds of motion, each a character and its
    /// colour (none for an empty cell).
    pub fn cells(&self, moved: f64) -> Vec<romacs_logo::Cell> {
        match self.kind {
            TextAnimationKind::Logo { cell_aspect, .. } => {
                let view = romacs_logo::View { cols: self.cols as usize, rows: self.rows as usize, cell_aspect };
                if view.valid() { romacs_logo::frame(view, moved) } else { vec![] }
            }
        }
    }
}

/// Put CELLS (rows of ANIMATION's columns) over FRAME's glyphs for
/// ANIMATION's block: each cell's character in its colour, a new face like
/// the one the evaluator laid the block out in with that foreground; an
/// empty cell in that face.  A character takes, in its new face, the
/// font and glyph layout resolved for it in the evaluator's frame (a
/// fallback font where the face's own lacks it).  A cell the evaluator
/// laid out no glyph for gets one like its row's others.
///
/// Slots count rows and columns from the frame's corner, so the block is
/// placed from the window's first text row and column: those of its text
/// glyphs, which the evaluator lays out for every cell of the block.
pub fn paint(frame: &mut FrameGlyphBuffer, animation: &TextAnimation, cells: &[romacs_logo::Cell]) {
    let (cols, rows) = (animation.cols as usize, animation.rows as usize);
    if cols == 0 || cells.len() != cols * rows { return; }
    let in_window = |glyph: &FrameGlyph| match glyph {
        FrameGlyph::Char { slot_id, row_role: GlyphRowRole::Text, composed: None, .. }
            if slot_id.window_id.get() as u64 == animation.window => Some(*slot_id),
        _ => None,
    };
    let Some((first_row, first_col)) = frame.glyphs.iter().filter_map(in_window)
        .fold(None, |origin: Option<(u32, u16)>, slot| Some(origin.map_or((slot.row, slot.col), |(r, c)| (r.min(slot.row), c.min(slot.col)))))
    else { return };
    let block = |slot: &DisplaySlotId| -> Option<usize> {
        let row = i64::from(slot.row) - i64::from(first_row) - i64::from(animation.top);
        let col = usize::from(slot.col).checked_sub(usize::from(first_col))?;
        ((0..rows as i64).contains(&row) && col < cols).then(|| row as usize * cols + col)
    };
    // Each cell's glyph, and one glyph per row to model missing cells on.
    let mut at: Vec<Option<usize>> = vec![None; cells.len()];
    let mut model: Vec<Option<usize>> = vec![None; rows];
    for (i, glyph) in frame.glyphs.iter().enumerate() {
        if let Some(slot_id) = in_window(glyph)
            && let Some(cell) = block(&slot_id) {
            at[cell] = Some(i);
            model[cell / cols].get_or_insert(i);
        }
    }
    let Some(base) = model.iter().flatten().next().and_then(|&i| face_of(&frame.glyphs[i])) else { return };
    let Some(face) = frame.faces.get(&base).cloned() else { return };
    let mut next = frame.faces.keys().map(|id| id.get()).max().unwrap_or(0).max(crate::face::BasicFaceId::SENTINEL) + 1;
    let mut coloured: Vec<(u32, FaceId)> = vec![];
    // Every character's font and glyph as the evaluator's frame resolved it.
    let mut resolved: std::collections::HashMap<char, crate::font::ResolvedCharGlyph> = std::collections::HashMap::new();
    for glyphs in frame.char_fonts.values() {
        for (&c, &glyph) in glyphs { resolved.entry(c).or_insert(glyph); }
    }
    let char_width = frame.char_width;
    for (cell, &(ch, colour)) in cells.iter().enumerate() {
        let index = match at[cell] {
            Some(index) => index,
            None => {
                let Some(model) = model[cell / cols] else { continue };
                let mut glyph = frame.glyphs[model].clone();
                if let FrameGlyph::Char { x, slot_id, .. } = &mut glyph {
                    let col = first_col + (cell % cols) as u16;
                    *x += (f32::from(col) - f32::from(slot_id.col)) * char_width;
                    slot_id.col = col;
                }
                frame.glyphs.push(glyph);
                frame.glyphs.len() - 1
            }
        };
        let face_id = match colour {
            None => base,
            Some(rgb) => match coloured.iter().find(|(c, _)| *c == rgb) {
                Some(&(_, id)) => id,
                None => {
                    let id = FaceId::new(next);
                    next += 1;
                    frame.faces.insert(id, Face { id, foreground: Color::from_pixel(rgb), ..face.clone() });
                    coloured.push((rgb, id));
                    id
                }
            },
        };
        if let FrameGlyph::Char { char, face_id: glyph_face, .. } = &mut frame.glyphs[index] {
            *char = ch;
            *glyph_face = face_id;
        }
        if face_id != base && let Some(&glyph) = resolved.get(&ch) {
            frame.char_fonts.entry(face_id).or_default().insert(ch, glyph);
        }
    }
}

fn face_of(glyph: &FrameGlyph) -> Option<FaceId> {
    match glyph { FrameGlyph::Char { face_id, .. } => Some(*face_id), _ => None }
}
