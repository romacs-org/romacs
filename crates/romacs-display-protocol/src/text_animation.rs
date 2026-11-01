//! Text animations: a block of a window's character cells that the render
//! thread redraws frame after frame, over the glyphs the evaluator laid
//! out, with no evaluator work per frame.  Lisp registers one with its
//! parameters (rlisp/romacs-animate.el) and changes it only when they
//! change; the render thread keeps the clock, the motion time and the
//! frames.
use crate::frame_glyphs::{DisplaySlotId, FrameGlyph, FrameGlyphBuffer, GlyphRowRole};
use crate::types::FaceId;

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
    /// How many shades its cells take, dark to light
    /// ([`romacs_armillary::shade_of`]): the evaluator's frame of the block
    /// shows each in its face.
    pub shades: u32,
    pub kind: TextAnimationKind,
}

/// What draws the cells.
#[derive(Clone, Debug, PartialEq)]
pub enum TextAnimationKind {
    /// The armillary sphere (romacs-armillary), turned to PITCH, YAW and
    /// ZOOM, then by the site's motion: yaw 0.16 and phase 0.2 a second of
    /// motion.  A new EPOCH starts the motion over.
    Armillary { pitch: f64, yaw: f64, zoom: f64, cell_aspect: f64, epoch: u64 },
}

impl TextAnimation {
    /// Whether NEXT continues this one's motion rather than restarting it.
    pub fn continues(&self, next: &Self) -> bool {
        match (&self.kind, &next.kind) {
            (TextAnimationKind::Armillary { epoch: a, .. }, TextAnimationKind::Armillary { epoch: b, .. }) => a == b,
        }
    }

    /// The cells after MOVED seconds of motion: bytes, each a character
    /// and through [`romacs_armillary::shade_runs`] a shade.
    pub fn cells(&self, moved: f64) -> Vec<u8> {
        match self.kind {
            TextAnimationKind::Armillary { pitch, yaw, zoom, cell_aspect, .. } => {
                let phase = moved * 0.2;
                let view = romacs_armillary::View {
                    cols: self.cols as usize, rows: self.rows as usize,
                    pitch: pitch + (phase * 0.47).sin() * 0.12, yaw: yaw + moved * 0.16,
                    roll: -0.15 + (phase * 0.3).sin() * 0.1, zoom, cell_aspect, stars: true,
                };
                if view.valid() { romacs_armillary::cells(&view) } else { vec![] }
            }
        }
    }
}

/// Put CELLS (rows of ANIMATION's columns) over FRAME's glyphs for
/// ANIMATION's block: each cell's character in its shade's face.  The
/// faces are the evaluator's: its frame of the block draws each character
/// in its shade's face, so the block's glyphs name them, as the theme has
/// them now; a shade it showed none of takes the nearest it did.  A cell
/// the evaluator laid out no glyph for gets one like its row's others.
pub fn paint(frame: &mut FrameGlyphBuffer, animation: &TextAnimation, cells: &[u8]) {
    let (cols, rows) = (animation.cols as usize, animation.rows as usize);
    if cols == 0 || cells.len() != cols * rows { return; }
    let block = |slot: &DisplaySlotId| -> Option<usize> {
        let row = i64::from(slot.row) - i64::from(animation.top);
        (slot.window_id.get() as u64 == animation.window && (0..rows as i64).contains(&row) && usize::from(slot.col) < cols)
            .then(|| row as usize * cols + usize::from(slot.col))
    };
    // Each cell's glyph, and one glyph per row to model missing cells on.
    let mut at: Vec<Option<usize>> = vec![None; cells.len()];
    let mut model: Vec<Option<usize>> = vec![None; rows];
    for (i, glyph) in frame.glyphs.iter().enumerate() {
        if let FrameGlyph::Char { slot_id, row_role: GlyphRowRole::Text, composed: None, .. } = glyph
            && let Some(cell) = block(slot_id) {
            at[cell] = Some(i);
            model[cell / cols].get_or_insert(i);
        }
    }
    let Some(base) = model.iter().flatten().next().and_then(|&i| face_of(&frame.glyphs[i])) else { return };
    let shades = shade_faces(frame, &at, animation.shades as usize);
    let char_width = frame.char_width;
    for (cell, &byte) in cells.iter().enumerate() {
        let index = match at[cell] {
            Some(index) => index,
            None => {
                let Some(model) = model[cell / cols] else { continue };
                let mut glyph = frame.glyphs[model].clone();
                if let FrameGlyph::Char { x, slot_id, .. } = &mut glyph {
                    *x += (cell % cols) as f32 * char_width - f32::from(slot_id.col) * char_width;
                    slot_id.col = (cell % cols) as u16;
                }
                frame.glyphs.push(glyph);
                frame.glyphs.len() - 1
            }
        };
        if let FrameGlyph::Char { char, face_id, .. } = &mut frame.glyphs[index] {
            *char = char::from(byte);
            *face_id = romacs_armillary::shade_of(byte, shades.len().max(1))
                .and_then(|shade| shades.get(shade).copied().flatten()).unwrap_or(base);
        }
    }
}

/// The face the evaluator drew each of SHADES shades in, from the glyphs AT
/// the block's cells; one it drew none of takes the nearest it did.
fn shade_faces(frame: &FrameGlyphBuffer, at: &[Option<usize>], shades: usize) -> Vec<Option<FaceId>> {
    let mut faces = vec![None; shades];
    if shades == 0 { return faces; }
    for &index in at.iter().flatten() {
        if let FrameGlyph::Char { char, face_id, .. } = &frame.glyphs[index]
            && let Ok(byte) = u8::try_from(*char)
            && let Some(shade) = romacs_armillary::shade_of(byte, shades) {
            faces[shade].get_or_insert(*face_id);
        }
    }
    (0..shades).map(|shade| (0..shades).flat_map(|d| [shade.checked_sub(d), Some(shade + d)])
        .flatten().find_map(|near| faces.get(near).copied().flatten())).collect()
}

fn face_of(glyph: &FrameGlyph) -> Option<FaceId> {
    match glyph { FrameGlyph::Char { face_id, .. } => Some(*face_id), _ => None }
}
