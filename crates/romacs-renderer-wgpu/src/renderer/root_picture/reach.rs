//! How far drawn ink reaches past the cells that own it.
//!
//! A glyph's raster can leave its cell: an ascender or a mark above a row
//! set tighter than its font, a descender below, an italic's overhang. A
//! patch of the picture must then repaint where a changed glyph's ink was
//! and is, and draw every glyph whose ink enters the patch. The text pass
//! notes each quad it builds against its cell (`note`) while a picture
//! draws (`seen`); the picture keeps what its passes noted and grows each
//! patch by it. A draw outside a picture notes nothing.
use std::cell::RefCell;

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

/// How far ink reached past its cells, by the row of cells it was drawn in
/// (the cells' window, 0 for none, y and height, bit for bit) and, within
/// it, by cell (x and right edge, bit for bit). Only cells whose ink left
/// them are kept: a record holds every ink pixel the picture has that lies
/// outside its cell inside some kept cell grown by its reach.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Rows(FxHashMap<(i64, u32, u32), Cells>);

/// A row's cells whose ink left them, by x and right edge, bit for bit.
type Cells = FxHashMap<(u32, u32), Reach>;

impl Rows {
    /// Cells (their window, x, y, width and height) and how far ink
    /// reached past each.
    #[cfg(test)]
    pub(crate) fn of_cells(cells: impl IntoIterator<Item = ((i64, Rect), Reach)>) -> Self {
        let mut rows = Rows::default();
        for ((window, cell), reach) in cells {
            rows.add(window, cell, reach);
        }
        rows
    }

    /// SELF with WINDOW's CELL reaching REACH, the farther where it has one.
    fn add(&mut self, window: i64, cell: Rect, reach: Reach) {
        if reach == Reach::default() {
            return;
        }
        let row = self.0.entry((window, cell.y.to_bits(), cell.height.to_bits())).or_default();
        let entry = row.entry((cell.x.to_bits(), (cell.x + cell.width).to_bits())).or_default();
        *entry = entry.max(reach);
    }

    /// The farthest ink drawn in a row of WINDOW whose height overlaps
    /// BOUNDS reached, from any of its cells: a group's own reach, and its
    /// neighbours' in any row it shares. None drawn there: none.
    pub(crate) fn of(&self, window: i64, bounds: Rect) -> Reach {
        self.0
            .iter()
            .filter(|((row_window, y, h), _)| {
                let (y, h) = (f32::from_bits(*y), f32::from_bits(*h));
                *row_window == window && y < bounds.y + bounds.height && bounds.y < y + h
            })
            .flat_map(|(_, cells)| cells.values())
            .fold(Reach::default(), |reach, cell| reach.max(*cell))
    }

    /// SELF with OTHER's cells, the farther reach where both have one.
    pub(crate) fn merge(&mut self, other: Rows) {
        for (row, cells) in other.0 {
            let mine = self.0.entry(row).or_default();
            for (cell, reach) in cells {
                let entry = mine.entry(cell).or_default();
                *entry = entry.max(reach);
            }
        }
    }

    /// Every kept cell, of any window, as its rectangle and reach.
    fn cells(&self) -> impl Iterator<Item = (i64, Rect, Reach)> + '_ {
        self.0.iter().flat_map(|(&(window, y, h), cells)| {
            cells.iter().map(move |(&(x0, x1), &reach)| {
                let (x0, x1) = (f32::from_bits(x0), f32::from_bits(x1));
                (window, Rect::new(x0, f32::from_bits(y), x1 - x0, f32::from_bits(h)), reach)
            })
        })
    }

    /// SELF after the picture's pixels in FROM were copied DY down (up when
    /// negative). A cell whose ink lies wholly where they land is gone: the
    /// copy overwrote all of it. A cell whose ink meets FROM had some of it
    /// copied, so it is kept there too, moved by DY. Every other cell, and
    /// every cell in FROM, keeps its pixels and its record.
    pub(crate) fn moved(&mut self, from: Rect, dy: f32) {
        let to = Rect::new(from.x, from.y + dy, from.width, from.height);
        let carried: Vec<(i64, Rect, Reach)> = self
            .cells()
            .filter(|(_, cell, reach)| meets(reach.grow(*cell), from))
            .map(|(window, cell, reach)| (window, Rect::new(cell.x, cell.y + dy, cell.width, cell.height), reach))
            .collect();
        for (&(_, y, h), cells) in &mut self.0 {
            let (y, h) = (f32::from_bits(y), f32::from_bits(h));
            cells.retain(|&(x0, x1), reach| {
                let (x0, x1) = (f32::from_bits(x0), f32::from_bits(x1));
                !inside(reach.grow(Rect::new(x0, y, x1 - x0, h)), to)
            });
        }
        self.0.retain(|_, cells| !cells.is_empty());
        for (window, cell, reach) in carried {
            self.add(window, cell, reach);
        }
    }
}

/// Whether A and B share an area.
fn meets(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// Whether A lies wholly in B.
fn inside(a: Rect, b: Rect) -> bool {
    b.x <= a.x && b.y <= a.y && a.x + a.width <= b.x + b.width && a.y + a.height <= b.y + b.height
}

/// The picture's reach, as a plan reads it: ALL, the farthest any ink it
/// drew went, for groups it has not drawn; and ROWS, by row of cells, for
/// those it has.
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
    /// What the draw `seen` runs has noted so far, the farthest reach and
    /// by cell; none outside one.
    static SEEN: RefCell<Option<(Reach, Rows)>> = const { RefCell::new(None) };
}

