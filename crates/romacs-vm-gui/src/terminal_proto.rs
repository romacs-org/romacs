//! What a terminal's helper process and a session say to each other.
//!
//! A terminal lives in a helper process of its own for its whole life: the
//! pty, the emulator and its readers (`terminal_helper.rs`). A session
//! talks to it over a conversation socket (`terminal_client.rs`), in
//! romacs-record records of class [`CLASS`].
//!
//! The helper sends the grid as it is: cells with Alacritty's own colours
//! and flags, the colours the program set, the cursor and the modes. What
//! that looks like as buffer text and faces is the session's business, so a
//! change of display policy never touches a running helper. A helper speaks
//! the version it was built with for as long as its terminal lives; a
//! session speaks that one and the one before ([`VERSIONS`]).
//!
//! Version 1's cell flags and named colours are those of the pinned
//! `alacritty_terminal`. A pin that changes them is a new version, with a
//! translation here for the old one.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use romacs_record::{ClassRange, Error, Reader, Writer};

pub const CLASS: u16 = 2;
pub const NAME: &str = "terminal";
/// The version a helper built from this source speaks.
pub const VERSION: u16 = 1;
/// The versions a session built from this source speaks.
pub const VERSIONS: std::ops::RangeInclusive<u16> = 1..=1;

#[must_use]
pub fn helper_class() -> ClassRange { ClassRange::new(CLASS, NAME, VERSION..=VERSION) }
#[must_use]
pub fn session_class() -> ClassRange { ClassRange::new(CLASS, NAME, VERSIONS) }

/// Record kinds.
pub mod kind {
    /// Session: columns u16, rows u16. Starts the shell at that size the
    /// first time, resizes after; the helper answers with a full push.
    pub const OPEN: u16 = 1;
    /// Session: bytes for the pty.
    pub const INPUT: u16 = 2;
    /// Session: the revision it has applied. The helper sends its next
    /// push only after this, so pushes coalesce while a session is busy.
    pub const ACK: u16 = 3;
    /// Session: hang up the shell and end the helper.
    pub const CLOSE: u16 = 4;
    /// Helper: a piece of a push too large for one record.
    pub const PART: u16 = 10;
    /// Helper: the last piece of a [`super::Push`].
    pub const PUSH: u16 = 11;
}

/// The colours a program set, by Alacritty's index.
pub const COLORS: usize = alacritty_terminal::term::color::COUNT;

/// One grid cell, as the emulator holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub zero_width: Vec<char>,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
}

impl Default for Cell {
    fn default() -> Self {
        Self { c: ' ', zero_width: Vec::new(), fg: Color::Named(NamedColor::Foreground), bg: Color::Named(NamedColor::Background), flags: Flags::empty() }
    }
}

/// A grid line: its cells up to the last one that is not a default blank.
pub type Row = Vec<Cell>;

fn color_bits(color: Color) -> u32 {
    match color {
        Color::Named(name) => name as u32,
        Color::Spec(rgb) => 1 << 24 | u32::from(rgb.r) << 16 | u32::from(rgb.g) << 8 | u32::from(rgb.b),
        Color::Indexed(index) => 2 << 24 | u32::from(index),
    }
}

/// The named colour of INDEX, as `NamedColor`'s own discriminants.
fn named(index: u32) -> NamedColor {
    use NamedColor::{Background, BrightBlack, BrightBlue, BrightCyan, BrightForeground, BrightGreen, BrightMagenta, BrightRed, BrightWhite,
                     BrightYellow, Black, Blue, Cursor, Cyan, DimBlack, DimBlue, DimCyan, DimForeground, DimGreen, DimMagenta, DimRed, DimWhite,
                     DimYellow, Foreground, Green, Magenta, Red, White, Yellow};
    [Black, Red, Green, Yellow, Blue, Magenta, Cyan, White, BrightBlack, BrightRed, BrightGreen, BrightYellow, BrightBlue, BrightMagenta,
     BrightCyan, BrightWhite, Foreground, Background, Cursor, DimBlack, DimRed, DimGreen, DimYellow, DimBlue, DimMagenta, DimCyan, DimWhite,
     BrightForeground, DimForeground].into_iter().find(|name| *name as u32 == index).unwrap_or(Foreground)
}

fn color_of(bits: u32) -> Color {
    let [tag, r, g, b] = bits.to_be_bytes();
    match tag {
        1 => Color::Spec(Rgb { r, g, b }),
        2 => Color::Indexed(b),
        _ => Color::Named(named(bits & 0x00ff_ffff)),
    }
}

/// A colour at the 0.66 a dim cell draws in, Alacritty's factor.
#[must_use]
pub fn dim(rgb: [u8; 3]) -> [u8; 3] { rgb.map(|v| u8::try_from(u32::from(v) * 66 / 100).unwrap_or(u8::MAX)) }

/// The colour of INDEX where neither the program nor the theme gives it:
/// the 256-colour cube and greys, and Alacritty's defaults otherwise.
#[must_use]
pub fn default_color(index: usize) -> [u8; 3] {
    const ANSI: [[u8; 3]; 16] = romacs_vm::DEFAULT_ANSI;
    let byte = |value: usize| u8::try_from(value).unwrap_or(u8::MAX);
    match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let value = index - 16;
            let component = |n: usize| if n == 0 { 0 } else { byte(55 + 40 * n) };
            [component(value / 36), component(value / 6 % 6), component(value % 6)]
        }
        232..=255 => [byte((index - 232) * 10 + 8); 3],
        index if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&index) => dim(ANSI[index - NamedColor::DimBlack as usize]),
        257 => ANSI[0],
        _ => ANSI[7],
    }
}

