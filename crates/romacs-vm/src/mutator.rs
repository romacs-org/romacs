//! The mutator's context: the state one running Lisp thread keeps, as GNU's
//! `struct thread_state` does, gathered off `Vm` into one inline struct
//! (`Vm::mutator`). On one thread this only puts that state in one place;
//! a second mutator (GNU's cooperative threads) would be a second
//! `Mutator`, not a second `Vm`. Fields move here slice by slice; each keeps
//! its collector class (`tests/no_remote_references.rs`) and is carried in
//! a world image as it was on `Vm`.
use crate::{Form, GeneratedName, Let, NameSet, Value, backtrace, gap_probe, quit, regexp, registers, threaded, watchers};

/// What one mutator is in the middle of. Reached by field path
/// (`vm.mutator.FIELD`), never through an accessor, so a borrow of one
/// field leaves the rest of `Vm` free.
#[derive(romacs_snapshot::Snapshot)]
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
    // Quitting and the escapes being unwound: the host's `C-g' word, and
    // what routes a quit or an escape through the engines (quit.rs).
    /// A primitive Lisp called runs a native loop that stops consistently at
    /// its polls, so a poll may quit (`Vm::native_poll`, `Leaf::quittable`).
    /// Off in redisplay, which never unwinds half done, and wherever Rust
    /// reaches the same loop without a caller that would unwind. Granted at
    /// a primitive's own entry only, and off in any Lisp or primitive entered
    /// below it (`Vm::outside_quit_scope`).
    pub(crate) native_quit: bool,
    pub(crate) propagated_escape_hooked: bool,
    pub(crate) propagated_eval_depth: Option<usize>,
    /// Set immediately before executing an `eval` form; consumed by the engine.
    pub(crate) propagate_next: bool,
    /// The host's `C-g', shared with its input thread (quit.rs).
    pub(crate) quit: std::sync::Arc<quit::QuitSignal>,
    /// A quit (or `throw-on-input' throw) `maybe_quit' raised from C-g or
    /// `quit-flag' is unwinding: no `catch' or `condition-case' has ended it
    /// since. An unwind-protect cleanup that replaces it keeps it set, so
    /// what leaves an expansion it began in is told apart from an error the
    /// macro raised after catching a quit (`quit_watched').
    pub(crate) quit_unwinding: bool,
    /// The writer notifying watchers is an engine write suspended for it,
    /// whose own Values the write roots (`threaded::watched_write`).
    pub(crate) rooted_write: bool,
    /// A native call's writes its watchers will hear (`watchers.rs`), each with
    /// the buffer current then, until the engine that called it completes it.
    /// Never traced: no collection runs while it holds any (`may_collect`).
    pub(crate) deferred_writes: Vec<(watchers::NativeWrite, Value)>,
    /// Escapes being unwound, in any engine: routing takes it, the driver gives
    /// it back. While nonzero no aftermath starts (`Vm::aftermath_held`).
    pub(crate) unwinding: usize,
    /// Runs of aftermath in progress (`Vm::run_aftermath'): while nonzero an
    /// interrupt naming the job waits (`threaded::isolate_quit'), so
    /// finalizers and `post-gc-hook' never take it.
    pub(crate) aftermath_runs: usize,
    /// Whether this is its process's own mutator, the one that takes the
    /// process's OS signals (`host.rs`'s mailbox: an isolate's interrupt and
    /// termination, `sigusr1'/`sigusr2' events, the call deadline's slot).
    /// True for every `Vm` made today; a second mutator in one process (a
    /// twin, a cooperative thread) is made without it, and reads only its
    /// own `quit` word.
    pub(crate) process_signals: bool,
    // The running engines: their register files and how deep they run.
    pub(crate) execution_fatal: bool,
    /// Active Lisp evaluation depth, shared by reentrant execution engines.
    pub(crate) eval_depth: usize,
    /// Engines between entry and exit, suspended or not; those not in
    /// `suspended` hold untraced registers and code.
    pub(crate) running_engines: usize,
    /// The register files, the running engine's current (`registers.rs`).
    pub(crate) stack: registers::Registers,
}

impl Mutator {
    /// A fresh context: its own `C-g' word, as a hosted VM's
    /// (`gap_probe::as_hosted`), its process's OS signals its own, an empty
    /// register file (`Registers::new`), and every other field empty.
    pub(crate) fn new() -> Self {
        Mutator {
            regexp: Default::default(),
            resolved_buffer: Default::default(),
            change_prepared: Default::default(),
            keymap_filters: Default::default(),
            keymap_pending_filter: Default::default(),
            compile_depth: Default::default(),
            pending_specials: Default::default(),
            file_specials: Default::default(),
            seed_file_specials: Default::default(),
            file_level_request: Default::default(),
            interpreting_source: Default::default(),
            defer_expansion_errors: Default::default(),
            evaluating: Default::default(),
            strict_root: Default::default(),
            reached_head: Default::default(),
            reached_generated: Default::default(),
            loader_order_request: Default::default(),
            keep_captures: Default::default(),
            closure_key_extra: Default::default(),
            lambda_depth: Default::default(),
            expanded_symbols: Default::default(),
            expanding: Default::default(),
            dynamic_scope: Default::default(),
            propagated_escape: Default::default(),
            signal_handlers: Default::default(),
            backtrace: Default::default(),
            retrap: Default::default(),
            lazy_regime: Default::default(),
            thrown_no_catch: Default::default(),
            gc_holds: Default::default(),
            external_roots: Default::default(),
            dynamic: Default::default(),
            native_quit: Default::default(),
            propagated_escape_hooked: Default::default(),
            propagated_eval_depth: Default::default(),
            propagate_next: Default::default(),
            quit: gap_probe::as_hosted(std::sync::Arc::default()),
            quit_unwinding: Default::default(),
            rooted_write: Default::default(),
            deferred_writes: Default::default(),
            unwinding: Default::default(),
            aftermath_runs: Default::default(),
            process_signals: true,
            execution_fatal: Default::default(),
            eval_depth: Default::default(),
            running_engines: Default::default(),
            stack: registers::Registers::new(),
        }
    }
}

/// The part of a mutator's context a world image never holds, renewed at
/// restore (`#[snapshot(at_rest)]` on `Vm::mutator_rest`): what the running
/// engines parked, which holds addresses and lowered code. The derive lets
/// only the image's root have at-rest fields, so it is a struct of its own
/// beside `Mutator`; a second mutator swaps both. Checked idle before a
/// capture (`Vm::world_quiescent`).
#[derive(Default)]
pub(crate) struct MutatorRest {
    /// The instruction and registers of the leaf a site is running with no
    /// activation, as addresses; (0, 0) otherwise. Lisp that leaf reaches
    /// records the activation first (`Vm::record_site_leaf`).
    pub(crate) site_leaf: (usize, usize),
    pub(crate) execution: threaded::Program,
    /// Programs of finished nested engine runs (registers all NIL again,
    /// lowered code kept), reused by the next nested run
    /// (`threaded::Vm::end_nested_execution`). Code invalidation and the
    /// collector reach them as they reach `execution`.
    pub(crate) spare_programs: Vec<threaded::Program>,
    /// Suspended engines' own state, owned here until they resume: traced and
    /// forwarded in place, and dropped with the engine by a caught panic.
    pub(crate) suspended: Vec<threaded::Suspended>,
    /// The compiled regexp patterns this mutator's searches use
    /// (`regexp.rs`): their SIMD searchers hold function pointers, and a
    /// pattern whose syntax table a collection may move goes at every
    /// collection (`forget_table_bound_patterns`).
    pub(crate) regexp_cache: regexp::PatternCache,
}
