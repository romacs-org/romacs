//! How far drawn ink reaches past the cells that own it.
//!
//! A glyph's raster can leave its cell: an ascender or a mark above a row
//! set tighter than its font, a descender below, an italic's overhang. A
//! patch of the picture must then repaint where a changed glyph's ink was
//! and is, and draw every glyph whose ink enters the patch. The text pass
//! notes each quad it builds against its cell (`note`); a picture keeps the
//! largest reach its passes have seen and grows each patch by it.
use std::cell::Cell;

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

thread_local! {
    static SEEN: Cell<Reach> = const { Cell::new(Reach { up: 0.0, down: 0.0, left: 0.0, right: 0.0 }) };
}

/// Note a glyph drawn as INK in its CELL.
pub(crate) fn note(cell: Rect, ink: Rect) {
    let reach = Reach {
        up: (cell.y - ink.y).max(0.0),
        down: (ink.y + ink.height - cell.y - cell.height).max(0.0),
        left: (cell.x - ink.x).max(0.0),
        right: (ink.x + ink.width - cell.x - cell.width).max(0.0),
    };
    SEEN.set(SEEN.get().max(reach));
}

/// Note POINTS, the corners of what was drawn for CELL.
pub(crate) fn note_points(cell: Rect, points: impl Iterator<Item = [f32; 2]>) {
    let (mut x0, mut y0, mut x1, mut y1) = (cell.x, cell.y, cell.x + cell.width, cell.y + cell.height);
    for [x, y] in points {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    note(cell, Rect::new(x0, y0, x1 - x0, y1 - y0));
}

/// The reach noted on this thread since the last take.
pub(crate) fn take() -> Reach {
    SEEN.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_past_its_cell_reaches_by_the_overflow_and_a_take_starts_over() {
        take();
        note(Rect::new(10.0, 10.0, 7.0, 12.0), Rect::new(11.0, 8.0, 5.0, 16.0));
        note(Rect::new(10.0, 10.0, 7.0, 12.0), Rect::new(9.5, 12.0, 9.0, 4.0));
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
        // A wave two pixels under its cell.
        note_points(
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
}
