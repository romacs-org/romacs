//! The mutator's context: the state one running Lisp thread keeps, as GNU's
//! `struct thread_state` does, gathered off `Vm` into one inline struct
//! (`Vm::mutator`). On one thread this only puts that state in one place;
//! a second mutator (GNU's cooperative threads) would be a second
//! `Mutator`, not a second `Vm`. Fields move here slice by slice; each keeps
//! its collector class (`tests/no_remote_references.rs`) and is carried in
//! a world image as it was on `Vm`.
use crate::{Form, GeneratedName, Let, NameSet, Value, backtrace, regexp, threaded};

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
    // The compile in progress: the loader's and the compiler's state for
    // the forms being compiled now.
    pub(crate) compile_depth: usize,
    pub(crate) pending_specials: NameSet<String>,
    /// Names a one-argument top-level `(defvar NAME)` declared in the file
    /// being loaded: special for the rest of that file only (GNU Fdefvar,
    /// eval.c:990-996, consing onto the `internal-interpreter-environment'
    /// that readevalloop binds once per file). Saved and restored per load.
    pub(crate) file_specials: NameSet<String>,
    /// Set by the loader for exactly one root compilation of a file's form,
    /// so that compilation (and nothing nested in it) starts from
    /// `file_specials`.
    pub(crate) seed_file_specials: bool,
    /// The next built function is a loading file's form (`Builder::file_level`):
    /// set by the loader around its top-level compile, and by a deferred
    /// form's thunk made in one.
    pub(crate) file_level_request: bool,
    /// Loading a file GNU would read as source (no compiled form,
    /// `Vm::gnu_loads_compiled'): GNU's readevalloop interprets its forms,
    /// so its `let's bind late (`Op::BindLate'), as `eval''s do.
    pub(crate) interpreting_source: bool,
    /// R215: compile a failing form as a deferred reach-time expansion.
    pub(crate) defer_expansion_errors: bool,
    /// Compiling for `eval', which GNU interprets: a macro call whose
    /// expansion escapes, or a call whose function a form run before it may
    /// still define, defers to its reach (`defer_expansion_errors'), and a
    /// `lambda' with nothing to capture compiles at its first call.
    pub(crate) evaluating: bool,
    /// The next form compiled is a deferred call reached (`force_lazy'): its
    /// own expansion signals there, the forms inside it defer as ever.
    pub(crate) strict_root: bool,
    /// The head of the deferred call being reached, while it compiles: with
    /// no function, that call compiles to signal void-function at once, not
    /// to defer again. Consumed by the call's own compilation.
    pub(crate) reached_head: Option<String>,
    /// As `reached_head', for a head no name reaches.
    pub(crate) reached_generated: Option<GeneratedName>,
    /// Set for the builder about to be created (`Builder::loader_order`).
    pub(crate) loader_order_request: bool,
    /// Consumed by the next `function` call: keep every capture register.
    pub(crate) keep_captures: bool,
    /// Consumed by the next lambda compiled: the closure slots `closure`
    /// takes out of its body (an interactive spec and its modes), which its
    /// structural key covers as GNU's `equal` compares them (`closure_key`).
    pub(crate) closure_key_extra: Vec<Form>,
    /// Eager `lambda' bodies being compiled around the current form.
    pub(crate) lambda_depth: usize,
    pub(crate) expanded_symbols: NameSet<String>,
    // The bindings, handlers, activations' record and roots of the Lisp
    // this mutator runs, and the escapes leaving it (GNU's specpdl and
    // handler list).
    pub(crate) expanding: Vec<usize>,
    /// GNU `eval` with a nil LEXICAL argument: every binding the form makes
    /// is dynamic (src/eval.c Feval, `internal-interpreter-environment' nil).
    pub(crate) dynamic_scope: bool,
    /// An escape leaving a reentrant `eval`'s engine, re-raised by the caller's
    /// engine. Traced by the collector while present.
    pub(crate) propagated_escape: Option<threaded::Escape>,
    pub(crate) signal_handlers: Vec<threaded::SignalHandler>,
    pub(crate) backtrace: backtrace::Backtrace,
    /// Variables whose watchers run now, each trapped again as its call ends
    /// (`Vm::notify_watchers_at`), or as a panic is caught (`Vm::catching`).
    pub(crate) retrap: Vec<usize>,
    /// R215: compile `lambda' bodies lazily (GNU's non-eager load regime).
    pub(crate) lazy_regime: bool,
    /// The error objects (CONDITION . DATA) of the `no-catch'es throws that
    /// found no live catch made, while those errors escape, the newest last:
    /// an error with one of these very objects, made afresh for each signal,
    /// is that throw, not one Lisp signalled (`defer_escape' holds it again
    /// as the throw). Each retires when its error is caught; they are weak
    /// (gc.rs), so one whose error a Rust caller discards goes at the next
    /// collection and keeps nothing alive.
    pub(crate) thrown_no_catch: Vec<Value>,
    /// Open frames that keep heap Values in untraced Rust storage across a
    /// call that can run Lisp: compiler builders and watcher notifications
    /// from native writers (`gc.rs`, `Vm::may_collect`).
    pub(crate) gc_holds: usize,
    pub(crate) external_roots: Vec<Value>,
    /// The `let` bindings in effect: (variable, saved value, buffer current
    /// when bound, kind).
    pub(crate) dynamic: Vec<(usize, Value, Value, Let)>,
}
