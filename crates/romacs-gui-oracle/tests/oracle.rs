//! The oracle's own test suite.
//!
//! Part 1 validates it on renderers already trusted: GNU Emacs 32 PGTK and
//! today's core-driven romacs, capturing the same declared scenes under a
//! private compositor (`harness/capture.py`). A failure there means the
//! oracle is wrong, except for the two romacs divergences recorded below,
//! which are checked to fail exactly where GNU's source says romacs differs.
//!
//! Part 2 proves it can fail. Every perturbation in `PERTURBATIONS.md` is a
//! test here: pixels perturbed in a real capture, the declaration perturbed
//! against an untouched capture, and real captures of perturbed scenes.
//! Each must be reported, under the kind of failure it is.

use std::path::{Path, PathBuf};

use romacs_gui_oracle::spec::{CursorSpec, Run};
use romacs_gui_oracle::{Capture, Kind, Report, Rgb, Spec, check};

const EDITORS: [&str; 2] = ["gnu", "romacs"];
const WHITE: Rgb = Rgb([0xff, 0xff, 0xff]);
const BLACK: Rgb = Rgb([0, 0, 0]);
const CURSOR: Rgb = Rgb([0x00, 0xa0, 0x00]);
const CW: i32 = 12;
const LH: i32 = 23;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn spec(rel: &str) -> Spec {
    let p = root().join("scenes").join(rel);
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    Spec::from_json(&text).unwrap()
}

fn capture(editor: &str, rel: &str) -> Capture {
    Capture::load_png(&root().join("tests/fixtures").join(editor).join(rel)).unwrap()
}

fn run(spec: &Spec, cap: &Capture) -> Report {
    check(spec, cap).unwrap_or_else(|e| panic!("{}: oracle error: {e}", spec.name))
}

fn assert_passes(spec: &Spec, cap: &Capture, what: &str) -> Report {
    let r = run(spec, cap);
    assert!(
        r.passed(),
        "{what}: expected a pass, got {:#?}\nrows read: {:?}",
        r.failures,
        r.rows_read
    );
    r
}

/// The perturbation must be reported, and as `kind`.
fn assert_fails(spec: &Spec, cap: &Capture, kind: Kind, what: &str) -> Report {
    let r = run(spec, cap);
    assert!(
        r.has(kind),
        "{what}: expected a {kind:?} failure, got {:#?}\nrows read: {:?}",
        r.failures,
        r.rows_read
    );
    r
}

fn kinds(r: &Report) -> Vec<Kind> {
    let mut v: Vec<Kind> = r.failures.iter().map(|f| f.kind).collect();
    v.sort_by_key(|k| format!("{k:?}"));
    v.dedup();
    v
}

// ---------------------------------------------------------------------------
// Part 1: validation on trusted renderers.

#[test]
fn validates_omega_hello_on_gnu_and_romacs() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let r = assert_passes(&s, &capture(ed, "omega-hello.png"), ed);
        assert_eq!(r.rows_read[0], "Ωμέγα Hello", "{ed}: exact read-back");
    }
}

#[test]
fn validates_twenty_scene_corpus_on_gnu_and_romacs() {
    for i in 0..20 {
        let name = format!("corpus-{i:02}");
        let s = spec(&format!("corpus/{name}.json"));
        for ed in EDITORS {
            assert_passes(
                &s,
                &capture(ed, &format!("{name}.png")),
                &format!("{ed} {name}"),
            );
        }
    }
}

#[test]
fn validates_wrapping_with_fringes_on_gnu() {
    assert_passes(
        &spec("wrap-fringe.json"),
        &capture("gnu", "wrap-fringe.png"),
        "gnu",
    );
}

#[test]
fn validates_wrapping_without_fringes_on_gnu() {
    let r = assert_passes(
        &spec("wrap-nofringe.json"),
        &capture("gnu", "wrap-nofringe.png"),
        "gnu",
    );
    assert!(r.rows_read[0].ends_with("lazy\\"), "{:?}", r.rows_read);
}

/// Recorded romacs divergence: romacs wraps text exactly as GNU does, but
/// does not paint the `fringe` face background (`#f2f2f2` declared; romacs
/// leaves `#ffffff`). Everything except the fringe must pass.
#[test]
fn romacs_wrap_with_fringes_fails_only_on_fringe_background() {
    let r = run(
        &spec("wrap-fringe.json"),
        &capture("romacs", "wrap-fringe.png"),
    );
    assert_eq!(kinds(&r), vec![Kind::Fringe], "{:#?}", r.failures);
}

