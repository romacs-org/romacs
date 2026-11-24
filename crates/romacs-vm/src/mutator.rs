//! The mutator's context: the state one running Lisp thread keeps, as GNU's
//! `struct thread_state` does, gathered off `Vm` into one inline struct
//! (`Vm::mutator`). On one thread this only puts that state in one place;
//! a second mutator (GNU's cooperative threads) would be a second
//! `Mutator`, not a second `Vm`. Fields move here slice by slice; each keeps
//! its collector class (`tests/no_remote_references.rs`) and is carried in
//! a world image as it was on `Vm`.
use crate::{Value, regexp};

/// What one mutator is in the middle of. Reached by field path
/// (`vm.mutator.FIELD`), never through an accessor, so a borrow of one
/// field leaves the rest of `Vm` free.
#[derive(Default, romacs_snapshot::Snapshot)]
pub(crate) struct Mutator {
    /// search.c's match data (`m_search_regs`): the last successful match.
    pub(crate) regexp: regexp::MatchState,
    /// The current buffer as `buffer_index` last resolved it: the slot's
    /// value then, a live buffer, and its object index. Kept by value, not
    /// by writer: `let` and its unwinding write the slot like any special
    /// variable, and a different value simply resolves again. Killing a
    /// buffer and a collection (which may move buffers) drop it.
    pub(crate) resolved_buffer: Option<(Value, usize)>,
    /// A storage kernel runs inside insdel's bracket, which checked read-only.
    pub(crate) change_prepared: bool,
    /// Menu-item `:filter' results for the keymap walk in progress, as
    /// (menu item, result); set and cleared around one walk, which never
    /// collects.
    pub(crate) keymap_filters: Vec<(Value, Value)>,
    /// The first filter that walk needed: (menu item, filter, definition).
    pub(crate) keymap_pending_filter: Option<(Value, Value, Value)>,
}
