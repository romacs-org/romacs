//! The register files and their one owner (lisp-speed.md §13.3, §13.6 I6):
//! the running engine's, those set aside beneath nested runs, and the file
//! natives borrow for their arguments. Engines and their frames hold raw
//! pointers into these allocations, so nothing outside this module can
//! reallocate, drop or swap one: the fields are private, there is no
//! `Default`, and the current file is reached only as a slice (`Deref`),
//! whose shape cannot change. Every allocation change is a method here.
use crate::Value;
use std::ops::{Deref, DerefMut};

/// A new register file's slots.
pub(crate) const FILE_START: usize = 4096;
/// A file longer than this is cut back to `FILE_START` when its engine ends.
pub(crate) const SPARE_CAP: usize = 131_072;

pub(crate) struct Registers {
    /// The running engine's file, or the kernel's while a native borrows it;
    /// empty outside any engine.
    current: Vec<Value>,
    /// Files of engines suspended across a nested run, traced and forwarded
    /// in place: their frames keep raw pointers into them.
    suspended: Vec<Vec<Value>>,
    /// The file natives borrow for their arguments, reused across calls
    /// (`enter_kernel`): empty between them.
    kernel: Vec<Value>,
}

impl Deref for Registers {
    type Target = [Value];
    fn deref(&self) -> &[Value] {
        &self.current
    }
}
impl DerefMut for Registers {
    fn deref_mut(&mut self) -> &mut [Value] {
        &mut self.current
    }
}

