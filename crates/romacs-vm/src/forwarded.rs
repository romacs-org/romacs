//! GNU's built-in variables: symbols FORWARDED to runtime storage.
//!
//! GNU defines 948 variables in C (`DEFVAR_LISP`, `DEFVAR_BOOL`, `DEFVAR_INT`,
//! `DEFVAR_PER_BUFFER`). Such a symbol's redirect is `SYMBOL_FORWARDED`
//! (lisp.h): its value cell holds no Lisp value but a `Lisp_Fwd` naming C
//! storage of one of four kinds, and every variable operation dispatches on it:
//!
//! | GNU (data.c / eval.c)              | VM2                              |
//! |------------------------------------|----------------------------------|
//! | `do_symval_forwarding`             | [`Vm::forwarded_value`]          |
//! | `store_symval_forwarding`          | `Vm::store_forwarded`            |
//! | `set_internal`, FORWARDED arm      | [`Vm::set_forwarded`]            |
//! | `default_value`, FORWARDED arm     | [`Vm::forwarded_default`]        |
//! | `set_default_internal`, FORWARDED  | [`Vm::set_forwarded_default`]    |
//! | `specbind` + `do_specbind`         | [`Vm::bind_forwarded`]           |
//! | `do_one_unbind` (LET/DEFAULT/LOCAL)| [`Vm::unbind_forwarded`]         |
//!
//! VM2's redirect is [`VariableRedirect::Forwarded`], resolved lazily from the
//! symbol NAME like the other redirects, so a code image (which rebuilds slots
//! by name) acquires it too, and an uninterned symbol of the same name never
//! does. The symbol's own value cell stays UNBOUND and is never read.
//!
//! Storage, the counterpart of GNU's C globals and `buffer_defaults`, is
//! [`Forwarded`]: objects (moving heap Values), booleans, integers, and
//! per-buffer defaults (moving heap Values). Per-buffer *values* live in the
//! buffer objects themselves (`PerBuffer`), as in GNU's `struct buffer`.
//!
//! COLLECTOR CONTRACT. `Forwarded::objects` and `Forwarded::defaults` are
//! roots outside every value cell: `gc.rs` marks them with the slots and
//! rewrites them with the slots. A buffer's per-buffer values are marked and
//! rewritten by that buffer's `visit_values`, so they live exactly as long as
//! the buffer. Nothing here retains a borrowed `&Value` across allocation:
//! every accessor copies the Value out. An initial value that needs the heap
//! (a keymap, a string, a list) is materialised on first access, so a bare
//! `Vm::new()` still allocates nothing; UNBOUND marks "not yet materialised"
//! and is never a stored value (`set_internal` refuses to store it).
use super::*;
use crate::prim::Fault;

/// GNU `Lisp_Fwd` kinds (lisp.h). The payload indexes the matching vector
/// of [`Forwarded`]. `Lisp_Fwd_Kboard_Obj` is not modelled (no variable the
/// chain or census reaches needs it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fwd {
    Obj(u16),
    Bool(u16),
    Int(u16),
    Buffer(u16),
}

/// GNU `enum Lisp_Fwd_Predicate`, checked by `store_symval_forwarding` for a
/// non-nil value stored into a per-buffer variable. Only the predicates of
/// variables VM2 defines are present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Predicate {
    Integerp,
    Stringp,
    Symbolp,
}