/// Recorded romacs divergence: with no right fringe GNU reserves the last
/// column for the continuation glyph `\` (xdisp.c:3510-3517 subtracts
/// `continuation_pixel_width` from `last_visible_x`; xdisp.c:33200 makes the
/// glyph `\`). romacs fills all 40 columns and draws no `\`.
#[test]
fn romacs_wrap_without_fringes_fails_on_missing_continuation_glyph() {
    let r = run(
        &spec("wrap-nofringe.json"),
        &capture("romacs", "wrap-nofringe.png"),
    );
    assert_eq!(kinds(&r), vec![Kind::Text], "{:#?}", r.failures);
    assert!(
        r.failures
            .iter()
            .any(|f| f.row == Some(0) && f.col == Some(39)),
        "{:#?}",
        r.failures
    );
}

// ---------------------------------------------------------------------------
// Pixel perturbation helpers. Each edits a real capture.

fn cell_xy(row: i32, col: i32) -> (i32, i32) {
    (col * CW, row * LH)
}

fn map_cell(cap: &mut Capture, row: i32, col: i32, f: impl Fn(Rgb) -> Rgb) {
    let (x0, y0) = cell_xy(row, col);
    for y in y0..y0 + LH {
        for x in x0..x0 + CW {
            let p = cap.get(x, y).unwrap();
            cap.set(x, y, f(p));
        }
    }
}

fn copy_cell(cap: &mut Capture, from: (i32, i32), to: (i32, i32)) {
    let src = cap.clone();
    let (fx, fy) = cell_xy(from.0, from.1);
    let (tx, ty) = cell_xy(to.0, to.1);
    for y in 0..LH {
        for x in 0..CW {
            cap.set(tx + x, ty + y, src.get(fx + x, fy + y).unwrap());
        }
    }
}

fn shift(cap: &Capture, dx: i32, dy: i32) -> Capture {
    let mut out = Capture::new(cap.width, cap.height, WHITE);
    for y in 0..cap.height as i32 {
        for x in 0..cap.width as i32 {
            if let Some(p) = cap.get(x - dx, y - dy) {
                out.set(x, y, p);
            }
        }
    }
    out
}

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let c =
        |i: usize| (f32::from(a.0[i]) + (f32::from(b.0[i]) - f32::from(a.0[i])) * t).round() as u8;
    Rgb([c(0), c(1), c(2)])
}

/// Coverage of a black-on-white or white-on-cursor pixel, from its red
/// channel (black and the cursor green both have red 0).
fn coverage_on(p: Rgb, bg_red: u8) -> f32 {
    if bg_red == 0xff {
        1.0 - f32::from(p.0[0]) / 255.0
    } else {
        f32::from(p.0[0]) / 255.0
    }
}

/// Turn a black-on-white cell into a cursor cell, as GNU and romacs both
/// draw it: cursor-coloured box, glyph in the frame background.
fn paint_cursor(cap: &mut Capture, row: i32, col: i32) {
    map_cell(cap, row, col, |p| lerp(CURSOR, WHITE, coverage_on(p, 0xff)));
}

/// The inverse: a cursor cell back to black on white.
fn erase_cursor(cap: &mut Capture, row: i32, col: i32) {
    map_cell(cap, row, col, |p| lerp(WHITE, BLACK, coverage_on(p, 0x00)));
}

// ---------------------------------------------------------------------------
// Part 2a: pixel perturbations of real captures (omega-hello).

#[test]
fn perturb_colour_of_hello_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        for col in 6..11 {
            map_cell(&mut cap, 0, col, |p| {
                if p == WHITE {
                    p
                } else {
                    Rgb([p.0[0], p.0[1].saturating_add(24), p.0[2]])
                }
            });
        }
        assert_fails(&s, &cap, Kind::Colour, ed);
    }
}

#[test]
fn perturb_background_tint_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        for p in &mut cap.px {
            if *p == WHITE {
                *p = Rgb([0xfb, 0xfb, 0xfb]);
            }
        }
        assert_fails(&s, &cap, Kind::Colour, ed);
    }
}

#[test]
fn perturb_text_one_pixel_right_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let cap = shift(&capture(ed, "omega-hello.png"), 1, 0);
        assert_fails(&s, &cap, Kind::Geometry, ed);
    }
}

#[test]
fn perturb_text_one_pixel_down_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let cap = shift(&capture(ed, "omega-hello.png"), 0, 1);
        assert_fails(&s, &cap, Kind::Geometry, ed);
    }
}

