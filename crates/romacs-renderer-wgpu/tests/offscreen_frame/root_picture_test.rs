//! A frame drawn through the root picture equals the same frame drawn whole,
//! byte for byte, over a sequence of frames: edits in either of two windows,
//! a scroll, cursor moves, styles and a glide, a blink, a selection and a
//! theme change, with a mode line, a border, a scroll bar, a boxed face and
//! a wave underline on screen; at a whole and at a fractional device scale,
//! and with rows that share slot ids.
use super::*;
use romacs_display_protocol::face::UnderlineStyle;
use romacs_display_protocol::frame_glyphs::{FringeBitmapData, FringeSide, WindowCursor};
use romacs_renderer_wgpu::{PaintTarget, Painted, RootPicture, SnapshotSize};

#[derive(Clone)]
struct Scene {
    left: Vec<&'static str>,
    right: Vec<&'static str>,
    first: usize,
    selected: Option<(usize, std::ops::Range<usize>)>,
    fg: Color,
    bg: Color,
    cursor: (usize, usize, CursorStyle),
    glide: Option<(f32, bool)>,
    /// A fringe bitmap wider than its column on the other window's last row.
    fringe: bool,
    /// The renderer's scroll bar track opacity.
    track_opacity: f32,
}

const ROW: f32 = 12.0;
const CELL: f32 = 7.0;
const SPLIT: f32 = 46.0;
const NORMAL: u32 = 0;
const SELECTED: u32 = 1;
const MODE: u32 = 2;
const BOXED: u32 = 3;
const WAVY: u32 = 4;
const UNDERLINED: u32 = 5;

fn face(frame: &mut FrameGlyphBuffer, id: u32, fg: Color, bg: Color) {
    frame.set_face(
        FaceId::new(id),
        fg,
        Some(bg),
        400,
        false,
        0,
        None,
        0,
        None,
        0,
        None,
    );
}

