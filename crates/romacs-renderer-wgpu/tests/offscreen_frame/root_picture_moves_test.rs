//! A scrolled frame drawn through the root picture equals it drawn whole,
//! byte for byte: scrolls up and down by a line, by several and by more
//! than a window, over rows of mixed heights, past an image whose box is
//! larger than it and a fringe bitmap wider than its column, with the
//! cursor moving as it scrolls; at device scales one, two and one and a
//! half (where nothing moves), with ink that leaves its rows, with a fringe bitmap that tiles in
//! a period the rows' heights are no multiple of, with a border its clip
//! cuts, and with an underline whose first glyphs change face as its row
//! moves. A scroll by a line repaints the row that entered and no other.
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
/// Two underlined faces whose lines lie apart.
const UNDER: u32 = 4;
const UNDER_LOW: u32 = 5;
/// The line underlined, its tail set a pixel shorter: one underline run
/// over glyphs of two heights, which takes its line from its first glyph.
const UNDERLINED_LINE: usize = 3;
const IMAGE: u32 = 702;
/// The line that holds an image.
const IMAGE_LINE: usize = 4;
/// The line with a fringe bitmap that spills into its text.
const FRINGE_LINE: usize = 5;
/// The line set in a font too large for it, when ink leaves its rows.
const INKY_LINE: usize = 6;
/// The line with a fringe bitmap that tiles every three pixels, when one
/// does.
const TILED_LINE: usize = 1;
/// The lines set taller than the rest, and how tall: the last a height
/// that is no whole number of device pixels at 1.5.
const TALL: [(usize, f32); 3] = [(9, 16.0), (10, 12.0), (11, 13.0)];
const BG: Color = Color::rgb(0.1, 0.12, 0.16);

/// Where the window shows its text from, where its cursor is, and the face
/// the underlined line starts in.
#[derive(Clone, Copy)]
struct View {
    first: usize,
    cursor: (usize, usize),
    lead: u32,
}

/// What a session's frames hold besides plain text.
#[derive(Clone, Copy, Default)]
#[expect(clippy::struct_excessive_bools, reason = "independent things a test frame may hold")]
struct Extras {
    /// A line whose glyphs leave its row.
    inky: bool,
    /// A fringe bitmap that tiles.
    tiled: bool,
    /// A border its clip cuts to the middle of the window.
    bordered: bool,
    /// An underlined line, its tail shorter.
    underlined: bool,
    /// The window drawn as a pane: rounded corners and an edge.
    paned: bool,
    /// The pane's right corners just beside the rows, which end short of
    /// the frame's edge, and a fringe bitmap whose ink reaches far past
    /// its column on the mode line, the window's rows' fringe bitmap gone.
    cornered: bool,
}