#[test]
fn perturb_latin_glyph_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        copy_cell(&mut cap, (0, 9), (0, 7)); // `e` becomes `l`
        let r = assert_fails(&s, &cap, Kind::Text, ed);
        assert_eq!(r.rows_read[0], "Ωμέγα Hlllo", "{ed}");
    }
}

#[test]
fn perturb_greek_glyph_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        copy_cell(&mut cap, (0, 4), (0, 3)); // `γ` becomes `α`
        let r = assert_fails(&s, &cap, Kind::Text, ed);
        assert_eq!(r.rows_read[0], "Ωμέαα Hello", "{ed}");
    }
}

#[test]
fn perturb_erased_glyph_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        map_cell(&mut cap, 0, 10, |_| WHITE);
        assert_fails(&s, &cap, Kind::Text, ed);
    }
}

#[test]
fn perturb_cursor_moved_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        erase_cursor(&mut cap, 0, 0);
        paint_cursor(&mut cap, 0, 3);
        let r = assert_fails(&s, &cap, Kind::Cursor, ed);
        // The text is still all there: only the cursor moved.
        assert_eq!(r.rows_read[0], "Ωμέγα Hello", "{ed}");
        assert!(!r.has(Kind::Text), "{ed}: {:#?}", r.failures);
    }
}

#[test]
fn perturb_second_cursor_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        paint_cursor(&mut cap, 0, 2);
        assert_fails(&s, &cap, Kind::Cursor, ed);
    }
}

#[test]
fn perturb_underline_drawn_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        for x in 6 * CW..11 * CW {
            cap.set(x, 19, Rgb([0xff, 0, 0]));
        }
        assert_fails(&s, &cap, Kind::Style, ed);
    }
}

/// One glyph one pixel right, inside its own cell; the rest of the row is
/// untouched.
#[test]
fn perturb_single_glyph_one_pixel_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let src = capture(ed, "omega-hello.png");
        let mut cap = src.clone();
        let (x0, y0) = cell_xy(0, 6);
        for y in y0..y0 + LH {
            cap.set(x0, y, WHITE);
            for x in x0 + 1..x0 + CW {
                cap.set(x, y, src.get(x - 1, y).unwrap());
            }
        }
        let r = assert_fails(&s, &cap, Kind::Geometry, ed);
        assert!(
            r.failures
                .iter()
                .all(|f| f.kind != Kind::Geometry || f.col == Some(6)),
            "{ed}: {:#?}",
            r.failures
        );
    }
}

/// GNU draws a bold face that has no bold font by overstriking: the glyph
/// again, one pixel right (xterm.c, `s->face->overstrike`). That is bold to
/// the eye, and must read as bold: weight is judged by ink, not by shape.
#[test]
fn overstruck_regular_reads_as_bold() {
    for ed in EDITORS {
        let src = capture(ed, "omega-hello.png");
        let mut cap = src.clone();
        for col in 6..11 {
            let (x0, y0) = cell_xy(0, col);
            for y in y0..y0 + LH {
                for x in x0 + 1..x0 + CW {
                    let (p, q) = (src.get(x, y).unwrap(), src.get(x - 1, y).unwrap());
                    cap.set(
                        x,
                        y,
                        Rgb([p.0[0].min(q.0[0]), p.0[1].min(q.0[1]), p.0[2].min(q.0[2])]),
                    );
                }
            }
        }
        // Weight only: the overstrike widens each glyph by a pixel, which
        // may legitimately move its best bold placement.
        let bold = with(spec("omega-hello.json"), |s| hello_run(s).bold = true);
        let r = run(&bold, &cap);
        assert!(!r.has(Kind::Style), "{ed}: {:#?}", r.failures);
        assert_fails(&spec("omega-hello.json"), &cap, Kind::Style, ed);
    }
}

#[test]
fn perturb_stray_ink_in_blank_cell_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        for y in 100..104 {
            for x in 300..304 {
                cap.set(x, y, BLACK);
            }
        }
        let r = run(&s, &cap);
        assert!(
            r.has(Kind::Text) || r.has(Kind::Stray),
            "{ed}: {:#?}",
            r.failures
        );
    }
}

#[test]
fn perturb_stray_ink_outside_grid_is_reported() {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let mut cap = capture(ed, "omega-hello.png");
        // 7 rows of 23 px end at y = 161; the frame is 168 high.
        cap.set(200, 164, BLACK);
        assert_fails(&s, &cap, Kind::Stray, ed);
    }
}

