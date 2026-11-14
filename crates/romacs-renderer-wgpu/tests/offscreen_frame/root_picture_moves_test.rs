//! A scrolled frame drawn through the root picture equals it drawn whole,
//! byte for byte: scrolls up and down by a line, by several and by more
//! than a window, over rows of mixed heights, past an image whose box is
//! larger than it and a fringe bitmap wider than its column, with the
//! cursor moving as it scrolls; at a whole and at a fractional device
//! scale, and with ink that leaves its rows. A scroll by a line repaints
//! the row that entered and no other.
use super::*;
use romacs_display_protocol::call_count;
use romacs_display_protocol::frame_glyphs::{FringeBitmapData, FringeSide};
use romacs_display_protocol::types::Rect;
use romacs_renderer_wgpu::{PaintTarget, Painted, RootPicture, SnapshotSize};

const ROW: f32 = 8.0;
const CELL: f32 = 5.0;
const NORMAL: u32 = 0;
const MODE: u32 = 1;
const PICTURE: u32 = 2;
const BIG: u32 = 3;
const IMAGE: u32 = 702;
/// The line that holds an image.
const IMAGE_LINE: usize = 4;
/// The line with a fringe bitmap that spills into its text.
const FRINGE_LINE: usize = 5;
/// The line set in a font too large for it, when ink leaves its rows.
const INKY_LINE: usize = 6;
/// The lines set taller than the rest, and how tall: the last a height
/// that is no whole number of device pixels at 1.5.
const TALL: [(usize, f32); 3] = [(9, 16.0), (10, 12.0), (11, 13.0)];
const BG: Color = Color::rgb(0.1, 0.12, 0.16);

/// Where the window shows its text from, and where its cursor is.
#[derive(Clone, Copy)]
struct View {
    first: usize,
    cursor: (usize, usize),
}

fn line(at: usize) -> String {
    match at {
        3 => String::new(),
        _ => format!("{at}: {}", "abcdefghij".chars().cycle().skip(at).take(4 + at % 7).collect::<String>()),
    }
}

fn height(at: usize) -> f32 {
    TALL.iter().find(|(tall, _)| *tall == at).map_or(ROW, |(_, h)| *h)
}

fn text_face(frame: &mut FrameGlyphBuffer, id: u32, bg: Color) {
    frame.set_face(FaceId::new(id), Color::rgb(0.9, 0.9, 0.85), Some(bg), 400, false, 0, None, 0, None, 0, None);
    frame.faces.get_mut(&FaceId::new(id)).unwrap().font_size = 6.0;
}

/// VIEW's frame at SCALE: one window of text above a mode line; with
/// INKY, a line whose glyphs leave its row.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    reason = "one view's whole frame, built in one place from a test's few rows and columns"
)]
fn view_frame(view: View, scale: f32, inky: bool) -> FrameGlyphBuffer {
    let (width, height_all) = (W as f32 / scale, H as f32 / scale);
    let body = height_all - ROW;
    let mut frame = FrameGlyphBuffer::with_size(width, height_all);
    (frame.char_width, frame.char_height) = (CELL, ROW);
    frame.background = BG;
    text_face(&mut frame, MODE, Color::rgb(0.35, 0.35, 0.4));
    let mut picture = Face::new(FaceId::new(PICTURE));
    picture.background = Color::GREEN;
    frame.faces.insert(FaceId::new(PICTURE), picture);
    text_face(&mut frame, BIG, BG);
    frame.faces.get_mut(&FaceId::new(BIG)).unwrap().font_size = 15.0;
    text_face(&mut frame, NORMAL, BG);
    frame.register_fringe_bitmap(
        1,
        FringeBitmapData { bits: vec![0xFFFF; 8], width: 16, height: 8, period: 0, align: 0 },
    );
    let window = DisplayWindowId::new(1);
    frame.set_draw_context(window, GlyphRowRole::Text, Some(Rect::new(0.0, 0.0, width, body)));
    frame.add_background(0.0, 0.0, width, body, BG);
    let (mut y, mut cursor_y) = (0.0, 0.0);
    for at in view.first.. {
        if y >= body {
            break;
        }
        let h = height(at);
        if at == view.first + view.cursor.0 {
            cursor_y = y;
        }
        let text = line(at);
        for (col, ch) in text.chars().enumerate() {
            frame.add_char(ch, 2.0 + col as f32 * CELL, y, CELL, h, 6.0, false);
            if inky && at == INKY_LINE
                && let Some(FrameGlyph::Char { face_id, .. }) = frame.glyphs.last_mut()
            {
                *face_id = FaceId::new(BIG);
            }
        }
        if at == FRINGE_LINE {
            frame.push_fringe_bitmap(0.0, y, 2.0, h, 1, FaceId::new(NORMAL), FringeSide::Left);
        }
        let end = 2.0 + text.chars().count() as f32 * CELL;
        frame.add_stretch(end, y, width - end, h, BG, FaceId::new(NORMAL), false);
        if at == IMAGE_LINE {
            let x = 2.0 + 14.0 * CELL;
            frame.glyphs.push(FrameGlyph::Image {
                window_id: window,
                row_role: GlyphRowRole::Text,
                clip_rect: Some(Rect::new(0.0, 0.0, width, body)),
                slot_id: None,
                image_id: ImageId::new(IMAGE),
                source_rect: ImageSourceRect::new(0.0, 0.0, 1.0, 1.0).unwrap(),
                slot_rect: Rect::new(x - 1.0, y, 8.0, h),
                box_rect: Rect::new(x - 1.0, y, 8.0, h),
                x,
                y: y + 1.0,
                width: 6.0,
                height: 6.0,
                face_id: FaceId::new(PICTURE),
                box_vertical_edges: BoxVerticalEdges::Both,
            });
        }
        y += h;
    }
    frame.set_draw_context(window, GlyphRowRole::ModeLine, None);
    frame.add_stretch(0.0, body, width, ROW, Color::rgb(0.35, 0.35, 0.4), FaceId::new(MODE), false);
    for (at, ch) in "-:-- scratch".chars().enumerate() {
        frame.add_char(ch, 2.0 + at as f32 * CELL, body, CELL, ROW, 6.0, false);
        if let Some(FrameGlyph::Char { face_id, .. }) = frame.glyphs.last_mut() {
            *face_id = FaceId::new(MODE);
        }
    }
    let (row, col) = view.cursor;
    frame.set_phys_cursor(PhysCursor {
        window_id: window,
        charpos: 0,
        row,
        col: col as u16,
        slot_id: DisplaySlotId { window_id: window, row: (cursor_y / ROW).round() as u32, col: col as u16 },
        x: 2.0 + col as f32 * CELL,
        y: cursor_y,
        width: CELL,
        height: height(view.first + row),
        ascent: 6.0,
        style: CursorStyle::FilledBox,
        color: Color::rgb(1.0, 0.5, 0.0),
        cursor_fg: Color::rgb(0.05, 0.05, 0.05),
    });
    frame
}