/// SCENE's frame at SCALE; with `shared_slots`, a frame cell taller than
/// its rows, so glyphs of two rows can share a slot id (derived from pixels).
#[expect(
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "one scene's whole frame, built in one place from a test's few rows and columns"
)]
fn scene_frame(scene: &Scene, scale: f32, shared_slots: bool) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(W as f32 / scale, H as f32 / scale);
    (frame.char_width, frame.char_height) = (CELL, if shared_slots { 2.0 * ROW } else { ROW });
    frame.background = scene.bg;
    face(&mut frame, SELECTED, scene.fg, Color::rgb(0.2, 0.3, 0.6));
    face(&mut frame, MODE, scene.fg, Color::rgb(0.35, 0.35, 0.4));
    face(&mut frame, BOXED, scene.fg, scene.bg);
    face(&mut frame, WAVY, scene.fg, scene.bg);
    face(&mut frame, UNDERLINED, scene.fg, scene.bg);
    face(&mut frame, NORMAL, scene.fg, scene.bg);
    {
        let boxed = frame.faces.get_mut(&FaceId::new(BOXED)).unwrap();
        boxed.attributes |= FaceAttributes::BOX;
        boxed.box_type = BoxType::Line;
        boxed.box_color = Some(Color::GREEN);
        boxed.box_line_width = 1.into();
        let wavy = frame.faces.get_mut(&FaceId::new(WAVY)).unwrap();
        wavy.attributes |= FaceAttributes::UNDERLINE;
        wavy.underline_style = UnderlineStyle::Wave;
        wavy.underline_color = Some(Color::RED);
        let underlined = frame.faces.get_mut(&FaceId::new(UNDERLINED)).unwrap();
        underlined.attributes |= FaceAttributes::UNDERLINE;
        underlined.underline_style = UnderlineStyle::Line;
        underlined.underline_color = Some(Color::rgb(0.3, 0.9, 0.3));
    }
    let text = |frame: &mut FrameGlyphBuffer,
                window: i64,
                x0: f32,
                lines: &[&str],
                first: usize,
                styled: &dyn Fn(usize, usize) -> u32| {
        frame.set_draw_context(DisplayWindowId::new(window), GlyphRowRole::Text, None);
        for (row, line) in lines.iter().skip(first).take(4).enumerate() {
            for (col, ch) in line.chars().enumerate() {
                let (x, y) = (x0 + 2.0 + col as f32 * CELL, 1.0 + row as f32 * ROW);
                let id = styled(row + first, col);
                // The first line's tail is set two pixels taller: one
                // underline run over glyphs of two heights.
                let height = if window == 1 && row + first == 0 && col >= 3 { ROW + 2.0 } else { ROW };
                if id == SELECTED {
                    frame.add_stretch(
                        x,
                        y,
                        CELL,
                        ROW,
                        Color::rgb(0.2, 0.3, 0.6),
                        FaceId::new(SELECTED),
                        false,
                    );
                }
                frame.add_char(ch, x, y, CELL, height, 9.0, false);
                if let Some(FrameGlyph::Char { face_id, .. }) = frame.glyphs.last_mut() {
                    *face_id = FaceId::new(id);
                }
            }
        }
    };
    frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
    frame.add_background(0.0, 0.0, SPLIT, 52.0, scene.bg);
    let selected = scene.selected.clone();
    text(
        &mut frame,
        1,
        0.0,
        &scene.left,
        scene.first,
        &|row, col| match &selected {
            Some((r, cols)) if *r == row && cols.contains(&col) => SELECTED,
            _ if row == 0 => UNDERLINED,
            _ if row == 1 && col < 4 => BOXED,
            _ => NORMAL,
        },
    );
    frame.add_border(SPLIT, 0.0, 2.0, 52.0, Color::rgb(0.5, 0.5, 0.5));
    frame.set_draw_context(DisplayWindowId::new(2), GlyphRowRole::Text, None);
    frame.add_background(SPLIT + 2.0, 0.0, W as f32 - SPLIT - 2.0, 52.0, scene.bg);
    text(&mut frame, 2, SPLIT + 2.0, &scene.right, 0, &|row, col| {
        if row == 2 && (1..5).contains(&col) {
            WAVY
        } else {
            NORMAL
        }
    });
    if scene.fringe {
        // Sixteen columns of ink in a two-pixel fringe, on a row with no text.
        frame.register_fringe_bitmap(
            1,
            FringeBitmapData { bits: vec![0xFFFF; 8], width: 16, height: 8, period: 0, align: 0 },
        );
        frame.push_fringe_bitmap(SPLIT + 2.0, 1.0 + 3.0 * ROW, 2.0, ROW, 1, FaceId::new(NORMAL), FringeSide::Left);
    }
    frame.add_scroll_bar(
        false,
        W as f32 - 4.0,
        0.0,
        4.0,
        52.0,
        scene.first as i64,
        4,
        10,
        scene.first as f32 * 5.0,
        20.0,
        Color::rgb(0.2, 0.2, 0.2),
        Color::rgb(0.6, 0.6, 0.6),
    );
    // The mode line shows point's line and column, as a status bar does.
    frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::ModeLine, None);
    frame.add_stretch(
        0.0,
        52.0,
        W as f32,
        ROW,
        Color::rgb(0.35, 0.35, 0.4),
        FaceId::new(MODE),
        false,
    );
    let (row, col, style) = scene.cursor;
    for (at, ch) in format!("L{row} C{col}").chars().enumerate() {
        frame.add_char(ch, 2.0 + at as f32 * CELL, 52.0, CELL, ROW, 9.0, false);
        if let Some(FrameGlyph::Char { face_id, .. }) = frame.glyphs.last_mut() {
            *face_id = FaceId::new(MODE);
        }
    }
    frame.set_phys_cursor(PhysCursor {
        window_id: DisplayWindowId::new(1),
        charpos: 0,
        row,
        col: col as u16,
        slot_id: DisplaySlotId {
            window_id: DisplayWindowId::new(1),
            row: row as u32,
            col: col as u16,
        },
        x: 2.0 + col as f32 * CELL,
        y: 1.0 + row as f32 * ROW,
        width: CELL,
        height: ROW,
        ascent: 9.0,
        style,
        color: Color::rgb(1.0, 0.5, 0.0),
        cursor_fg: Color::rgb(0.05, 0.05, 0.05),
    });
    // The other window's cursor, unselected: drawn hollow.
    frame.window_cursors.push(WindowCursor {
        window_id: DisplayWindowId::new(2),
        slot_id: DisplaySlotId {
            window_id: DisplayWindowId::new(2),
            row: 0,
            col: 1,
        },
        x: SPLIT + 2.0 + CELL,
        y: 1.0,
        width: CELL,
        height: ROW,
        style: CursorStyle::Hollow,
        color: Color::rgb(1.0, 0.5, 0.0),
        cursor_fg: Color::BLACK,
        ascent: 9.0,
        active: false,
    });
    frame
}

