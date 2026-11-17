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