fn line(at: usize) -> String {
    match at {
        3 => "underlined text".to_owned(),
        15 => String::new(),
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

/// VIEW's frame at SCALE: one window of text above a mode line, with
/// EXTRAS.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    reason = "one view's whole frame, built in one place from a test's few rows and columns"
)]
fn view_frame(view: View, scale: f32, extras: Extras) -> FrameGlyphBuffer {
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
    // Both lines inside the cell, a pixel apart: no ink leaves the row.
    for (id, position) in [(UNDER, 0), (UNDER_LOW, 1)] {
        text_face(&mut frame, id, BG);
        let face = frame.faces.get_mut(&FaceId::new(id)).unwrap();
        face.attributes |= FaceAttributes::UNDERLINE;
        face.underline_style = romacs_display_protocol::face::UnderlineStyle::Line;
        face.underline_color = Some(Color::rgb(0.3, 0.9, 0.3));
        face.underline_placement = romacs_display_protocol::face::UnderlinePosition::FontMetric { offset_from_baseline: position };
        face.underline_thickness = 1;
    }
    text_face(&mut frame, NORMAL, BG);
    frame.register_fringe_bitmap(
        1,
        FringeBitmapData { bits: vec![0xFFFF; 8], width: 16, height: 8, period: 0, align: 0 },
    );
    frame.register_fringe_bitmap(
        2,
        FringeBitmapData { bits: vec![0xC000, 0x0000, 0x0000], width: 2, height: 3, period: 3, align: 0 },
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
            let underlined = extras.underlined && at == UNDERLINED_LINE;
            let short = if underlined && col >= 4 { 1.0 } else { 0.0 };
            frame.add_char(ch, 2.0 + col as f32 * CELL, y, CELL, h - short, 6.0, false);
            if underlined && let Some(FrameGlyph::Char { face_id, .. }) = frame.glyphs.last_mut() {
                *face_id = FaceId::new(if col < 4 { view.lead } else { UNDER });
            }
            if extras.inky && at == INKY_LINE
                && let Some(FrameGlyph::Char { face_id, .. }) = frame.glyphs.last_mut()
            {
                *face_id = FaceId::new(BIG);
            }
        }
        if at == FRINGE_LINE && !extras.cornered {
            frame.push_fringe_bitmap(0.0, y, 2.0, h, 1, FaceId::new(NORMAL), FringeSide::Left);
        }
        if extras.tiled && at == TILED_LINE {
            frame.push_fringe_bitmap(0.0, y, 2.0, h, 2, FaceId::new(NORMAL), FringeSide::Left);
        }
        let end = 2.0 + text.chars().count() as f32 * CELL;
        let right = if extras.cornered { width - 10.0 } else { width };
        frame.add_stretch(end, y, (right - end).max(0.0), h, BG, FaceId::new(NORMAL), false);
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
    if extras.paned || extras.cornered {
        // Round the window and its mode line, its corners clear of the rows,
        // its top edge on the first row; cornered, its right corners start
        // where the rows end.
        let right = if extras.cornered { width - 6.0 } else { width + 6.0 };
        frame.panes.push(romacs_display_protocol::frame_glyphs::Pane {
            bounds: Rect::new(-6.0, 0.0, right + 6.0, body + ROW),
            radius: 4.0,
            edge: Some(Color::rgb(0.9, 0.9, 0.2)),
            dim: 0.0,
        });
    }
    if extras.bordered {
        frame.set_draw_context(window, GlyphRowRole::Text, Some(Rect::new(0.0, 2.0 * ROW, width, 2.0 * ROW)));
        frame.add_border(width - 6.0, 0.0, 2.0, body, Color::rgb(0.8, 0.2, 0.2));
    }
    frame.set_draw_context(window, GlyphRowRole::ModeLine, None);
    if extras.cornered {
        frame.push_fringe_bitmap(0.0, body, 2.0, ROW, 1, FaceId::new(NORMAL), FringeSide::Left);
    }
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
    let at = |first, cursor| View { first, cursor, lead: UNDER };
    vec![
        ("start", at(0, (0, 0)), None),
        ("the start again", at(0, (0, 0)), None),
        ("down a line", at(1, (0, 0)), Some(1)),
        ("down a line past the image", at(2, (0, 0)), Some(1)),
        ("up a line", at(1, (0, 0)), Some(1)),
        ("up a line as the underline's head changes face", View { lead: UNDER_LOW, ..at(0, (0, 0)) }, None),
        ("down a line again", at(1, (0, 0)), None),
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

fn scrolled_through_picture_equals_whole(scale: f32, extras: Extras) {
    let Some(mut h) = try_harness() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    h.renderer.effects.cursor_color_cycle.enabled = false;
    h.renderer.set_scale_factor(scale);
    load_image(&mut h);
    let mut picture = RootPicture::new(&h.renderer, SnapshotSize::new(W, H).unwrap());
    let bands = call_count::of(romacs_renderer_wgpu::cost::BANDS_MOVED);
    for (name, view, entered) in scrolls() {
        let frame = view_frame(view, scale, extras);
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
        if let Some(entered) = entered.filter(|_| (scale - 1.0).abs() < f32::EPSILON && !(extras.inky || extras.tiled || extras.bordered || extras.underlined || extras.paned || extras.cornered)) {
            assert!(matches!(painted, Painted::Regions(_)), "{name}: {painted:?}");
            assert_eq!(rows, entered, "{name}: the rows that entered, and no other");
        }
    }
    if (extras.paned || extras.cornered) && (scale - 1.5).abs() > f32::EPSILON {
        assert!(call_count::of(romacs_renderer_wgpu::cost::BANDS_MOVED) > bands, "bands move under the pane's edge");
    }
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole() {
    scrolled_through_picture_equals_whole(1.0, Extras::default());
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_at_a_scale_of_two() {
    scrolled_through_picture_equals_whole(2.0, Extras::default());
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_at_a_fractional_scale() {
    // No band moves at a scale that is no whole number: every frame is
    // repainted, and still equals the frame drawn whole.
    scrolled_through_picture_equals_whole(1.5, Extras::default());
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_where_ink_leaves_its_rows() {
    scrolled_through_picture_equals_whole(1.0, Extras { inky: true, ..Extras::default() });
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_where_a_fringe_tiles() {
    // A row moves by eight pixels, the tiles' phase by two of three.
    scrolled_through_picture_equals_whole(1.0, Extras { tiled: true, ..Extras::default() });
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_where_a_border_is_clipped() {
    // The border runs the window's height, its clip two rows of it: moved
    // with the rows, its visible stretch would move too.
    scrolled_through_picture_equals_whole(1.0, Extras { bordered: true, ..Extras::default() });
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_in_a_pane() {
    // The pane's corners and edges stay where the rows move under them; its
    // top edge lies on the first row and is drawn over each band moved.
    scrolled_through_picture_equals_whole(1.0, Extras { paned: true, ..Extras::default() });
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_in_a_pane_at_a_scale_of_two() {
    scrolled_through_picture_equals_whole(2.0, Extras { paned: true, ..Extras::default() });
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_beside_a_pane_s_corner() {
    // Ink far from the rows reaches past its cell: the rows' band, drawn
    // by its own ink, stays clear of the corner beside it, and moves.
    scrolled_through_picture_equals_whole(1.0, Extras { cornered: true, ..Extras::default() });
}

#[test]
fn a_scrolled_frame_through_the_root_picture_equals_it_drawn_whole_where_an_underline_changes_its_head() {
    // The underline run takes its line from its first glyphs, which change
    // face as the row moves; its shorter tail does not change.
    scrolled_through_picture_equals_whole(1.0, Extras { underlined: true, ..Extras::default() });
}

/// Two underlined faces whose lines lie two pixels apart in a 12-pixel row.
const SEAM_LEFT: u32 = 10;
const SEAM_RIGHT: u32 = 11;
const SEAM_ROW: f32 = 12.0;
const SEAM_CELL: f32 = 7.0;

/// Two windows side by side, each row underlined: the left one row at the
/// top ending at x 50, the right one from x 50 showing ROWS from FIRST.
/// Without LEFT, the right window alone.
#[expect(clippy::cast_precision_loss, reason = "a test's few rows and columns")]
fn seam_frame(rows: &[&str], first: usize, left: bool) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(W as f32, H as f32);
    (frame.char_width, frame.char_height) = (SEAM_CELL, SEAM_ROW);
    frame.background = BG;
    for (id, offset) in [(SEAM_LEFT, 1), (SEAM_RIGHT, 3)] {
        text_face(&mut frame, id, BG);
        let face = frame.faces.get_mut(&FaceId::new(id)).unwrap();
        face.attributes |= FaceAttributes::UNDERLINE;
        face.underline_style = romacs_display_protocol::face::UnderlineStyle::Line;
        face.underline_color = Some(Color::rgb(0.3, 0.9, 0.3));
        face.underline_placement = romacs_display_protocol::face::UnderlinePosition::FontMetric { offset_from_baseline: offset };
        face.underline_thickness = 1;
    }
    let body = 4.0 * SEAM_ROW;
    let put = |frame: &mut FrameGlyphBuffer, window: i64, x0: f32, y: f32, text: &str, face: u32| {
        frame.set_draw_context(DisplayWindowId::new(window), GlyphRowRole::Text, None);
        for (col, ch) in text.chars().enumerate() {
            frame.add_char(ch, x0 + col as f32 * SEAM_CELL, y, SEAM_CELL, SEAM_ROW, 8.0, false);
            if let Some(FrameGlyph::Char { face_id, .. }) = frame.glyphs.last_mut() {
                *face_id = FaceId::new(face);
            }
        }
    };
    if left {
        frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
        frame.add_background(0.0, 0.0, 50.0, body, BG);
        put(&mut frame, 1, 1.0, 0.0, "leftwin", SEAM_LEFT);
    }
    frame.set_draw_context(DisplayWindowId::new(2), GlyphRowRole::Text, None);
    frame.add_background(50.0, 0.0, W as f32 - 50.0, body, BG);
    for (row, text) in rows.iter().skip(first).take(4).enumerate() {
        put(&mut frame, 2, 50.0, row as f32 * SEAM_ROW, text, SEAM_RIGHT);
    }
    frame
}

const SEAM_ROWS: [&str; 6] = ["wren", "mink", "vole", "hare", "lynx", "orca"];

/// FRAME drawn whole at 1x.
fn drawn_whole(h: &mut Harness, frame: &FrameGlyphBuffer) -> Vec<u8> {
    let mapping = mapping_for_scale(frame, W, H, 1.0);
    h.atlas.set_current_frame_fonts(frame.font_bindings());
    let (texture, view) = make_tex(&h.renderer, "whole");
    h.renderer.render_frame_glyphs(&view, frame, &mut h.atlas, mapping, false, None, None, None, None);
    read_tex(&h.renderer, &texture)
}

/// FRAMES painted in turn through one picture at SCALE, each against it
/// drawn whole, byte for byte; the bands the picture moved.
fn through_picture(h: &mut Harness, frames: &[FrameGlyphBuffer], scale: f32) -> u64 {
    h.renderer.set_scale_factor(scale);
    let mut picture = RootPicture::new(&h.renderer, SnapshotSize::new(W, H).unwrap());
    let bands = call_count::of(romacs_renderer_wgpu::cost::BANDS_MOVED);
    for (at, frame) in frames.iter().enumerate() {
        let mapping = mapping_for_scale(frame, W, H, scale);
        h.atlas.set_current_frame_fonts(frame.font_bindings());
        let (whole_texture, whole_view) = make_tex(&h.renderer, "whole");
        h.renderer.render_frame_glyphs(&whole_view, frame, &mut h.atlas, mapping, false, None, None, None, None);
        let whole = read_tex(&h.renderer, &whole_texture);
        let (texture, view) = make_tex(&h.renderer, "through-picture");
        let target = PaintTarget { view: &view, mapping, cursor_visible: false, animated_cursor: None };
        let painted = picture.paint(&mut h.renderer, &mut h.atlas, frame, target);
        let through = read_tex(&h.renderer, &texture);
        let differing: Vec<u32> = (0..W * H)
            .filter(|&i| whole[i as usize * 4..i as usize * 4 + 4] != through[i as usize * 4..i as usize * 4 + 4])
            .collect();
        assert!(
            differing.is_empty(),
            "frame {at} at {scale}, {painted:?}: {} pixels differ, at {:?}",
            differing.len(),
            differing.iter().map(|i| (i % W, i / W)).collect::<Vec<_>>(),
        );
    }
    call_count::of(romacs_renderer_wgpu::cost::BANDS_MOVED) - bands
}

#[test]
fn a_window_s_underline_does_not_join_its_neighbour_s() {
    // The left window's underlined row ends where the right window's
    // starts, under the same rule: the right one draws its own line.
    let Some(mut h) = try_harness() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    let both = drawn_whole(&mut h, &seam_frame(&SEAM_ROWS, 0, true));
    let alone = drawn_whole(&mut h, &seam_frame(&SEAM_ROWS, 0, false));
    let right = |pixels: &[u8]| -> Vec<u8> {
        (0..H).flat_map(|y| pixels[((y * W + 50) * 4) as usize..((y * W + W) * 4) as usize].to_vec()).collect()
    };
    assert!(right(&both) == right(&alone), "the right window's pixels depend on the left window's underline");
}

#[test]
fn a_scrolled_window_beside_an_underlined_neighbour_equals_it_drawn_whole() {
    let Some(mut h) = try_harness() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    let frames = [0, 0, 1].map(|first| seam_frame(&SEAM_ROWS, first, true));
    assert!(through_picture(&mut h, &frames, 1.0) > 0, "the right window's rows move");
}

#[test]
#[expect(clippy::cast_precision_loss, reason = "the test surface's few pixels")]
fn a_band_moved_to_a_dimmed_pane_s_edge_at_seven_times_equals_it_drawn_whole() {
    // At 7x a pane 9 logical pixels tall has its bottom edge on device row
    // 62, a seventh of a pixel above its end. Rows three tall move from 1
    // and 4 to 3 and 6, into the edge: the edge is drawn over them, and the
    // dim on the row above the edge once.
    let Some(mut h) = try_harness() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    let frame = |rows: [(f32, &str); 2]| {
        let mut frame = FrameGlyphBuffer::with_size(W as f32 / 7.0, H as f32 / 7.0);
        (frame.char_width, frame.char_height) = (2.0, 3.0);
        // The dim pulls toward the frame's background: unlike the window's,
        // so a second dim shows.
        frame.background = Color::rgb(0.0, 0.0, 0.4);
        text_face(&mut frame, NORMAL, BG);
        frame.faces.get_mut(&FaceId::new(NORMAL)).unwrap().font_size = 2.0;
        frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
        frame.add_background(0.0, 0.0, 13.0, 9.0, BG);
        for (y, text) in rows {
            for (col, ch) in text.chars().enumerate() {
                frame.add_char(ch, 2.0 + 2.0 * f32::from(u8::try_from(col).unwrap()), y, 2.0, 3.0, 2.0, false);
            }
        }
        frame.panes.push(romacs_display_protocol::frame_glyphs::Pane {
            bounds: Rect::new(0.0, 0.0, 13.0, 9.0),
            radius: 0.0,
            edge: Some(Color::rgb(0.9, 0.9, 0.2)),
            dim: 0.3,
        });
        frame
    };
    let old = frame([(1.0, "ab"), (4.0, "cd")]);
    let new = frame([(3.0, "ab"), (6.0, "cd")]);
    assert!(through_picture(&mut h, &[old.clone(), old, new], 7.0) > 0, "the rows move");
}
