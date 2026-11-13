//! The VM's functions, and which of them changed since the last collection.
//!
//! A minor collection reads no function but those: one installed, replaced
//! or changed in place may name what was made since, and every other names
//! only what a collection has promoted. So the table hands out `&mut` to a
//! function through nothing but `set`, `push` and `change`, each of which
//! records the id: a write that records nothing does not compile. The
//! collector's own rewrite (`collector_iter_mut`) changes where values are,
//! not which, and records nothing.
use crate::Function;
use std::sync::Arc;

/// Ids recorded before the record is given up as lost: a minor that finds
/// it lost promotes everything instead.
const CHANGED_LIMIT: usize = 64 << 10;

#[derive(Clone, Debug, Default)]
pub(crate) struct Functions {
    list: Vec<Arc<Function>>,
    /// The ids installed, replaced or changed in place since the last
    /// collection, as they came: one may be there twice.
    changed: Vec<usize>,
    /// More changed than `CHANGED_LIMIT`: the record is void until the next
    /// collection.
    lost: bool,
}
romacs_snapshot::snapshot_struct!(Functions { list, changed, lost });

impl std::ops::Deref for Functions {
    type Target = [Arc<Function>];
    #[inline(always)]
    fn deref(&self) -> &[Arc<Function>] {
        &self.list
    }
}

impl<'a> IntoIterator for &'a Functions {
    type Item = &'a Arc<Function>;
    type IntoIter = std::slice::Iter<'a, Arc<Function>>;
    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

impl Functions {
    fn record(&mut self, id: usize) {
        if self.lost {
            return;
        }
        if self.changed.len() >= CHANGED_LIMIT {
            (self.changed, self.lost) = (Vec::new(), true);
            return;
        }
        self.changed.push(id);
    }
    /// FUNCTION as a new function: its id.
    pub(crate) fn push(&mut self, function: Arc<Function>) -> usize {
        let id = self.list.len();
        self.record(id);
        self.list.push(function);
        id
    }
    /// FUNCTION in place of function ID.
    pub(crate) fn set(&mut self, id: usize, function: Arc<Function>) {
        self.record(id);
        self.list[id] = function;
    }
    /// Function ID, to change in place.
    pub(crate) fn change(&mut self, id: usize) -> &mut Arc<Function> {
        self.record(id);
        &mut self.list[id]
    }
    /// No function at all: a test's.
    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.list.clear();
        self.settle();
    }
    /// The ids changed since the last collection, or none if the record was
    /// lost.
    pub(crate) fn changed(&self) -> Option<&[usize]> {
        (!self.lost).then_some(&self.changed)
    }
    /// A collection ended: every function's values are old.
    pub(crate) fn settle(&mut self) {
        (self.changed, self.lost) = (Vec::new(), false);
    }
    /// Whether nothing is recorded: what a completed collection leaves.
    #[cfg(test)]
    pub(crate) fn settled(&self) -> bool {
        self.changed.is_empty() && !self.lost
    }
    /// Every function, for the collector to rewrite the values of or drop
    /// the body of: it records nothing.
    pub(crate) fn collector_iter_mut(&mut self) -> std::slice::IterMut<'_, Arc<Function>> {
        self.list.iter_mut()
    }
}
