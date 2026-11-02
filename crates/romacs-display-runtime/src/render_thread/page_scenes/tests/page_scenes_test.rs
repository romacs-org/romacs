use super::*;
use romacs_display_protocol::face::BoxVerticalEdges;
use romacs_display_protocol::frame_glyphs::DisplaySlotId;
use romacs_display_protocol::types::DisplayWindowId;

/// A frame whose window WINDOW shows ROWS rows of COLS cells, 9 by 22
/// pixels, from (5, 20).
pub(super) fn frame(window: u64, rows: u32, cols: u16) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::new();
    frame.char_width = 9.0;
    for row in 0..rows {
        for col in 0..cols {
            let (x, y) = (5.0 + f32::from(col) * 9.0, 20.0 + row as f32 * 22.0);
            frame.glyphs.push(FrameGlyph::Char {
                window_id: DisplayWindowId::new(window as i64),
                row_role: GlyphRowRole::Text,
                clip_rect: None,
                slot_id: DisplaySlotId { window_id: DisplayWindowId::new(window as i64), row, col },
                bidi_level: 0,
                char: ' ',
                composed: None,
                x,
                y,
                baseline: y + 16.0,
                width: 9.0,
                height: 22.0,
                ascent: 16.0,
                face_id: FaceId::new(3),
                box_vertical_edges: BoxVerticalEdges::Both,
            });
        }
    }
    frame
}

fn logo(window: u64, top: i32) -> TextAnimation {
    TextAnimation { window, top, cols: 10, rows: 4, fps: 25, moving: true, still: false,
                    kind: TextAnimationKind::Logo { cell_aspect: 0.4, epoch: 0 } }
}

#[test]
fn a_block_is_placed_from_its_window_s_text_cells() {
    let block = block(&frame(1, 6, 12), &logo(1, 0)).expect("a block");
    assert_eq!(block.origin, kurbo::Point::new(5.0, 20.0));
    assert_eq!(block.cell, Size::new(9.0, 22.0));
    assert_eq!((block.window, block.face), (DisplayWindowId::new(1), FaceId::new(3)));
}

#[test]
fn a_block_scrolled_above_its_window_starts_above_it() {
    let block = block(&frame(1, 6, 12), &logo(1, -2)).expect("its last rows show");
    assert_eq!(block.origin, kurbo::Point::new(5.0, 20.0 - 2.0 * 22.0));
}

#[test]
fn a_block_needs_a_text_cell_of_its_own_window() {
    assert_eq!(block(&frame(2, 6, 12), &logo(1, 0)), None, "another window's text");
    assert_eq!(block(&frame(1, 6, 12), &logo(1, -6)), None, "scrolled out of view");
}
