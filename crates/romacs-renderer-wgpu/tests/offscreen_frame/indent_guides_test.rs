//! A window's indent guides are its own: two windows' rows at one y are two
//! rows. A row of spaces draws no guides, and a row however far off the
//! frame draws only those on it, and ends.
use super::*;

const ROW: f32 = 12.0;
const NORMAL: u32 = 0;
const BG: Color = Color::rgb(0.1, 0.12, 0.16);

/// A frame of ROWS: each a window, an x, a y and its text, in cells WIDE.
#[expect(clippy::cast_precision_loss, reason = "the test surface's few pixels")]
fn guided(wide: f32, rows: &[(i64, f32, f32, &str)]) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(W as f32, H as f32);
    (frame.char_width, frame.char_height) = (wide, ROW);
    frame.background = BG;
    frame.set_face(FaceId::new(NORMAL), Color::WHITE, Some(BG), 400, false, 0, None, 0, None, 0, None);
    for &(window, x0, y, text) in rows {
        frame.set_draw_context(DisplayWindowId::new(window), GlyphRowRole::Text, None);
        for (col, ch) in text.chars().enumerate() {
            frame.add_char(ch, x0 + wide * f32::from(u8::try_from(col).unwrap()), y, wide, ROW, 8.0, false);
        }
    }
    frame
}

/// FRAME drawn whole with red indent guides.
fn drawn(h: &mut Harness, frame: &FrameGlyphBuffer) -> Vec<u8> {
    h.renderer.effects.indent_guides.enabled = true;
    h.renderer.effects.indent_guides.color = (1.0, 0.0, 0.0, 1.0);
    h.atlas.set_current_frame_fonts(frame.font_bindings());
    let (texture, view) = make_tex(&h.renderer, "guides");
    h.renderer.render_frame_glyphs(&view, frame, &mut h.atlas, mapping_for(frame, W, H), false, None, None, None, None, None);
    read_tex(&h.renderer, &texture)
}

fn red(pixels: &[u8], x: u32, y: u32) -> bool {
    let p = pxb(pixels, x, y);
    p[0] > 200 && p[1] < 60
}

#[test]
fn a_window_s_indent_guides_are_its_own_beside_another_window() {
    let Some(mut h) = try_harness() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    // The left row unindented, the right one indented past a tab stop
    // (four cells, at 64): the right one draws its guide.
    let pixels = drawn(&mut h, &guided(4.0, &[(1, 0.0, 0.0, "abc"), (2, 48.0, 0.0, "      def")]));
    assert!(red(&pixels, 64, 6), "the right window's guide at x 64: {:?}", pxb(&pixels, 64, 6));
}

#[test]
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a whole row height")]
fn a_row_of_spaces_draws_no_guides() {
    let Some(mut h) = try_harness() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    let before_another = guided(4.0, &[(1, 0.0, 0.0, "      "), (2, 48.0, 0.0, "def")]);
    assert!(!red(&drawn(&mut h, &before_another), 16, 6));
    let last = guided(4.0, &[(1, 0.0, 0.0, "abc"), (1, 0.0, ROW, "      ")]);
    assert!(!red(&drawn(&mut h, &last), 16, ROW as u32 + 6));
}

#[test]
fn a_row_far_off_the_frame_draws_its_guides_on_it_and_ends() {
    let Some(mut h) = try_harness() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    // A tab stop added to these starts rounds back to them in f32.
    let far = guided(4.0, &[(1, -1_073_741_824.0, 0.0, " "), (1, 80.0, 0.0, "x")]);
    assert!(red(&drawn(&mut h, &far), 64, 6), "a visible guide of the far row");
    let fractional = guided(7.8, &[(1, -18_014_398_509_481_984.0, 0.0, " "), (1, 80.0, 0.0, "x")]);
    drawn(&mut h, &fractional);
}
