//! Debug keeps the edit loop short; release always runs the original budgets.
pub(super) const fn iterations(debug: usize, full: usize) -> usize {
    if cfg!(debug_assertions) { debug } else { full }
}