impl Registers {
    /// No file at all. Only `Vm::new` makes one, and the engine's modules may
    /// not (`tools/lib/smells.py`, `vm-replacement`): a second owner would
    /// let safe code swap a running engine's file away.
    pub(crate) const fn new() -> Self {
        Registers { current: Vec::new(), suspended: Vec::new(), kernel: Vec::new() }
    }
    /// How many files are set aside beneath the current one.
    pub(crate) fn depth(&self) -> usize {
        self.suspended.len()
    }
    /// No engine runs and none is set aside.
    pub(crate) fn at_rest(&self) -> bool {
        self.current.is_empty() && self.suspended.is_empty()
    }
    /// File I, in file order: the suspended ones, then the current one.
    pub(crate) fn file(&self, i: usize) -> &[Value] {
        self.suspended.get(i).unwrap_or(&self.current)
    }
    /// File I, writable in place.
    pub(crate) fn file_mut(&mut self, i: usize) -> &mut [Value] {
        self.suspended.get_mut(i).unwrap_or(&mut self.current)
    }
    /// Every file, in file order.
    pub(crate) fn files(&self) -> impl Iterator<Item = &[Value]> {
        self.suspended.iter().chain(std::iter::once(&self.current)).map(Vec::as_slice)
    }
    /// Every file, writable in place.
    pub(crate) fn files_mut(&mut self) -> impl Iterator<Item = &mut [Value]> {
        self.suspended.iter_mut().chain(std::iter::once(&mut self.current)).map(Vec::as_mut_slice)
    }
    /// Set the current file aside, still traced, its allocation unmoved for
    /// the frames' raw pointers: an engine's for a nested run, or for a
    /// storage kernel's.
    pub(crate) fn suspend(&mut self) {
        let file = std::mem::take(&mut self.current);
        self.suspended.push(file);
    }
    /// Take back the file `suspend` set aside. The nested run has returned
    /// its own: overwriting a file would free its engine's.
    pub(crate) fn resume(&mut self) {
        assert!(self.current.is_empty(), "a nested run returned its registers");
        self.current = self.suspended.pop().expect("a suspended register stack");
    }
    /// Make ARGS the current file's first slots, in the reused kernel file,
    /// with the current one set aside beneath it (`leave_kernel` undoes it).
    pub(crate) fn enter_kernel(&mut self, args: &[Value]) {
        self.suspend();
        self.current = std::mem::take(&mut self.kernel);
        self.current.clear();
        self.current.extend_from_slice(args);
    }
    /// Return the kernel file, emptied: untraced between calls, nothing it
    /// held may stay in it.
    pub(crate) fn leave_kernel(&mut self) {
        self.current.clear();
        self.kernel = std::mem::take(&mut self.current);
        self.resume();
    }
    /// An engine's entry: FILE (a spare, or empty) becomes the current file,
    /// at least SIZE slots and all NIL past ARGS, which fill its first slots.
    /// The current file must be empty, as its caller set it aside: taking a
    /// file over it would free a running engine's.
    pub(crate) fn enter(&mut self, file: Vec<Value>, size: usize, args: &[Value]) {
        assert!(self.current.is_empty(), "a nested run's caller set its registers aside");
        self.current = file;
        // A spare file comes back all NIL, as long as its last run grew it.
        if self.current.len() < size.max(FILE_START) {
            self.current.resize(size.max(FILE_START), Value::NIL);
        }
        self.current[..args.len()].copy_from_slice(args);
    }
    /// An engine's end: clear the USED slots it wrote and give its file back
    /// for the next run. A file a deep run grew is kept, up to a cap; past
    /// it, cut back, not dropped: an empty file means "no program" to the
    /// spares.
    pub(crate) fn leave(&mut self, used: usize) -> Vec<Value> {
        self.current[..used].fill(Value::NIL);
        if self.current.len() > SPARE_CAP {
            self.current.truncate(FILE_START);
            self.current.shrink_to_fit();
        }
        std::mem::take(&mut self.current)
    }
    /// Grow the current file to SIZE slots, NIL-filled. It may move: every
    /// pointer into it must be rebased (`threaded::grow_registers`).
    pub(crate) fn grow(&mut self, size: usize) {
        self.current.resize(size, Value::NIL);
    }
    /// Outside any engine, after a caught panic: every file but the kernel's
    /// is dropped with the engines that pointed into them.
    pub(crate) fn reset(&mut self) {
        self.suspended.clear();
        self.current = Vec::new();
    }
    /// The current file as a `Vec`, for tests that stage arguments by hand.
    #[cfg(test)]
    pub(crate) fn test_vec(&mut self) -> &mut Vec<Value> {
        &mut self.current
    }
}
romacs_snapshot::snapshot_struct!(Registers { current, suspended, kernel });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "a nested run returned its registers")]
    fn resuming_over_a_live_file_panics() {
        let mut registers = Registers::new();
        registers.suspend();
        registers.test_vec().push(Value::NIL);
        registers.resume();
    }

    #[test]
    #[should_panic(expected = "a nested run's caller set its registers aside")]
    fn entering_over_a_live_file_panics() {
        let mut registers = Registers::new();
        registers.test_vec().push(Value::NIL);
        registers.enter(Vec::new(), 1, &[]);
    }

    /// A set-aside file keeps its allocation beneath a kernel's and a nested
    /// engine's, and comes back in file order.
    #[test]
    fn a_suspended_file_stays_where_it_is() {
        let mut registers = Registers::new();
        registers.enter(Vec::new(), 1, &[Value::TRUE]);
        let base = registers.as_ptr();
        assert_eq!(registers.len(), FILE_START);
        registers.enter_kernel(&[Value::NIL, Value::TRUE]);
        assert_eq!((registers.depth(), &registers[..]), (1, &[Value::NIL, Value::TRUE][..]));
        assert_eq!(registers.file(0).as_ptr(), base);
        registers.leave_kernel();
        registers.suspend();
        registers.enter(Vec::new(), 1, &[]);
        assert_eq!(registers.files().map(<[Value]>::as_ptr).next(), Some(base));
        let spare = registers.leave(0);
        assert_eq!(spare.len(), FILE_START);
        registers.resume();
        assert_eq!((registers.as_ptr(), registers[0]), (base, Value::TRUE));
        let file = registers.leave(1);
        assert!(file.iter().all(|&v| v == Value::NIL) && registers.at_rest());
    }
}