/// Where SCENE's gliding cursor is drawn this frame, if it glides.
fn animated(scene: &Scene) -> Option<AnimatedCursor> {
    scene.glide.map(|(x, trail)| AnimatedCursor {
        window_id: DisplayWindowId::new(1),
        x,
        y: 1.0 + ROW,
        width: CELL,
        height: ROW,
        corners: trail.then_some([
            (x - 9.0, 4.0),
            (x + CELL, 1.0 + ROW),
            (x + CELL, 1.0 + 2.0 * ROW),
            (x - 9.0, 9.0),
        ]),
        frame_id: DisplayFrameId::new(0),
    })
}

/// The scenes a session goes through, and whether each shows its cursor.
fn sequence() -> Vec<(&'static str, Scene, bool)> {
    let start = Scene {
        left: vec!["first", "boxed one", "third", "fourth", "fifth", "sixth"],
        right: vec!["right", "side text", "a wavy word", "last"],
        first: 0,
        selected: None,
        fg: Color::rgb(0.9, 0.9, 0.85),
        bg: Color::rgb(0.10, 0.12, 0.16),
        cursor: (0, 0, CursorStyle::FilledBox),
        glide: None,
        fringe: false,
        track_opacity: 0.6,
    };
    let mut steps = vec![("start", start.clone(), true)];
    let mut next = |name, change: &dyn Fn(&mut Scene), visible| {
        let mut scene = steps.last().unwrap().1.clone();
        scene.glide = None;
        change(&mut scene);
        steps.push((name, scene, visible));
    };
    next("cursor moves", &|s| s.cursor.1 = 3, true);
    next("cursor blinks off", &|_| {}, false);
    next("cursor blinks on", &|_| {}, true);
    next(
        "cursor moves down",
        &|s| s.cursor = (1, 2, CursorStyle::FilledBox),
        true,
    );
    next("cursor glides", &|s| s.glide = Some((9.5, false)), true);
    next(
        "cursor glides with a trail",
        &|s| s.glide = Some((12.0, true)),
        true,
    );
    next("a character is typed", &|s| s.left[2] = "thxird", true);
    next(
        "a character is typed in the other window",
        &|s| s.right[1] = "sidee text",
        true,
    );
    next(
        "a line is deleted",
        &|s| {
            s.left.remove(2);
        },
        true,
    );
    next("the window scrolls a line", &|s| s.first = 1, true);
    next(
        "a bar cursor",
        &|s| s.cursor = (1, 2, CursorStyle::Bar(2.0)),
        true,
    );
    next(
        "a hollow cursor",
        &|s| s.cursor = (1, 2, CursorStyle::Hollow),
        true,
    );
    // No glyph in its slot: the row's last glyph places it.
    next(
        "a cursor past its line's end",
        &|s| s.cursor = (0, 5, CursorStyle::FilledBox),
        true,
    );
    next(
        "a selection",
        &|s| {
            s.selected = Some((3, 1..4));
            s.cursor = (2, 4, CursorStyle::FilledBox);
        },
        true,
    );
    next("the selection grows", &|s| s.selected = Some((3, 1..5)), true);
    next(
        "a theme change",
        &|s| {
            s.fg = Color::rgb(0.1, 0.1, 0.1);
            s.bg = Color::rgb(0.95, 0.94, 0.9);
        },
        true,
    );
    next("nothing changed", &|_| {}, true);
    // The cursor inside the taller tail of an underlined line.
    next("a cursor in a line of two heights", &|s| { s.first = 0; s.cursor = (0, 4, CursorStyle::FilledBox); }, true);
    next("a fringe indicator on a line with no text", &|s| { s.right[3] = ""; s.fringe = true; }, true);
    next("the fringe indicator goes", &|s| s.fringe = false, true);
    next("the scroll bar's track opacity changes", &|s| s.track_opacity = 0.2, true);
    steps
}