/// A C initial value. Heap shapes are built fresh on first access.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Init {
    Nil,
    True,
    Fixnum(i64),
    Symbol(&'static str),
    Str(&'static str),
    SystemName,
    SourceDirectory,
    /// A proper list of the given elements.
    List(&'static [Init]),
    Cons(&'static Init, &'static Init),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    /// `DEFVAR_LISP`.
    Lisp(Init),
    /// `DEFVAR_BOOL`.
    Bool(bool),
    /// `DEFVAR_INT` (an `intmax_t` in GNU; VM2 has fixnums only).
    Int(i64),
    /// `DEFVAR_PER_BUFFER` with a positive `PER_BUFFER_IDX`: local in a
    /// buffer only once set there.
    PerBuffer { default: Init, predicate: Option<Predicate> },
}

pub(crate) struct CVariable {
    pub name: &'static str,
    /// GNU 32.0.50 source of the DEFVAR (provenance; checked by the tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub gnu: &'static str,
    pub kind: Kind,
}

/// Every built-in variable VM2 defines, and why. GNU has 948; VM2 defines
/// only those the bootstrap chain, the census or the variable harness
/// actually reached (ruling R216: no speculative ports).
pub(crate) static C_VARIABLES: &[CVariable] = &[
    // cl-generic's dispatcher compiler checks whether GNU is being dumped.
    CVariable { name: "purify-flag", gnu: "alloc.c:7487", kind: Kind::Lisp(Init::Nil) },
    // GNU warning text expands command keys while compiling obsolete method calls.
    CVariable { name: "overriding-local-map", gnu: "keyboard.c:14137", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "tab-width", gnu: "buffer.c:5284", kind: Kind::PerBuffer { default: Init::Fixnum(8), predicate: Some(Predicate::Integerp) } },
    CVariable { name: "noninteractive", gnu: "emacs.c:3534", kind: Kind::Bool(false) },
    CVariable { name: "auto-window-vscroll", gnu: "window.c:9296", kind: Kind::Bool(true) },
    CVariable { name: "inhibit-point-motion-hooks", gnu: "textprop.c:2412", kind: Kind::Lisp(Init::True) },
    CVariable { name: "selective-display", gnu: "buffer.c:5475", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "inhibit-field-text-motion", gnu: "editfns.c:4830", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "buffer-invisibility-spec", gnu: "buffer.c:5849", kind: Kind::PerBuffer { default: Init::True, predicate: None } },
    CVariable { name: "overlay-arrow-variable-list", gnu: "xdisp.c:38716", kind: Kind::Lisp(Init::List(&[Init::Symbol("overlay-arrow-position")])) },
    CVariable { name: "frame-internal-parameters", gnu: "frame.c:7764", kind: Kind::Lisp(Init::List(&[Init::Symbol("undeleted"), Init::Symbol("cloned-from"), Init::Symbol("frame-id"), Init::Symbol("name"), Init::Symbol("parent-id"), Init::Symbol("window-id")])) },
    CVariable { name: "command-error-function", gnu: "keyboard.c:14322", kind: Kind::Lisp(Init::Symbol("command-error-default-function")) },
    CVariable { name: "minor-mode-map-alist", gnu: "keymap.c:3366", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "max-lisp-eval-depth", gnu: "eval.c:4424", kind: Kind::Int(1600) },
    CVariable { name: "lisp-eval-depth-reserve", gnu: "eval.c:4434", kind: Kind::Int(200) },
    CVariable { name: "current-time-list", gnu: "timefns.c:2110", kind: Kind::Bool(false) },
    // Chain: subr.el form 303 (line 3929) `read-char-from-minibuffer-map'.
    // keymap.c:3364 initialises it with Fmake_sparse_keymap (Qnil).
    CVariable { name: "minibuffer-local-map", gnu: "keymap.c:3362", kind: Kind::Lisp(Init::List(&[Init::Symbol("keymap")])) },
    // Census (first failure of a file at 6eb76ddc, WARM and COLD counts):
    // 19/19 files.
    CVariable { name: "system-type", gnu: "emacs.c:3500", kind: Kind::Lisp(Init::Symbol("gnu/linux")) },
    CVariable { name: "system-name", gnu: "editfns.c:4858", kind: Kind::Lisp(Init::SystemName) },
    // GNU expands "../" against the build-time PATH_DUMPLOADSEARCH's first
    // entry (the source lisp directory). Preserve that build source root,
    // not the runtime working directory or the oracle's separate checkout.
    CVariable { name: "source-directory", gnu: "lread.c:5724", kind: Kind::Lisp(Init::SourceDirectory) },
    // 3/3 files; dispnew.c initialises it to nil.
    CVariable { name: "standard-display-table", gnu: "dispnew.c:7560", kind: Kind::Lisp(Init::Nil) },
    // 2/1 files; emacs.c:3611 builds it from the version VM2 is measured
    // against (GNU 32.0.50).
    CVariable { name: "emacs-version", gnu: "emacs.c:3604", kind: Kind::Lisp(Init::Str("32.0.50")) },
    // 1/0 files; keyboard.c:13955 XSETINT (Vhelp_char, Ctl ('H')).
    CVariable { name: "help-char", gnu: "keyboard.c:13951", kind: Kind::Lisp(Init::Fixnum(8)) },
    // 1/0 files; window.c: list1 (Fcons (Qclone_of, Qt)).
    CVariable { name: "window-persistent-parameters", gnu: "window.c:9550", kind: Kind::Lisp(Init::List(&[Init::Cons(&Init::Symbol("clone-of"), &Init::True)])) },
    // 0/1 files; window.c initialises it to nil.
    CVariable { name: "scroll-preserve-screen-position", gnu: "window.c:9304", kind: Kind::Lisp(Init::Nil) },
    // GNU window.el's split-window (window.el:5755-5756, 5843) reads these
    // three window.c variables on every split (prims/window.rs). C values:
    // window.c:9548 Qwindow_size, :9510 nil, :9584 false.
    CVariable { name: "window-combination-limit", gnu: "window.c:9512", kind: Kind::Lisp(Init::Symbol("window-size")) },
    CVariable { name: "window-combination-resize", gnu: "window.c:9491", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "window-resize-pixelwise", gnu: "window.c:9575", kind: Kind::Bool(false) },
    // run-window-scroll-functions (window.c:3862) runs it; xdisp.c:38881 nil.
    CVariable { name: "window-scroll-functions", gnu: "xdisp.c:38865", kind: Kind::Lisp(Init::Nil) },
    // Variable harness (R216 task 2): a DEFVAR_BOOL that subr.el reads.
    // get-load-suffixes (lread.c:961) reads both; it is reached through
    // GNU's Lisp locate-library (subr.el:3440) once lazy bodies replaced
    // VM2's native one. GNU 32.0.50 here is built with modules
    // (MODULES_SUFFIX ".so"), so ".so" leads load-suffixes. load-file-rep-
    // suffixes is the C value (""); the dumped GNU image adds ".gz" from
    // jka-cmpr-hook.el, which VM2 has not loaded.
    CVariable { name: "load-suffixes", gnu: "lread.c:5580", kind: Kind::Lisp(Init::List(&[Init::Str(".so"), Init::Str(".elc"), Init::Str(".el")])) },
    CVariable { name: "load-file-rep-suffixes", gnu: "lread.c:5616", kind: Kind::Lisp(Init::List(&[Init::Str("")])) },
    // Census (warm): pcase.el form 12 (line 173) reads after-load-alist and
    // form 52 (line 1190) binds current-load-list. Both are nil in C. GNU's
    // C maintains them while it runs (load specbinds current-load-list,
    // LOADHIST_ATTACH conses onto it, provide runs after-load-alist); VM2
    // does neither (DIVERGENCES.md VM2-LOADHIST1).
    CVariable { name: "after-load-alist", gnu: "lread.c:5637", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "current-load-list", gnu: "lread.c:5693", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "enable-recursive-minibuffers", gnu: "minibuf.c:2603", kind: Kind::Bool(false) },
    // read-from-minibuffer / read-string (prims/minibuf.rs, lane vm2/minibuf):
    // the C variables read_minibuf and its command loop read. All nil/false
    // in C except history-add-new-input (minibuf.c:2594) and the prompt
    // properties (minibuf.c:2697, list2 (Qread_only, Qt); GNU's faces.el:2914
    // appends `face minibuffer-prompt' from Lisp).
    CVariable { name: "unread-command-events", gnu: "keyboard.c:13776", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "executing-kbd-macro", gnu: "macros.c:435", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "inhibit-interaction", gnu: "minibuf.c:2708", kind: Kind::Bool(false) },
    CVariable { name: "minibuffer-allow-text-properties", gnu: "minibuf.c:2684", kind: Kind::Bool(false) },
    CVariable { name: "minibuffer-prompt-properties", gnu: "minibuf.c:2693", kind: Kind::Lisp(Init::List(&[Init::Symbol("read-only"), Init::True])) },
    CVariable { name: "history-add-new-input", gnu: "minibuf.c:2589", kind: Kind::Lisp(Init::True) },
    CVariable { name: "minibuffer-setup-hook", gnu: "minibuf.c:2564", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "debug-on-error", gnu: "eval.c:4485", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "signal-hook-function", gnu: "eval.c:4487", kind: Kind::Lisp(Init::Nil) },
    // Variable harness (R216 task 2): a DEFVAR_INT that subr.el reads.
    CVariable { name: "undo-limit", gnu: "undo.c:437", kind: Kind::Int(160_000) },
    // Variable harness (R216 task 2) and DIVERGENCES.md VM2-VARALIAS1: the
    // per-buffer kind. buffer.c:4898 default 70, predicate Qintegerp.
    CVariable { name: "major-mode", gnu: "buffer.c:5242", kind: Kind::PerBuffer { default: Init::Symbol("fundamental-mode"), predicate: Some(Predicate::Symbolp) } },
    CVariable { name: "mode-name", gnu: "buffer.c:5255", kind: Kind::PerBuffer { default: Init::Str("Fundamental"), predicate: None } },
    CVariable { name: "fill-column", gnu: "buffer.c:5259", kind: Kind::PerBuffer { default: Init::Fixnum(70), predicate: Some(Predicate::Integerp) } },
    // insert-file-contents / write-region (prims/fileio.rs): the per-buffer
    // C variables those DEFUNs read or set. Their other C variables are
    // plain specials (`Vm::declare_fileio_specials`) so they stay
    // localizable, as add-hook LOCAL needs for the change hooks.
    CVariable { name: "write-region-inhibit-fsync", gnu: "fileio.c:6971", kind: Kind::Bool(true) },
    CVariable { name: "buffer-file-name", gnu: "buffer.c:5441", kind: Kind::PerBuffer { default: Init::Nil, predicate: Some(Predicate::Stringp) } },
    CVariable { name: "buffer-file-truename", gnu: "buffer.c:5446", kind: Kind::PerBuffer { default: Init::Nil, predicate: Some(Predicate::Stringp) } },
    CVariable { name: "buffer-read-only", gnu: "buffer.c:5458", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "buffer-file-format", gnu: "buffer.c:5837", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "mark-active", gnu: "buffer.c:5795", kind: Kind::PerBuffer { default: Init::Nil, predicate: None } },
    CVariable { name: "transient-mark-mode", gnu: "buffer.c:5875", kind: Kind::Lisp(Init::Nil) },
    CVariable { name: "mark-even-if-inactive", gnu: "callint.c:898", kind: Kind::Lisp(Init::True) },
];

/// The runtime storage GNU keeps in C globals and `buffer_defaults`.
pub(crate) struct Forwarded {
    objects: Vec<Value>,
    fundamental_mode_name: Value,
    unbound_symbol: Value,
    bools: Vec<bool>,
    ints: Vec<i64>,
    eval_limit_index: usize,
    /// Per-buffer defaults (`buffer_defaults`).
    defaults: Vec<Value>,
    /// Table index behind each Obj / Buffer payload, for lazy initial values.
    object_vars: Vec<usize>,
    buffer_vars: Vec<usize>,
    by_name: HashMap<&'static str, Fwd>,
}

impl Default for Forwarded {
    fn default() -> Self {
        let mut store = Forwarded {
            objects: vec![],
            fundamental_mode_name: Value::UNBOUND,
            unbound_symbol: Value::UNBOUND,
            bools: vec![],
            ints: vec![],
            eval_limit_index: 0,
            defaults: vec![],
            object_vars: vec![],
            buffer_vars: vec![],
            by_name: HashMap::new(),
        };
        for (index, var) in C_VARIABLES.iter().enumerate() {
            let fwd = match var.kind {
                Kind::Lisp(_) => {
                    store.objects.push(Value::UNBOUND);
                    store.object_vars.push(index);
                    Fwd::Obj(store.objects.len() as u16 - 1)
                }
                Kind::Bool(b) => {
                    store.bools.push(b);
                    Fwd::Bool(store.bools.len() as u16 - 1)
                }
                Kind::Int(n) => {
                    store.ints.push(n);
                    Fwd::Int(store.ints.len() as u16 - 1)
                }
                Kind::PerBuffer { .. } => {
                    store.defaults.push(Value::UNBOUND);
                    store.buffer_vars.push(index);
                    Fwd::Buffer(store.defaults.len() as u16 - 1)
                }
            };
            store.by_name.insert(var.name, fwd);
        }
        let Some(Fwd::Int(i)) = store.lookup("max-lisp-eval-depth") else { unreachable!() };
        store.eval_limit_index = i as usize;
        store
    }
}

impl Forwarded {
    #[inline(always)]
    pub(super) fn eval_limit(&self) -> i64 { self.ints[self.eval_limit_index] }
    /// Direct C-global integer access, without constructing a Lisp Value.
    pub(super) fn integer(&self, name: &str) -> i64 {
        let Some(Fwd::Int(i)) = self.lookup(name) else { panic!("not a forwarded integer: {name}") };
        self.ints[i as usize]
    }
    pub(super) fn set_integer(&mut self, name: &str, value: i64) {
        let Some(Fwd::Int(i)) = self.lookup(name) else { panic!("not a forwarded integer: {name}") };
        self.ints[i as usize] = value;
    }
    pub(crate) fn names() -> impl Iterator<Item = &'static str> {
        C_VARIABLES.iter().map(|v| v.name)
    }
    pub(crate) fn lookup(&self, name: &str) -> Option<Fwd> {
        self.by_name.get(name).copied()
    }
    /// Every collector-visible Value held outside value cells and buffers.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.fundamental_mode_name);
        visit(&mut self.unbound_symbol);
        for v in self.objects.iter_mut().chain(self.defaults.iter_mut()) {
            visit(v);
        }
    }
    #[cfg(test)]
    pub(crate) fn object(&self, fwd: Fwd) -> Value {
        match fwd {
            Fwd::Obj(i) => self.objects[i as usize],
            Fwd::Buffer(i) => self.defaults[i as usize],
            _ => Value::NIL,
        }
    }
}