#[test]
fn perturb_fringe_mark_removed_is_reported() {
    let s = spec("wrap-fringe.json");
    let mut cap = capture("gnu", "wrap-fringe.png");
    for y in 0..LH {
        for x in 472..480 {
            cap.set(x, y, Rgb([0xf2, 0xf2, 0xf2]));
        }
    }
    assert_fails(&s, &cap, Kind::Fringe, "gnu");
}

#[test]
fn perturb_fringe_mark_added_is_reported() {
    let s = spec("wrap-fringe.json");
    let mut cap = capture("gnu", "wrap-fringe.png");
    cap.set(475, 2 * LH + 10, BLACK); // row 2 is not continued
    assert_fails(&s, &cap, Kind::Fringe, "gnu");
}

// ---------------------------------------------------------------------------
// Part 2b: declaration perturbations against untouched captures.

fn with(mut s: Spec, f: impl FnOnce(&mut Spec)) -> Spec {
    f(&mut s);
    s
}

fn hello_run(s: &mut Spec) -> &mut Run {
    &mut s.expect.as_mut().unwrap().rows[0].runs[0]
}

#[test]
fn declared_wrong_text_is_reported() {
    let s = with(spec("omega-hello.json"), |s| {
        s.expect.as_mut().unwrap().rows[0].text = "Ωμέγα Hellp".into();
    });
    for ed in EDITORS {
        assert_fails(&s, &capture(ed, "omega-hello.png"), Kind::Text, ed);
    }
}

#[test]
fn declared_extra_row_is_reported() {
    let s = with(spec("omega-hello.json"), |s| {
        let e = s.expect.as_mut().unwrap();
        e.rows.push(Default::default());
        e.rows.push(romacs_gui_oracle::spec::RowSpec {
            text: "x".into(),
            runs: vec![],
        });
    });
    for ed in EDITORS {
        assert_fails(&s, &capture(ed, "omega-hello.png"), Kind::Text, ed);
    }
}

#[test]
fn declared_rows_beyond_the_frame_are_reported() {
    // 168 px holds 7 rows of 23; declare 8.
    let s = with(spec("omega-hello.json"), |s| {
        let rows = &mut s.expect.as_mut().unwrap().rows;
        rows.resize(7, Default::default());
        rows.push(romacs_gui_oracle::spec::RowSpec {
            text: "lost".into(),
            runs: vec![],
        });
    });
    for ed in EDITORS {
        assert_fails(&s, &capture(ed, "omega-hello.png"), Kind::Text, ed);
    }
}

/// Tolerance boundary: 9 levels from the drawn colour fails, 7 passes.
#[test]
fn declared_colour_tolerance_boundary() {
    for ed in EDITORS {
        let cap = capture(ed, "omega-hello.png");
        let off9 = with(spec("omega-hello.json"), |s| {
            hello_run(s).fg = Some(Rgb([0xf6, 0, 0]))
        });
        assert_fails(&off9, &cap, Kind::Colour, ed);
        let off7 = with(spec("omega-hello.json"), |s| {
            hello_run(s).fg = Some(Rgb([0xf8, 0, 0]))
        });
        assert_passes(&off7, &cap, ed);
    }
}

#[test]
fn declared_bold_italic_underline_are_reported() {
    for ed in EDITORS {
        let cap = capture(ed, "omega-hello.png");
        let b = with(spec("omega-hello.json"), |s| hello_run(s).bold = true);
        assert_fails(&b, &cap, Kind::Style, ed);
        let i = with(spec("omega-hello.json"), |s| hello_run(s).italic = true);
        assert_fails(&i, &cap, Kind::Style, ed);
        let u = with(spec("omega-hello.json"), |s| hello_run(s).underline = true);
        assert_fails(&u, &cap, Kind::Style, ed);
    }
}

#[test]
fn declared_cursor_elsewhere_or_absent_is_reported() {
    for ed in EDITORS {
        let cap = capture(ed, "omega-hello.png");
        let moved = with(spec("omega-hello.json"), |s| {
            s.expect.as_mut().unwrap().cursor = Some(CursorSpec { row: 0, col: 1 });
        });
        assert_fails(&moved, &cap, Kind::Cursor, ed);
        let none = with(spec("omega-hello.json"), |s| {
            s.expect.as_mut().unwrap().cursor = None
        });
        assert_fails(&none, &cap, Kind::Cursor, ed);
    }
}

