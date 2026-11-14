//! Romacs Icons (`RomacsIcons.ttf`) from assets/icons, into `OUT_DIR`.
//!
//! Each `assets/icons/NAME.svg` is one glyph: a single `<path>` in a box 500
//! wide and 1000 tall, the baseline at y=800 (DIVERGENCES.md VM2-MODE-LINE);
//! `assets/icons/codepoints.txt` gives each name its codepoint in the
//! Supplementary Private Use Area-A. The font is 1000 units per em, every
//! advance 500, ascent 800 and descent 200. An icon is one column, and most
//! drawings are wider than one: each is scaled, whole, to SIDE clear of the
//! cell either side and centred in it, about its own middle height, so no
//! icon reaches into the character beside it. `FULL_SIZE` names the
//! exception. Outlines are TrueType's quadratic contours, clockwise: a cubic
//! curve becomes quadratics within a unit. The build is deterministic: no
//! timestamps, glyphs in codepoint order.
//!
//! allsorts writes each table; this lays out the font file around them.
use allsorts::binary::read::ReadArrayCow;
use allsorts::binary::write::{WriteBinary, WriteBinaryDep, WriteBuffer};
use allsorts::post::{Header, PostTable};
use allsorts::tables::glyf::{GlyfRecord, GlyfTable, Point, SimpleGlyph, SimpleGlyphFlag};
use allsorts::tables::loca::owned::LocaTable;
use allsorts::tables::os2::{Os2, Version0, Version1, Version2to4};
use allsorts::tables::owned::{NameRecord, NameTable};
use allsorts::tables::{Fixed, HeadTable, HheaTable, HmtxTable, IndexToLocFormat, LongHorMetric, MaxpTable,
                       MaxpVersion1SubTable};
use allsorts::tag;
use kurbo::{BezPath, CubicBez, Point as KPoint, Shape};
use std::path::Path;

const FAMILY: &str = "Romacs Icons";
const VERSION: &str = "1.000";
const COPYRIGHT: &str = "Copyright (c) 2014, Ryan L McIntyre (Symbols Nerd Font, Nerd Fonts 3.4.0), of which this is a \
    Modified Version; glyphs from Material Design Icons (Pictogrammers), Devicon (konpa), Octicons (GitHub Inc.) and Nerd Fonts";
const LICENSE: &str = "This Font Software is licensed under the SIL Open Font License, Version 1.1; its Material Design \
    Icons glyphs also under the Apache License 2.0, its Devicon, Octicons and Nerd Fonts glyphs under MIT. \
    See assets/fonts/OFL.txt and assets/icons/LICENSE in Romacs.";
const ADVANCE: f64 = 500.0;
const SIDE: f64 = 10.0;
const ASCENT: i16 = 800;
const DESCENT: i16 = 200;
const BASELINE: f64 = 800.0;
/// The R, Romacs's mark, keeps its size, centred on its cell and wider than
/// it: fitted it is too small to read as the mark. Only the status bar's
/// identity pill draws it (romacs-mode-line.el's `mode-icon`), with a space
/// cell either side that holds the overhang.
const FULL_SIZE: &[&str] = &["romacs"];

fn main() {
    let icons = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
    println!("cargo::rerun-if-changed={}", icons.display());
    let font = build(&icons).unwrap_or_else(|e| panic!("RomacsIcons.ttf: {e}"));
    let out = Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("RomacsIcons.ttf");
    std::fs::write(&out, font).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}

/// Each icon's name and codepoint (codepoints.txt), in codepoint order.
fn codepoints(text: &str) -> Result<Vec<(String, u32)>, String> {
    let mut found = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty() && !line.starts_with('#')) {
        let (name, point) = line.split_once(' ').ok_or_else(|| format!("codepoints.txt: `{line}`"))?;
        let point = u32::from_str_radix(point.trim().trim_start_matches("U+"), 16).map_err(|e| format!("`{line}`: {e}"))?;
        found.push((name.to_owned(), point));
    }
    found.sort_by_key(|(_, point)| *point);
    if let Some(pair) = found.windows(2).find(|pair| pair[0].1 == pair[1].1) {
        return Err(format!("codepoints.txt: {} and {} share U+{:X}", pair[0].0, pair[1].0, pair[0].1));
    }
    Ok(found)
}

/// The `d` of the drawing SVG's one path.
fn path_data(svg: &str) -> Option<&str> {
    let start = svg.find(" d=\"")? + 4;
    Some(&svg[start..start + svg[start..].find('"')?])
}