/// A buffer's per-buffer variable values (GNU keeps them in `struct buffer`
/// and a `local_flags` bit each). `None`, or an index past the end, is "not
/// local here": the buffer sees the default.
#[derive(Clone, Default)]
pub(crate) struct PerBuffer(Vec<Option<Value>>, pub(crate) VisitedModtime, Option<(u64, u64)>, Option<String>, Option<Value>);

/// Native scalar state, not Lisp Values: safe through moving collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum VisitedModtime {
    #[default]
    Unknown,
    Missing,
    Known { seconds: i64, nanos: i64, size: Option<u64> },
}

pub(crate) fn file_identity(path: &str) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.dev(), metadata.ino()))
}

impl PerBuffer {
    pub(crate) fn editor_mark(&self) -> Option<Value> { self.4 }
    pub(crate) fn set_editor_mark(&mut self, marker: Value) { self.4 = Some(marker); }
    pub(crate) fn get(&self, i: u16) -> Option<Value> {
        self.0.get(i as usize).copied().flatten()
    }
    fn set(&mut self, i: u16, value: Value) {
        let i = i as usize;
        if self.0.len() <= i {
            self.0.resize(i + 1, None);
        }
        self.0[i] = Some(value);
    }
    /// Forget one binding without disturbing the other values or visit metadata.
    /// Absence is the local flag: removing the value also removes that flag.
    pub(crate) fn clear_local(&mut self, i: u16) {
        if let Some(value) = self.0.get_mut(i as usize) {
            *value = None;
        }
    }
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        if let Some(marker) = &mut self.4 { visit(marker); }
        for v in self.0.iter_mut().flatten() {
            visit(v);
        }
    }
}

