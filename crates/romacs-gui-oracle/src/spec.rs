//! The declared scene: what a test says the frame must show.
//!
//! A spec has two halves. `buffer` and `point` tell the capture harness how to
//! build the scene in an editor; the checker never reads them. `expect` is the
//! declaration the checker asserts the capture against.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// An sRGB colour, written `#rrggbb` in specs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    /// Largest per-channel difference.
    pub fn dist(self, other: Rgb) -> u8 {
        (0..3)
            .map(|i| self.0[i].abs_diff(other.0[i]))
            .max()
            .unwrap_or(0)
    }
}

impl fmt::Debug for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2])
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl TryFrom<String> for Rgb {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let hex = s
            .strip_prefix('#')
            .filter(|h| h.len() == 6)
            .ok_or_else(|| format!("colour {s:?} is not #rrggbb"))?;
        let byte = |i: usize| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| format!("colour {s:?}: {e}"))
        };
        Ok(Rgb([byte(0)?, byte(2)?, byte(4)?]))
    }
}

impl From<Rgb> for String {
    fn from(c: Rgb) -> String {
        c.to_string()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spec {
    pub name: String,
    pub frame: Frame,
    pub font: FontSpec,
    pub cursor_color: Rgb,
    /// Harness input only; the checker ignores it.
    #[serde(default)]
    pub buffer: Vec<serde_json::Value>,
    /// Harness input only; the checker ignores it.
    #[serde(default)]
    pub point: Option<usize>,
    #[serde(default)]
    pub expect: Option<Expect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bg: Rgb,
    pub fg: Rgb,
    /// Width of each fringe in pixels; 0 means none.
    #[serde(default)]
    pub fringe: u32,
    /// `fringe` face colours; default to the frame's.
    #[serde(default)]
    pub fringe_bg: Option<Rgb>,
    #[serde(default)]
    pub fringe_fg: Option<Rgb>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FontSpec {
    pub family: String,
    pub pixel_size: u32,
    pub regular: PathBuf,
    #[serde(default)]
    pub bold: Option<PathBuf>,
    #[serde(default)]
    pub italic: Option<PathBuf>,
    /// Overrides for the metrics derived from the font file.
    #[serde(default)]
    pub ascent: Option<i32>,
    #[serde(default)]
    pub descent: Option<i32>,
    #[serde(default)]
    pub cell_width: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Expect {
    /// Frame pixel position of the text area's top-left cell.
    pub origin: [i32; 2],
    /// Screen rows from the top; rows not listed must be blank.
    pub rows: Vec<RowSpec>,
    #[serde(default)]
    pub cursor: Option<CursorSpec>,
    /// Text-area width in columns; default: every whole column to the
    /// frame's right edge, less the right fringe.
    #[serde(default)]
    pub cols: Option<usize>,
    /// Rows whose fringe shows a bitmap (continuation arrows). Every other
    /// fringe row must be plain fringe background.
    #[serde(default)]
    pub fringe_marks: Vec<FringeMark>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct FringeMark {
    pub row: usize,
    pub side: Side,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RowSpec {
    pub text: String,
    #[serde(default)]
    pub runs: Vec<Run>,
}

/// Attributes over the column range `start..end` of one row.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    #[serde(default)]
    pub fg: Option<Rgb>,
    #[serde(default)]
    pub bg: Option<Rgb>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CursorSpec {
    pub row: usize,
    pub col: usize,
}

impl Spec {
    pub fn from_json(text: &str) -> Result<Spec, String> {
        serde_json::from_str(text).map_err(|e| format!("spec: {e}"))
    }
}