/// Note a glyph of WINDOW drawn as INK in its CELL.
pub(crate) fn note(window: i64, cell: Rect, ink: Rect) {
    let reach = Reach {
        up: (cell.y - ink.y).max(0.0),
        down: (ink.y + ink.height - cell.y - cell.height).max(0.0),
        left: (cell.x - ink.x).max(0.0),
        right: (ink.x + ink.width - cell.x - cell.width).max(0.0),
    };
    SEEN.with_borrow_mut(|seen| {
        if let Some((all, rows)) = seen {
            *all = all.max(reach);
            rows.add(window, cell, reach);
        }
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

/// What DRAW returns, and how far the ink it drew reached past its cells:
/// the farthest, and by cell.
pub(crate) fn seen<T>(draw: impl FnOnce() -> T) -> (T, Reach, Rows) {
    let outer = SEEN.replace(Some((Reach::default(), Rows::default())));
    let out = draw();
    let (reach, rows) = SEEN.replace(outer).unwrap_or_default();
    (out, reach, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_past_its_cell_reaches_by_the_overflow_and_only_while_seen() {
        note(1, Rect::new(0.0, 0.0, 1.0, 1.0), Rect::new(0.0, 0.0, 9.0, 9.0));
        assert!(SEEN.with_borrow(Option::is_none), "a draw outside `seen` keeps nothing");
        let ((), reach, rows) = seen(|| {
            note(1, Rect::new(10.0, 10.0, 7.0, 12.0), Rect::new(11.0, 8.0, 5.0, 16.0));
            note(1, Rect::new(10.0, 10.0, 7.0, 12.0), Rect::new(9.5, 12.0, 9.0, 4.0));
        });
        assert_eq!(
            reach,
            Reach {
                up: 2.0,
                down: 2.0,
                left: 0.5,
                right: 1.5
            }
        );
        assert_eq!(rows.cells().count(), 1, "the note outside `seen` is not kept");
        assert_eq!(seen(|| ()).1, Reach::default());
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
        // A wave two pixels under its cell.
        let ((), reach, _) = seen(|| {
            note_points(1, Rect::new(0.0, 0.0, 10.0, 20.0), [[0.0, 18.0], [10.0, 22.0]].into_iter());
        });
        assert_eq!(
            reach,
            Reach {
                down: 2.0,
                ..Reach::default()
            }
        );
    }

    #[test]
    fn cells_keep_their_reach_until_a_copy_covers_their_ink() {
        let ((), _, mut rows) = seen(|| {
            // Window 1: a row at 0 reaching 3 right, a row at 30 reaching 2
            // left from x 10, and a cell at x 90 of that row reaching 16 right.
            note(1, Rect::new(0.0, 0.0, 7.0, 10.0), Rect::new(0.0, 0.0, 10.0, 10.0));
            note(1, Rect::new(10.0, 30.0, 7.0, 10.0), Rect::new(8.0, 30.0, 9.0, 10.0));
            note(1, Rect::new(90.0, 30.0, 2.0, 10.0), Rect::new(90.0, 30.0, 18.0, 10.0));
            // Window 2, beside: a row at 30 reaching 2 left; none past its cell.
            note(2, Rect::new(120.0, 30.0, 7.0, 10.0), Rect::new(118.0, 30.0, 9.0, 10.0));
            note(2, Rect::new(130.0, 0.0, 7.0, 10.0), Rect::new(130.0, 0.0, 7.0, 10.0));
        });
        let right = Reach { right: 3.0, ..Reach::default() };
        let left = Reach { left: 2.0, ..Reach::default() };
        let far = Reach { right: 16.0, ..Reach::default() };
        assert_eq!(rows.of(1, Rect::new(50.0, 0.0, 5.0, 10.0)), right, "a row's own");
        assert_eq!(rows.of(1, Rect::new(0.0, 10.0, 5.0, 10.0)), Reach::default(), "none drawn there");
        assert_eq!(rows.of(1, Rect::new(0.0, 5.0, 5.0, 30.0)), right.max(left).max(far), "every row it overlaps");
        assert_eq!(rows.of(2, Rect::new(0.0, 0.0, 5.0, 10.0)), Reach::default(), "another window's");
        assert_eq!(rows, Rows::of_cells([
            ((1, Rect::new(0.0, 0.0, 7.0, 10.0)), right),
            ((1, Rect::new(10.0, 30.0, 7.0, 10.0)), left),
            ((1, Rect::new(90.0, 30.0, 2.0, 10.0)), far),
            ((2, Rect::new(120.0, 30.0, 7.0, 10.0)), left),
        ]), "a cell whose ink stayed in it is not kept");
        // The pixels of 0..20 by 0..20 are copied down 30. Window 1's cell
        // at 10, 30 had all its ink there and is gone; its cell at 90 had
        // ink past them and keeps its reach, as does window 2's; the row at
        // 0 reaches there too, and still where its pixels stay.
        rows.moved(Rect::new(0.0, 0.0, 20.0, 20.0), 30.0);
        assert_eq!(rows, Rows::of_cells([
            ((1, Rect::new(0.0, 0.0, 7.0, 10.0)), right),
            ((1, Rect::new(0.0, 30.0, 7.0, 10.0)), right),
            ((1, Rect::new(90.0, 30.0, 2.0, 10.0)), far),
            ((2, Rect::new(120.0, 30.0, 7.0, 10.0)), left),
        ]));
        assert_eq!(rows.of(1, Rect::new(0.0, 30.0, 5.0, 10.0)), right.max(far));
        // A cell whose ink met the copied pixels without lying in them is
        // carried too: some of its ink went with them.
        rows.moved(Rect::new(100.0, 30.0, 10.0, 10.0), -30.0);
        assert_eq!(rows.of(1, Rect::new(90.0, 0.0, 2.0, 10.0)), right.max(far));
    }
}
