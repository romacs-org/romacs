//! How far drawn ink reaches past the cells that own it.
//!
//! A glyph's raster can leave its cell: an ascender or a mark above a row
//! set tighter than its font, a descender below, an italic's overhang. A
//! patch of the picture must then repaint where a changed glyph's ink was
//! and is, and draw every glyph whose ink enters the patch. The text pass
//! notes each quad it builds against its cell (`note`); a picture keeps the
//! largest reach its passes have seen and grows each patch by it.
use std::cell::{Cell, RefCell};

use rustc_hash::FxHashMap;

use romacs_display_protocol::types::Rect;

/// Logical pixels ink has reached past its cell, per side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Reach {
    pub(crate) up: f32,
    pub(crate) down: f32,
    pub(crate) left: f32,
    pub(crate) right: f32,
}

impl Reach {
    /// The larger of SELF and OTHER on every side.
    pub(crate) fn max(self, other: Self) -> Self {
        Self {
            up: self.up.max(other.up),
            down: self.down.max(other.down),
            left: self.left.max(other.left),
            right: self.right.max(other.right),
        }
    }

    /// Whether SELF reaches at least as far as OTHER on every side.
    pub(crate) fn covers(self, other: Self) -> bool {
        self.max(other) == self
    }

    /// RECT grown by SELF: where ink of a glyph in RECT may lie.
    pub(crate) fn grow(self, rect: Rect) -> Rect {
        Rect::new(
            rect.x - self.left,
            rect.y - self.up,
            rect.width + self.left + self.right,
            rect.height + self.up + self.down,
        )
    }
}

/// How far ink reached past its cells, by the row of cells it was drawn in:
/// the cells' window (0 for none), y and height, bit for bit.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Rows(FxHashMap<(i64, u32, u32), Reach>);

impl Rows {
    /// Rows of cells (their window, y and height) and how far ink reached
    /// in each.
    #[cfg(test)]
    pub(crate) fn of_rows(rows: impl IntoIterator<Item = ((i64, f32, f32), Reach)>) -> Self {
        Rows(rows.into_iter().map(|((window, y, h), reach)| ((window, y.to_bits(), h.to_bits()), reach)).collect())
    }

    /// The farthest ink drawn in a row of WINDOW whose height overlaps
    /// BOUNDS' reached: a group's own reach, and its neighbours' in any row
    /// it shares. None drawn there: none.
    pub(crate) fn of(&self, window: i64, bounds: Rect) -> Reach {
        self.0
            .iter()
            .filter(|((row_window, y, h), _)| {
                if *row_window != window {
                    return false;
                }
                let (y, h) = (f32::from_bits(*y), f32::from_bits(*h));
                y < bounds.y + bounds.height && bounds.y < y + h
            })
            .fold(Reach::default(), |reach, (_, row)| reach.max(*row))
    }

    /// SELF with OTHER's rows, the farther reach where both have one.
    pub(crate) fn merge(&mut self, other: Rows) {
        for (row, reach) in other.0 {
            let entry = self.0.entry(row).or_default();
            *entry = entry.max(reach);
        }
    }

    /// SELF after WINDOW's rows within FROM's height were copied DY away:
    /// what that window drew where they land is gone, and theirs reaches
    /// there too, and still where they were, whose pixels the copy leaves.
    /// Another window's rows at the same heights are not touched.
    pub(crate) fn moved(&mut self, window: i64, from: Rect, dy: f32) {
        let within = |(row_window, y, h): (i64, u32, u32), top: f32| {
            let (y, h) = (f32::from_bits(y), f32::from_bits(h));
            row_window == window && top <= y && y + h <= top + from.height
        };
        let carried: Vec<((i64, u32, u32), Reach)> = self
            .0
            .iter()
            .filter(|(row, _)| within(**row, from.y))
            .map(|(&(w, y, h), &reach)| ((w, (f32::from_bits(y) + dy).to_bits(), h), reach))
            .collect();
        self.0.retain(|row, _| !within(*row, from.y + dy));
        self.merge(Rows(carried.into_iter().collect()));
    }
}

/// The picture's reach, as a plan reads it: ALL, the farthest any ink it
/// drew went, for groups it has not drawn; and ROWS, by row, for those it
/// has.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Inks<'a> {
    pub(crate) all: Reach,
    pub(crate) rows: &'a Rows,
}

impl Inks<'_> {
    /// How far the ink of a group of WINDOW the picture drew at BOUNDS can
    /// reach.
    pub(crate) fn drawn(&self, window: i64, bounds: Rect) -> Reach {
        self.rows.of(window, bounds)
    }
}

thread_local! {
    static SEEN: Cell<Reach> = const { Cell::new(Reach { up: 0.0, down: 0.0, left: 0.0, right: 0.0 }) };
    static SEEN_ROWS: RefCell<Rows> = RefCell::new(Rows::default());
}