fn through_picture_equals_whole(scale: f32, shared_slots: bool, alpha: f32) {
    let Some(mut h) = try_harness() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    h.renderer.effects.cursor_color_cycle.enabled = false;
    h.renderer.set_scale_factor(scale);
    let mut picture = RootPicture::new(&h.renderer, SnapshotSize::new(W, H).unwrap());
    let mut order = Vec::new();
    for (name, scene, visible) in sequence() {
        let mut frame = scene_frame(&scene, scale, shared_slots);
        frame.background_alpha = alpha;
        h.renderer.effects.scroll_bar.track_opacity = scene.track_opacity;
        let mapping = mapping_for_scale(&frame, W, H, scale);
        h.atlas.set_current_frame_fonts(frame.font_bindings());
        let (whole_texture, whole_view) = make_tex(&h.renderer, "whole");
        h.renderer.render_frame_glyphs(
            &whole_view,
            &frame,
            &mut h.atlas,
            mapping,
            visible,
            animated(&scene),
            None,
            None,
            None,
        );
        let whole = read_tex(&h.renderer, &whole_texture);
        let (texture, view) = make_tex(&h.renderer, "through-picture");
        let target = PaintTarget {
            view: &view,
            mapping,
            cursor_visible: visible,
            animated_cursor: animated(&scene),
        };
        order.push((name, picture.paint(&mut h.renderer, &mut h.atlas, &frame, target)));
        let through = read_tex(&h.renderer, &texture);
        let differing = (0..W * H)
            .filter(|&i| {
                whole[i as usize * 4..i as usize * 4 + 4] != through[i as usize * 4..i as usize * 4 + 4]
            })
            .collect::<Vec<_>>();
        assert!(
            differing.is_empty(),
            "{name} at {scale}: {} pixels differ, at {:?}, the first: whole {:?}, through the picture {:?}",
            differing.len(),
            differing.iter().map(|i| (i % W, i / W)).collect::<Vec<_>>(),
            differing.first().map(|&i| pxb(&whole, i % W, i / W)),
            differing.first().map(|&i| pxb(&through, i % W, i / W)),
        );
    }
    // The picture did what each step asks of it, no more. A frame after one
    // drawn directly pictures itself whole first.
    let painted: std::collections::HashMap<_, _> = order.iter().copied().collect();
    for pair in order.windows(2) {
        if pair[0].1 == Painted::Direct {
            assert!(
                matches!(pair[1].1, Painted::Whole | Painted::Direct),
                "{} after a direct frame: {:?}",
                pair[1].0,
                pair[1].1
            );
        }
    }
    let after_direct = |name: &str| {
        order
            .windows(2)
            .any(|p| p[1].0 == name && p[0].1 == Painted::Direct)
    };
    assert_eq!(painted["start"], Painted::Direct, "nothing to compare with");
    assert_eq!(
        painted["cursor moves"],
        Painted::Whole,
        "a frame repeating one drawn directly is pictured"
    );
    for name in [
        "cursor blinks off",
        "cursor blinks on",
        "cursor glides",
        "cursor glides with a trail",
        "a bar cursor",
        "a hollow cursor",
    ] {
        if !after_direct(name) {
            assert_eq!(painted[name], Painted::Clean, "{name} repaints nothing");
        }
    }
    assert_eq!(
        painted["cursor moves down"],
        Painted::Regions(1),
        "the mode line's row alone"
    );
    assert_eq!(
        painted["a character is typed in the other window"],
        Painted::Regions(1),
        "its row, not the other window's"
    );
    assert_eq!(painted["a theme change"], Painted::Direct);
    assert_eq!(painted["nothing changed"], Painted::Whole);
    assert_eq!(painted["the scroll bar's track opacity changes"], Painted::Direct, "a setting the picture was drawn under changed");
}

#[test]
fn a_frame_through_the_root_picture_equals_it_drawn_whole() {
    through_picture_equals_whole(1.0, false, 1.0);
}

#[test]
fn a_translucent_frame_through_the_root_picture_equals_it_drawn_whole() {
    // A frame whose background lets the desktop through (`alpha-background').
    through_picture_equals_whole(1.0, false, 0.9);
}

#[test]
fn a_frame_through_the_root_picture_equals_it_drawn_whole_where_rows_share_slots() {
    // The whole pass draws every glyph in the cursor's slot inverse: so must
    // the cursor's patch.
    through_picture_equals_whole(1.0, true, 1.0);
}

#[test]
fn a_frame_through_the_root_picture_equals_it_drawn_whole_at_a_fractional_scale() {
    through_picture_equals_whole(1.5, false, 1.0);
}