/// GNU `enum Set_Internal_Bind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetFlag {
    Set,
    Bind,
}

/// The text a forwarded store fault takes across VM2's string-error
/// boundaries; [`Vm::forwarded_condition`] turns it back into GNU's condition.
pub(crate) fn fault_text(fault: Fault) -> String {
    match fault {
        Fault::Wrong(predicate, _) => format!("{WRONG_TYPE}{predicate}"),
        Fault::Error(message) => message,
        _ => "invalid forwarded variable fault".into(),
    }
}
const WRONG_TYPE: &str = "forwarded variable wrong-type-argument ";
const UNBINDABLE: &str = "Built-in variable may not be unbound : ";

impl Vm {
    /// GNU `SYMBOL_FORWARDED`: the forwarding of interned SLOT, if it names a
    /// built-in variable. Initialises and caches SLOT's lazy redirect.
    pub(super) fn forwarding(&mut self, slot: usize) -> Option<Fwd> {
        if self.variable_redirects.get(slot).copied().flatten().is_none() {
            let name = self.symbol_names.get(slot)?.clone();
            if self.names.get(&name) != Some(&slot) {
                return None;
            }
            self.initialize_variable_redirect(&name, slot);
        }
        match self.variable_redirects.get(slot).copied().flatten() {
            Some(VariableRedirect::Forwarded(fwd)) => Some(fwd),
            _ => None,
        }
    }
    /// [`Vm::forwarding`] without caching, for `&self` readers.
    pub(super) fn forwarding_ref(&self, slot: usize) -> Option<Fwd> {
        match self.variable_redirects.get(slot).copied().flatten() {
            Some(VariableRedirect::Forwarded(fwd)) => Some(fwd),
            Some(_) => None,
            None => {
                let name = self.symbol_names.get(slot)?;
                (self.names.get(name) == Some(&slot)).then(|| self.forwarded.lookup(name)).flatten()
            }
        }
    }
    fn materialize(&mut self, init: Init) -> Value {
        match init {
            Init::Nil => Value::NIL,
            Init::True => Value::TRUE,
            Init::Fixnum(n) => Value::integer(n).expect("C initial value is a fixnum"),
            Init::Symbol(name) => self.intern(name),
            // build_string: unibyte unless the text has a non-ASCII char.
            Init::Str(text) => self.make_lisp_string(text.as_bytes().to_vec(), !text.is_ascii()),
            Init::SystemName => prim::prims::sysname::initial_value(self),
            Init::SourceDirectory => {
                // CARGO_MANIFEST_DIR is captured while building this crate.
                // Like GNU's expansion, this is lexical: do not canonicalize
                // or require that the build tree still exists at runtime.
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent().unwrap().parent().unwrap();
                let text = format!("{}/", root.to_str().expect("build source path is UTF-8"));
                self.make_lisp_string(text.as_bytes().to_vec(), !text.is_ascii())
            }
            Init::List(items) => {
                let values: Vec<Value> = items.iter().map(|&i| self.materialize(i)).collect();
                self.list_value(&values)
            }
            Init::Cons(car, cdr) => {
                let car = self.materialize(*car);
                let cdr = self.materialize(*cdr);
                self.keymap_cons(car, cdr)
            }
        }
    }
    fn forwarded_object(&mut self, i: u16) -> Value {
        let value = self.forwarded.objects[i as usize];
        if value != Value::UNBOUND {
            return value;
        }
        let Kind::Lisp(init) = C_VARIABLES[self.forwarded.object_vars[i as usize]].kind else { unreachable!() };
        let value = self.materialize(init);
        self.forwarded.objects[i as usize] = value;
        value
    }
    /// GNU's private Qunbound symbol, exposed by the pinned permanent-hook fault.
    pub(super) fn forwarded_unbound_symbol(&mut self) -> Value {
        if self.forwarded.unbound_symbol == Value::UNBOUND {
            let symbol = self.generated_symbol(expand::generated_id());
            self.symbol_names[symbol.index(7).unwrap() - 1] = "unbound".into();
            self.forwarded.unbound_symbol = symbol;
        }
        self.forwarded.unbound_symbol
    }
    fn per_buffer_initial(&mut self, i: u16) -> Value {
        let variable = &C_VARIABLES[self.forwarded.buffer_vars[i as usize]];
        if variable.name == "mode-name" {
            if self.forwarded.fundamental_mode_name == Value::UNBOUND {
                self.forwarded.fundamental_mode_name = self.materialize(Init::Str("Fundamental"));
            }
            self.forwarded.fundamental_mode_name
        } else {
            let Kind::PerBuffer { default, .. } = variable.kind else { unreachable!() };
            self.materialize(default)
        }
    }
    fn per_buffer_default(&mut self, i: u16) -> Value {
        let value = self.forwarded.defaults[i as usize];
        if value != Value::UNBOUND {
            return value;
        }
        let Kind::PerBuffer { default, .. } = C_VARIABLES[self.forwarded.buffer_vars[i as usize]].kind else { unreachable!() };
        let value = self.materialize(default);
        self.forwarded.defaults[i as usize] = value;
        value
    }
    fn per_buffer(&self, buffer: usize) -> Option<&PerBuffer> {
        match self.objects.get(buffer)? {
            data::Object::Buffer(b) => Some(&b.per_buffer),
            data::Object::ByteBuffer(b) => Some(&b.per_buffer),
            _ => None,
        }
    }
    fn per_buffer_mut(&mut self, buffer: usize) -> Option<&mut PerBuffer> {
        match self.objects.get_mut(buffer)? {
            data::Object::Buffer(b) => Some(&mut b.per_buffer),
            data::Object::ByteBuffer(b) => Some(&mut b.per_buffer),
            _ => None,
        }
    }
    /// GNU `PER_BUFFER_VALUE_P (buf, idx)`.
    pub(super) fn per_buffer_always_local(&self, i: u16) -> bool {
        matches!(C_VARIABLES[self.forwarded.buffer_vars[i as usize]].name,
            "buffer-invisibility-spec" | "buffer-file-name" | "buffer-file-truename" | "buffer-read-only" | "buffer-file-format" | "major-mode" | "mode-name" | "mark-active")
    }
    pub(super) fn clear_forwarded_local(&mut self, buffer: usize, i: u16) {
        if !self.per_buffer_always_local(i) {
            self.per_buffer_mut(buffer).expect("buffer").clear_local(i);
        }
    }
    pub(super) fn forwarded_buffer_value(&mut self, buffer: usize, i: u16) -> Value {
        if let Some(value) = self.per_buffer(buffer).and_then(|p| p.get(i)) { return value; }
        if self.per_buffer_always_local(i) {
            let value = self.per_buffer_initial(i);
            self.per_buffer_mut(buffer).expect("buffer").set(i, value);
            value
        } else { self.per_buffer_default(i) }
    }
    pub(super) fn reset_forwarded_modes(&mut self, buffer: usize) {
        for name in ["major-mode", "mode-name"] {
            let Some(Fwd::Buffer(i)) = self.forwarded.lookup(name) else { unreachable!() };
            self.per_buffer_mut(buffer).expect("buffer").clear_local(i);
            self.forwarded_buffer_value(buffer, i);
        }
    }
    pub(super) fn reset_forwarded_visit(&mut self, buffer: usize) {
        for name in ["buffer-file-name", "buffer-file-truename", "buffer-read-only", "buffer-file-format"] {
            let Some(Fwd::Buffer(i)) = self.forwarded.lookup(name) else { unreachable!() };
            self.per_buffer_mut(buffer).expect("buffer").clear_local(i);
        }
        let locals = self.per_buffer_mut(buffer).expect("buffer");
        locals.1 = VisitedModtime::Unknown;
        locals.2 = None;
        locals.3 = None;
    }
    pub(super) fn per_buffer_local_p(&self, buffer: usize, i: u16) -> bool {
        self.per_buffer_always_local(i) || self.per_buffer(buffer).and_then(|p| p.get(i)).is_some()
    }
    fn current_buffer_object(&mut self) -> std::result::Result<usize, Fault> {
        self.buffer_index().map_err(Fault::Error)
    }
    /// Snapshot live visits without changing current-buffer or dynamic bindings.
    /// Callers are leaf primitives: no collection occurs while Values are held.
    pub(crate) fn file_visits(&self) -> Vec<(Value, String, Option<(u64, u64)>, Option<String>)> {
        let Some(Fwd::Buffer(i)) = self.forwarded.lookup("buffer-file-name") else { return Vec::new() };
        let mut slots: Vec<_> = self.names.iter()
            .filter(|(name, _)| name.starts_with("#:vm2-buffer:"))
            .map(|(_, slot)| *slot).collect();
        slots.sort_unstable();
        let mut buffers: Vec<_> = slots.into_iter().map(|slot| self.slots[slot].value).collect();
        let current = self.slots[self.buffer_slot].value;
        if !buffers.contains(&current) { buffers.push(current); }
        buffers.into_iter().filter_map(|buffer| {
            let index = buffer.index(0).ok()?.checked_sub(1)?;
            let locals = self.per_buffer(index)?;
            let name = self.eval_buffer_name(locals.get(i)?).ok()??;
            Some((buffer, name, locals.2, locals.3.clone()))
        }).collect()
    }