/// The drawing's outline as one path, in the SVG's coordinates.
fn outline(d: &str) -> Result<BezPath, String> {
    use svgtypes::SimplePathSegment as S;
    let mut path = BezPath::new();
    for segment in svgtypes::SimplifyingPathParser::from(d) {
        match segment.map_err(|e| e.to_string())? {
            S::MoveTo { x, y } => path.move_to((x, y)),
            S::LineTo { x, y } => path.line_to((x, y)),
            S::Quadratic { x1, y1, x, y } => path.quad_to((x1, y1), (x, y)),
            S::CurveTo { x1, y1, x2, y2, x, y } => path.curve_to((x1, y1), (x2, y2), (x, y)),
            S::ClosePath => path.close_path(),
        }
    }
    Ok(path)
}

/// Font units, rounded half up.
#[expect(clippy::cast_possible_truncation, reason = "a glyph's coordinates lie within a few ems of its origin")]
fn round(value: f64) -> i16 {
    (value + 0.5).floor() as i16
}

/// The drawing D as a glyph centred in its cell, fitted unless it is full
/// size: its contours as (point, on curve) runs.
fn contours(d: &str, fit: bool) -> Result<Vec<Vec<(Point, bool)>>, String> {
    let path = outline(d)?;
    let bounds = path.bounding_box();
    let scale = if fit { (ADVANCE - 2.0 * SIDE) / bounds.width() } else { 1.0 }.min(1.0);
    let middle = f64::midpoint(bounds.y0, bounds.y1);
    // SVG y grows down from the top of the box; font y up from the baseline.
    let place = |p: KPoint| Point(round(scale * p.x + ADVANCE / 2.0 - scale * f64::midpoint(bounds.x0, bounds.x1)),
                                  round(-scale * p.y + BASELINE - middle + scale * middle));
    let mut contours: Vec<Vec<(Point, bool)>> = Vec::new();
    let mut last = KPoint::ZERO;
    for element in path.elements() {
        use kurbo::PathEl as E;
        match *element {
            E::MoveTo(p) => contours.push(vec![(place(p), true)]),
            E::LineTo(p) => contours.last_mut().ok_or("a line before a move")?.push((place(p), true)),
            E::QuadTo(c, p) => contours.last_mut().ok_or("a curve before a move")?.extend([(place(c), false), (place(p), true)]),
            E::CurveTo(c1, c2, p) => {
                let contour = contours.last_mut().ok_or("a curve before a move")?;
                for (_, _, quad) in CubicBez::new(last, c1, c2, p).to_quads(1.0 / scale) {
                    contour.extend([(place(quad.p1), false), (place(quad.p2), true)]);
                }
            }
            E::ClosePath => {}
        }
        last = match *element {
            E::MoveTo(p) | E::LineTo(p) | E::QuadTo(_, p) | E::CurveTo(_, _, p) => p,
            E::ClosePath => last,
        };
    }
    for contour in &mut contours {
        // A closing point on the start is the start.
        if contour.len() > 1 && contour.last() == contour.first() {
            contour.pop();
        }
        // The flip made the drawing's direction counter-clockwise: reverse
        // each contour about its first point.
        contour[1..].reverse();
    }
    // A lone move, or a path back to where it began, draws nothing.
    contours.retain(|contour| contour.len() > 1);
    Ok(contours)
}

fn simple_glyph(contours: &[Vec<(Point, bool)>]) -> Result<SimpleGlyph, String> {
    let mut coordinates = Vec::new();
    let mut end_pts_of_contours = Vec::new();
    for contour in contours {
        for &(point, on) in contour {
            let flags = if on { SimpleGlyphFlag::ON_CURVE_POINT.into() } else { allsorts::tables::glyf::SimpleGlyphFlags::empty() };
            coordinates.push((flags, point));
        }
        end_pts_of_contours.push(u16::try_from(coordinates.len() - 1).map_err(|e| e.to_string())?);
    }
    let bounding_box = allsorts::tables::glyf::BoundingBox::from_points(coordinates.iter().map(|&(_, point)| point));
    Ok(SimpleGlyph { bounding_box, end_pts_of_contours, instructions: Box::default(), coordinates, phantom_points: None })
}

/// The table T's bytes, as allsorts writes it.
fn table<'a, T: WriteBinary<&'a T>>(value: &'a T) -> Result<Vec<u8>, String> {
    let mut buffer = WriteBuffer::new();
    T::write(&mut buffer, value).map_err(|e| format!("{e:?}"))?;
    Ok(buffer.into_inner())
}