#[test]
fn moving_filled_box_matches_whole_over_adjacent_ink() {
    let mut h = try_harness().expect("this comparison requires a GPU adapter");
    h.renderer.effects.cursor_color_cycle.enabled = false;

    for (label, scale, frame_alpha, base_alpha, cursor_alpha) in [
        ("opaque", 1.0, 1.0, 1.0, 1.0),
        ("alpha-over-alpha", 1.0, 0.71, 1.0, 0.43),
        ("fractional-device-scale", 1.25, 0.71, 1.0, 0.43),
        ("nonopaque-base-color", 1.25, 0.71, 0.82, 0.43),
    ] {
        h.renderer.set_scale_factor(scale);
        let mut frame = filled_box_frame();
        frame.background_alpha = frame_alpha;
        frame.background.a = base_alpha;
        frame.active_cursor_mut().unwrap().color.a = cursor_alpha;
        frame.active_cursor_mut().unwrap().cursor_fg.a = 0.73;

        // The original failure: this glyph is outside the destination
        // cursor slot, but intersects the moving box.
        frame.add_char('W', 9.0, 16.0, 10.0, 18.0, 14.0, false);

        // Its second layer has a different ink color and baseline. The
        // overlay and the face background must also stay in draw order.
        face(
            &mut frame,
            1,
            Color::new(0.80, 0.21, 0.31, 0.51),
            Color::new(0.12, 0.42, 0.72, 0.58),
        );
        frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
        let row = frame.slot_row(16.0);
        frame.add_char_on_row(
            'f',
            (12.0, 16.0, row),
            (10.0, 18.0, 12.0),
            FaceId::new(1),
            BoxVerticalEdges::Both,
        );
        if let Some(FrameGlyph::Char { composed, bidi_level, .. }) = frame.glyphs.last_mut() {
            *composed = Some("ffi".into());
            *bidi_level = 1;
        }
        frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::ModeLine, None);
        frame.add_stretch(20.0, 25.0, 4.0, 5.0, Color::new(0.2, 0.7, 0.2, 0.53), FaceId::new(1), false);
        frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
        // Match the destination's real slot to exercise settled inversion too.
        let slot = frame.glyphs[0].slot_id().unwrap();
        frame.active_cursor_mut().unwrap().slot_id = slot;

        h.atlas.set_current_frame_fonts(frame.font_bindings());
        let mapping = mapping_for_scale(&frame, W, H, scale);
        let (expected_texture, expected_view) = make_tex(&h.renderer, label);
        let (actual_texture, actual_view) = make_tex(&h.renderer, label);
        let mut picture = RootPicture::new(&h.renderer, SnapshotSize::new(W, H).unwrap());
        let mut previous = None;
        for (step, moving, visible) in [
            ("first", None, true),
            ("warm", None, true),
            ("adjacent W", Some((12.5, 16.0, None)), true),
            (
                "skewed trail",
                Some((12.5, 16.0, Some([(13.0, 11.0), (24.0, 18.0), (23.0, 33.0), (9.0, 29.0)]))),
                true,
            ),
            ("blinked", None, false),
            ("settled", None, true),
        ] {
            let animated_cursor = moving.map(|(x, y, corners)| AnimatedCursor {
                window_id: frame.active_cursor().unwrap().window_id,
                x,
                y,
                width: 10.0,
                height: 18.0,
                corners,
                frame_id: DisplayFrameId::new(0),
            });
            h.renderer.render_frame_glyphs(
                &expected_view,
                &frame,
                &mut h.atlas,
                mapping,
                visible,
                animated_cursor,
                None,
                None,
                None,
            );
            let expected = read_tex(&h.renderer, &expected_texture);
            let painted = picture.paint(
                &mut h.renderer,
                &mut h.atlas,
                &frame,
                PaintTarget {
                    view: &actual_view,
                    mapping,
                    cursor_visible: visible,
                    animated_cursor,
                },
            );
            let actual = read_tex(&h.renderer, &actual_texture);
            let differing: Vec<_> = expected.chunks_exact(4).zip(actual.chunks_exact(4))
                .enumerate().filter(|(_, (a, b))| a != b).collect();
            assert!(
                differing.is_empty(),
                "{label}, {step}, {painted:?}: {} pixels differ, first {:?}",
                differing.len(),
                differing.first().map(|(i, (a, b))| (i % W as usize, i / W as usize, a, b)),
            );
            if let Some(previous) = previous.as_ref().filter(|_| step != "warm") {
                assert_ne!(&expected, previous, "{label}, {step} must visibly change");
            }
            let expected_path = match (step, base_alpha == 1.0) {
                (_, false) | ("first", true) => Painted::Direct,
                ("warm", true) => Painted::Whole,
                _ => Painted::Clean,
            };
            assert_eq!(painted, expected_path, "{label}, {step}");
            previous = Some(expected);
        }
    }
}