#[test]
fn declared_origin_one_pixel_off_is_reported() {
    for ed in EDITORS {
        let cap = capture(ed, "omega-hello.png");
        let s = with(spec("omega-hello.json"), |s| {
            s.expect.as_mut().unwrap().origin = [1, 0]
        });
        assert_fails(&s, &cap, Kind::Geometry, ed);
        let s = with(spec("omega-hello.json"), |s| {
            s.expect.as_mut().unwrap().origin = [0, 1]
        });
        assert_fails(&s, &cap, Kind::Geometry, ed);
    }
}

#[test]
fn declared_line_height_one_pixel_off_is_reported() {
    // The font gives ascent 17 (hhea 1705/2048 x 20 px, hinted up) and
    // descent 6: a 23-pixel line. Declare one pixel either way.
    for ascent in [16, 18] {
        let s = with(spec("wrap-fringe.json"), |s| s.font.ascent = Some(ascent));
        for ed in EDITORS {
            let r = run(&s, &capture(ed, "wrap-fringe.png"));
            assert!(
                r.has(Kind::Geometry),
                "{ed} ascent {ascent}: {:#?}",
                r.failures
            );
        }
    }
}

#[test]
fn derived_metrics_match_the_captures() {
    let s = spec("omega-hello.json");
    let m = romacs_gui_oracle::glyphs::FontModel::load(&s.font, &['0']).unwrap();
    assert_eq!((m.cell_w, m.ascent, m.descent), (CW, 17, 6));
}

#[test]
fn declared_wrong_wrap_point_is_reported() {
    // Declare the break one column later than GNU makes it.
    let s = with(spec("wrap-fringe.json"), |s| {
        let rows = &mut s.expect.as_mut().unwrap().rows;
        let full: String = rows[0].text.clone() + &rows[1].text;
        let (a, b) = full.split_at(full.char_indices().nth(39).unwrap().0);
        rows[0].text = a.into();
        rows[1].text = b.into();
    });
    for ed in EDITORS {
        assert_fails(&s, &capture(ed, "wrap-fringe.png"), Kind::Text, ed);
    }
}

/// A limit, recorded as a test: Latin `o` and Greek `ο` are the same glyph
/// in Liberation Mono, so the oracle accepts either and says it did.
#[test]
fn homoglyph_is_accepted_and_reported_as_pixel_equivalent() {
    let s = with(spec("omega-hello.json"), |s| {
        s.expect.as_mut().unwrap().rows[0].text = "Ωμέγα Hellο".into(); // Greek omicron
    });
    for ed in EDITORS {
        let r = assert_passes(&s, &capture(ed, "omega-hello.png"), ed);
        assert!(
            !r.pixel_equivalent.is_empty(),
            "{ed}: equivalence not reported"
        );
    }
}

// ---------------------------------------------------------------------------
// Part 2c: real captures of perturbed scenes, checked against omega-hello.

fn real(name: &str, kind_any: &[Kind]) {
    let s = spec("omega-hello.json");
    for ed in EDITORS {
        let cap = capture(ed, &format!("real-perturb/{name}.png"));
        let r = run(&s, &cap);
        assert!(
            kind_any.iter().any(|k| r.has(*k)),
            "{ed} {name}: expected one of {kind_any:?}, got {:#?}\nrows read: {:?}",
            r.failures,
            r.rows_read
        );
    }
}

#[test]
fn real_colour_31_levels_off_is_reported() {
    real("colour-e00000", &[Kind::Colour]);
}

#[test]
fn real_colour_11_levels_off_is_reported() {
    real("colour-f40000", &[Kind::Colour]);
}

#[test]
fn real_cursor_moved_is_reported() {
    real("cursor-col2", &[Kind::Cursor]);
}

#[test]
fn real_latin_glyph_changed_is_reported() {
    real("glyph-hallo", &[Kind::Text]);
}

#[test]
fn real_greek_glyph_changed_is_reported() {
    real("glyph-greek", &[Kind::Text]);
}

#[test]
fn real_bold_is_reported() {
    real("bold-hello", &[Kind::Style]);
}

#[test]
fn real_italic_is_reported() {
    real("italic-hello", &[Kind::Style]);
}

#[test]
fn real_underline_is_reported() {
    real("underline-hello", &[Kind::Style]);
}

#[test]
fn real_wrong_font_is_reported() {
    real("font-adwaita", &[Kind::Text, Kind::Geometry]);
}