    /// GNU `bset_<field> (current_buffer, VALUE)`: write the current buffer's
    /// own slot of the per-buffer variable NAME and flag it local, bypassing
    /// `let` bindings and the predicate, as C does when it sets a BVAR
    /// directly (fileio.c:5157 `bset_filename`).
    pub(crate) fn bset_current(&mut self, name: &str, value: Value) -> std::result::Result<(), Fault> {
        let Some(Fwd::Buffer(i)) = self.forwarded.lookup(name) else {
            return Err(Fault::Error(format!("{name} is not a VM2 per-buffer variable")));
        };
        let buffer = self.current_buffer_object()?;
        if name == "buffer-file-name" {
            let identity = self.eval_buffer_name(value).ok().flatten()
                .and_then(|path| file_identity(&path));
            let truename = self.eval_buffer_name(value).ok().flatten().map(|path| {
                std::fs::canonicalize(&path).ok().and_then(|p| p.to_str().map(str::to_owned)).unwrap_or(path)
            });
            let locals = self.per_buffer_mut(buffer).expect("live buffer");
            locals.2 = identity;
            locals.3 = truename;
        }
        self.per_buffer_mut(buffer).expect("live buffer").set(i, value);
        Ok(())
    }
    /// GNU `do_symval_forwarding`: the value the current buffer sees.
    pub(super) fn forwarded_value(&mut self, fwd: Fwd) -> Result<Value> {
        Ok(match fwd {
            Fwd::Obj(i) => self.forwarded_object(i),
            Fwd::Bool(i) => if self.forwarded.bools[i as usize] { Value::TRUE } else { Value::NIL },
            Fwd::Int(i) => Value::integer(self.forwarded.ints[i as usize])?,
            Fwd::Buffer(i) => {
                let buffer = self.buffer_index()?;
                self.forwarded_buffer_value(buffer, i)
            }
        })
    }
    /// GNU `default_value`, SYMBOL_FORWARDED arm: a per-buffer variable's
    /// default, otherwise the current value.
    pub(super) fn forwarded_default(&mut self, fwd: Fwd) -> Result<Value> {
        match fwd {
            Fwd::Buffer(i) => Ok(self.per_buffer_default(i)),
            _ => self.forwarded_value(fwd),
        }
    }
    /// GNU `store_symval_forwarding` into BUFFER (an object index; ignored
    /// unless FWD is per-buffer).
    fn store_forwarded(&mut self, fwd: Fwd, value: Value, buffer: usize) -> std::result::Result<(), Fault> {
        match fwd {
            Fwd::Obj(i) => self.forwarded.objects[i as usize] = value,
            Fwd::Bool(i) => self.forwarded.bools[i as usize] = value != Value::NIL,
            Fwd::Int(i) => {
                // CHECK_INTEGER; VM2 has no bignums, so every integer fits
                // and GNU's overflow-error arm is unreachable.
                let n = value.integer_value().ok_or(Fault::Wrong("integerp", value))?;
                self.forwarded.ints[i as usize] = n;
            }
            Fwd::Buffer(i) => {
                if value != Value::NIL {
                    let Kind::PerBuffer { predicate, .. } = C_VARIABLES[self.forwarded.buffer_vars[i as usize]].kind else { unreachable!() };
                    match predicate {
                        Some(Predicate::Integerp) if value.integer_value().is_none() => {
                            return Err(Fault::Wrong("integerp", value));
                        }
                        Some(Predicate::Symbolp) if value != Value::TRUE && value.index_if(7).is_none() => {
                            return Err(Fault::Wrong("symbolp", value));
                        }
                        Some(Predicate::Stringp) if self.str_view(value).is_none() => {
                            return Err(Fault::Wrong("stringp", value));
                        }
                        _ => {}
                    }
                }
                // GNU writes the buffer's slot whether or not its flag is
                // set; an unflagged slot always mirrors the default, so VM2
                // keeps only flagged (local) values.
                if self.per_buffer_local_p(buffer, i) {
                    self.per_buffer_mut(buffer).expect("live buffer").set(i, value);
                }
            }
        }
        Ok(())
    }
    /// GNU `let_shadows_buffer_binding_p`: a `let` of SLOT's default made
    /// while BUFFER was current is in effect.
    fn let_shadows_buffer_binding(&self, slot: usize, buffer: Value) -> bool {
        self.dynamic.iter().any(|&(bound, _, owner, let_default)| bound == slot && let_default && owner == buffer)
    }
    /// GNU `set_internal`, SYMBOL_FORWARDED arm (data.c:1791-1818).
    pub(super) fn set_forwarded(&mut self, slot: usize, fwd: Fwd, value: Value, flag: SetFlag) -> std::result::Result<(), Fault> {
        if value == Value::UNBOUND {
            return Err(Fault::Error(format!("{UNBINDABLE}{}", self.symbol_names[slot])));
        }
        let mut buffer = 0;
        if let Fwd::Buffer(i) = fwd {
            buffer = self.current_buffer_object()?;
            if flag == SetFlag::Set && !self.per_buffer_local_p(buffer, i) {
                let owner = self.slots[self.buffer_slot].value;
                if self.let_shadows_buffer_binding(slot, owner) {
                    self.set_forwarded_default(slot, fwd, value, flag)?;
                } else {
                    // SET_PER_BUFFER_VALUE_P before the predicate check, as
                    // GNU: a rejected value still leaves the buffer local.
                    let current = self.per_buffer_default(i);
                    self.per_buffer_mut(buffer).expect("live buffer").set(i, current);
                }
            }
        }
        self.store_forwarded(fwd, value, buffer)
    }
    /// GNU `set_default_internal`, SYMBOL_FORWARDED arm (data.c:2085-2124):
    /// a per-buffer default is stored WITHOUT the predicate.
    pub(super) fn set_forwarded_default(&mut self, slot: usize, fwd: Fwd, value: Value, flag: SetFlag) -> std::result::Result<(), Fault> {
        match fwd {
            Fwd::Buffer(i) => {
                self.forwarded.defaults[i as usize] = value;
                Ok(())
            }
            _ => self.set_forwarded(slot, fwd, value, flag),
        }
    }
    /// GNU `specbind` + `do_specbind` for a forwarded SLOT (the end of any
    /// alias chain). Pushes the binding only once the store succeeded: GNU
    /// pushes first, but a failed store changed nothing, so unwinding that
    /// entry is a no-op either way.
    pub(super) fn bind_forwarded(&mut self, slot: usize, fwd: Fwd, value: Value) -> std::result::Result<(), Fault> {
        let old = self.forwarded_value(fwd).map_err(Fault::Error)?;
        let buffer = self.current_buffer_object()?;
        let owner = self.slots[self.buffer_slot].value;
        // SPECPDL_LET_DEFAULT for a per-buffer variable with no local value
        // here; SPECPDL_LET_LOCAL if it has one; SPECPDL_LET otherwise.
        let let_default = matches!(fwd, Fwd::Buffer(i) if !self.per_buffer_local_p(buffer, i));
        if let_default {
            self.set_forwarded_default(slot, fwd, value, SetFlag::Bind)?;
        } else {
            self.set_forwarded(slot, fwd, value, SetFlag::Bind)?;
        }
        self.dynamic.push((slot, old, owner, let_default));
        Ok(())
    }
    /// GNU `do_one_unbind` for a forwarded binding. The saved value was read
    /// from the same storage, so it is restored without re-checking.
    pub(super) fn unbind_forwarded(&mut self, fwd: Fwd, old: Value, owner: Value, let_default: bool) {
        match fwd {
            Fwd::Obj(i) => self.forwarded.objects[i as usize] = old,
            Fwd::Bool(i) => self.forwarded.bools[i as usize] = old != Value::NIL,
            Fwd::Int(i) => {
                if let Some(n) = old.integer_value() {
                    self.forwarded.ints[i as usize] = n;
                }
            }
            Fwd::Buffer(i) if let_default => self.forwarded.defaults[i as usize] = old,
            // SPECPDL_LET_LOCAL: only if OWNER still has its local value.
            Fwd::Buffer(i) => {
                if let Some(buffer) = owner.index_if(0).and_then(|n| n.checked_sub(1))
                    && self.per_buffer_local_p(buffer, i)
                {
                    self.per_buffer_mut(buffer).expect("live buffer").set(i, old);
                }
            }
        }
    }
    /// GNU `make-local-variable` on a forwarded variable (data.c:2245).
    pub(super) fn make_forwarded_local(&mut self, slot: usize, fwd: Fwd) -> Result<()> {
        match fwd {
            Fwd::Buffer(i) => {
                let buffer = self.buffer_index()?;
                if !self.per_buffer_local_p(buffer, i) {
                    let current = self.per_buffer_default(i);
                    self.per_buffer_mut(buffer).expect("live buffer").set(i, current);
                }
                Ok(())
            }
            _ => Err(unlocalizable(&self.symbol_names[slot])),
        }
    }
    /// The condition GNU signals for a forwarded store fault rendered by
    /// [`fault_text`], with VALUE the value being stored.
    pub(super) fn forwarded_condition(&mut self, error: &str, value: Value) -> Option<(Value, Value)> {
        let (condition, data) = if let Some(predicate) = error.strip_prefix(WRONG_TYPE) {
            let predicate = self.intern(predicate);
            ("wrong-type-argument", vec![predicate, value])
        } else if error.starts_with(UNBINDABLE) || error.starts_with(UNLOCALIZABLE) {
            let message = self.make_lisp_string(error.as_bytes().to_vec(), true);
            ("error", vec![message])
        } else {
            return None;
        };
        let condition = self.intern(condition);
        Some((condition, self.list_value(&data)))
    }
}