/// One layout slot. The cluster has two glyphs in the same slot, the second
/// extending beyond the first glyph's cell.
fn typing_slot(frame: &mut FrameGlyphBuffer, ch: char, slot: DisplaySlotId, x: f32, y: f32) -> f32 {
    let width = match ch {
        'W' => 12.0,
        'i' => 4.0,
        '@' => 8.0,
        _ => unreachable!("unexpected typing fixture character"),
    };
    let mut add = |ch, x, width| {
        frame.add_char(ch, x, y, width, ROW, 9.0, false);
        let Some(FrameGlyph::Char { slot_id, .. }) = frame.glyphs.last_mut() else {
            unreachable!("add_char appends a character")
        };
        *slot_id = slot;
    };
    if ch == '@' {
        add('i', x, 4.0);
        add('l', x + 4.0, 4.0);
    } else {
        add(ch, x, width);
    }
    width
}

fn typing_frame(lines: [&str; 2], (row, col): (u32, u16)) -> FrameGlyphBuffer {
    let mut frame = FrameGlyphBuffer::with_size(96.0, 64.0);
    let window_id = DisplayWindowId::new(1);
    frame.background = Color::BLACK;
    frame.set_draw_context(window_id, GlyphRowRole::Text, None);
    frame.set_face_with_font(
        FaceId::new(NORMAL), Color::WHITE, Some(Color::BLACK), "DejaVu Sans",
        400, false, 12.0, 0, None, 0, None, 0, None, false,
    );
    for (line, y, row) in [(lines[0], 0.0, 0), (lines[1], ROW, 1)] {
        let mut x = 2.0;
        for (ch, col) in line.chars().zip(0_u16..) {
            x += typing_slot(&mut frame, ch, DisplaySlotId { window_id, row, col }, x, y);
        }
    }
    frame.window_cursors.push(WindowCursor {
        window_id,
        slot_id: DisplaySlotId { window_id, row, col },
        x: 2.0 + f32::from(col) * CELL,
        y: if row == 0 { 0.0 } else { ROW },
        width: CELL,
        height: ROW,
        style: CursorStyle::FilledBox,
        color: Color::rgb(1.0, 0.5, 0.0),
        cursor_fg: Color::BLACK,
        ascent: 9.0,
        active: true,
    });
    frame
}

#[test]
fn typing_through_the_root_picture_equals_whole_frames() {
    use Painted::{Clean, Direct, Regions, Whole};

    let mut h = try_harness().expect("typing pixel comparison requires an offscreen GPU adapter");
    h.renderer.effects.cursor_color_cycle.enabled = false;
    let mut picture = RootPicture::new(&h.renderer, SnapshotSize::new(W, H).unwrap());
    let steps = [
        (["WiW", ""], (0, 3), Direct),
        (["WiW", ""], (0, 3), Whole),
        (["WiWi", ""], (0, 4), Regions(1)),
        // Inserting the cluster wraps it to the next row. The empty cursor
        // slot follows two glyphs tied at the preceding column.
        (["WiWi", "@"], (1, 1), Regions(1)),
        (["WiWi", "@"], (1, 0), Clean),
        (["WiW@", ""], (0, 3), Regions(1)),
        (["WiW", ""], (0, 3), Regions(1)),
    ];
    for (step, (lines, cursor, expected)) in steps.into_iter().enumerate() {
        let frame = typing_frame(lines, cursor);
        let mapping = mapping_for(&frame, W, H);
        h.atlas.set_current_frame_fonts(frame.font_bindings());
        h.renderer.render_frame_glyphs(
            &h.view, &frame, &mut h.atlas, mapping, true, None, None, None, None,
        );
        let whole = read_back(&h);
        let target = PaintTarget { view: &h.view, mapping, cursor_visible: true, animated_cursor: None };
        let painted = picture.paint(&mut h.renderer, &mut h.atlas, &frame, target);
        let retained = read_back(&h);
        assert!(retained == whole, "typing step {step}, {lines:?}, cursor {cursor:?}");
        assert_eq!(painted, expected, "typing step {step}, {lines:?}");
    }
}
