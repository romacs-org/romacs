//! Glyph templates rasterised independently with FreeType.
//!
//! romacs rasterises through swash into a wgpu atlas; GNU PGTK through cairo
//! and FreeType with its own hinting and gamma. Neither produces the bitmaps
//! here, so neither renderer can bless its own glyphs. Templates are compared
//! by correlation, never by pixel equality, because no two rasterisers agree
//! on antialiasing.

use std::path::Path;

use freetype::face::LoadFlag;

use crate::spec::FontSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Style {
    Regular,
    Bold,
    Italic,
}

/// FreeType hinting mode a template was rasterised with.
///
/// Each character gets one template per mode and a cell matches a character
/// if it matches any of them. Measured on `omega-hello`: GNU PGTK (cairo,
/// slight hinting) correlates 1.000 with `Light` on every cell, and romacs
/// (swash) 0.984-0.998 with `Full`; with a single mode, GNU's `H` matched
/// `Full` only one pixel up and its `e` lost to `c`. Variants of one
/// character never vote for another, so this widens legitimate rendering,
/// not the set of characters accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Hint {
    Light,
    Full,
    None,
}

impl Hint {
    pub const ALL: [Hint; 3] = [Hint::Light, Hint::Full, Hint::None];

    fn flag(self) -> LoadFlag {
        match self {
            Hint::Light => LoadFlag::TARGET_LIGHT,
            Hint::Full => LoadFlag::DEFAULT,
            Hint::None => LoadFlag::NO_HINTING,
        }
    }
}

/// One glyph's coverage, positioned relative to its cell's top-left pixel.
#[derive(Clone, Debug)]
pub struct Template {
    pub ch: char,
    pub style: Style,
    pub hint: Hint,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Coverage 0.0..=1.0, row-major `w * h`.
    pub cov: Vec<f32>,
}

impl Template {
    pub fn at(&self, i: i32, j: i32) -> f32 {
        self.cov[(j * self.w + i) as usize]
    }
}

#[derive(Clone, Debug)]
pub struct FontModel {
    pub cell_w: i32,
    pub ascent: i32,
    pub descent: i32,
    pub templates: Vec<Template>,
}

impl FontModel {
    pub fn line_h(&self) -> i32 {
        self.ascent + self.descent
    }
}

/// The candidate alphabet a cell is classified over: printable ASCII, the
/// basic Greek block, and every character the scene declares. Read-back is an
/// argmax over this set, so a wrong glyph reads as the glyph it is.
pub fn alphabet(extra: impl IntoIterator<Item = char>) -> Vec<char> {
    let mut v: Vec<char> = ('!'..='~').collect();
    v.extend('\u{0386}'..='\u{03ce}');
    v.extend(extra);
    v.retain(|c| !c.is_whitespace());
    v.sort_unstable();
    v.dedup();
    v
}

fn face(lib: &freetype::Library, path: &Path, px: u32) -> Result<freetype::Face, String> {
    let f = lib
        .new_face(path, 0)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    f.set_pixel_sizes(0, px)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(f)
}

impl FontModel {
    /// Metrics follow GNU's cairo driver: `ftcrfont.c` sets
    /// `font->ascent = lround (extents.ascent)` and
    /// `font->descent = lround (extents.descent)`, and cairo-ft takes those
    /// extents from the hinted `size->metrics`. The cell width is the
    /// hinted advance of `0`. A spec may override any of the three.
    pub fn load(spec: &FontSpec, alphabet: &[char]) -> Result<FontModel, String> {
        let lib = freetype::Library::init().map_err(|e| format!("freetype: {e}"))?;
        let regular = face(&lib, &spec.regular, spec.pixel_size)?;
        let m = regular.size_metrics().ok_or("font has no size metrics")?;
        fn round26(v: impl Into<i64>) -> i32 {
            (v.into() as f64 / 64.0).round() as i32
        }
        let ascent = spec.ascent.unwrap_or_else(|| round26(m.ascender));
        let descent = spec.descent.unwrap_or_else(|| round26(-m.descender));
        regular
            .load_char('0' as usize, LoadFlag::DEFAULT)
            .map_err(|e| format!("advance of 0: {e}"))?;
        let cell_w = spec
            .cell_width
            .unwrap_or_else(|| round26(regular.glyph().advance().x));
        let mut faces = vec![(Style::Regular, regular)];
        if let Some(p) = &spec.bold {
            faces.push((Style::Bold, face(&lib, p, spec.pixel_size)?));
        }
        if let Some(p) = &spec.italic {
            faces.push((Style::Italic, face(&lib, p, spec.pixel_size)?));
        }
        let mut templates = Vec::new();
        for (style, f) in &faces {
            for hint in Hint::ALL {
                for &ch in alphabet {
                    if f.get_char_index(ch as usize).is_none() {
                        continue;
                    }
                    f.load_char(ch as usize, LoadFlag::RENDER | hint.flag())
                        .map_err(|e| format!("render {ch:?}: {e}"))?;
                    let g = f.glyph();
                    let bm = g.bitmap();
                    let (w, h, pitch) = (bm.width(), bm.rows(), bm.pitch());
                    if w == 0 || h == 0 {
                        continue;
                    }
                    let buf = bm.buffer();
                    let mut cov = Vec::with_capacity((w * h) as usize);
                    for j in 0..h {
                        for i in 0..w {
                            cov.push(f32::from(buf[(j * pitch.abs() + i) as usize]) / 255.0);
                        }
                    }
                    templates.push(Template {
                        ch,
                        style: *style,
                        hint,
                        x: g.bitmap_left(),
                        y: ascent - g.bitmap_top(),
                        w,
                        h,
                        cov,
                    });
                }
            }
        }
        Ok(FontModel {
            cell_w,
            ascent,
            descent,
            templates,
        })
    }
}