#[test]
fn real_shift_by_one_column_is_reported() {
    real("shift-one-column", &[Kind::Text]);
}

#[test]
fn fixtures_exist_and_are_frame_sized() {
    for ed in EDITORS {
        let c = capture(ed, "omega-hello.png");
        assert_eq!((c.width, c.height), (480, 168), "{ed}");
        assert!(Path::new(&spec("omega-hello.json").font.regular).exists());
    }
}

// ---------------------------------------------------------------------------
// Part 3: pair mode. GNU's capture is the reference; no rows are declared.

use romacs_gui_oracle::pair::pair;

fn pair_diffs(scene: &str, a: &Capture, b: &Capture) -> Vec<romacs_gui_oracle::pair::PairDiff> {
    pair(&spec(scene), a, b).unwrap().diffs
}

#[test]
fn pair_gnu_and_romacs_agree_on_omega_hello_and_corpus() {
    let d = pair_diffs(
        "omega-hello.json",
        &capture("gnu", "omega-hello.png"),
        &capture("romacs", "omega-hello.png"),
    );
    assert!(d.is_empty(), "omega-hello: {d:#?}");
    for i in 0..20 {
        let name = format!("corpus-{i:02}");
        let d = pair_diffs(
            &format!("corpus/{name}.json"),
            &capture("gnu", &format!("{name}.png")),
            &capture("romacs", &format!("{name}.png")),
        );
        assert!(d.is_empty(), "{name}: {d:#?}");
    }
}

#[test]
fn pair_finds_the_missing_continuation_glyph_without_a_declaration() {
    let d = pair_diffs(
        "wrap-nofringe.json",
        &capture("gnu", "wrap-nofringe.png"),
        &capture("romacs", "wrap-nofringe.png"),
    );
    assert!(
        d.iter()
            .any(|x| x.row == Some(0) && x.col == Some(39) && x.what.starts_with("text")),
        "{d:#?}"
    );
}

#[test]
fn pair_finds_only_the_fringe_background_on_wrap_fringe() {
    let d = pair_diffs(
        "wrap-fringe.json",
        &capture("gnu", "wrap-fringe.png"),
        &capture("romacs", "wrap-fringe.png"),
    );
    assert!(!d.is_empty(), "fringe background difference not found");
    assert!(
        d.iter()
            .all(|x| x.row.is_none() && x.what.contains("fringe background")),
        "{d:#?}"
    );
}

#[test]
fn pair_reports_every_real_perturbation_against_the_gnu_reference() {
    let reference = capture("gnu", "omega-hello.png");
    for name in [
        "colour-e00000",
        "colour-f40000",
        "cursor-col2",
        "glyph-hallo",
        "glyph-greek",
        "bold-hello",
        "italic-hello",
        "underline-hello",
        "font-adwaita",
        "shift-one-column",
    ] {
        for ed in EDITORS {
            let d = pair_diffs(
                "omega-hello.json",
                &reference,
                &capture(ed, &format!("real-perturb/{name}.png")),
            );
            assert!(!d.is_empty(), "{ed} {name}: no difference found");
        }
    }
}

#[test]
fn pair_reports_a_one_pixel_shift() {
    let reference = capture("gnu", "omega-hello.png");
    let shifted = shift(&capture("romacs", "omega-hello.png"), 1, 0);
    let d = pair_diffs("omega-hello.json", &reference, &shifted);
    assert!(
        d.iter().any(|x| x.what.starts_with("glyph offset")),
        "{d:#?}"
    );
}

// ---------------------------------------------------------------------------
// Independence (ruling D-F3): the oracle must not link anything romacs
// renders with, or it could end up asking the renderer what it drew.

fn romacs_deps(manifest: &str) -> Vec<String> {
    let mut in_deps = false;
    let mut out = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_deps = line.contains("dependencies");
            continue;
        }
        if in_deps && line.starts_with("romacs") {
            out.push(line.split(['=', ' ', '.']).next().unwrap_or("").to_string());
        }
    }
    out
}

#[test]
fn oracle_depends_on_no_romacs_crate() {
    // The detector itself must see a romacs dependency when there is one.
    let planted = "[dependencies]\npng.workspace = true\nromacs-font.workspace = true\n";
    assert_eq!(romacs_deps(planted), vec!["romacs-font".to_string()]);
    let manifest = std::fs::read_to_string(root().join("Cargo.toml")).unwrap();
    assert_eq!(romacs_deps(&manifest), Vec::<String>::new());
}
