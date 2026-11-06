//! Publishing resolved fonts into a frame's paint list (Seam P).

use crate::cluster::{is_composition_joiner, representative_char_for_cluster};
use crate::resolver::{FontResolveError, FontResolver};
use crate::style::FontRequest;
use romacs_display_protocol::font::ResolvedFontId;
use romacs_display_protocol::frame_glyphs::FrameGlyphBuffer;
use romacs_display_protocol::types::FaceId;

/// What [`publish_face_text`] bound.
#[derive(Clone, Debug, PartialEq)]
pub struct PublishedFace {
    pub primary: ResolvedFontId,
    /// Distinct non-ASCII characters given an exact font and glyph.
    pub bound_chars: usize,
    /// Characters no font could be found for, with the reason. The renderer
    /// draws these through its emergency path, so a caller that needs every
    /// glyph exact must treat a non-empty list as failure.
    pub unresolved: Vec<(char, FontResolveError)>,
}

/// Resolve FACE_ID's font from REQUEST and bind every character of TEXT that
/// needs one, so the renderer makes no font decision for this face.
///
/// The face must already be in `frame.faces` (for example through
/// `FrameGlyphBuffer::set_face_with_font`). Its `default_resolved_font_id`,
/// ascent and descent are set from the resolved primary font; non-ASCII
/// characters and each cluster's representative character are bound in
/// `frame.char_fonts`. ASCII stays on the primary font, as GNU's
/// `face_for_char` keeps it.
pub fn publish_face_text(
    resolver: &mut FontResolver,
    frame: &mut FrameGlyphBuffer,
    face_id: FaceId,
    request: &FontRequest,
    text: &str,
) -> Result<PublishedFace, FontResolveError> {
    let primary = resolver.resolve_primary(request)?;
    let primary_id = primary.id;
    if let Some(face) = frame.faces.get_mut(&face_id) {
        face.default_resolved_font_id = Some(primary_id);
        face.font_ascent = primary.ascent_px.round() as i32;
        face.font_descent = primary.descent_px.round() as i32;
    }
    frame.fonts.insert(primary_id, primary);

    let mut wanted: Vec<char> = text
        .chars()
        .filter(|&ch| !ch.is_ascii() && !is_composition_joiner(ch))
        .collect();
    // The renderer chooses a cluster's font through its representative
    // character (U+1F600 for any VS16 cluster), so that one must be bound
    // even when it never appears in the text.
    wanted.extend(representative_char_for_cluster(text));
    wanted.sort_unstable();
    wanted.dedup();

    let mut bound_chars = 0;
    let mut unresolved = Vec::new();
    for ch in wanted {
        match resolver.resolve_char(request, ch) {
            Ok(resolution) => {
                frame.fonts.insert(resolution.font.id, resolution.font);
                frame
                    .char_fonts
                    .entry(face_id)
                    .or_default()
                    .insert(ch, resolution.glyph);
                bound_chars += 1;
            }
            Err(error) => unresolved.push((ch, error)),
        }
    }
    Ok(PublishedFace {
        primary: primary_id,
        bound_chars,
        unresolved,
    })
}
