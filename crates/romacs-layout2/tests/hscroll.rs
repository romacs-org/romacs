//! A horizontally scrolled window (GNU's first_visible_x): its rows show
//! from the scrolled x, truncated on both sides with the `$' glyph or the
//! fringes' arrows, and point's cursor stays inside the text area, on the
//! truncation glyph past a row's last position (set_cursor_from_row).
use romacs_display_protocol::{FrameGlyph, frame_glyphs::{FringeSide, GlyphRowRole}};
use romacs_layout_input::{BitmapAlign, Face, FaceHandle, FrameSnapshot, FringeBitmap, Hscroll, Indicators, LineWrap,
    PlacedWindowSnapshot, TextSlice, WindowChrome, WindowEnd, WindowSnapshot};

fn face() -> Face {
    Face { foreground: [255; 3], background: [0; 3], family: "Liberation Mono".into(), pixel_size: 16.0, weight: 400, italic: false, width: 5 }
}

fn cell() -> f32 { romacs_layout2::frame_metrics(&face()).unwrap().1 as f32 }

/// A window ten cells wide showing TEXT with point at byte POINT, scrolled
/// HSCROLL cells (on point's row POINT_ROW cells), with 8-pixel fringes
/// showing the truncation arrows (bitmaps 3 and 4) or none.
fn window(text: &str, point: usize, (hscroll, point_row): (u32, u32), fringes: bool) -> WindowSnapshot {
    let bitmap = |number| FringeBitmap { number, bits: vec![0x1800; 8], width: 8, align: BitmapAlign::Center, period: 0, face: FaceHandle(0) };
    let indicators = Indicators { truncation: [Some(bitmap(3)), Some(bitmap(4))], ..Indicators::default() };
    let px = |cells: u32| cells * cell() as u32;
    WindowSnapshot {
        id: 1, width: px(10), height: 200, start_byte: 0, point_byte: point, buffer_version: 1,
        text: TextSlice { utf8: text.into(), face_runs: vec![], raw: vec![] },
        mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
        vscroll_px: 0, hscroll: Hscroll { rows: px(hscroll), point_row: px(point_row) }, tab_width: 8, wrap: LineWrap::Truncate,
        cursor_off: false, stretches: vec![], wrap_flips: vec![], line_numbers: None, faces: vec![face()],
        chrome: if fringes { WindowChrome { left_fringe: 8, right_fringe: 8, indicators, ..WindowChrome::default() } } else { WindowChrome::default() },
    }
}

/// SNAPSHOT laid out at the frame's origin with its cursor: the text
/// rows' characters by row, the cursor's rectangle, the fringe bitmaps and
/// what the layout reported.
fn lay_out(snapshot: WindowSnapshot) -> (Vec<String>, Option<(f32, f32)>, Vec<(u16, FringeSide)>, WindowEnd) {
    let width = snapshot.width + snapshot.chrome.left_fringe + snapshot.chrome.right_fringe;
    let x = snapshot.chrome.left_fringe;
    let frame = FrameSnapshot { font_selection: Default::default(), font_alternatives: Default::default(), width, height: 200,
        windows: vec![PlacedWindowSnapshot { x, y: 0, snapshot }], background_alpha: 1.0, children: vec![], tab_bar: None, chrome: Default::default(), status_bar: None };
    let color = romacs_display_protocol::Color::from_pixel(0xffffff);
    let (painted, mut ends) = romacs_layout2::LayoutEngine::new().unwrap().layout_frame(&frame, 1, Some((1, color))).unwrap();
    let mut rows: Vec<(i64, Vec<(i64, char)>)> = vec![];
    for g in &painted.glyphs {
        if let FrameGlyph::Char { char, x, y, row_role: GlyphRowRole::Text, .. } = g {
            match rows.iter_mut().find(|(at, _)| *at == *y as i64) {
                Some((_, row)) => row.push((*x as i64, *char)),
                None => rows.push((*y as i64, vec![(*x as i64, *char)])),
            }
        }
    }
    let rows = rows.into_iter().map(|(_, mut row)| { row.sort(); row.into_iter().map(|(_, c)| c).collect() }).collect();
    let cursor = painted.window_cursors.first().map(|c| (c.x - x as f32, c.width));
    let marks = painted.glyphs.iter().filter_map(|g| match g {
        FrameGlyph::FringeBitmap { bitmap_index, side, .. } => Some((*bitmap_index, *side)),
        _ => None,
    }).collect();
    (rows, cursor, marks, ends.remove(0))
}

#[test]
fn scrolled_rows_show_from_the_scrolled_column_with_both_truncations() {
    // No fringes: `$' at both edges of a row cut on both sides.
    let (rows, ..) = lay_out(window("abcdefghijklmnopqrstuvwxyz\nshort\n", 0, (4, 4), false));
    assert_eq!(rows, vec!["$fghijklm$", "$"]);
    // With fringes the arrows show instead, every scrolled row's left one.
    let (rows, _, marks, _) = lay_out(window("abcdefghijklmnopqrstuvwxyz\nshort\n", 0, (4, 4), true));
    assert_eq!(rows, vec!["efghijklmn", "t"]);
    assert_eq!(marks, vec![(3, FringeSide::Left), (4, FringeSide::Right), (3, FringeSide::Left)]);
}

#[test]
fn the_cursor_stays_inside_the_text_area_and_reports_where_point_is() {
    let text = "abcdefghijklmnopqrstuvwxyz\n";
    let c = cell();
    // Point past the right edge: on the row's end, as GNU's glyph_after.
    let (_, cursor, _, end) = lay_out(window(text, 26, (0, 0), true));
    assert_eq!(cursor.map(|c| c.0), Some(10.0 * c), "in the right fringe after a full row");
    let place = end.point.unwrap();
    assert_eq!((place.x, place.line_x, place.truncated_right, place.at_eol), (10.0 * c, 26.0 * c, true, true));
    // Without a right fringe the cursor is on the `$'.
    let (_, cursor, _, _) = lay_out(window(text, 20, (0, 0), false));
    assert_eq!(cursor.map(|c| c.0), Some(9.0 * c));
    // Scrolled, point shows where its column now is.
    let (_, cursor, _, end) = lay_out(window(text, 20, (14, 14), true));
    assert_eq!(cursor.map(|c| c.0), Some(6.0 * c));
    assert!(end.point.unwrap().truncated_left);
    // Scrolled out on the left: on the row's first glyph.
    let (_, cursor, _, end) = lay_out(window(text, 2, (14, 14), true));
    assert_eq!(cursor.map(|c| c.0), Some(0.0));
    assert_eq!(end.point.unwrap().line_x, 2.0 * c);
}

#[test]
fn only_points_row_scrolls_beyond_the_rows_minimum() {
    // `auto-hscroll-mode' `current-line': other rows keep min_hscroll (0).
    let (rows, ..) = lay_out(window("abcdefghijklmn\nabcdefghijklmn\n", 20, (0, 3), true));
    assert_eq!(rows, vec!["abcdefghij", "defghijklm"]);
}