fn utf16be(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// A cmap of format 12 for (0, 4) and (3, 10): the icons are beyond the BMP.
fn cmap(chars: &[(u32, u16)]) -> Vec<u8> {
    let mut groups: Vec<(u32, u32, u32)> = Vec::new();
    for &(char, glyph) in chars {
        match groups.last_mut() {
            Some((start, end, start_glyph)) if *end + 1 == char && *start_glyph + (char - *start) == u32::from(glyph) => *end = char,
            _ => groups.push((char, char, u32::from(glyph))),
        }
    }
    let subtable_len = 16 + 12 * groups.len();
    let mut out = Vec::new();
    out.extend(0u16.to_be_bytes());
    out.extend(2u16.to_be_bytes());
    for (platform, encoding) in [(0u16, 4u16), (3, 10)] {
        out.extend(platform.to_be_bytes());
        out.extend(encoding.to_be_bytes());
        out.extend(20u32.to_be_bytes());
    }
    out.extend(12u16.to_be_bytes());
    out.extend(0u16.to_be_bytes());
    out.extend(u32::try_from(subtable_len).expect("a small cmap").to_be_bytes());
    out.extend(0u32.to_be_bytes());
    out.extend(u32::try_from(groups.len()).expect("a small cmap").to_be_bytes());
    for (start, end, glyph) in groups {
        out.extend(start.to_be_bytes());
        out.extend(end.to_be_bytes());
        out.extend(glyph.to_be_bytes());
    }
    out
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// The font file of TABLES, tag and bytes each: the table directory, each
/// table four-byte aligned, and head's checkSumAdjustment.
fn sfnt(mut tables: Vec<(u32, Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| *tag);
    let count = u16::try_from(tables.len()).expect("a few tables");
    let power = 1u16 << count.ilog2();
    let mut out = Vec::new();
    out.extend(0x0001_0000u32.to_be_bytes());
    out.extend(count.to_be_bytes());
    out.extend((power * 16).to_be_bytes());
    out.extend(u16::try_from(power.trailing_zeros()).expect("a small log").to_be_bytes());
    out.extend((count * 16 - power * 16).to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, data) in &tables {
        out.extend(tag.to_be_bytes());
        out.extend(checksum(data).to_be_bytes());
        out.extend(u32::try_from(offset).expect("a small font").to_be_bytes());
        out.extend(u32::try_from(data.len()).expect("a small table").to_be_bytes());
        if *tag == tag::HEAD {
            head_at = Some(offset);
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    let head = head_at.expect("a head table") + 8;
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    out[head..head + 4].copy_from_slice(&adjustment.to_be_bytes());
    out
}

fn build(icons: &Path) -> Result<Vec<u8>, String> {
    let read = |name: &str| std::fs::read_to_string(icons.join(name)).map_err(|e| format!("assets/icons/{name}: {e}"));
    let points = codepoints(&read("codepoints.txt")?)?;
    let mut glyphs = vec![GlyfRecord::empty()];
    let mut metrics = vec![LongHorMetric { advance_width: 500, lsb: 0 }];
    let (mut max_points, mut max_contours) = (0u16, 0u16);
    let (mut x_min, mut y_min, mut x_max, mut y_max) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
    let (mut min_rsb, mut x_max_extent) = (i16::MAX, i16::MIN);
    for (name, _) in &points {
        let svg = read(&format!("{name}.svg"))?;
        let d = path_data(&svg).ok_or_else(|| format!("assets/icons/{name}.svg: no path"))?;
        let glyph = simple_glyph(&contours(d, !FULL_SIZE.contains(&name.as_str()))?)?;
        let bounds = glyph.bounding_box;
        metrics.push(LongHorMetric { advance_width: 500, lsb: bounds.x_min });
        max_points = max_points.max(u16::try_from(glyph.coordinates.len()).map_err(|e| e.to_string())?);
        max_contours = max_contours.max(u16::try_from(glyph.end_pts_of_contours.len()).map_err(|e| e.to_string())?);
        (x_min, y_min, x_max, y_max) = (x_min.min(bounds.x_min), y_min.min(bounds.y_min), x_max.max(bounds.x_max), y_max.max(bounds.y_max));
        min_rsb = min_rsb.min(500 - bounds.x_max);
        x_max_extent = x_max_extent.max(bounds.x_max);
        glyphs.push(GlyfRecord::from(glyph));
    }
    let num_glyphs = u16::try_from(glyphs.len()).map_err(|e| e.to_string())?;
    let chars: Vec<(u32, u16)> = points.iter().zip(1u16..).map(|((_, point), glyph)| (*point, glyph)).collect();

    let mut glyf = WriteBuffer::new();
    let loca: LocaTable = GlyfTable::write_dep(&mut glyf, GlyfTable::new(glyphs).map_err(|e| format!("{e:?}"))?,
                                               IndexToLocFormat::Long).map_err(|e| format!("{e:?}"))?;
    let mut loca_bytes = WriteBuffer::new();
    LocaTable::write_dep(&mut loca_bytes, loca, IndexToLocFormat::Long).map_err(|e| format!("{e:?}"))?;

    let head = HeadTable {
        major_version: 1, minor_version: 0, font_revision: Fixed::from(1.0f32), check_sum_adjustment: 0,
        magic_number: 0x5F0F_3CF5, flags: 3, units_per_em: 1000, created: 0, modified: 0,
        x_min, y_min, x_max, y_max, mac_style: allsorts::tables::MacStyle::empty(), lowest_rec_ppem: 3,
        font_direction_hint: 2, index_to_loc_format: IndexToLocFormat::Long, glyph_data_format: 0,
    };
    let hhea = HheaTable {
        ascender: ASCENT, descender: -DESCENT, line_gap: 0, advance_width_max: 500,
        min_left_side_bearing: metrics[1..].iter().map(|m| m.lsb).min().unwrap_or(0), min_right_side_bearing: min_rsb,
        x_max_extent, caret_slope_rise: 1, caret_slope_run: 0, caret_offset: 0, num_h_metrics: num_glyphs,
    };
    let maxp = MaxpTable {
        num_glyphs,
        version1_sub_table: Some(MaxpVersion1SubTable {
            max_points, max_contours, max_composite_points: 0, max_composite_contours: 0, max_zones: 2,
            max_twilight_points: 0, max_storage: 0, max_function_defs: 0, max_instruction_defs: 0,
            max_stack_elements: 0, max_size_of_instructions: 0, max_component_elements: 0, max_component_depth: 0,
        }),
    };
    let hmtx = HmtxTable { h_metrics: ReadArrayCow::Owned(metrics), left_side_bearings: ReadArrayCow::Owned(Vec::new()) };
    let names = [(0, COPYRIGHT.to_owned()), (1, FAMILY.to_owned()), (2, "Regular".to_owned()),
                 (3, format!("{FAMILY} {VERSION}")), (4, format!("{FAMILY} Regular")), (5, format!("Version {VERSION}")),
                 (6, "RomacsIcons-Regular".to_owned()), (13, LICENSE.to_owned()), (14, "https://openfontlicense.org".to_owned())];
    let name = NameTable {
        name_records: names.into_iter().map(|(name_id, text)| NameRecord {
            platform_id: 3, encoding_id: 1, language_id: 0x409, name_id, string: utf16be(&text).into(),
        }).collect(),
        langtag_records: Vec::new(),
    };
    let first = chars.first().map_or(0, |(c, _)| *c);
    let last = chars.last().map_or(0, |(c, _)| *c);
    let os2 = Os2 {
        version: 4, x_avg_char_width: 500, us_weight_class: 400, us_width_class: 5, fs_type: 0,
        y_subscript_x_size: 650, y_subscript_y_size: 600, y_subscript_x_offset: 0, y_subscript_y_offset: 75,
        y_superscript_x_size: 650, y_superscript_y_size: 600, y_superscript_x_offset: 0, y_superscript_y_offset: 350,
        y_strikeout_size: 50, y_strikeout_position: 300, s_family_class: 0, panose: [0; 10],
        // Bit 57: characters beyond the BMP; bit 90: the supplementary
        // private use planes.
        ul_unicode_range1: 0, ul_unicode_range2: 1 << (57 - 32), ul_unicode_range3: 1 << (90 - 64), ul_unicode_range4: 0,
        ach_vend_id: u32::from_be_bytes(*b"RMCS"), fs_selection: allsorts::tables::os2::FsSelectionFlag::REGULAR.into(),
        us_first_char_index: u16::try_from(first).unwrap_or(0xFFFF), us_last_char_index: u16::try_from(last).unwrap_or(0xFFFF),
        version0: Some(Version0 { s_typo_ascender: ASCENT, s_typo_descender: -DESCENT, s_typo_line_gap: 0,
                                  us_win_ascent: ASCENT.unsigned_abs(), us_win_descent: DESCENT.unsigned_abs() }),
        version1: Some(Version1 { ul_code_page_range1: 0, ul_code_page_range2: 0 }),
        version2to4: Some(Version2to4 { s_x_height: 0, s_cap_height: 0, us_default_char: 0, us_break_char: 32, us_max_context: 0 }),
        version5: None,
    };
    let post = PostTable {
        header: Header { version: 0x0003_0000, italic_angle: 0, underline_position: 0, underline_thickness: 0,
                         is_fixed_pitch: 0, min_mem_type_42: 0, max_mem_type_42: 0, min_mem_type_1: 0, max_mem_type_1: 0 },
        opt_sub_table: None,
    };
    Ok(sfnt(vec![
        (tag::HEAD, table(&head)?), (tag::HHEA, table(&hhea)?), (tag::MAXP, table(&maxp)?), (tag::HMTX, table(&hmtx)?),
        (tag::NAME, table(&name)?), (tag::OS_2, table(&os2)?), (tag::POST, table(&post)?), (tag::CMAP, cmap(&chars)),
        (tag::GLYF, glyf.into_inner()), (tag::LOCA, loca_bytes.into_inner()),
    ]))
}
