//! Object extents retained from an accepted glyph matrix, separate from
//! physical row/cursor/hit rectangles. Immutable numeric data; concurrent
//! readers need no mutator or Lisp-state cache.

use crate::glyph_matrix::{
    FrameDisplayState, Glyph, GlyphArea, GlyphAreaPlacement, GlyphMatrix, GlyphRowAreaLayout,
    GlyphType, WindowMatrixEntry,
};

/// Source role of one point rectangle. This byte-sized numeric role survives
/// compact row storage and transport; it carries no Lisp identity or cache.
/// Copied roles are immutable numeric data and may be read by independent mutators.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PosnPointRole {
    #[default]
    Glyph,
    OverlaidMarker,
    InsertionBoundary,
    SyntheticBoundary,
}

impl PosnPointRole {
    pub const fn is_position(self) -> bool {
        !matches!(self, Self::OverlaidMarker)
    }

    pub const fn is_default(&self) -> bool {
        matches!(self, Self::Glyph)
    }
}

/// GNU distinguishes an enabled stored glyph, the enabled row after its used
/// count, and a disabled/missing row. Never infer this from physical width.
/// Independent mutators may copy this immutable numeric observation without sharing Lisp state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PosnObjectExtent {
    Undrawn,
    Glyph { width: i64, height: i64 },
    RowFallback { height: i64 },
    Specialized { width: i64, height: i64 },
}

impl PosnObjectExtent {
    pub const fn dimensions(self) -> (i64, i64) {
        match self {
            Self::Undrawn => (0, 0),
            Self::Glyph { width, height } | Self::Specialized { width, height } => (width, height),
            Self::RowFallback { height } => (0, height),
        }
    }
}

/// Plain numeric provenance of one enabled/disabled TTY matrix row. Areas own
/// the actual materialized terminal cells, including nil-object line-end
/// spaces and marker cells. Row height is independent of glyph object height.
/// One exclusive producer initializes each owned row; readers share it only through
/// an immutable retained snapshot, with no Lisp state or shared mutable row cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PosnMatrixRow {
    pub enabled: bool,
    pub y: i64,
    pub height: i64,
    pub areas: [usize; GlyphArea::COUNT],
}

/// Immutable current-matrix facts published only when the enclosing window
/// snapshot is accepted. A canonical query may build the same numeric data,
/// but consumers must consult the retained accepted snapshot, never promote a
/// query's matrix. All fields are numbers; independent mutators/readers may
/// share a fully initialized snapshot through Arc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PosnMatrixSnapshot {
    pub rows: Vec<PosnMatrixRow>,
}

impl PosnMatrixSnapshot {
    /// GNU terminal append_glyph stores each materialized terminal cell as a
    /// glyph with pixel_width=1 and zero ascent/descent (term.c:1577–1608).
    /// Count terminal emitted cells rather than source-identity slots: bare
    /// zero-width characters and contextual/automatic-composition padding
    /// have different ownership in the TTY adapter. The caller supplies its
    /// immutable GNU character-width table; no Lisp state is retained.
    pub(crate) fn from_terminal_matrix(
        matrix: &GlyphMatrix,
        character_width: impl Fn(char) -> usize + Copy,
    ) -> Self {
        Self {
            rows: matrix
                .rows
                .iter()
                .map(|row| PosnMatrixRow {
                    enabled: row.enabled,
                    y: row.pixel_y.round() as i64,
                    height: row.height_px.round().max(1.0) as i64,
                    areas: std::array::from_fn(|area| {
                        terminal_area_used_cells(&row.glyphs[area], character_width)
                    }),
                })
                .collect(),
        }
    }

    pub fn at(&self, row: i64, column: i64, area: GlyphArea) -> PosnObjectExtent {
        let Some(row) = usize::try_from(row).ok().and_then(|row| self.rows.get(row)) else {
            return PosnObjectExtent::Undrawn;
        };
        if !row.enabled {
            return PosnObjectExtent::Undrawn;
        }
        if usize::try_from(column)
            .ok()
            .is_some_and(|column| column < row.areas[area.index()])
        {
            PosnObjectExtent::Glyph {
                width: 1,
                height: 0,
            }
        } else {
            PosnObjectExtent::RowFallback { height: row.height }
        }
    }

    pub(crate) fn at_y(&self, y: i64, column: i64, area: GlyphArea) -> PosnObjectExtent {
        // GNU marginal_area_string stops at the first disabled current row;
        // it does not skip holes to reach a later enabled mode-line slot.
        let Some((index, _)) = self
            .rows
            .iter()
            .enumerate()
            .take_while(|(_, row)| row.enabled)
            .find(|(_, row)| y >= row.y && y < row.y.saturating_add(row.height))
        else {
            return PosnObjectExtent::Undrawn;
        };
        self.at(index as i64, column, area)
    }
}

/// Number of terminal cells emitted by the TTY adapter's glyph-area loop
/// before its TEXT tail. Source-position padding consumed by a composite does
/// not produce a second cell; ordinary blank wide padding does. Numeric only;
/// the supplied character-width function is consulted synchronously, never
/// cached. This follows TtyRif::rasterize_glyph_row's closed glyph variants.
pub(crate) fn terminal_area_used_cells(
    glyphs: &[Glyph],
    character_width: impl Fn(char) -> usize,
) -> usize {
    fn run_member_padding(glyph: &Glyph) -> bool {
        glyph.padding
            && match &glyph.glyph_type {
                GlyphType::Char { ch } => *ch != ' ',
                GlyphType::Composite { .. } | GlyphType::AutomaticComposite { .. } => true,
                _ => false,
            }
    }
    let mut index = 0;
    let mut used = 0usize;
    while let Some(glyph) = glyphs.get(index) {
        let mut consumed = 0;
        let cells = if glyph.padding {
            1
        } else {
            match &glyph.glyph_type {
                GlyphType::AutomaticComposite { text, terminal } => {
                    consumed = glyphs[index + 1..]
                        .iter()
                        .take(text.chars().count().saturating_sub(1))
                        .take_while(|glyph| run_member_padding(glyph))
                        .count();
                    terminal.cells.iter().fold(0usize, |sum, cell| {
                        sum.saturating_add(usize::from(cell.width_cols.max(1)))
                    })
                }
                GlyphType::Composite { .. } => {
                    consumed = glyphs[index + 1..]
                        .iter()
                        .take_while(|glyph| run_member_padding(glyph))
                        .count();
                    1usize.saturating_add(consumed)
                }
                GlyphType::Stretch { width_cols } | GlyphType::Surface { width_cols, .. } => {
                    usize::from((*width_cols).max(1))
                }
                GlyphType::Glyphless { .. } => 1,
                GlyphType::Char { ch } if character_width(*ch) == 0 => 0,
                _ if glyph.wide && !glyphs.get(index + 1).is_some_and(|next| next.padding) => 2,
                _ => 1,
            }
        };
        used = used.saturating_add(cells);
        index += 1 + consumed;
    }
    used
}

#[cfg(test)]
#[path = "tests/posn_object_extent_test.rs"]
mod tests;
