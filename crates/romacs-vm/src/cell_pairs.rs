//! A capture cell's lexical pair, both ways. A closure environment Lisp
//! reads (`aref` 2) or hands back (`make-interpreted-closure`, as nadvice and
//! every `add-function` do) names a captured variable by its `(NAME . VALUE)`
//! cons; the cell and the pair then share the value, the pair being the one
//! the collector reads (`gc.rs` `settle_cell_pairs`). Lisp gives the pair, so
//! finding its cell is by pair: an index, not a search of every pair made
//! since the last collection (romacs#367).
//!
//! The pair is unique to its cell and the cell to its pair: a pair is made
//! for a cell that has none (`lexical_pair`), and a cell for a pair that has
//! none. Both maps change together, here alone.
use crate::{HashMap, Value};

#[derive(Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct CellPairs {
    by_cell: HashMap<usize, Value>,
    /// By the pair's bits, which are what `Value`'s equality compares (it has
    /// no `Hash`). Bits are stable as the heap's indices are: only a
    /// compaction moves a pair, through `rewrite`; an image carries the
    /// tables dense and unmoved, so both maps restore as they were.
    by_pair: HashMap<u64, usize>,
}

impl CellPairs {
    pub(crate) fn get(&self, cell: usize) -> Option<Value> {
        self.by_cell.get(&cell).copied()
    }
    /// The cell PAIR is the lexical pair of.
    pub(crate) fn cell_of(&self, pair: Value) -> Option<usize> {
        self.by_pair.get(&pair.0).copied()
    }
    /// PAIR becomes CELL's, neither having been paired: two cells sharing a
    /// pair, or a cell two, would capture the same variable apart.
    pub(crate) fn insert(&mut self, cell: usize, pair: Value) {
        // Before either map changes: a caught panic leaves them one relation.
        assert!(
            !self.by_cell.contains_key(&cell) && !self.by_pair.contains_key(&pair.0),
            "cell {cell} or its new lexical pair was already paired"
        );
        self.by_cell.insert(cell, pair);
        self.by_pair.insert(pair.0, cell);
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (usize, Value)> + '_ {
        self.by_cell.iter().map(|(&cell, &pair)| (cell, pair))
    }
    pub(crate) fn pairs(&self) -> impl Iterator<Item = Value> + '_ {
        self.by_cell.values().copied()
    }
    /// Keep the cells KEEP takes: a sweep's survivors.
    pub(crate) fn retain(&mut self, keep: impl Fn(usize) -> bool) {
        self.by_cell.retain(|&cell, _| keep(cell));
        self.by_pair.retain(|_, cell| keep(*cell));
    }
    /// Each entry as MOVE gives it, or gone: a compaction's rewrite.
    pub(crate) fn rewrite(&mut self, mut moved: impl FnMut(usize, Value) -> Option<(usize, Value)>) {
        let by_cell: HashMap<usize, Value> = std::mem::take(&mut self.by_cell)
            .into_iter()
            .filter_map(|(cell, pair)| moved(cell, pair))
            .collect();
        self.by_pair = by_cell.iter().map(|(&cell, pair)| (pair.0, cell)).collect();
        self.by_cell = by_cell;
    }
    /// Under verify: the two maps are one relation, and every entry names a
    /// cell CELL_LIVE takes and a pair PAIR_LIVE takes.
    pub(crate) fn check(&self, cell_live: impl Fn(usize) -> bool, pair_live: impl Fn(Value) -> bool) {
        assert_eq!(self.by_cell.len(), self.by_pair.len(), "lexical pairs: the maps differ in size");
        for (&cell, &pair) in &self.by_cell {
            assert_eq!(self.by_pair.get(&pair.0), Some(&cell), "lexical pair of cell {cell} is not indexed by its pair");
            assert!(cell_live(cell), "lexical pair names freed cell {cell}");
            assert!(pair_live(pair), "cell {cell}'s lexical pair is not a live cons");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CellPairs;
    use crate::Value;

    fn pair(i: usize) -> Value {
        Value::reference(i, 3)
    }

    #[test]
    fn insert_indexes_both_ways() {
        let mut pairs = CellPairs::default();
        pairs.insert(1, pair(10));
        pairs.insert(2, pair(20));
        assert_eq!((pairs.cell_of(pair(10)), pairs.get(2), pairs.cell_of(pair(30))), (Some(1), Some(pair(20)), None));
        pairs.check(|_| true, |_| true);
    }

    #[test]
    #[should_panic(expected = "already paired")]
    fn a_pair_has_one_cell() {
        let mut pairs = CellPairs::default();
        pairs.insert(1, pair(10));
        pairs.insert(2, pair(10));
    }

    #[test]
    fn retain_and_rewrite_move_both_ways() {
        let mut pairs = CellPairs::default();
        for i in 0..4 {
            pairs.insert(i, pair(10 + i));
        }
        pairs.retain(|cell| cell % 2 == 0);
        assert_eq!((pairs.cell_of(pair(11)), pairs.cell_of(pair(12))), (None, Some(2)));
        pairs.rewrite(|cell, p| (cell != 0).then(|| (cell + 100, pair(p.payload() as usize + 100))));
        assert_eq!((pairs.cell_of(pair(112)), pairs.get(102), pairs.iter().count()), (Some(102), Some(pair(112)), 1));
        pairs.check(|_| true, |_| true);
    }
}