/// Append one row: a cell count, then each cell.
#[must_use]
pub fn write_row(mut w: Writer, row: &[Cell]) -> Writer {
    w = w.u16(u16::try_from(row.len()).unwrap_or(u16::MAX));
    for cell in row.iter().take(usize::from(u16::MAX)) {
        w = w.u32(cell.c as u32).u32(color_bits(cell.fg)).u32(color_bits(cell.bg)).u16(cell.flags.bits())
            .u8(u8::try_from(cell.zero_width.len()).unwrap_or(u8::MAX));
        for c in cell.zero_width.iter().take(usize::from(u8::MAX)) { w = w.u32(*c as u32); }
    }
    w
}

/// # Errors
/// [`Error::Truncated`].
pub fn read_row(r: &mut Reader<'_>) -> Result<Row, Error> {
    let character = |bits: u32| char::from_u32(bits).unwrap_or(' ');
    (0..r.u16()?).map(|_| {
        let (c, fg, bg, flags, extra) = (character(r.u32()?), color_of(r.u32()?), color_of(r.u32()?), Flags::from_bits_truncate(r.u16()?), r.u8()?);
        let zero_width = (0..extra).map(|_| r.u32().map(character)).collect::<Result<_, _>>()?;
        Ok(Cell { c, zero_width, fg, bg, flags })
    }).collect()
}

/// What changed in a terminal since the push before.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Push {
    pub revision: u64,
    /// How many lines moved into history since the last push, when this
    /// push is a delta: `lines` then holds those and the screen. `None` for
    /// a full push, which carries every line and replaces what the session has.
    pub scrolled: Option<u32>,
    pub columns: u16,
    pub rows: u16,
    /// Lines above the screen after this push.
    pub history: u32,
    pub cursor_line: u16,
    pub cursor_column: u16,
    pub cursor_wraps: bool,
    /// Alacritty's `TermMode` bits.
    pub mode: u32,
    pub running: bool,
    /// Whether the shell's own process group has the terminal.
    pub shell_foreground: bool,
    /// The colours the program set: (index, rgb).
    pub colors: Vec<(u16, [u8; 3])>,
    /// (line, row): line 0 is the screen's first, history is negative.
    pub lines: Vec<(i32, Row)>,
}

impl Push {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default().u64(self.revision).u8(u8::from(self.scrolled.is_none())).u32(self.scrolled.unwrap_or(0)).u16(self.columns).u16(self.rows)
            .u32(self.history).u16(self.cursor_line).u16(self.cursor_column).u8(u8::from(self.cursor_wraps)).u32(self.mode)
            .u8(u8::from(self.running)).u8(u8::from(self.shell_foreground)).u16(u16::try_from(self.colors.len()).unwrap_or(0));
        for (index, [r, g, b]) in &self.colors { w = w.u16(*index).u8(*r).u8(*g).u8(*b); }
        w = w.u32(u32::try_from(self.lines.len()).unwrap_or(0));
        for (line, row) in &self.lines { w = write_row(w.i64(i64::from(*line)), row); }
        w.finish()
    }

    /// # Errors
    /// [`Error::Truncated`].
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader(bytes);
        let (revision, full, scrolled) = (r.u64()?, r.u8()? != 0, r.u32()?);
        let mut push = Push { revision, scrolled: (!full).then_some(scrolled), columns: r.u16()?, rows: r.u16()?, history: r.u32()?,
                              cursor_line: r.u16()?, cursor_column: r.u16()?, cursor_wraps: r.u8()? != 0, mode: r.u32()?, running: r.u8()? != 0,
                              shell_foreground: r.u8()? != 0, ..Push::default() };
        for _ in 0..r.u16()? { push.colors.push((r.u16()?, [r.u8()?, r.u8()?, r.u8()?])); }
        for _ in 0..r.u32()? {
            let line = i32::try_from(r.i64()?).map_err(|_| Error::Truncated)?;
            push.lines.push((line, read_row(&mut r)?));
        }
        Ok(push)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_push_reads_back_with_every_kind_of_cell() {
        let cells = vec![
            Cell::default(),
            Cell { c: 'é', zero_width: vec!['\u{301}', '\u{302}'], fg: Color::Spec(Rgb { r: 1, g: 2, b: 3 }), bg: Color::Indexed(200), flags: Flags::INVERSE | Flags::WRAPLINE },
            Cell { c: '漢', fg: Color::Named(NamedColor::DimForeground), bg: Color::Named(NamedColor::BrightWhite), flags: Flags::WIDE_CHAR | Flags::DIM, ..Cell::default() },
        ];
        let push = Push { revision: 7, scrolled: Some(3), columns: 80, rows: 24, history: 10_000, cursor_line: 23, cursor_column: 79, cursor_wraps: true,
                          mode: 0xdead_beef, running: true, shell_foreground: true, colors: vec![(1, [9, 8, 7]), (268, [0, 0, 0])],
                          lines: vec![(-10_000, vec![]), (0, cells)] };
        assert_eq!(Push::decode(&push.encode()), Ok(push));
        let full = Push { scrolled: None, ..Push::default() };
        assert_eq!(Push::decode(&full.encode()), Ok(full));
    }

    #[test]
    fn every_named_colour_survives() {
        for index in 0..COLORS as u32 {
            let name = named(index);
            assert_eq!(color_of(color_bits(Color::Named(name))), Color::Named(name));
        }
        assert_eq!(Push::decode(&[1, 2, 3]), Err(Error::Truncated));
    }
}