/// Note a glyph of WINDOW drawn as INK in its CELL.
pub(crate) fn note(window: i64, cell: Rect, ink: Rect) {
    let reach = Reach {
        up: (cell.y - ink.y).max(0.0),
        down: (ink.y + ink.height - cell.y - cell.height).max(0.0),
        left: (cell.x - ink.x).max(0.0),
        right: (ink.x + ink.width - cell.x - cell.width).max(0.0),
    };
    SEEN.set(SEEN.get().max(reach));
    SEEN_ROWS.with_borrow_mut(|rows| {
        let entry = rows.0.entry((window, cell.y.to_bits(), cell.height.to_bits())).or_default();
        *entry = entry.max(reach);
    });
}

/// Note POINTS, the corners of what was drawn for a glyph of WINDOW in CELL.
pub(crate) fn note_points(window: i64, cell: Rect, points: impl Iterator<Item = [f32; 2]>) {
    let (mut x0, mut y0, mut x1, mut y1) = (cell.x, cell.y, cell.x + cell.width, cell.y + cell.height);
    for [x, y] in points {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    note(window, cell, Rect::new(x0, y0, x1 - x0, y1 - y0));
}

/// The reach noted on this thread since the last take.
pub(crate) fn take() -> Reach {
    SEEN.take()
}

/// The reach noted on this thread since the last take of rows, by row.
pub(crate) fn take_rows() -> Rows {
    SEEN_ROWS.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_past_its_cell_reaches_by_the_overflow_and_a_take_starts_over() {
        take();
        note(1, Rect::new(10.0, 10.0, 7.0, 12.0), Rect::new(11.0, 8.0, 5.0, 16.0));
        note(1, Rect::new(10.0, 10.0, 7.0, 12.0), Rect::new(9.5, 12.0, 9.0, 4.0));
        assert_eq!(
            take(),
            Reach {
                up: 2.0,
                down: 2.0,
                left: 0.5,
                right: 1.5
            }
        );
        assert_eq!(take(), Reach::default());
        let reach = Reach {
            up: 1.0,
            down: 2.0,
            left: 0.0,
            right: 3.0,
        };
        assert_eq!(
            reach.grow(Rect::new(10.0, 10.0, 5.0, 5.0)),
            Rect::new(10.0, 9.0, 8.0, 8.0)
        );
        assert!(
            reach.covers(Reach {
                up: 1.0,
                ..Reach::default()
            }) && !reach.covers(Reach {
                left: 0.1,
                ..Reach::default()
            })
        );
        take_rows();
        // A wave two pixels under its cell.
        note_points(
            1,
            Rect::new(0.0, 0.0, 10.0, 20.0),
            [[0.0, 18.0], [10.0, 22.0]].into_iter(),
        );
        assert_eq!(
            take(),
            Reach {
                down: 2.0,
                ..Reach::default()
            }
        );
    }

    #[test]
    fn rows_keep_their_reach_and_move_with_their_rows() {
        take_rows();
        note(1, Rect::new(0.0, 0.0, 7.0, 10.0), Rect::new(0.0, 0.0, 10.0, 10.0));
        note(1, Rect::new(0.0, 30.0, 7.0, 10.0), Rect::new(-2.0, 30.0, 9.0, 10.0));
        note(2, Rect::new(90.0, 30.0, 7.0, 10.0), Rect::new(88.0, 30.0, 9.0, 10.0));
        let mut rows = take_rows();
        let right = Reach { right: 3.0, ..Reach::default() };
        let left = Reach { left: 2.0, ..Reach::default() };
        assert_eq!(rows.of(1, Rect::new(50.0, 0.0, 5.0, 10.0)), right, "a row's own");
        assert_eq!(rows.of(1, Rect::new(0.0, 10.0, 5.0, 10.0)), Reach::default(), "none drawn there");
        assert_eq!(rows.of(1, Rect::new(0.0, 5.0, 5.0, 30.0)), right.max(left), "every row it overlaps");
        assert_eq!(rows.of(2, Rect::new(0.0, 0.0, 5.0, 10.0)), Reach::default(), "another window's");
        // Window 1's rows of 0..20 are copied down 30: its row at 30 is
        // overwritten, its row at 0 keeps its pixels until drawn again, and
        // window 2's row at 30 is untouched.
        rows.moved(1, Rect::new(0.0, 0.0, 10.0, 20.0), 30.0);
        assert_eq!(rows.of(1, Rect::new(0.0, 30.0, 5.0, 10.0)), right);
        assert_eq!(rows.of(1, Rect::new(0.0, 0.0, 5.0, 10.0)), right);
        assert_eq!(rows.of(2, Rect::new(90.0, 30.0, 5.0, 10.0)), left);
    }
}