/// The views a session scrolls through, and the text rows a scroll by a
/// line repaints at a whole scale, where it moves the rest.
fn scrolls() -> Vec<(&'static str, View, Option<u64>)> {
    let at = |first, cursor| View { first, cursor };
    vec![
        ("start", at(0, (0, 0)), None),
        ("the start again", at(0, (0, 0)), None),
        ("down a line", at(1, (0, 0)), Some(1)),
        ("down a line past the image", at(2, (0, 0)), Some(1)),
        ("up a line", at(1, (0, 0)), Some(1)),
        ("down three lines", at(4, (0, 0)), Some(2)),
        ("down a line onto the taller lines", at(5, (1, 2)), None),
        ("down a line with the cursor moving", at(6, (2, 3)), None),
        ("a taller line leaves", at(9, (0, 0)), None),
        ("another", at(10, (0, 0)), None),
        ("down past a window", at(30, (0, 0)), None),
        ("there again", at(30, (0, 0)), None),
        ("up past a window", at(13, (0, 1)), None),
        ("there again too", at(13, (0, 1)), None),
        ("up a line again", at(12, (0, 1)), Some(1)),
    ]
}

fn load_image(h: &mut Harness) {
    let mut argb = Vec::new();
    for y in 0..6u32 {
        for _ in 0..6 {
            argb.extend_from_slice(if y < 3 { &[255, 255, 0, 0] } else { &[255, 0, 0, 255] });
        }
    }
    h.renderer.load_image_argb32_with_id(test_image_load(IMAGE), &argb, 6, 6, 24);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline && !h.renderer.is_image_ready(ImageId::new(IMAGE)) {
        h.renderer.process_pending_images();
        std::thread::yield_now();
    }
    assert!(h.renderer.is_image_ready(ImageId::new(IMAGE)), "test image must decode");
}

fn scrolled_through_picture_equals_whole(scale: f32, inky: bool) {
    let Some(mut h) = try_harness() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    h.renderer.effects.cursor_color_cycle.enabled = false;
    h.renderer.set_scale_factor(scale);
    load_image(&mut h);
    let mut picture = RootPicture::new(&h.renderer, SnapshotSize::new(W, H).unwrap());
    for (name, view, entered) in scrolls() {
        let frame = view_frame(view, scale, inky);
        let mapping = mapping_for_scale(&frame, W, H, scale);
        h.atlas.set_current_frame_fonts(frame.font_bindings());
        let (whole_texture, whole_view) = make_tex(&h.renderer, "whole");
        h.renderer.render_frame_glyphs(&whole_view, &frame, &mut h.atlas, mapping, true, None, None, None, None);
        let whole = read_tex(&h.renderer, &whole_texture);
        let (texture, view) = make_tex(&h.renderer, "through-picture");
        let target = PaintTarget { view: &view, mapping, cursor_visible: true, animated_cursor: None };
        let rows = call_count::of(romacs_renderer_wgpu::cost::ROWS_TESSELLATED);
        let painted = picture.paint(&mut h.renderer, &mut h.atlas, &frame, target);
        let rows = call_count::of(romacs_renderer_wgpu::cost::ROWS_TESSELLATED) - rows;
        let through = read_tex(&h.renderer, &texture);
        let differing = (0..W * H)
            .filter(|&i| whole[i as usize * 4..i as usize * 4 + 4] != through[i as usize * 4..i as usize * 4 + 4])
            .collect::<Vec<_>>();
        assert!(
            differing.is_empty(),
            "{name} at {scale}, {painted:?}: {} pixels differ, at {:?}, the first: whole {:?}, through the picture {:?}",
            differing.len(),
            differing.iter().map(|i| (i % W, i / W)).collect::<Vec<_>>(),
            differing.first().map(|&i| pxb(&whole, i % W, i / W)),
            differing.first().map(|&i| pxb(&through, i % W, i / W)),
        );
        if let Some(entered) = entered.filter(|_| scale == 1.0 && !inky) {
            assert!(matches!(painted, Painted::Regions(_)), "{name}: {painted:?}");
            assert_eq!(rows, entered, "{name}: the rows that entered, and no other");
        }
    }
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole() {
    scrolled_through_picture_equals_whole(1.0, false);
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_at_a_fractional_scale() {
    scrolled_through_picture_equals_whole(1.5, false);
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_where_ink_leaves_its_rows() {
    scrolled_through_picture_equals_whole(1.0, true);
}
