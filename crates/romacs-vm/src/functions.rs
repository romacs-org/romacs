//! The VM's functions, and which of them changed since the last collection.
//!
//! A minor collection reads no function but those: one installed, replaced
//! or changed in place may name what was made since, and every other names
//! only what a collection has promoted. So the table hands out `&mut` to a
//! function through nothing but `set`, `push` and `change`, each of which
//! records the id, once however often it changes: a minor reads each
//! function's code, in every suspended engine, once per id recorded. A write
//! that records nothing does not compile. The collector's own rewrite
//! (`collector_iter_mut`) changes where values are, not which, and records
//! nothing.
use crate::Function;
use std::sync::Arc;

/// Ids recorded before the record is given up as lost: a minor that finds
/// it lost promotes everything instead.
const CHANGED_LIMIT: usize = 64 << 10;

/// An id's place in the restored image (`Functions::image`): not the image's,
/// free there, or one of its functions, as it was then or changed since.
const NOT_IMAGE: u8 = 0;
const UNTOUCHED: u8 = 1;
const TOUCHED: u8 = 2;

#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct Functions {
    list: Vec<Arc<Function>>,
    /// The ids installed, replaced or changed in place since the last
    /// collection, each once, in the order they first changed.
    changed: Vec<usize>,
    /// By id, whether it is in `changed`.
    recorded: Vec<bool>,
    /// By id, the functions of the restored image (`Vm::make_image_permanent`)
    /// and whether each changed since: never forgotten, so a full reads the
    /// changed and no other image function (`Vm::image_roots`). An id free at
    /// the restore, which a request may take again, is not the image's.
    /// Empty in a process that restored none.
    image: Vec<u8>,
    /// The image's ids changed since the restore, each once.
    touched: Vec<usize>,
    /// More changed than `CHANGED_LIMIT`: the record is void until the next
    /// collection.
    lost: bool,
}

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
        if let Some(kept @ &mut UNTOUCHED) = self.image.get_mut(id) {
            *kept = TOUCHED;
            self.touched.push(id);
        }
        if self.lost || self.recorded.get(id).copied().unwrap_or(false) {
            return;
        }
        if self.changed.len() >= CHANGED_LIMIT {
            (self.changed, self.recorded, self.lost) = (Vec::new(), Vec::new(), true);
            return;
        }
        if self.recorded.len() <= id {
            self.recorded.resize(id + 1, false);
        }
        self.recorded[id] = true;
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
        for &id in &self.changed {
            self.recorded[id] = false;
        }
        (self.changed, self.lost) = (Vec::new(), false);
    }
    /// Whether nothing is recorded: what a completed collection leaves.
    #[cfg(test)]
    pub(crate) fn settled(&self) -> bool {
        self.changed.is_empty() && !self.lost
    }
    /// Every function there is but the FREE ids becomes the image's, none
    /// changed since.
    pub(crate) fn make_image(&mut self, free: &[usize]) {
        self.image = vec![UNTOUCHED; self.list.len()];
        for &id in free {
            if let Some(kept) = self.image.get_mut(id) { *kept = NOT_IMAGE; }
        }
        self.touched.clear();
    }
    /// The image's functions changed since the restore.
    pub(crate) fn image_touched(&self) -> &[usize] {
        &self.touched
    }
    /// The image's functions not changed since the restore: each names
    /// only what the image held then.
    pub(crate) fn image_untouched(&self) -> impl Iterator<Item = usize> + '_ {
        self.image.iter().enumerate().filter(|&(_, &kept)| kept == UNTOUCHED).map(|(id, _)| id)
    }
    /// Every function, for the collector to rewrite the values of or drop
    /// the body of: it records nothing.
    pub(crate) fn collector_iter_mut(&mut self) -> std::slice::IterMut<'_, Arc<Function>> {
        self.list.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An id changed again is recorded once, until a collection settles the
    /// record; then it is recorded afresh.
    #[test]
    fn a_function_changed_again_is_recorded_once() {
        let mut vm = crate::Vm::new();
        vm.eval_source_raw("(defun recorded-once () 1)").unwrap();
        let any = vm.functions.last().expect("a function").clone();
        let mut functions = Functions::default();
        let (a, b) = (functions.push(any.clone()), functions.push(any.clone()));
        for _ in 0..1000 {
            functions.change(b);
            functions.set(a, any.clone());
        }
        assert_eq!(functions.changed(), Some(&[a, b][..]));
        functions.settle();
        assert!(functions.settled());
        functions.change(b);
        assert_eq!(functions.changed(), Some(&[b][..]));
    }

    /// An image function changed since the restore stays so through every
    /// collection's settle; a new one is not the image's, nor one free at
    /// the restore that a request took again.
    #[test]
    fn an_image_function_changed_stays_touched() {
        let mut vm = crate::Vm::new();
        vm.eval_source_raw("(defun touched-once () 1)").unwrap();
        let any = vm.functions.last().expect("a function").clone();
        let mut functions = Functions::default();
        let (a, b, free) = (functions.push(any.clone()), functions.push(any.clone()), functions.push(any.clone()));
        functions.make_image(&[free]);
        let c = functions.push(any.clone());
        functions.change(b);
        functions.change(c);
        functions.set(free, any.clone());
        functions.settle();
        functions.change(b);
        assert_eq!(functions.image_touched(), [b], "neither a new id nor one free at the restore is the image's");
        assert_eq!(functions.image_untouched().collect::<Vec<_>>(), [a]);
    }
}