/// The DEFVAR_LISP variables `insert-file-contents' and `write-region' read
/// (all nil in C; GNU's -Q image fills `file-name-handler-alist',
/// `file-coding-system-alist' and `set-auto-coding-function' from Lisp, VM2
/// keeps nil). Declared special with GNU's C value so a `let' of one is
/// dynamic and seen by the primitive, but NOT forwarded: forwarded object
/// variables cannot be made buffer-local yet (VM2-CVAR1), and packages make
/// the change hooks and `deactivate-mark' local.
const FILEIO_SPECIALS: [(&str, &str); 17] = [
    ("first-change-hook", "buffer.c:5738"),
    ("inhibit-read-only", "buffer.c:5897"),
    ("before-change-functions", "buffer.c:5704"),
    ("after-change-functions", "buffer.c:5720"),
    ("coding-system-for-read", "coding.c:11971"),
    ("coding-system-for-write", "coding.c:11980"),
    ("last-coding-system-used", "coding.c:11994"),
    ("file-coding-system-alist", "coding.c:12030"),
    ("select-safe-coding-system-function", "coding.c:12152"),
    ("file-name-handler-alist", "fileio.c:6854"),
    ("inhibit-file-name-handlers", "fileio.c:6943"),
    ("inhibit-file-name-operation", "fileio.c:6948"),
    ("set-auto-coding-function", "fileio.c:6876"),
    ("after-insert-file-functions", "fileio.c:6889"),
    ("write-region-annotate-functions", "fileio.c:6899"),
    ("write-region-post-annotation-function", "fileio.c:6925"),
    ("deactivate-mark", "keyboard.c:14030"),
];

impl Vm {
    pub(crate) fn declare_fileio_specials(&mut self) {
        self.declare_special_raw("change-major-mode-hook", Value::NIL);
        self.declare_special_raw("create-lockfiles", Value::TRUE);
        // GNU coding.c:12084-12088, before Lisp selects a locale coding.
        self.declare_special_raw("locale-coding-system", Value::NIL);
        for (name, _gnu) in FILEIO_SPECIALS {
            if !self.specials.contains(name) {
                self.declare_special_raw(name, Value::NIL);
            }
        }
    }
}

const UNLOCALIZABLE: &str = "VM2 cannot make built-in variable buffer-local: ";
/// GNU turns an object, boolean or integer built-in variable into
/// SYMBOL_LOCALIZED with a forwarded realvalue (`make_blv`). VM2 does not
/// model that combination yet and refuses instead (DIVERGENCES.md
/// VM2-CVAR1), rather than silently creating a binding nothing would read.
pub(super) fn unlocalizable(name: &str) -> String {
    format!("{UNLOCALIZABLE}{name}")
}

#[cfg(test)]
#[path = "forwarded_tests.rs"]
mod forwarded_tests;
