//! A deliberately small Rust register compiler and strand-private VM.
//! There is no dependency on the old evaluator, reader, or byte compiler.
#![deny(private_interfaces, private_bounds)]
#[cfg(test)]
mod callint_tests;
#[cfg(test)]
mod file_name_handler_tests;
#[cfg(test)]
mod keymap_tests;
#[cfg(test)]
mod lazy_tests;
#[cfg(test)]
mod plist_tests;
mod keymap;
mod char_table;
mod map_keymap;
#[cfg(test)]
mod map_keymap_tests;
mod charset;
mod obarray;
mod command;
mod interpreted_closure;
mod oclosure;
mod host_input;
mod quit;
pub use quit::QuitSignal;
mod bidi;
mod layout_host;
mod display_host;
mod session_host;
mod terminal_host;
mod process_jobs;
mod subprocess;
mod tls;
mod html;
mod html_entities;
pub mod host;
pub mod world;
pub mod compiled_lisp;
mod environment;
mod watchers;
mod user_db;
mod locale_info;
pub use command::{CommandEvent, CommandOutcome, CommandSignal};
pub use host_input::{HostEvent, HostFrame, HostInput, NativeWindow, QUIT_CHAR};
pub use subprocess::ProcessWatch;
pub use layout_host::LayoutHost;
pub use display_host::{DisplayHost, FontEntity, FontQuery, OpenedFont, OtfTable};
pub use session_host::SessionHost;
pub use romacs_layout_input::WindowEnd;
pub use terminal_host::{TerminalColor, TerminalHost, TerminalInputState, TerminalSnapshot, TerminalStyle, TerminalStyleRun};
// Maps keyed by slots and ids the VM allocates use FxHash. Maps keyed by
// names, which Lisp, files and the network supply, use foldhash with a random
// seed per map, so a precomputed set of colliding names cannot make their
// lookups quadratic.
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
type NameMap<K, V> = std::collections::HashMap<K, V, NameState>;
type NameSet<K> = std::collections::HashSet<K, NameState>;

/// foldhash seeded by values the map itself holds: its own random seed and a
/// shared one this process leaked to the heap. A map in a world image
/// (`world::arena`) then hashes alike in every process that maps it, where
/// foldhash's own `RandomState` reads a seed each process draws anew.
#[derive(Clone)]
pub(crate) struct NameState(foldhash::fast::SeedableRandomState);
// Its own seed is a number, and its shared seed the one `default` leaked
// to the heap, never foldhash's global one in a writable static.
unsafe impl romacs_snapshot::SnapshotSafe for NameState {}

impl Default for NameState {
    fn default() -> Self {
        use std::hash::BuildHasher;
        static SHARED: std::sync::OnceLock<&'static foldhash::SharedSeed> = std::sync::OnceLock::new();
        let random = || std::collections::hash_map::RandomState::new().hash_one(0u8);
        let shared = *SHARED.get_or_init(|| Box::leak(Box::new(foldhash::SharedSeed::from_u64(random()))));
        NameState(foldhash::fast::SeedableRandomState::with_seed(random(), shared))
    }
}

impl std::hash::BuildHasher for NameState {
    type Hasher = foldhash::fast::FoldHasher<'static>;
    fn build_hasher(&self) -> Self::Hasher {
        self.0.build_hasher()
    }
}
use std::sync::Arc;
mod code_image;
mod sqlite;
mod treesit;
mod random;
mod digest;
mod posix_tz;
mod tzfile;
mod undo;
mod insdel;
mod indirect;
mod gc;
mod handle;
pub use handle::{Arg, Fixed, Local, Rooted};
mod image;
mod frozen;
mod isolate;
pub use isolate::worker_socket as isolate_worker_socket;
mod request_code;
pub use gc::HeapStats;
pub use image::{ImageBuilder, ImageRegion};
pub mod anchors;
mod annotation_primitives;
mod annotations;
mod buffer;
mod buffer_swap;
mod bytes;
mod modification;
mod conditions;
mod sealed;
pub mod coordinates;
mod data;
mod bool_vector;
mod lstring;
mod let_forms;
mod symkey;
mod display;
mod fringe;
mod tab_bar;
mod overlay_order;
mod window;
mod frame;
mod hash;
mod intervals;
mod properties;
pub mod text;
mod backtrace;
mod profiler;
mod threaded;

mod expand;
mod save_excursion;
mod numbers;
mod metadata;
mod variable_definitions;
mod variable_runtime;
mod forwarded;
mod eval_depth;
mod eval_data;
mod eval_buffer;
mod source_cache;
mod macroexpand;
pub use source_cache::{DIRECT_NODE_LIMIT, PreparedHandle, SourceCacheStats};
mod bootstrap;
mod capture;
mod loader;
pub use loader::{Step as LoadStep, StepStatus as LoadStepStatus};
mod autoload;
mod eval_form;
mod productivity;
mod protocol;
mod reader;
mod regexp;
mod syntax_scan;
mod surface;
mod tails;
mod prim;

pub type Result<T> = std::result::Result<T, String>;
/// Opaque compiler identity. Source APIs cannot construct one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GeneratedName(u64);
romacs_snapshot::snapshot_tuple!(GeneratedName(f0));
impl std::fmt::Display for GeneratedName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
#[derive(Clone, Debug)]
pub enum Form {
    Int(i64),
    Float(f64),
    Sym(String),
    /// Compiler-only identity; neither the reader nor JSON transport produces it.
    #[doc(hidden)]
    Generated(GeneratedName),
    List(Vec<Form>),
    String(String),
    Bytes(Vec<u8>),
    Vector(Vec<Form>),
    Dotted(Vec<Form>, Box<Form>),
    /// Compiler-only: a heap object with no source spelling (a closure, a
    /// buffer, ...) carried by a macro expansion or an `eval` datum, exactly
    /// as GNU's evaluator carries it (src/eval.c eval_sub). Produced only by
    /// `Vm::expansion_form`; the object is rooted while the form lives
    /// (`FormObject`), and it must not outlive the compilation that
    /// consumes it, which moves it into a traced `Op::Const`.
    #[doc(hidden)]
    Object(handle::FormObject),
}
romacs_snapshot::snapshot_enum!(Form {
    Form::Int(f0) => (f0),
    Form::Float(f0) => (f0),
    Form::Sym(f0) => (f0),
    Form::Generated(f0) => (f0),
    Form::List(f0) => (f0),
    Form::String(f0) => (f0),
    Form::Bytes(f0) => (f0),
    Form::Vector(f0) => (f0),
    Form::Dotted(f0, f1) => (f0, f1),
    Form::Object(f0) => (f0),
});
impl Form {
    pub fn from_json(v: serde_json::Value) -> Result<Self> {
        match v {
            serde_json::Value::Number(n) => n
                .as_i64()
                .map(Self::Int)
                .or_else(|| n.as_f64().map(Self::Float))
                .ok_or_else(|| "number required".into()),
            serde_json::Value::String(s) => Ok(Self::Sym(s)),
            serde_json::Value::Array(a) => Ok(Self::List(
                a.into_iter().map(Self::from_json).collect::<Result<_>>()?,
            )),
            serde_json::Value::Object(mut o) if o.len() == 1 && o.contains_key("string") => {
                Ok(Self::String(
                    o.remove("string")
                        .unwrap()
                        .as_str()
                        .ok_or("string literal required")?
                        .to_owned(),
                ))
            }
            serde_json::Value::Object(mut o) if o.len() == 1 && o.contains_key("vector") => {
                let a = o.remove("vector").unwrap();
                Ok(Self::Vector(
                    a.as_array()
                        .ok_or("vector literal required")?
                        .iter()
                        .cloned()
                        .map(Self::from_json)
                        .collect::<Result<_>>()?,
                ))
            }
            _ => Err("invalid already-read form transport".into()),
        }
    }
    fn name(&self) -> Result<Name> {
        match self {
            Self::Sym(s) => Ok(Name::Source(s.clone())),
            Self::Generated(id) => Ok(Name::Generated(*id)),
            _ => Err("symbol required".into()),
        }
    }
    fn sym(&self) -> Result<&str> {
        if let Self::Sym(s) = self {
            Ok(s)
        } else {
            Err("symbol required".into())
        }
    }
    fn list(&self) -> Result<&[Form]> {
        if let Self::List(a) = self {
            Ok(a)
        } else if matches!(self, Self::Sym(s) if s == "nil") {
            Ok(&[])
        } else {
            Err("list required".into())
        }
    }
}

/// Two-bit fixnums and three-bit reference tags; private indices move at collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Value(u64);
romacs_snapshot::snapshot_tuple!(Value(f0));
impl Value {
    pub const NIL: Self = Self(0);
    pub const TRUE: Self = Self(2);
    const UNBOUND: Self = Self(7);
    pub const MAX_FIXNUM: i64 = (1i64 << 61) - 1;
    pub const MIN_FIXNUM: i64 = -(1i64 << 61);
    // Logical tags retain their arena numbers. Physical 101 belongs to fixnums;
    // cells use 010 with a one-based payload, leaving bare 010 for t.
    #[inline(always)]
    fn tag(self) -> u64 {
        if self.0 & 3 == 1 {
            1
        } else if self.0 & 7 == 2 && self != Self::TRUE {
            5
        } else {
            self.0 & 7
        }
    }
    #[inline(always)]
    fn payload(self) -> u64 {
        (self.0 >> 3) - u64::from(self.0 & 7 == 2 && self != Self::TRUE)
    }
    pub fn integer(n: i64) -> Result<Self> {
        Self::fixnum(n).ok_or_else(|| "fixnum overflow (bignums not implemented)".into())
    }
    /// The fixnum N, allocation-free for threaded handlers: a Result's error
    /// String, or an out-of-line call returning through a stack slot, stops
    /// LLVM from tail-dispatching to the next handler, so loops would grow
    /// the native stack.
    #[inline(always)]
    pub fn fixnum(n: i64) -> Option<Self> {
        (Self::MIN_FIXNUM..=Self::MAX_FIXNUM).contains(&n).then_some(Self(((n as u64) << 2) | 1))
    }
    fn integer_value(self) -> Option<i64> {
        (self.tag() == 1).then_some((self.0 as i64) >> 2)
    }
    pub fn as_integer(self) -> Result<i64> {
        self.integer_value()
            .ok_or_else(|| "integer required".into())
    }
    fn reference(index: usize, tag: u64) -> Self {
        if tag == 5 {
            Self((((index as u64) + 1) << 3) | 2)
        } else {
            Self(((index as u64) << 3) | tag)
        }
    }
    fn index_if(self, tag: u64) -> Option<usize> {
        (self.tag() == tag).then_some(self.payload() as usize)
    }
    fn index(self, tag: u64) -> Result<usize> {
        self.index_if(tag)
            .ok_or_else(|| format!("wrong value type (expected tag {tag})"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
enum Primitive {
    Add,
    Div,
    Abs,
    Sub,
    Mul,
    Less,
    Equal,
    Inc,
    Dec,
    Cons,
    Car,
    Cdr,
    Length,
    Nreverse,
    Apply,
    Throw,
    Signal,
    Vector,
    Aref,
    Aset,
    CopySequence,
    Assq,
    Setcar,
    Setcdr,
    Remainder,
    CurrentBuffer,
    SetBuffer,
    Insert,
    Point,
    PointMin,
    PointMax,
    BufferSize,
    GotoChar,
    CharAfter,
    DeleteRegion,
    ForwardChar,
    BackwardChar,
    BeginningOfLine,
    EndOfLine,
    Substring,
    SearchForward,
    Narrow,
    Widen,
    CharToByte,
    ByteToChar,
    GetProperty,
    SetMultibyte,
    BufferString,
    ByteString,
    EncodeUtf8,
    DecodeUtf8,
    StringToBytes,
    IsMultibyte,
    InsertFileBytes,
    EncodeBufferUtf8,
    DecodeBufferUtf8,
    ProcessLines,
    Format,
    CacheStats,
    CacheClear,
    CacheEntries,
    Load,
    Require,
    Eval,
    Provide,
    Featurep,
    Message,
    List,
    Member,
    Nth,
    Stringp,
    Null,
    Nconc,
    ExpandFileName,
    FileDirectoryP,
    Max,
    Boundp,
    Defalias,
    Get,
    Put,
    BareSymbol,
    Eq,
    Memq,
    RawPropertiesAt,
    GetCharProperty,
    BufferSubstring,
    InsertInherit,
    KillNew,
    CurrentKill,
    YankPart,
    YankFinalize,
    YankPropertyRun,
    YankFontLockFace,
    YankCategory,
    MakeMarker,
    SetMarker,
    CopyMarker,
    MarkerPosition,
    MarkerBuffer,
    MarkerInsertionType,
    SetMarkerInsertionType,
    InsertBeforeMarkers,
    MakeOverlay,
    MoveOverlay,
    DeleteOverlay,
    OverlayStart,
    OverlayEnd,
    OverlayGet,
    OverlayPut,
    OverlaysAt,
    OverlaysIn,
    NextOverlayChange,
    RemoveOverlays,
    Consp,
    CarSafe,
    Symbolp,
    DefineError,
    ErrorMessage,
    // Hash tables: appended, so earlier executable-image indices are unchanged.
    MakeHashTable,
    Gethash,
    Puthash,
    Remhash,
    Clrhash,
    Maphash,
    HashTableCount,
    HashTableP,
    HashTableTest,
    HashTableSize,
    HashTableWeakness,
    HashTableRehashSize,
    HashTableRehashThreshold,
    CopyHashTable,
    DefineHashTableTest,
    HashTableKeys,
    HashTableValues,
    HashTableContainsP,
    HashNext,
    HashKeyAt,
    HashValueAt,
    Integerp,
    Fixnump,
    Bignump,
    LookingAt,
    StringMatch,
    MatchString,
    MatchBeginning,
    MatchEnd,
    MatchData,
    ReSearchForward,
    ReSearchBackward,
    PosixLookingAt,
    PosixStringMatch,
    PosixSearchForward,
    PosixSearchBackward,
    SyntaxTable,
    StandardSyntaxTable,
    CopySyntaxTable,
    SetSyntaxTable,
    ModifySyntaxEntry,
    DisplayDefineFace,
    SelectedWindow,
    SelectedFrame,
    WindowBuffer,
    WindowStart,
    WindowPoint,
    FramePixelWidth,
    FramePixelHeight,
    Fset,
    WindowVscroll,
    SetWindowVscroll,
    DisplayError,
    DeleteWindowInternal,
    FrameSelectedWindow,
    MinibufferWindow,
    SetWindowMargins,
    SetWindowParameter,
    SetWindowPoint,
    SetWindowStart,
    WindowTotalHeight,
    SplitWindowInternal,
    WindowResizeApply,
    SetWindowNewPixel,
    WindowNewPixel,
    WindowParent,
    WindowNextSibling,
    WindowPrevSibling,
    WindowChild,
    FrameRootWindow,
    Windowp,
    WindowLiveP,
    WindowParameter,
    WindowParameters,
    WindowMargins,
    WindowPixelWidth,
    WindowPixelHeight,
    WindowTotalWidth,
    DefineKey,
    MakeSparseKeymap,
    Keymapp,
    KeyBinding,
    LookupKey,
    GlobalSetKey,
    CurrentGlobalMap,
    UseLocalMap,
    Kbd,
    MakeObsolete,
    MakeObsoleteVariable,
    SetAdvertisedCallingConvention,
    SymbolValue,
    SpecialVariableP,
    MakeVariableBufferLocal,
    CustomVariableP,
    DefvarBoundp,
    DefvarSetDefault,
    DeepEqual,
    Min,
    Append,
    GetBuffer,
    ForwardLine,
    SwitchToBuffer,
    EmacsLispMode,
    MenuBarMode,
    ForceModeLineUpdate,
    LocateLibrary,
    Daemonp,
    DaemonInitialized,
    Fboundp,
    Set,
    Makunbound,
    MakeLocalVariable,
    TempBuffer,
    // Engine control like `load': it loads a file (autoload.rs).
    AutoloadDoLoad,
    // Engine control like `load': waits for a host key, redisplaying in a
    // nested engine first (host_input.rs).
    ReadHostEvent,
    FormatModeLine,
    Macroexpand,
    ClosureMetadata,
    ClosureDocument,
    MakeInterpretedClosure,
    SearchBackward,
}
const PRIMITIVES: &[(&str, Primitive)] = &[
    ("+", Primitive::Add),
    ("/", Primitive::Div),
    ("abs", Primitive::Abs),
    ("-", Primitive::Sub),
    ("*", Primitive::Mul),
    ("<", Primitive::Less),
    ("=", Primitive::Equal),
    ("1+", Primitive::Inc),
    ("1-", Primitive::Dec),
    ("cons", Primitive::Cons),
    ("car", Primitive::Car),
    ("cdr", Primitive::Cdr),
    ("length", Primitive::Length),
    ("nreverse", Primitive::Nreverse),
    ("apply", Primitive::Apply),
    ("throw", Primitive::Throw),
    ("signal", Primitive::Signal),
    ("vector", Primitive::Vector),
    ("aref", Primitive::Aref),
    ("aset", Primitive::Aset),
    ("copy-sequence", Primitive::CopySequence),
    ("assq", Primitive::Assq),
    ("setcar", Primitive::Setcar),
    ("setcdr", Primitive::Setcdr),
    ("%", Primitive::Remainder),
    ("current-buffer", Primitive::CurrentBuffer),
    ("set-buffer", Primitive::SetBuffer),
    // The storage kernel under `insert' (prims/insdel.rs), which runs no hooks.
    ("#:vm2-insert", Primitive::Insert),
    ("point", Primitive::Point),
    ("point-min", Primitive::PointMin),
    ("point-max", Primitive::PointMax),
    ("buffer-size", Primitive::BufferSize),
    ("goto-char", Primitive::GotoChar),
    ("char-after", Primitive::CharAfter),
    // The storage kernel under `delete-region' (prims/insdel.rs), which runs no hooks.
    ("#:vm2-delete-region", Primitive::DeleteRegion),
    ("forward-char", Primitive::ForwardChar),
    ("backward-char", Primitive::BackwardChar),
    ("beginning-of-line", Primitive::BeginningOfLine),
    ("end-of-line", Primitive::EndOfLine),
    ("buffer-substring-no-properties", Primitive::Substring),
    ("search-forward", Primitive::SearchForward),
    ("narrow-to-region", Primitive::Narrow),
    ("widen", Primitive::Widen),
    ("position-bytes", Primitive::CharToByte),
    ("byte-to-position", Primitive::ByteToChar),
    ("get-text-property", Primitive::GetProperty),
    ("set-buffer-multibyte", Primitive::SetMultibyte),
    ("buffer-string", Primitive::BufferString),
    ("unibyte-string", Primitive::ByteString),
    ("string-as-unibyte", Primitive::EncodeUtf8),
    ("string-as-multibyte", Primitive::DecodeUtf8),
    ("string-to-unibyte", Primitive::StringToBytes),
    ("multibyte-string-p", Primitive::IsMultibyte),
    ("insert-file-contents-literally", Primitive::InsertFileBytes),
    ("buffer-encode-utf8", Primitive::EncodeBufferUtf8),
    ("buffer-decode-utf8", Primitive::DecodeBufferUtf8),
    ("process-lines", Primitive::ProcessLines),
    ("format", Primitive::Format),
    ("eval-cache-stats", Primitive::CacheStats),
    ("eval-cache-clear", Primitive::CacheClear),
    ("eval-cache-entries", Primitive::CacheEntries),
    ("load", Primitive::Load),
    ("require", Primitive::Require),
    ("eval", Primitive::Eval),
    // `provide' is a T2 defsubr (prims/provide.rs) so it can run load hooks;
    // the row stays because legacy primitive references are table ordinals.
    ("#:vm2-provide", Primitive::Provide),
    ("featurep", Primitive::Featurep),
    ("#:vm2-message", Primitive::Message), // T2 defsubr now (prims/message.rs); the ordinal stays.
    ("list", Primitive::List),
    ("member", Primitive::Member),
    ("nth", Primitive::Nth),
    ("stringp", Primitive::Stringp),
    ("null", Primitive::Null),
    ("#:vm2-nconc", Primitive::Nconc), // T1 defsubr now (prims/fns_seq.rs); the ordinal stays.
    // T2 defsubrs (prims/expand_file_name.rs, prims/filesave.rs) so they can
    // call file name handlers; the rows stay because legacy primitive
    // references are table ordinals.
    ("#:vm2-expand-file-name", Primitive::ExpandFileName),
    ("#:vm2-file-directory-p", Primitive::FileDirectoryP),
    ("max", Primitive::Max),
    ("boundp", Primitive::Boundp),
    ("defalias", Primitive::Defalias),
    ("get", Primitive::Get),
    ("put", Primitive::Put),
    ("bare-symbol", Primitive::BareSymbol),
    ("eq", Primitive::Eq),
    ("memq", Primitive::Memq),
    ("text-properties-at", Primitive::RawPropertiesAt),
    ("get-char-property", Primitive::GetCharProperty),
    ("buffer-substring", Primitive::BufferSubstring),
    // The storage kernel under `insert-and-inherit' (prims/insdel.rs), which runs no hooks.
    ("#:vm2-insert-and-inherit", Primitive::InsertInherit),
    ("kill-new", Primitive::KillNew),
    ("current-kill", Primitive::CurrentKill),
    ("#:vm2-yank-part", Primitive::YankPart),
    ("#:vm2-yank-finalize", Primitive::YankFinalize),
    ("#:vm2-yank-property-run", Primitive::YankPropertyRun),
    (
        "yank-handle-font-lock-face-property",
        Primitive::YankFontLockFace,
    ),
    ("yank-handle-category-property", Primitive::YankCategory),
    ("make-marker", Primitive::MakeMarker),
    ("set-marker", Primitive::SetMarker),
    ("copy-marker", Primitive::CopyMarker),
    ("marker-position", Primitive::MarkerPosition),
    ("marker-buffer", Primitive::MarkerBuffer),
    ("marker-insertion-type", Primitive::MarkerInsertionType),
    (
        "set-marker-insertion-type",
        Primitive::SetMarkerInsertionType,
    ),
    // The storage kernel under `insert-before-markers' (prims/insdel.rs), which runs no hooks.
    ("#:vm2-insert-before-markers", Primitive::InsertBeforeMarkers),
    ("make-overlay", Primitive::MakeOverlay),
    ("move-overlay", Primitive::MoveOverlay),
    ("delete-overlay", Primitive::DeleteOverlay),
    ("overlay-start", Primitive::OverlayStart),
    ("overlay-end", Primitive::OverlayEnd),
    ("overlay-get", Primitive::OverlayGet),
    ("overlay-put", Primitive::OverlayPut),
    ("overlays-at", Primitive::OverlaysAt),
    ("overlays-in", Primitive::OverlaysIn),
    ("next-overlay-change", Primitive::NextOverlayChange),
    ("remove-overlays", Primitive::RemoveOverlays),
    ("consp", Primitive::Consp),
    ("car-safe", Primitive::CarSafe),
    ("symbolp", Primitive::Symbolp),
    ("define-error", Primitive::DefineError),
    ("error-message-string", Primitive::ErrorMessage),
    ("make-hash-table", Primitive::MakeHashTable),
    ("gethash", Primitive::Gethash),
    ("puthash", Primitive::Puthash),
    ("remhash", Primitive::Remhash),
    ("clrhash", Primitive::Clrhash),
    ("maphash", Primitive::Maphash),
    ("hash-table-count", Primitive::HashTableCount),
    ("hash-table-p", Primitive::HashTableP),
    ("hash-table-test", Primitive::HashTableTest),
    ("hash-table-size", Primitive::HashTableSize),
    ("hash-table-weakness", Primitive::HashTableWeakness),
    ("hash-table-rehash-size", Primitive::HashTableRehashSize),
    (
        "hash-table-rehash-threshold",
        Primitive::HashTableRehashThreshold,
    ),
    ("copy-hash-table", Primitive::CopyHashTable),
    ("define-hash-table-test", Primitive::DefineHashTableTest),
    ("hash-table-keys", Primitive::HashTableKeys),
    ("hash-table-values", Primitive::HashTableValues),
    ("hash-table-contains-p", Primitive::HashTableContainsP),
    ("#:vm2-hash-next", Primitive::HashNext),
    ("#:vm2-hash-key", Primitive::HashKeyAt),
    ("#:vm2-hash-value", Primitive::HashValueAt),
    ("integerp", Primitive::Integerp),
    ("fixnump", Primitive::Fixnump),
    ("bignump", Primitive::Bignump),
    ("looking-at", Primitive::LookingAt),
    ("string-match", Primitive::StringMatch),
    ("match-string", Primitive::MatchString),
    ("match-beginning", Primitive::MatchBeginning),
    ("match-end", Primitive::MatchEnd),
    ("match-data", Primitive::MatchData),
    ("re-search-forward", Primitive::ReSearchForward),
    ("re-search-backward", Primitive::ReSearchBackward),
    ("posix-looking-at", Primitive::PosixLookingAt),
    ("posix-string-match", Primitive::PosixStringMatch),
    ("posix-search-forward", Primitive::PosixSearchForward),
    ("posix-search-backward", Primitive::PosixSearchBackward),
    ("syntax-table", Primitive::SyntaxTable),
    ("standard-syntax-table", Primitive::StandardSyntaxTable),
    ("copy-syntax-table", Primitive::CopySyntaxTable),
    ("set-syntax-table", Primitive::SetSyntaxTable),
    ("modify-syntax-entry", Primitive::ModifySyntaxEntry),
    ("#:vm2-defface", Primitive::DisplayDefineFace),
    ("selected-window", Primitive::SelectedWindow),
    ("selected-frame", Primitive::SelectedFrame),
    ("window-buffer", Primitive::WindowBuffer),
    ("window-start", Primitive::WindowStart),
    ("window-point", Primitive::WindowPoint),
    ("frame-pixel-width", Primitive::FramePixelWidth),
    ("frame-pixel-height", Primitive::FramePixelHeight),
    ("fset", Primitive::Fset),
    ("window-vscroll", Primitive::WindowVscroll),
    ("set-window-vscroll", Primitive::SetWindowVscroll),
    ("error", Primitive::DisplayError),
    ("delete-window-internal", Primitive::DeleteWindowInternal),
    ("frame-selected-window", Primitive::FrameSelectedWindow),
    ("minibuffer-window", Primitive::MinibufferWindow),
    ("set-window-margins", Primitive::SetWindowMargins),
    ("set-window-parameter", Primitive::SetWindowParameter),
    ("set-window-point", Primitive::SetWindowPoint),
    ("set-window-start", Primitive::SetWindowStart),
    ("window-total-height", Primitive::WindowTotalHeight),
    ("split-window-internal", Primitive::SplitWindowInternal),
    ("window-resize-apply", Primitive::WindowResizeApply),
    ("set-window-new-pixel", Primitive::SetWindowNewPixel),
    ("window-new-pixel", Primitive::WindowNewPixel),
    ("window-parent", Primitive::WindowParent),
    ("window-next-sibling", Primitive::WindowNextSibling),
    ("window-prev-sibling", Primitive::WindowPrevSibling),
    ("window-child", Primitive::WindowChild),
    ("frame-root-window", Primitive::FrameRootWindow),
    ("windowp", Primitive::Windowp),
    ("window-live-p", Primitive::WindowLiveP),
    ("window-parameter", Primitive::WindowParameter),
    ("window-parameters", Primitive::WindowParameters),
    ("window-margins", Primitive::WindowMargins),
    ("window-pixel-width", Primitive::WindowPixelWidth),
    ("window-pixel-height", Primitive::WindowPixelHeight),
    ("window-total-width", Primitive::WindowTotalWidth),

    ("define-key", Primitive::DefineKey),
    ("make-sparse-keymap", Primitive::MakeSparseKeymap),
    ("keymapp", Primitive::Keymapp),
    ("key-binding", Primitive::KeyBinding),
    // `lookup-key' is a T2 subr (prims/keymap.rs) so menu filters can run.
    ("#:vm2-lookup-key", Primitive::LookupKey),
    ("global-set-key", Primitive::GlobalSetKey),
    ("current-global-map", Primitive::CurrentGlobalMap),
    ("use-local-map", Primitive::UseLocalMap),
    ("kbd", Primitive::Kbd),
    ("make-obsolete", Primitive::MakeObsolete),
    ("make-obsolete-variable", Primitive::MakeObsoleteVariable),
    ("set-advertised-calling-convention", Primitive::SetAdvertisedCallingConvention),
    ("symbol-value", Primitive::SymbolValue),
    ("special-variable-p", Primitive::SpecialVariableP),
    ("make-variable-buffer-local", Primitive::MakeVariableBufferLocal),
    ("custom-variable-p", Primitive::CustomVariableP),
    ("#:vm2-defvar-boundp", Primitive::DefvarBoundp),
    ("#:vm2-defvar-set-default", Primitive::DefvarSetDefault),
    ("equal", Primitive::DeepEqual),
    ("min", Primitive::Min),
    ("append", Primitive::Append),
    ("get-buffer", Primitive::GetBuffer),
    ("forward-line", Primitive::ForwardLine),
    ("switch-to-buffer", Primitive::SwitchToBuffer),
    ("emacs-lisp-mode", Primitive::EmacsLispMode),
    ("menu-bar-mode", Primitive::MenuBarMode),
    ("force-mode-line-update", Primitive::ForceModeLineUpdate),
    ("locate-library", Primitive::LocateLibrary),
    ("daemonp", Primitive::Daemonp),
    ("daemon-initialized", Primitive::DaemonInitialized),
    ("fboundp", Primitive::Fboundp),
    ("set", Primitive::Set),
    ("makunbound", Primitive::Makunbound),
    ("make-local-variable", Primitive::MakeLocalVariable),
    ("#:vm2-temp-buffer", Primitive::TempBuffer),
    // Table index must equal the enum discriminant: append only.
    ("autoload-do-load", Primitive::AutoloadDoLoad),
    ("romacs--read-host-event", Primitive::ReadHostEvent),
    ("format-mode-line", Primitive::FormatModeLine),
    ("macroexpand", Primitive::Macroexpand),
    ("#:vm2-closure-metadata", Primitive::ClosureMetadata),
    ("#:vm2-closure-document", Primitive::ClosureDocument),
    ("make-interpreted-closure", Primitive::MakeInterpretedClosure),
    ("search-backward", Primitive::SearchBackward),
];
#[cfg(test)]
mod rest_tests;

#[derive(Clone, Copy, Debug)]
enum Op {
    Const(usize, Value),
    Move(usize, usize),
    Cell(usize),
    ReadCell(usize, usize),
    WriteCell(usize, usize),
    Global(usize, usize),
    SetGlobal(usize, usize),
    Bind(usize, usize),
    Unbind(usize),
    Jump(usize),
    Unless(usize, usize),
    Closure(usize, usize, usize),
    Define(usize, usize),
    DefineMacro(usize, usize),
    DeclareSpecial(usize),
    Call(usize, usize, usize, usize),
    Direct(usize, usize, usize, usize, usize),
    /// A call GNU compiles to a byte code (sealed.rs): the primitive itself,
    /// never the function cell. (destination, symbol slot, first argument, count)
    Sealed(usize, usize, usize, usize),
    Return(usize),
    Catch(usize, usize, usize),
    Condition(usize, Value, usize),
    PopHandlers(usize),
    Protect(usize, usize, usize),
    LeaveProtect(usize),
    ResumeExit,
}
romacs_snapshot::snapshot_enum!(Op {
    Op::Const(f0, f1) => (f0, f1),
    Op::Move(f0, f1) => (f0, f1),
    Op::Cell(f0) => (f0),
    Op::ReadCell(f0, f1) => (f0, f1),
    Op::WriteCell(f0, f1) => (f0, f1),
    Op::Global(f0, f1) => (f0, f1),
    Op::SetGlobal(f0, f1) => (f0, f1),
    Op::Bind(f0, f1) => (f0, f1),
    Op::Unbind(f0) => (f0),
    Op::Jump(f0) => (f0),
    Op::Unless(f0, f1) => (f0, f1),
    Op::Closure(f0, f1, f2) => (f0, f1, f2),
    Op::Define(f0, f1) => (f0, f1),
    Op::DefineMacro(f0, f1) => (f0, f1),
    Op::DeclareSpecial(f0) => (f0),
    Op::Call(f0, f1, f2, f3) => (f0, f1, f2, f3),
    Op::Direct(f0, f1, f2, f3, f4) => (f0, f1, f2, f3, f4),
    Op::Sealed(f0, f1, f2, f3) => (f0, f1, f2, f3),
    Op::Return(f0) => (f0),
    Op::Catch(f0, f1, f2) => (f0, f1, f2),
    Op::Condition(f0, f1, f2) => (f0, f1, f2),
    Op::PopHandlers(f0) => (f0),
    Op::Protect(f0, f1, f2) => (f0, f1, f2),
    Op::LeaveProtect(f0) => (f0),
    Op::ResumeExit => (),
});
impl Op {
    // Register operands only: slots, function ids, branch targets, and arities
    // are different coordinate spaces. Argument windows stay contiguous because
    // capture pruning removes only unused registers from the frame prefix.
    fn map_registers(&mut self, map: &mut impl FnMut(usize) -> usize) {
        match self {
            Self::Const(d, _)
            | Self::Global(d, _)
            | Self::Closure(d, _, _)
            | Self::Condition(d, _, _)
            | Self::Protect(d, _, _) => *d = map(*d),
            Self::Move(d, s) | Self::ReadCell(d, s) | Self::WriteCell(d, s) => {
                *d = map(*d);
                *s = map(*s);
            }
            Self::Cell(r)
            | Self::SetGlobal(_, r)
            | Self::Bind(_, r)
            | Self::Define(_, r)
            | Self::DefineMacro(_, r)
            | Self::Return(r)
            | Self::LeaveProtect(r)
            | Self::Unless(r, _) => *r = map(*r),
            Self::Call(d, target, start, _) => {
                *d = map(*d);
                *target = map(*target);
                *start = map(*start);
            }
            Self::Direct(d, _, _, start, _) | Self::Sealed(d, _, start, _) => {
                *d = map(*d);
                *start = map(*start);
            }
            Self::Catch(d, tag, _) => {
                *d = map(*d);
                *tag = map(*tag);
            }
            Self::Unbind(_)
            | Self::Jump(_)
            | Self::DeclareSpecial(_)
            | Self::PopHandlers(_)
            | Self::ResumeExit => {}
        }
    }
    fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        match self {
            Self::Const(_, v) | Self::Condition(_, v, _) => visit(v),
            _ => {}
        }
    }
}
#[derive(Clone, Debug)]
struct Function {
    // Compiler-only derived metadata for newly compiled closure trees. Imported
    // executable bodies are never children of a new lexical closure site.
    capture_mutations: Vec<bool>,
    mapped_code: Option<usize>,
    code: Vec<Op>,
    regs: usize,
    captures: usize,
    arity: usize,
    required: usize,
    rest: bool,
    sites: Vec<Vec<usize>>,
    /// R215: a body held UNEXPANDED until its first call, as GNU's evaluator
    /// holds an interpreted closure's body before macroexp.el is loaded.
    /// `code` is empty while this is present; every frame entry forces it.
    lazy: Option<Arc<LazyBody>>,
    /// Whether it was compiled under dynamic binding, recorded where its
    /// closure site sees no lexical binding: GNU's interpreted closure
    /// keeps its whole visible environment, so there `(aref CLOSURE 2)'
    /// is the dialect's alone, nil or (t). Code images and older cache
    /// entries do not record it.
    dynamic: Option<bool>,
    /// A compiled-Lisp body not yet decoded (`compiled_lisp`), decoded at
    /// its first call as a `mapped_code` body is. `code` is empty while it
    /// is here.
    pending: Option<Box<PendingBody>>,
    /// The lambda list as written, GNU's interpreted-closure slot 0
    /// (`(aref CLOSURE 0)', which `help-function-arglist' reads). Code
    /// images and source-cache entries holding a generated symbol do not
    /// record it.
    params: Option<Vec<Form>>,
}
romacs_snapshot::snapshot_struct!(Function { capture_mutations, mapped_code, code, regs, captures, arity, required, rest, sites, lazy, dynamic, pending, params });
/// A function body a compiled load left in its artifact's bytes
/// (`Vm::decode_pending`), with what the body names: a collection keeps
/// its VALUES (moving them with the heap) and CHILDREN alive, and its
/// inline CACHES claimed, as it would through decoded code.
#[derive(Clone, Debug)]
struct PendingBody {
    bytes: Arc<Vec<u8>>,
    body: std::ops::Range<usize>,
    values: Vec<Value>,
    children: Vec<usize>,
    caches: Vec<usize>,
    /// The artifact's names, as slots.
    names: Arc<Vec<usize>>,
}
romacs_snapshot::snapshot_struct!(PendingBody { bytes, body, values, children, caches, names });
/// The unexpanded body of a function defined in GNU's non-eager load regime
/// (src/lread.c:2183: `internal-macroexpand-for-load' not yet fbound).
///
/// `body` is ordinary heap Lisp data: the collector marks it through the
/// owning function's code mark and forwards it with every other code Value
/// (gc.rs). It is dropped, and so becomes garbage, when the body is compiled.
/// Everything else is compiler-owned Rust data and needs no tracing. The
/// captured lexical environment is NOT here: it is the closure's ordinary
/// capture vector, one register per visible lexical binding in `names` order.
#[derive(Debug)]
struct LazyBody {
    body: Value,
    args: Vec<Form>,
    names: Vec<Name>,
    cells: Vec<bool>,
    local_specials: NameSet<String>,
    pending_specials: NameSet<String>,
    dynamic_scope: bool,
    /// A form whose expansion failed and was deferred to its reach (GNU
    /// expands each form when evaluation reaches it). Its own root is
    /// compiled strictly, so a failure signals instead of deferring again.
    deferred: bool,
}
romacs_snapshot::snapshot_struct!(LazyBody { body, args, names, cells, local_specials, pending_specials, dynamic_scope, deferred });
#[derive(Clone, Copy)]
struct Binding {
    reg: usize,
    cell: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Name {
    Source(String),
    Generated(GeneratedName),
}
romacs_snapshot::snapshot_enum!(Name {
    Name::Source(f0) => (f0),
    Name::Generated(f0) => (f0),
});
impl Name {
    fn source(&self) -> Option<&str> {
        match self {
            Self::Source(s) => Some(s),
            Self::Generated(_) => None,
        }
    }
}
struct Builder {
    code: Vec<Op>,
    next: usize,
    env: NameMap<Name, Binding>,
    sites: Vec<Vec<usize>>,
    local_specials: NameSet<String>,
    /// GNU's loader order for an unbound head (its loader is its
    /// interpreter): a top-level request (`compile`) or a form deferred to
    /// its reach, never a lambda body nested in one.
    loader_order: bool,
}
impl Builder {
    fn reg(&mut self) -> usize {
        let r = self.next;
        self.next += 1;
        r
    }
    fn emit(&mut self, op: Op) -> usize {
        let i = self.code.len();
        self.code.push(op);
        i
    }
}
/// Validate a lambda list as `Vm::function` does and return
/// (positional parameters, required parameters, &rest present).
fn lambda_arity(args: &[Form]) -> Result<(usize, usize, bool)> {
    let mut positional = 0;
    let mut required = 0;
    let mut optional = false;
    let mut rest = false;
    for (i, arg) in args.iter().enumerate() {
        let name = arg.name()?;
        if name.source() == Some("&optional") && !optional && !rest {
            optional = true;
            continue;
        }
        if name.source() == Some("&rest") && !rest && i + 2 == args.len() {
            rest = true;
            continue;
        }
        // GNU binds a repeated name again; the last binding wins.
        if name.source().is_some_and(|n| n.starts_with('&')) {
            return Err("invalid lambda argument list".into());
        }
        if matches!(name.source(), Some("nil" | "t")) {
            return Err("cannot bind a constant".into());
        }
        if !optional && !rest {
            required += 1;
        }
        positional += 1;
    }
    if rest && positional == 0 {
        return Err("missing rest parameter".into());
    }
    Ok((positional - usize::from(rest), required, rest))
}
#[derive(Clone, Copy)]
struct Cache {
    version: u64,
    target: Value,
}
romacs_snapshot::snapshot_struct!(Cache { version, target });
struct Slot {
    value: Value,
    function: Value,
    version: u64,
    /// GNU `u.s.plist` (lisp.h): the symbol's own property list, owned by the
    /// symbol OBJECT, never by its name. A moving heap Value: a collector
    /// root, marked and forwarded with the slot (gc.rs).
    plist: Value,
    /// Buffer-local binding table (GNU keeps these in the value redirect's
    /// `struct Lisp_Buffer_Local_Value`, never on the plist). NIL when the
    /// variable was never localized. A collector root like `plist`.
    local: Value,
    /// `make-local-variable` without `make-variable-buffer-local`: the table
    /// exists but a plain `set` does not create a local binding.
    explicit_local: bool,
    /// Object-owned declaration flag for uninterned symbols.
    declared_special: bool,
}
romacs_snapshot::snapshot_struct!(Slot { value, function, version, plist, local, explicit_local, declared_special });
impl Slot {
    const fn new() -> Self {
        Self {
            value: Value::UNBOUND,
            function: Value::UNBOUND,
            version: 1,
            plist: Value::NIL,
            local: Value::NIL,
            explicit_local: false,
            declared_special: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VariableRedirect {
    Plain,
    /// GNU `SYMBOL_LOCALIZED`: the slot's own `local` table holds per-buffer
    /// bindings. Set-only: never cleared once published.
    Localized,
    /// Native Boolean defaults with canonical per-buffer values in `local`.
    LocalizedForwarded(forwarded::Fwd),
    /// GNU `SYMBOL_VARALIAS` (lisp.h): this variable IS another one. Holds a
    /// stable slot ID, never a moving heap Value, so it needs no tracing.
    Alias(usize),
    /// GNU `SYMBOL_FORWARDED` (lisp.h): a built-in variable whose value lives
    /// in runtime storage (`forwarded.rs`), never in the symbol's value cell.
    Forwarded(forwarded::Fwd),
}
romacs_snapshot::snapshot_enum!(VariableRedirect {
    VariableRedirect::Plain => (),
    VariableRedirect::Localized => (),
    VariableRedirect::LocalizedForwarded(f0) => (f0),
    VariableRedirect::Alias(f0) => (f0),
    VariableRedirect::Forwarded(f0) => (f0),
});
struct Closure {
    function: usize,
    captures: Vec<Value>,
    metadata: Vec<Value>,
    document: Value,
}
romacs_snapshot::snapshot_struct!(Closure { function, captures, metadata, document });

/// Source interning is compiler-owned; cold faults intern diagnostics. Hot calls use slots and guards.
#[cfg(test)]
#[path = "window_tests.rs"]
mod window_tests;

/// A symbol's declaration as the compiler sees it (`Vm::special_kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SpecialKind {
    Lexical = 0,
    FileLocal = 1,
    Global = 2,
}
romacs_snapshot::snapshot_enum!(SpecialKind {
    SpecialKind::Lexical => (),
    SpecialKind::FileLocal => (),
    SpecialKind::Global => (),
});
/// One recent_keys entry: an input character or event symbol, or the
/// command it ran, which `recent-keys' shows as (nil . COMMAND).
#[derive(Clone)]
pub(crate) enum RecentKey { Char(i64), Event(String), Command(String) }
romacs_snapshot::snapshot_enum!(RecentKey {
    RecentKey::Char(f0) => (f0),
    RecentKey::Event(f0) => (f0),
    RecentKey::Command(f0) => (f0),
});
/// lossage_limit's default.
pub(crate) const RECENT_KEYS: usize = 300;

/// timefns.c's local time zone after set-time-zone-rule: the process's
/// TZ (the start), the system wall clock (nil or `wall'), GNU's utc_tz
/// (t or 0), or a rule string.
#[derive(Clone)]
pub(crate) enum LocalZone { Process, Wall, Utc, Rule(String) }
romacs_snapshot::snapshot_enum!(LocalZone {
    LocalZone::Process => (),
    LocalZone::Wall => (),
    LocalZone::Utc => (),
    LocalZone::Rule(f0) => (f0),
});

pub struct Vm {
    charsets: charset::Registry,
    standard_case_tables: [Value; 4],
    /// undo.c's point_before_last_command_or_undo and its buffer.
    command_point: (i64, Value),
    /// A storage kernel runs inside insdel's bracket, which checked read-only.
    change_prepared: bool,
    /// The storage kernels' argument stack, reused across changes
    /// (`insdel::edit_kernel').
    kernel_stack: Vec<Value>,
    /// The slots insdel's bracket reads on every change, once resolved.
    change_slots: Option<insdel::ChangeSlots>,
    case_symbols_slot: usize,
    /// print.c's print_number_index: the last print-circle label assigned.
    print_number_index: usize,
    /// buffer-undo-list's per-buffer index, once looked up.
    undo_list_slot: u16,
    display_state: display::DisplayState,
    window_state: window::WindowState,
    world_version: u64,
    #[cfg(test)]
    variable_read_probes: std::cell::Cell<usize>,
    #[cfg(test)]
    speculative_hits: usize,
    #[cfg(test)]
    inline_hits: usize,
    #[cfg(test)]
    disable_inline: bool,
    #[cfg(test)]
    speculative_misses: usize,
    regexp: regexp::MatchState,
    regexp_cache: regexp::PatternCache,
    execution_fatal: bool,
    loader: loader::Loader,
    source_cache: source_cache::SourceCache,
    macros: HashSet<usize>,
    expanding: Vec<usize>,
    compile_depth: usize,
    /// Active Lisp evaluation depth, shared by reentrant execution engines.
    eval_depth: usize,
    /// GNU `eval` with a nil LEXICAL argument: every binding the form makes
    /// is dynamic (src/eval.c Feval, `internal-interpreter-environment' nil).
    dynamic_scope: bool,
    /// An escape leaving a reentrant `eval`'s engine, re-raised by the caller's
    /// engine. Traced by the collector while present.
    propagated_escape: Option<threaded::Escape>,
    signal_handlers: Vec<threaded::SignalHandler>,
    backtrace: backtrace::Backtrace,
    /// profiler.c's CPU and memory profilers (`profiler.rs`).
    profiler: profiler::Profiler,
    propagated_escape_hooked: bool,
    propagated_eval_depth: Option<usize>,
    /// Set immediately before executing an `eval` form; consumed by the engine.
    propagate_next: bool,
    /// The embedder's keyboard while one is attached (host_input.rs).
    host_input: Option<Box<dyn host_input::HostInput>>,
    /// The embedder's layout while one is attached (layout_host.rs).
    layout_host: Option<Box<dyn layout_host::LayoutHost>>,
    /// The embedder's window system while one is attached (display_host.rs).
    display: Option<display_host::Display>,
    /// The embedder's end of the session while one is attached (session_host.rs).
    session_host: Option<session_host::SessionHost>,
    /// keyboard.c timer_idleness_start_time: when the command loop began
    /// waiting idle for a key, while it does.
    idle_since: Option<std::time::Instant>,
    /// The host's `C-g', shared with its input thread (quit.rs).
    quit: std::sync::Arc<quit::QuitSignal>,
    /// keyboard.c force_quit_count: `C-g's in a row that found `quit-flag' set.
    force_quit_count: u8,
    /// `quit-flag', `inhibit-quit' and `throw-on-input''s symbol slots, for
    /// the evaluator's quit polls (quit.rs).
    quit_slots: [usize; 3],
    /// Counts keymap stores and parent changes, GNU's two flushes of the
    /// reverse-map cache `where-is-internal' keeps (keymap.c where_is_cache).
    keymap_generation: u64,
    process_jobs: process_jobs::Jobs,
    /// GNU process.c chan_process and friends: subprocess descriptors.
    subprocesses: subprocess::Table,
    environment_snapshot: Option<Vec<Vec<u8>>>,
    /// SYMBOL_TRAPPED_WRITE base slots: variables with watchers (watchers.rs).
    trapped: HashSet<usize>,
    /// etc/DOC as last read, keyed by path and modification time (prims/doc.rs).
    doc_file: Option<(std::path::PathBuf, std::time::SystemTime, std::sync::Arc<Vec<u8>>)>,
    terminal_host: Option<Box<dyn terminal_host::TerminalHost>>,
    terminal_bindings: Vec<terminal_host::TerminalBinding>,
    pending_specials: NameSet<String>,
    /// Names a one-argument top-level `(defvar NAME)` declared in the file
    /// being loaded: special for the rest of that file only (GNU Fdefvar,
    /// eval.c:990-996, consing onto the `internal-interpreter-environment'
    /// that readevalloop binds once per file). Saved and restored per load.
    file_specials: NameSet<String>,
    /// Set by the loader for exactly one root compilation of a file's form,
    /// so that compilation (and nothing nested in it) starts from
    /// `file_specials`.
    seed_file_specials: bool,
    /// R215: compile `lambda' bodies lazily (GNU's non-eager load regime).
    lazy_regime: bool,
    /// R215: compile a failing form as a deferred reach-time expansion.
    defer_expansion_errors: bool,
    /// Set for the builder about to be created (`Builder::loader_order`).
    loader_order_request: bool,
    /// Consumed by the next `function` call: keep every capture register.
    keep_captures: bool,
    /// Eager `lambda' bodies being compiled around the current form.
    lambda_depth: usize,
    /// The windows the last window change functions run saw
    /// (host_input.rs `window_change_signature').
    window_change_seen: Vec<u64>,
    expanded_macros: Vec<(usize, u64)>,
    expanded_symbols: NameSet<String>,
    image: Option<std::sync::Arc<ImageRegion>>,
    /// Frozen regions by id (`frozen.rs`); index 0 is the image above.
    regions: Vec<Option<Box<frozen::Region>>>,
    region_free: Vec<usize>,
    /// Three quarters of the region ids are live: collect at the next entry.
    region_collect: bool,
    /// Worker isolates and, in a worker, the parent peer (`isolate.rs`).
    isolates: isolate::Isolates,
    /// GNU inotify.c inotifyfd: the file notification instance, made by
    /// the first watch and closed with the last (`prim/leaf/inotify.rs`).
    inotify: Option<std::os::fd::OwnedFd>,
    buffer_slot: usize,
    pub(crate) multibyte_slot: usize,
    truncate_lines_slot: usize,
    /// Owner of syntax.c's defun-start cache (`BufferState::defun_start`).
    defun_start_epoch: u64,
    variable_redirects: Vec<Option<VariableRedirect>>,
    /// Storage of GNU's built-in (C) variables; collector roots (`gc.rs`).
    forwarded: forwarded::Forwarded,
    fixnum_slots: [usize; 2],
    local_binding_slots: HashSet<usize>,
    free_local_binding_slots: Vec<usize>,
    gc_stats: HeapStats,
    gc_retained: usize,
    /// Compiled code units (IR ops, lowered instructions) the last
    /// collection traced: with `gc_retained', what a collection traverses.
    gc_code: usize,
    /// An embedder's pacing floor (`set_collection_floor'), which replaces
    /// the Lisp pacing variables while set.
    gc_floor: Option<usize>,
    /// Open frames that keep heap Values in untraced Rust storage across a
    /// call that can run Lisp: compiler builders and watcher notifications
    /// from native writers (`gc.rs`, `Vm::may_collect`).
    gc_holds: usize,
    /// Engines between entry and exit, suspended or not; those not in
    /// `suspended_code` hold untraced registers and code.
    running_engines: usize,
    /// The writer notifying watchers is an engine write suspended for it,
    /// whose own Values the write roots (`threaded::watched_write`).
    rooted_write: bool,
    /// Test mode: a moving collection at every Nth Lisp entry (`gc.rs`).
    gc_stress: gc::Stress,
    names: NameMap<String, usize>,
    /// By slot, whether `names' maps the slot's name to it: membership of
    /// the initial obarray, answered without hashing the name.
    initial_obarray: Vec<bool>,
    symbol_names: Vec<String>,
    obarray_symbol_names: HashMap<usize, Value>,
    permanent_symbol_names: HashMap<u64, Value>,
    native_case_roots: [std::sync::Weak<std::sync::Mutex<char_table::CharTableNode>>; 4],
    obarray_free_slots: Vec<usize>,
    generated_symbols: HashMap<GeneratedName, usize>,
    generated_slots: HashMap<usize, GeneratedName>,
    slots: Vec<Slot>,
    specials: NameSet<String>,
    functions: Vec<Arc<Function>>,
    request_code: request_code::RequestCode,
    register_width: usize,
    /// Registers kept free at the stack's end for `apply`'s spread
    /// arguments; it grows to fit the longest spread (GNU has no limit).
    spread_reserve: usize,
    execution: threaded::Program,
    /// Programs of finished nested engine runs (registers all NIL again,
    /// lowered code kept), reused by the next nested run
    /// (`threaded::Vm::end_nested_execution`). Code invalidation and the
    /// collector reach them as they reach `execution`.
    spare_programs: Vec<threaded::Program>,
    closures: Vec<Closure>,
    cells: Vec<Value>,
    cell_pairs: HashMap<usize, Value>,
    /// fns.c `random's generator, seeded from entropy as at GNU startup.
    random: random::Random,
    /// A world build's P fence (`world`); None outside build mode.
    fence: Option<Box<world::Fence>>,
    /// glibc's localtime_offset: the last mktime result's difference from
    /// its local time, where the next encode-time starts its search.
    mktime_offset: i64,
    /// glibc's rule_dstoff, which __tzfile_default lends a rule-less TZ.
    tz_rule_dstoff: i64,
    /// The local time zone set-time-zone-rule installed.
    local_zone: LocalZone,
    /// keyboard.c recent_keys: the last input events and the commands they
    /// ran, as names (the heap may compact under the command loop).
    recent_keys: std::collections::VecDeque<RecentKey>,
    /// keyboard.c lossage_limit: how many RECENT_KEYS entries are kept.
    lossage_limit: usize,
    conses: Vec<(Value, Value)>,
    objects: Vec<data::Object>,
    /// The objects that name a buffer: each buffer's markers and overlays
    /// (GNU's BUF_MARKERS and overlay trees) and the windows, so a buffer's
    /// or a window are found without walking the heap. None until
    /// `referrers` lists them again after a collection moved objects; it
    /// may name dead ones.
    referrers: Option<buffer::Referrers>,
    /// The slots of the variables each buffer, by its index in `objects`,
    /// has a Lisp local binding of: GNU's per-buffer `local_var_alist`, so
    /// killing a buffer or resetting its locals visits its own bindings,
    /// not every symbol. None until `local_slots` lists them again after a
    /// collection moved objects; it may name slots whose binding is gone.
    local_owners: Option<HashMap<usize, Vec<usize>>>,
    /// The `all-completions' walks begun, and the case ranges the latest
    /// one folds with: (walk, canon table root, ranges)
    /// (prim/leaf/completion.rs `walk_case_ranges').
    completion_walks: u64,
    completion_case_memo: Option<(u64, usize, std::sync::Arc<Vec<(u32, u32, i64)>>)>,
    caches: Vec<Cache>,
    stack: Vec<Value>,
    external_roots: Vec<Value>,
    /// alloc.c's `finalizers': every finalizer not yet found unreachable.
    /// Not roots; the collector dooms those it does not reach.
    finalizers: Vec<Value>,
    /// alloc.c's `doomed_finalizers': unreachable finalizers kept alive
    /// until `romacs--run-finalizers' calls their functions.
    doomed_finalizers: Vec<Value>,
    /// Register stacks of engines suspended across a nested run, traced and
    /// forwarded in place: their frames keep raw pointers into them.
    suspended_stacks: Vec<Vec<Value>>,
    /// Suspended engines' code and entry caches, traced and forwarded in place.
    suspended_code: Vec<threaded::SuspendedCode>,
    /// Menu-item `:filter' results for the keymap walk in progress, as
    /// (menu item, result); set and cleared around one walk, which never
    /// collects.
    keymap_filters: Vec<(Value, Value)>,
    /// The first filter that walk needed: (menu item, filter, definition).
    keymap_pending_filter: Option<(Value, Value, Value)>,
    dynamic: Vec<(usize, Value, Value, bool)>,
    conditions: conditions::Registry,
    /// Embedder `Rooted` handles; see handle.rs.
    rooted: handle::RootSet,
}
// What a world image may hold of a VM (`world::safe`): every field, each
// SnapshotSafe or, at rest, checked by `world_quiescent`: the hosts, the
// children and jobs, the mapped code image, the fence, the lowered code
// (function pointers), the suspended engines (pointers into it), the
// compiled patterns (their SIMD searchers hold function pointers) and the
// idle clock reading (process-relative).
//
// Its fields are listed, not the VM itself made SnapshotSafe: a VM enters
// an image only as the primary, which `world::image::capture` validates,
// never inside a carry, where nothing would.
romacs_snapshot::snapshot_fields!(Vm {
    charsets,
    standard_case_tables,
    command_point,
    change_prepared,
    kernel_stack,
    change_slots,
    case_symbols_slot,
    print_number_index,
    undo_list_slot,
    display_state,
    window_state,
    world_version,
    #[cfg(test)] variable_read_probes,
    #[cfg(test)] speculative_hits,
    #[cfg(test)] inline_hits,
    #[cfg(test)] disable_inline,
    #[cfg(test)] speculative_misses,
    regexp,
    execution_fatal,
    loader,
    source_cache,
    macros,
    expanding,
    compile_depth,
    eval_depth,
    dynamic_scope,
    propagated_escape,
    signal_handlers,
    backtrace,
    propagated_escape_hooked,
    propagated_eval_depth,
    propagate_next,
    keymap_generation,
    environment_snapshot,
    trapped,
    doc_file,
    terminal_bindings,
    pending_specials,
    file_specials,
    seed_file_specials,
    lazy_regime,
    defer_expansion_errors,
    loader_order_request,
    keep_captures,
    lambda_depth,
    window_change_seen,
    expanded_macros,
    expanded_symbols,
    buffer_slot,
    multibyte_slot,
    truncate_lines_slot,
    defun_start_epoch,
    variable_redirects,
    forwarded,
    fixnum_slots,
    local_binding_slots,
    free_local_binding_slots,
    gc_stats,
    gc_retained,
    gc_code,
    gc_holds,
    running_engines,
    rooted_write,
    gc_stress,
    suspended_stacks,
    gc_floor,
    names,
    initial_obarray,
    symbol_names,
    obarray_symbol_names,
    permanent_symbol_names,
    native_case_roots,
    obarray_free_slots,
    generated_symbols,
    generated_slots,
    slots,
    specials,
    functions,
    request_code,
    register_width,
    closures,
    cells,
    cell_pairs,
    random,
    mktime_offset,
    tz_rule_dstoff,
    local_zone,
    recent_keys,
    lossage_limit,
    conses,
    objects,
    referrers,
    local_owners,
    completion_walks,
    completion_case_memo,
    caches,
    stack,
    external_roots,
    finalizers,
    doomed_finalizers,
    keymap_filters,
    keymap_pending_filter,
    dynamic,
    conditions,
    rooted,
    spread_reserve,
    quit,
    force_quit_count,
    quit_slots,
} at_rest {
    regexp_cache,
    suspended_code,
    host_input,
    layout_host,
    display,
    session_host,
    terminal_host,
    subprocesses,
    process_jobs,
    image,
    regions,
    region_free,
    region_collect,
    isolates,
    inotify,
    fence,
    execution,
    spare_programs,
    idle_since,
    profiler,
});

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}
impl Vm {
    /// Embedder pacing in place of `gc-cons-threshold' and
    /// `gc-cons-percentage': collect once the heap holds at least OBJECTS and
    /// twice what the last collection retained. Hosts and tests use it to
    /// force or defer collection. Checked only at strand wait points; not a
    /// hard memory limit.
    pub fn set_collection_floor(&mut self, objects: usize) {
        self.gc_floor = Some(objects.max(1));
    }
    /// The collector's measure of the heap: private objects plus the
    /// weight of the frozen regions attached (`frozen.rs`).
    pub fn heap_objects(&self) -> usize {
        self.private_objects() + self.region_weight()
    }
    pub(crate) fn private_objects(&self) -> usize {
        self.conses.len() + self.cells.len() + self.closures.len() + self.objects.len()
    }
    pub fn new() -> Self {
        // emacs_wd is taken when Emacs starts, not when first used.
        let _ = environment::startup_directory();
        let mut vm = Self {
            charsets: charset::Registry::default(),
            standard_case_tables: [Value::NIL; 4],
            command_point: (0, Value::NIL),
            change_prepared: false,
            kernel_stack: Vec::new(),
            change_slots: None,
            case_symbols_slot: usize::MAX,
            print_number_index: 0,
            undo_list_slot: u16::MAX,
            display_state: display::DisplayState::new(),
            window_state: window::WindowState::default(),
            world_version: 1,
            #[cfg(test)]
            variable_read_probes: std::cell::Cell::new(0),
            #[cfg(test)]
            speculative_hits: 0,
            #[cfg(test)]
            inline_hits: 0,
            #[cfg(test)]
            disable_inline: false,
            #[cfg(test)]
            speculative_misses: 0,
            regexp: regexp::MatchState::default(),
            regexp_cache: regexp::PatternCache::default(),
            execution_fatal: false,
            loader: loader::Loader::default(),
            source_cache: source_cache::SourceCache::default(),
            macros: HashSet::default(),
            expanding: vec![],
            compile_depth: 0,
            eval_depth: 0,
            dynamic_scope: false,
            propagated_escape: None,
            signal_handlers: Vec::new(),
            backtrace: backtrace::Backtrace::default(),
            profiler: profiler::Profiler::default(),
            propagated_escape_hooked: false,
            propagated_eval_depth: None,
            host_input: None,
            layout_host: None,
            display: None,
            session_host: None,
            idle_since: None,
            quit: std::sync::Arc::default(),
            force_quit_count: 0,
            quit_slots: [0; 3],
            keymap_generation: 0,
            process_jobs: process_jobs::Jobs::default(),
            subprocesses: subprocess::Table::default(),
            environment_snapshot: Some(environment::snapshot()),
            trapped: HashSet::default(),
            doc_file: None,
            terminal_host: None,
            terminal_bindings: Vec::new(),
            propagate_next: false,
            pending_specials: NameSet::default(),
            file_specials: NameSet::default(),
            seed_file_specials: false,
            lazy_regime: false,
            defer_expansion_errors: false,
            loader_order_request: false,
            keep_captures: false,
            lambda_depth: 0,
            window_change_seen: Vec::new(),
            expanded_macros: vec![],
            expanded_symbols: NameSet::default(),
            image: None,
            regions: vec![None],
            region_free: Vec::new(),
            region_collect: false,
            isolates: isolate::Isolates::default(),
            inotify: None,
            buffer_slot: 0,
            multibyte_slot: 0,
            truncate_lines_slot: usize::MAX,
            defun_start_epoch: 0,
            variable_redirects: Vec::new(),
            forwarded: forwarded::Forwarded::default(),
            fixnum_slots: [usize::MAX; 2],
            local_binding_slots: HashSet::default(),
            free_local_binding_slots: Vec::new(),
            gc_stats: HeapStats::default(),
            gc_retained: 0,
            gc_code: 0,
            gc_floor: None,
            gc_holds: 0,
            running_engines: 0,
            rooted_write: false,
            gc_stress: gc::Stress::from_env(),
            names: NameMap::default(),
            initial_obarray: Vec::new(),
            symbol_names: vec![],
            obarray_symbol_names: HashMap::default(),
            permanent_symbol_names: HashMap::default(),
            native_case_roots: std::array::from_fn(|_| std::sync::Weak::new()),
            obarray_free_slots: Vec::new(),
            generated_symbols: HashMap::default(),
            generated_slots: HashMap::default(),
            slots: vec![],
            specials: NameSet::default(),
            functions: vec![],
            request_code: request_code::RequestCode::default(),
            register_width: 1,
            spread_reserve: 4096,
            execution: threaded::Program::default(),
            spare_programs: Vec::new(),
            closures: vec![],
            cells: vec![],
            cell_pairs: HashMap::default(),
            random: random::Random::from_entropy(),
            fence: None,
            mktime_offset: 0,
            tz_rule_dstoff: 0,
            local_zone: LocalZone::Process,
            recent_keys: std::collections::VecDeque::new(),
            lossage_limit: RECENT_KEYS,
            conses: vec![],
            objects: vec![],
            referrers: None,
            local_owners: None,
            completion_walks: 0,
            completion_case_memo: None,
            caches: vec![],
            stack: vec![],
            external_roots: vec![],
            finalizers: vec![],
            doomed_finalizers: vec![],
            suspended_stacks: vec![],
            suspended_code: vec![],
            keymap_filters: Vec::new(),
            keymap_pending_filter: None,
            dynamic: vec![],
            conditions: conditions::Registry::default(),
            rooted: handle::RootSet::default(),
        };
        for index in 0..PRIMITIVES.len() {
            let d = prim::descriptor(index).unwrap();
            let i = vm.slot(d.name());
            vm.slots[i].function = Value::reference(index, 6);
        }
        vm.register_generated();
        let not = vm.slot("not");
        vm.slots[not].function = Value::reference(Primitive::Null as usize, 6);
        vm.declare_special_raw("#:vm2-current-buffer", Value::NIL);
        vm.buffer_slot = vm.slot("#:vm2-current-buffer");
        vm.declare_special_raw("enable-multibyte-characters", Value::TRUE);
        vm.multibyte_slot = vm.slot("enable-multibyte-characters");
        vm.declare_special_raw(
            "most-positive-fixnum",
            Value::integer(Value::MAX_FIXNUM).unwrap(),
        );
        vm.declare_special_raw(
            "most-negative-fixnum",
            Value::integer(Value::MIN_FIXNUM).unwrap(),
        );
        vm.fixnum_slots = [
            vm.slot("most-positive-fixnum"),
            vm.slot("most-negative-fixnum"),
        ];
        vm.initialize_loader();
        vm.initialize_autoload();
        vm.declare_fileio_specials();
        vm.specials.extend(["process-environment".into(), "initial-environment".into(), "exec-path".into()]);
        // C specials are special before their symbols are first interned, so
        // already-read forms compile their bindings dynamically.
        vm.specials.extend(forwarded::C_SPECIALS.iter().map(|v| v.name)
            .filter(|name| !forwarded::NOT_SPECIAL.contains(name)).map(str::to_owned));
        vm.quit_slots = ["quit-flag", "inhibit-quit", "throw-on-input"].map(|name| vm.slot(name));
        vm.declare_special_raw("inhibit-eol-conversion", Value::NIL);
        vm.declare_special_raw("text-quoting-style", Value::NIL);
        vm.declare_special_raw("internal--text-quoting-flag", Value::TRUE);
        vm.declare_special_raw("default-text-properties", Value::NIL);
        vm.declare_special_raw("char-property-alias-alist", Value::NIL);
        vm.specials.insert("text-property-default-nonsticky".into());
        vm.specials.insert("yank-excluded-properties".into());
        vm.specials.insert("yank-handled-properties".into());
        vm.declare_special_raw("font-lock-defaults", Value::NIL);
        // keyboard.c:13843: the command that entered the innermost
        // minibuffer, which read_minibuf binds (rlisp/vm/minibuf.el).
        vm.declare_special_raw("current-minibuffer-command", Value::NIL);
        for name in [
            "kill-ring",
            "kill-buffer-hook",
            "kill-buffer-query-functions",
            "kill-ring-yank-pointer",
            "yank-undo-function",
            // keyboard.c / minibuf.c DEFVARs that call-interactively reads,
            // saves and restores (callint.c:282-299, 792-795).
            "this-command",
            "this-original-command",
            "real-this-command",
            "real-last-command",
            "last-command",
            "prefix-arg",
            "current-prefix-arg",
            "last-command-event",
            "command-history",
            // keyboard.c this_command_keys, this_single_command_key_start
            // and raw_keybuf (prims/keyboard.rs, rlisp/vm/keyboard.el).
            "romacs--command-keys",
            "romacs--single-command-key-start",
            "romacs--raw-command-keys",
            // minibuf.c / buffer.c DEFVARs read by call-interactively's
            // add-to-history fallback and `*' code. GNU's buffer-read-only
            // is per-buffer; VM2's is one global (DIVERGENCES VM2-CALLINT1).
            "history-delete-duplicates",
            "buffer-read-only",
            "inhibit-read-only",
            "inhibit-mouse-event-check",
        ] {
            vm.declare_special_raw(name, Value::NIL);
        }
        vm.declare_special_raw("kill-ring-max", Value::integer(120).unwrap());
        // minibuf.c: DEFVAR_LISP ("history-length", ...) = make_fixnum (100).
        vm.declare_special_raw("history-length", Value::integer(100).unwrap());
        vm.declare_special_raw("case-fold-search", Value::TRUE);
        vm.declare_special_raw("completion-ignore-case", Value::NIL);
        vm.declare_special_raw("completion-regexp-list", Value::NIL);
        vm.declare_special_raw("case-symbols-as-words", Value::NIL);
        vm.case_symbols_slot = vm.slot("case-symbols-as-words");
        // The automatic local table is initialized lazily by variable_binding_slot.
        vm.declare_special_raw("inhibit-changing-match-data", Value::NIL);
        vm.declare_special_raw("#:vm2-match-buffer", Value::NIL);
        vm.declare_special_raw("#:vm2-standard-syntax", Value::NIL);
        vm.declare_special_raw("parse-sexp-lookup-properties", Value::NIL);
        vm.initialize_conditions();
        for name in ["float-output-format", "print-gensym", "print-continuous-numbering", "print-number-table", "print-integers-as-characters", "print-unreadable-function", "print-charset-text-property", "print-symbols-bare"] {
            vm.declare_special_raw(name, Value::NIL);
        }
        vm.declare_special_raw("print-quoted", Value::TRUE);
        vm.declare_special_raw("standard-output", Value::TRUE);
        // xdisp.c: GNU ERT uses the native log limit when collecting results.
        vm.declare_special_raw("message-log-max", Value::integer(1000).unwrap());
        vm.declare_special_raw("meta-prefix-char", Value::integer(27).unwrap());
        let command_error_default = vm.intern("command-error-default-function");
        vm.declare_special_raw("command-error-function", command_error_default);
        let ignore = vm.intern("ignore");
        vm.declare_special_raw("pre-redisplay-function", ignore);
        vm.declare_special_raw("this-command-keys-shift-translated", Value::NIL);
        vm.declare_special_raw("transient-mark-mode", Value::NIL);
        vm.declare_special_raw("standard-input", Value::TRUE);
        for name in ["print-length", "print-level", "print-escape-nonascii", "print-escape-multibyte", "print-escape-control-characters", "print-circle", "print-escape-newlines", "menu-bar-mode", "truncate-lines"] {
            vm.declare_special_raw(name, Value::NIL);
        }
        for name in ["kill-buffer-hook", "kill-buffer-query-functions", "buffer-list-update-hook"] {
            vm.declare_special_raw(name, Value::NIL);
        }
        vm.declare_special_raw("macroexp--dynvars", Value::NIL);
        vm.truncate_lines_slot = vm.slot("truncate-lines");
        vm.declare_special_raw("menu-bar-lines", Value::integer(0).unwrap());
        vm.specials.insert("major-mode".into());
        // GNU's DEFVAR macros mark every built-in variable special.
        for name in forwarded::Forwarded::names() {
            vm.specials.insert(name.into());
        }
        // Code-image loading reuses this constructor before replacing the
        // symbol catalog. Intrinsic redirects must therefore start cold too;
        // retaining Plain here would mask an image's already-interned plist.
        vm.variable_redirects.clear();
        vm
    }
    /// Publish a semantic function-cell write atomically with both invalidators.
    /// Slot stamps retain precise source-macro dependencies; the world epoch
    /// guards speculative dispatch, including intervening writes to other slots.
    fn define_function(&mut self, slot: usize, value: Value, is_macro: bool) -> Result<()> {
        if self.fence.is_some() {
            self.fence_function(slot);
        }
        let world = self
            .world_version
            .checked_add(1)
            .ok_or("world version overflow")?;
        let version = self.slots[slot]
            .version
            .checked_add(1)
            .ok_or("function version overflow")?;
        self.slots[slot].function = value;
        self.slots[slot].version = version;
        if is_macro {
            self.macros.insert(slot);
        } else {
            self.macros.remove(&slot);
        }
        self.world_version = world;
        Ok(())
    }
    /// Record whether `names' maps SLOT's name to SLOT.
    fn set_initial_obarray(&mut self, slot: usize, interned: bool) {
        if self.initial_obarray.len() <= slot { self.initial_obarray.resize(slot + 1, false); }
        self.initial_obarray[slot] = interned;
    }
    /// Whether SLOT's symbol is interned in the initial obarray.
    fn in_initial_obarray(&self, slot: usize) -> bool {
        self.initial_obarray.get(slot).copied().unwrap_or(false)
    }
    fn slot(&mut self, name: &str) -> usize {
        if let Some(&i) = self.names.get(name) {
            return i;
        }
        let i = self.slots.len();
        self.names.insert(name.to_owned(), i);
        self.set_initial_obarray(i, true);
        self.symbol_names.push(name.to_owned());
        self.slots.push(Slot::new());
        self.initialize_variable_redirect(name, i);
        self.initialize_annotation_slot(name, i);
        self.initialize_case_table_slot(name, i);
        self.initialize_face_slot(name, i);
        self.initialize_environment_slot(name);
        self.initialize_c_special(name, i);
        if name == "charset-list" { self.ensure_charsets(); }
        prim::initialize_coding_slot(self, name);
        if name == "obarray" {
            self.slots[i].value = self.allocate_object(data::Object::Obarray(Box::new(obarray::Obarray::new(3, true))));
            self.specials.insert(name.to_owned());
        }
        if name == "current-prefix-arg" {
            self.slots[i].value = Value::NIL;
            self.specials.insert(name.to_owned());
        }
        i
    }
    fn generated_symbol(&mut self, id: GeneratedName) -> Value {
        let slot = if let Some(&slot) = self.generated_symbols.get(&id) {
            slot
        } else {
            let slot = self.slots.len();
            self.symbol_names.push(format!("#:vm2-temp-{id}"));
            self.slots.push(Slot::new());
            self.generated_symbols.insert(id, slot);
            self.generated_slots.insert(slot, id);
            slot
        };
        Value::reference(slot + 1, 7)
    }
    /// How the compiler treats NAME, for cache validity: the two kinds of
    /// special compile differently (a global special binds lambda parameters
    /// dynamically, a file-local one only `let`, VM2-VD1), so a cache key must
    /// tell them apart.
    pub(crate) fn special_kind(&self, name: &str) -> SpecialKind {
        if self.specials.contains(name) {
            SpecialKind::Global
        } else if self.file_specials.contains(name) {
            SpecialKind::FileLocal
        } else {
            SpecialKind::Lexical
        }
    }
    /// Declare before compilation; only these names get dynamic let bindings.
    pub(crate) fn declare_special_raw(&mut self, name: &str, value: Value) {
        self.specials.insert(name.to_owned());
        let i = self.slot(name);
        self.slots[i].value = value;
    }
    pub fn compile(&mut self, form: &Form) -> Result<usize> {
        if self.compile_depth == 0 {
            self.pending_specials.clear();
            self.expanded_macros.clear();
            self.expanded_symbols.clear();
        }
        self.compile_depth += 1;
        let seed = if std::mem::take(&mut self.seed_file_specials) {
            self.file_specials.clone()
        } else {
            NameSet::default()
        };
        self.loader_order_request = true;
        let result = self
            .function(&[], std::slice::from_ref(form), &NameMap::default(), &seed)
            .map(|(id, _)| id);
        self.compile_depth -= 1;
        if self.compile_depth == 0 {
            self.pending_specials.clear();
        }
        result
    }
    fn install_function(&mut self, function: Function) -> usize {
        self.install_code(function)
    }
    /// Compile one function. Its builder keeps constants in untraced Rust
    /// storage while macro expansion runs Lisp, so it holds collection.
    fn function(
        &mut self,
        args: &[Form],
        body: &[Form],
        outer: &NameMap<Name, Binding>,
        local_specials: &NameSet<String>,
    ) -> Result<(usize, Vec<Name>)> {
        let _unwind = self.unwind_mark();
        self.gc_holds += 1;
        let result = self.build_function(args, body, outer, local_specials);
        self.gc_holds -= 1;
        result
    }
    fn build_function(
        &mut self,
        args: &[Form],
        body: &[Form],
        outer: &NameMap<Name, Binding>,
        local_specials: &NameSet<String>,
    ) -> Result<(usize, Vec<Name>)> {
        let keep_captures = std::mem::take(&mut self.keep_captures);
        let mut b = Builder {
            code: vec![],
            next: 0,
            env: NameMap::default(),
            sites: vec![],
            local_specials: local_specials.clone(),
            loader_order: std::mem::take(&mut self.loader_order_request),
        };
        // Stable order is essential: HashMap iteration order is not a closure ABI.
        let mut names: Vec<_> = outer.keys().cloned().collect();
        names.sort();
        for name in &names {
            let reg = b.reg();
            b.env.insert(
                name.clone(),
                Binding {
                    reg,
                    cell: outer[name].cell,
                },
            );
        }
        let mut dynamic = 0;
        let mut parameters = Vec::new();
        let mut required = 0;
        let mut optional = false;
        let mut rest = false;
        for (i, arg) in args.iter().enumerate() {
            let name = arg.name()?;
            if name.source() == Some("&optional") && !optional && !rest {
                optional = true;
                continue;
            }
            if name.source() == Some("&rest") && !rest && i + 2 == args.len() {
                rest = true;
                continue;
            }
            if name.source().is_some_and(|n| n.starts_with('&')) {
                return Err("invalid lambda argument list".into());
            }
            if !optional && !rest {
                required += 1;
            }
            parameters.push(arg);
        }
        if rest && parameters.is_empty() {
            return Err("missing rest parameter".into());
        }
        for arg in &parameters {
            let name = arg.name()?;
            if matches!(name.source(), Some("nil" | "t")) {
                return Err("cannot bind a constant".into());
            }
            let reg = b.reg();
            // Deliberately does NOT consult `b.local_specials`, unlike `let`.
            // GNU's `funcall_lambda` (`src/eval.c:3433`) binds arguments by
            // consing onto `lexenv` whenever `lexenv` is non-nil and never
            // checks a local-special marker, while `Flet`/`FletX` do check
            // `declared_special` and the interpreter environment. So a
            // one-argument `(defvar x)` reaches a later `let` but never a
            // lambda parameter. Verified on live GNU 32.0.50 interpreted with
            // `lexical-binding: t`: the parameter form signals `void-variable`
            // while the `let` form answers 18.
            if self.binds_dynamically(&b, &name, false) {
                let slot = self.name_slot(&name);
                b.emit(Op::Bind(slot, reg));
                dynamic += 1;
            } else {
                let cell = true;
                b.env.insert(name, Binding { reg, cell });
                if cell {
                    b.emit(Op::Cell(reg));
                }
            }
        }
        // GNU's definition expansion consumes metadata before compiling the
        // executable body. Do not invoke a user-defined `declare' macro here.
        let mut body = body;
        if body.len() > 1 && body.first().is_some_and(|form| self.form_is_string(form)) {
            body = &body[1..];
        }
        while matches!(body.first(), Some(Form::List(items)) if matches!(items.first(), Some(Form::Sym(name)) if name == "declare"))
        {
            body = &body[1..];
        }
        let r = self.progn(&mut b, body)?;
        if dynamic != 0 {
            b.emit(Op::Unbind(dynamic));
        }
        b.emit(Op::Return(r));
        let mutations = self.classify_bindings(&mut b);
        // Discover actual free variables from compiled operands, after macro
        // expansion and lexical shadowing. Nested lambdas have already pruned
        // their sites, so transitive free variables are included precisely.
        let mut used = vec![false; names.len()];
        for op in &mut b.code {
            op.map_registers(&mut |r| {
                if r < used.len() {
                    used[r] = true;
                }
                r
            });
        }
        for &r in b.sites.iter().flatten() {
            if r < used.len() {
                used[r] = true;
            }
        }
        if keep_captures { used.fill(true); }
        let capture_mutations = mutations[..names.len()]
            .iter()
            .zip(&used)
            .filter_map(|(&mutated, &keep)| keep.then_some(mutated || keep_captures))
            .collect();
        if keep_captures {
            used.fill(true);
        }
        let visible = names.len();
        let retained = used.iter().filter(|&&keep| keep).count();
        let removed = names.len() - retained;
        if removed != 0 {
            let mut remap = vec![0; names.len()];
            let mut next = 0;
            for (i, &keep) in used.iter().enumerate() {
                remap[i] = next;
                next += usize::from(keep);
            }
            let mut map = |r: usize| {
                if r < remap.len() {
                    remap[r]
                } else {
                    r - removed
                }
            };
            for op in &mut b.code {
                op.map_registers(&mut map);
            }
            for r in b.sites.iter_mut().flatten() {
                *r = map(*r);
            }
            b.next -= removed;
            names = names
                .into_iter()
                .zip(used)
                .filter_map(|(name, keep)| keep.then_some(name))
                .collect();
        }
        let id = self.install_function(Function {
            capture_mutations,
            mapped_code: None,
            code: optimize(b.code, &b.sites, b.next),
            regs: b.next,
            captures: names.len(),
            arity: parameters.len() - usize::from(rest),
            required,
            rest,
            sites: b.sites,
            lazy: None,
            pending: None,
            dynamic: (visible == 0).then_some(self.dynamic_scope),
            params: Some(args.to_vec()),
        });
        Ok((id, names))
    }
    fn progn(&mut self, b: &mut Builder, forms: &[Form]) -> Result<usize> {
        let mut out = b.reg();
        b.emit(Op::Const(out, Value::NIL));
        for f in forms {
            out = self.expr(b, f)?;
        }
        Ok(out)
    }
    fn lookup(&mut self, b: &mut Builder, name: &str) -> usize {
        let r = b.reg();
        if let Some(v) = b.env.get(&Name::Source(name.to_owned())) {
            b.emit(if v.cell {
                Op::ReadCell(r, v.reg)
            } else {
                Op::Move(r, v.reg)
            });
        } else {
            let s = self.slot(name);
            b.emit(Op::Global(r, s));
        }
        r
    }
    fn lambda(&mut self, b: &mut Builder, args: &[Form], body: &[Form]) -> Result<usize> {
        // A lambda inside another's body that sees no lexical binding, as
        // `with-eval-after-load' wraps a `defun', compiles at its first
        // call: GNU's interpreter decides whether each `let' binds
        // dynamically when it runs, after the files loaded since have
        // declared their variables special (VM2-LATE-SPECIAL1).
        if self.lazy_regime || (self.lambda_depth > 0 && b.env.is_empty()) {
            return self.lazy_lambda(b, args, body, false);
        }
        // Ordinary closures retain only executable state. Source metadata is
        // reserved for explicit make-interpreted-closure values.
        self.lambda_depth += 1;
        let compiled = self.function(args, body, &b.env, &b.local_specials);
        self.lambda_depth -= 1;
        let (id, names) = match compiled {
            Ok(function) => function,
            // Loading a function must not execute its body. Keep the owner's
            // constant-binding rejection at call time, without aborting all
            // later declarations in an otherwise valid GNU source file.
            Err(error) if self.source_load_active() && error.contains("constant") => {
                return self.lazy_lambda(b, args, body, true);
            }
            Err(error) => return Err(error),
        };
        let regs = names.iter().map(|n| b.env[n].reg).collect();
        let site = b.sites.len();
        b.sites.push(regs);
        let r = b.reg();
        b.emit(Op::Closure(r, id, site));
        Ok(r)
    }
    /// R215: a closure over an UNEXPANDED body, compiled at its first call.
    /// It captures every visible lexical binding (GNU's interpreted closure
    /// captures its whole environment), since free variables are unknown
    /// until expansion. Each capture is reported as mutated, so no binding
    /// it can see is ever unboxed out from under the later compilation.
    fn lazy_lambda(&mut self, b: &mut Builder, args: &[Form], body: &[Form], deferred: bool) -> Result<usize> {
        let (arity, required, rest) = lambda_arity(args)?;
        let mut names: Vec<Name> = b.env.keys().cloned().collect();
        names.sort();
        let cells = names.iter().map(|n| b.env[n].cell).collect();
        let body_value = self.literal(&Form::List(body.to_vec()))?;
        let lazy = LazyBody {
            body: body_value,
            args: args.to_vec(),
            names: names.clone(),
            cells,
            local_specials: b.local_specials.clone(),
            pending_specials: self.pending_specials.clone(),
            dynamic_scope: self.dynamic_scope,
            deferred,
        };
        let id = self.install_function(Function {
            capture_mutations: vec![true; names.len()],
            mapped_code: None,
            code: vec![],
            regs: names.len() + arity + usize::from(rest),
            captures: names.len(),
            arity,
            required,
            rest,
            sites: vec![],
            dynamic: names.is_empty().then_some(self.dynamic_scope),
            lazy: Some(Arc::new(lazy)),
            pending: None,
            params: Some(args.to_vec()),
        });
        let regs = names.iter().map(|n| b.env[n].reg).collect();
        let site = b.sites.len();
        b.sites.push(regs);
        let r = b.reg();
        b.emit(Op::Closure(r, id, site));
        Ok(r)
    }
    /// Expand and compile a lazily held body in place, keeping its function
    /// ID and capture layout so every closure already made from it, and every
    /// cache naming it, now runs the compiled code. Its Lisp (macro
    /// expansion) runs inside the builder, which holds collection. On failure the
    /// body stays lazy, so the next call expands again, as GNU does.
    pub(crate) fn force_lazy(&mut self, id: usize) -> Result<()> {
        let Some(lazy) = self.functions.get(id).and_then(|f| f.lazy.clone()) else {
            return Ok(());
        };
        let body = match self.expansion_form(lazy.body, 0)? {
            Form::List(forms) => forms,
            Form::Sym(nil) if nil == "nil" => vec![],
            _ => return Err("invalid lazy body".into()),
        };
        let outer: NameMap<Name, Binding> = lazy
            .names
            .iter()
            .zip(&lazy.cells)
            .enumerate()
            .map(|(reg, (name, &cell))| (name.clone(), Binding { reg, cell }))
            .collect();
        let saved_regime = std::mem::replace(&mut self.lazy_regime, true);
        let saved_defer = std::mem::replace(&mut self.defer_expansion_errors, !lazy.deferred);
        let saved_scope = std::mem::replace(&mut self.dynamic_scope, lazy.dynamic_scope);
        let saved_pending = std::mem::replace(&mut self.pending_specials, lazy.pending_specials.clone());
        if self.compile_depth == 0 {
            self.expanded_macros.clear();
            self.expanded_symbols.clear();
        }
        self.compile_depth += 1;
        self.keep_captures = true;
        // A deferred form is evaluated at its reach, as GNU's interpreter
        // evaluates it: an unbound head is named before the arguments run.
        self.loader_order_request = lazy.deferred;
        let result = self.compile_lazy_request(&lazy.args, &body, &outer, &lazy.local_specials);
        self.keep_captures = false;
        self.compile_depth -= 1;
        self.lazy_regime = saved_regime;
        self.defer_expansion_errors = saved_defer;
        self.dynamic_scope = saved_scope;
        self.pending_specials = saved_pending;
        let (compiled, names) = result?;
        if names != lazy.names {
            return Err("lazy body changed its capture layout".into());
        }
        let mut function = self.take_compiled_function(compiled);
        function.capture_mutations = vec![true; names.len()];
        self.functions[id] = Arc::new(function);
        Ok(())
    }
    fn expr(&mut self, b: &mut Builder, f: &Form) -> Result<usize> {
        // GNU macro output can be much deeper than the reader's source
        // nesting. Protect recursive Rust compilation independently of the
        // Lisp evaluation budget (syntax traversal is not a Lisp call).
        if !eval_depth::native_stack_room() {
            return Err("native stack exhausted while compiling Lisp".into());
        }
        self.expr_inner(b, f)
    }
    fn expr_inner(&mut self, b: &mut Builder, f: &Form) -> Result<usize> {
        if !self.defer_expansion_errors || !matches!(f, Form::List(a) if !a.is_empty()) {
            return self.expr_now(b, f);
        }
        let (code, next, sites) = (b.code.len(), b.next, b.sites.len());
        let (env, local_specials) = (b.env.clone(), b.local_specials.clone());
        match self.expr_now(b, f) {
            Ok(r) => Ok(r),
            Err(_) => {
                // GNU expands this form only when evaluation reaches it, and
                // signals then (eval.c eval_sub). Discard the partial code and
                // compile a reach-time expansion instead.
                self.propagated_escape = None;
                b.code.truncate(code);
                b.next = next;
                b.sites.truncate(sites);
                b.env = env;
                b.local_specials = local_specials;
                let thunk = self.lazy_lambda(b, &[], std::slice::from_ref(f), true)?;
                let r = b.reg();
                b.emit(Op::Call(r, thunk, b.next, 0));
                Ok(r)
            }
        }
    }
    /// The special form HEAD's function names through a chain of symbol
    /// aliases (GNU indirect_function), if it ends at one.
    fn special_form_alias(&self, head: &str) -> Option<String> {
        let mut slot = *self.names.get(head)?;
        for _ in 0..self.slots.len() {
            let function = self.slots[slot].function;
            if let Some(name) = prim::name(function) {
                return prim::prims::special_forms::special_form_min(name).map(|_| name.to_owned());
            }
            slot = function.index_if(7)?.checked_sub(1)?;
            let name = &self.symbol_names[slot];
            if prim::prims::special_forms::special_form_min(name).is_some() {
                return Some(name.clone());
            }
        }
        None
    }
    fn expr_now(&mut self, b: &mut Builder, f: &Form) -> Result<usize> {
        if let Some(expanded) = self.expand_macro(f)? {
            return self.expr(b, &expanded);
        }
        if let Some(expanded) = expand::expand(f)? {
            return self.expr(b, &expanded);
        }
        match f {
            // GNU eval_sub checks the argument list before the head, so a
            // one-element dotted form is (wrong-type-argument listp TAIL); with
            // more elements a non-symbol head is invalid, and a function call
            // signals on the final atom when it walks its arguments.
            Form::Dotted(items, tail) => {
                let quote = |f: Form| Form::List(vec![Form::Sym("quote".into()), f]);
                // A function object (closure, subr) is a function head too.
                let lambda = matches!(&items[0], Form::List(l) if matches!(l.first(), Some(Form::Sym(s)) if s == "lambda"))
                    || matches!(&items[0], Form::Object(object) if object.value(self).is_ok_and(|v| matches!(v.tag(), 4 | 6)));
                let (condition, data) = if items.len() == 1 || matches!(items[0], Form::Sym(_) | Form::Generated(_)) || lambda {
                    ("wrong-type-argument", vec![Form::Sym("listp".into()), (**tail).clone()])
                } else {
                    ("invalid-function", vec![items[0].clone()])
                };
                let signal = Form::List(vec![
                    Form::Sym("signal".into()),
                    quote(Form::Sym(condition.into())),
                    quote(Form::List(data)),
                ]);
                self.expr(b, &signal)
            }
            Form::Float(_) | Form::String(_) | Form::Bytes(_) | Form::Vector(_) => {
                let value = self.literal(f)?;
                let r = b.reg();
                b.emit(Op::Const(r, value));
                Ok(r)
            }
            Form::Int(n) => {
                let r = b.reg();
                b.emit(Op::Const(r, Value::integer(*n)?));
                Ok(r)
            }
            // GNU eval_sub: an object that is neither a symbol nor a cons
            // evaluates to itself.
            Form::Object(object) => {
                let r = b.reg();
                b.emit(Op::Const(r, object.value(self)?));
                Ok(r)
            }
            Form::Sym(s) if s == "nil" || s == "t" => {
                let r = b.reg();
                b.emit(Op::Const(
                    r,
                    if s == "nil" { Value::NIL } else { Value::TRUE },
                ));
                Ok(r)
            }
            Form::Sym(s) if s.starts_with(':') => {
                let r = b.reg();
                b.emit(Op::Const(r, self.intern(s)));
                Ok(r)
            }
            Form::Sym(s) => Ok(self.lookup(b, s)),
            Form::Generated(id) => {
                let r = b.reg();
                if let Some(binding) = b.env.get(&Name::Generated(*id)) {
                    b.emit(if binding.cell {
                        Op::ReadCell(r, binding.reg)
                    } else {
                        Op::Move(r, binding.reg)
                    });
                } else {
                    let symbol = self.generated_symbol(*id);
                    b.emit(Op::Global(r, symbol.index(7)? - 1));
                }
                Ok(r)
            }
            Form::List(a) if a.is_empty() => {
                let r = b.reg();
                b.emit(Op::Const(r, Value::NIL));
                Ok(r)
            }
            Form::List(a) => {
                let head = match &a[0] {
                    Form::Sym(head) => head.as_str(),
                    Form::Generated(_) => a[0].sym()?,
                    // GNU eval_sub (src/eval.c) passes a non-symbol head through
                    // `function`: a lambda becomes a closure applied to the
                    // evaluated arguments; any other head signals
                    // (invalid-function HEAD) before an argument is evaluated.
                    Form::List(l) if matches!(l.first(), Some(Form::Sym(s)) if s == "lambda") => {
                        let function = Form::List(vec![Form::Sym("function".into()), a[0].clone()]);
                        let mut call = vec![Form::Sym("funcall".into()), function];
                        call.extend(a[1..].iter().cloned());
                        return self.expr(b, &Form::List(call));
                    }
                    // GNU eval_sub applies a function object in head
                    // position (a closure, a subr) to the evaluated arguments.
                    Form::Object(object) if object.value(self).is_ok_and(|v| matches!(v.tag(), 4 | 6)) => {
                        let mut call = vec![Form::Sym("funcall".into()), a[0].clone()];
                        call.extend(a[1..].iter().cloned());
                        return self.expr(b, &Form::List(call));
                    }
                    head => {
                        let quote = |f: Form| Form::List(vec![Form::Sym("quote".into()), f]);
                        let signal = Form::List(vec![
                            Form::Sym("signal".into()),
                            quote(Form::Sym("invalid-function".into())),
                            quote(Form::List(vec![head.clone()])),
                        ]);
                        return self.expr(b, &signal);
                    }
                };
                let rest = &a[1..];
                // GNU eval_sub dispatches on the head's function: a symbol
                // whose function is, through aliases, a special form is
                // that form, as byte-run.el's `inline' is `progn'.
                if prim::prims::special_forms::special_form_min(head).is_none()
                    && let Some(form) = self.special_form_alias(head)
                {
                    let mut aliased = a.clone();
                    aliased[0] = Form::Sym(form);
                    return self.expr(b, &Form::List(aliased));
                }
                match head {
                    "save-excursion" => self.save_excursion(b, rest),
                    "save-restriction" => self.save_restriction(b, rest),
                    "progn" => self.progn(b, rest),
                    "catch" if !rest.is_empty() => {
                        let tag = self.expr(b, &rest[0])?;
                        let dest = b.reg();
                        let handler = b.emit(Op::Catch(dest, tag, 0));
                        let value = self.progn(b, &rest[1..])?;
                        b.emit(Op::Move(dest, value));
                        b.emit(Op::PopHandlers(1));
                        b.code[handler] = Op::Catch(dest, tag, b.code.len());
                        Ok(dest)
                    }
                    "unwind-protect" if !rest.is_empty() => {
                        let dest = b.reg();
                        let protect = b.emit(Op::Protect(dest, 0, 0));
                        let value = self.expr(b, &rest[0])?;
                        b.emit(Op::LeaveProtect(value));
                        let cleanup = b.code.len();
                        self.progn(b, &rest[1..])?;
                        b.emit(Op::ResumeExit);
                        b.code[protect] = Op::Protect(dest, cleanup, b.code.len());
                        Ok(dest)
                    }
                    "condition-case" if rest.len() >= 2 => match let_forms::malformed_condition_case(&rest[0], &rest[2..]) {
                        Some(form) => self.expr(b, &form),
                        None => self.condition_case(b, rest),
                    },
                    "quote" if rest.len() == 1 => {
                        let value = self.literal(&rest[0])?;
                        let r = b.reg();
                        b.emit(Op::Const(r, value));
                        Ok(r)
                    }
                    "lambda" if rest.len() >= 2 => self.closure(b, &rest[0], &rest[1..], false, false),
                    "function" if rest.len() == 1 => match &rest[0] {
                        Form::List(l) if l.first().and_then(|v| v.sym().ok()) == Some("lambda") => {
                            if l.len() < 2 {
                                return Err("lambda needs an argument list".into());
                            }
                            self.closure(b, &l[1], &l[2..], false, false)
                        }
                        Form::Sym(_) => self.expr(
                            b,
                            &Form::List(vec![Form::Sym("quote".into()), rest[0].clone()]),
                        ),
                        _ => self.expr(
                            b,
                            &Form::List(vec![Form::Sym("quote".into()), rest[0].clone()]),
                        ),
                    },
                    "defvar-local" | "defcustom" | "defgroup" | "defvar-keymap" => self.variable_definition(b, head, rest),
                    "defvar" | "defconst" if !rest.is_empty() && rest.len() <= 3 => {
                        // eval_sub's minimum arity, then Fdefconst's CHECK_SYMBOL.
                        if head == "defconst" && rest.len() < 2 {
                            return self.expr(b, &let_forms::signal("wrong-number-of-arguments", vec![Form::Sym("defconst".into()), Form::Int(1)]));
                        }
                        if let Some(form) = let_forms::malformed_definition(&rest[0]) {
                            return self.expr(b, &form);
                        }
                        let name = rest[0].sym()?;
                        if matches!(name, "nil" | "t") {
                            return Err("cannot declare a constant".into());
                        }
                        let slot = self.slot(name);
                        if let Some(doc) = rest.get(2) {
                            if !matches!(doc, Form::Sym(s) if s == "nil") {
                                let doc = Form::List(vec![Form::Sym("put".into()), Form::List(vec![Form::Sym("quote".into()), rest[0].clone()]), Form::List(vec![Form::Sym("quote".into()), Form::Sym("variable-documentation".into())]), Form::List(vec![Form::Sym("quote".into()), doc.clone()])]);
                                self.expr(b, &doc)?;
                            }
                        }
                        if rest.len() > 1 {
                            self.pending_specials.insert(name.into());
                            b.emit(Op::DeclareSpecial(slot));
                            if head == "defconst" {
                                let r = self.expr(b, &rest[1])?;
                                b.emit(Op::SetGlobal(slot, r));
                            } else {
                                let lowered = Form::List(vec![
                                    Form::Sym("if".into()),
                                    Form::List(vec![
                                        Form::Sym("#:vm2-defvar-boundp".into()),
                                        Form::List(vec![
                                            Form::Sym("quote".into()),
                                            rest[0].clone(),
                                        ]),
                                    ]),
                                    Form::Sym("nil".into()),
                                    Form::List(vec![
                                        Form::Sym("#:vm2-defvar-set-default".into()),
                                        Form::List(vec![Form::Sym("quote".into()), rest[0].clone()]),
                                        rest[1].clone(),
                                    ]),
                                ]);
                                self.expr(b, &lowered)?;
                            }
                        } else {
                            b.local_specials.insert(name.into());
                        }
                        let r = b.reg();
                        b.emit(Op::Const(r, Value::reference(slot + 1, 7)));
                        Ok(r)
                    }
                    "define-obsolete-function-alias" if (3..=4).contains(&rest.len()) => {
                        let alias = Form::List(vec![Form::Sym("defalias".into()), rest[0].clone(), rest[1].clone(), rest.get(3).cloned().unwrap_or(Form::Sym("nil".into()))]);
                        let obsolete = Form::List(vec![Form::Sym("make-obsolete".into()), rest[0].clone(), rest[1].clone(), rest[2].clone()]);
                        self.progn(b, &[alias, obsolete])
                    }
                    "defun" | "defmacro" if rest.len() >= 3 => {
                        let name = rest[0].sym()?;
                        let slot = self.slot(name);
                        let closure = if head == "defmacro" {
                            self.lambda(b, rest[1].list()?, &rest[2..])?
                        } else {
                            self.closure(b, &rest[1], &rest[2..], true, false)?
                        };
                        b.emit(if head == "defmacro" {
                            Op::DefineMacro(slot, closure)
                        } else {
                            Op::Define(slot, closure)
                        });
                        let r = b.reg();
                        b.emit(Op::Const(r, Value::reference(slot + 1, 7)));
                        Ok(r)
                    }
                    "interactive" => self.expr(b, &Form::Sym("nil".into())),
                    "setq" => {
                        if let Some(form) = let_forms::malformed_setq(rest) {
                            return self.expr(b, &form);
                        }
                        let mut r = b.reg();
                        b.emit(Op::Const(r, Value::NIL));
                        for pair in rest.chunks_exact(2) {
                            let name = pair[0].name()?;
                            if matches!(name.source(), Some("nil" | "t")) {
                                return Err("cannot set a constant".into());
                            }
                            r = self.expr(b, &pair[1])?;
                            if let Some(v) = b.env.get(&name) {
                                b.emit(if v.cell {
                                    Op::WriteCell(v.reg, r)
                                } else {
                                    Op::Move(v.reg, r)
                                });
                            } else {
                                let slot = match name {
                                    Name::Source(ref source) => self.slot(source),
                                    Name::Generated(id) => self.generated_symbol(id).index(7)? - 1,
                                };
                                b.emit(Op::SetGlobal(slot, r));
                            }
                        }
                        Ok(r)
                    }
                    "if" if rest.len() >= 2 => {
                        let cond = self.expr(b, &rest[0])?;
                        let no = b.emit(Op::Unless(cond, 0));
                        let r = b.reg();
                        let yes = self.expr(b, &rest[1])?;
                        b.emit(Op::Move(r, yes));
                        let end = b.emit(Op::Jump(0));
                        b.code[no] = Op::Unless(cond, b.code.len());
                        let other = self.progn(b, &rest[2..])?;
                        b.emit(Op::Move(r, other));
                        b.code[end] = Op::Jump(b.code.len());
                        Ok(r)
                    }
                    "while" if !rest.is_empty() => {
                        let start = b.code.len();
                        let cond = self.expr(b, &rest[0])?;
                        let end = b.emit(Op::Unless(cond, 0));
                        self.progn(b, &rest[1..])?;
                        b.emit(Op::Jump(start));
                        b.code[end] = Op::Unless(cond, b.code.len());
                        let r = b.reg();
                        b.emit(Op::Const(r, Value::NIL));
                        Ok(r)
                    }
                    "let" | "let*" if !rest.is_empty() => {
                        if let Some(form) = let_forms::malformed_let(head == "let*", &rest[0]) {
                            return self.expr(b, &form);
                        }
                        let saved = b.env.clone();
                        let saved_specials = b.local_specials.clone();
                        let mut pending = vec![];
                        let mut dyn_count = 0;
                        for spec in rest[0].list()? {
                            let (name, init) = match spec {
                                Form::Sym(_) | Form::Generated(_) => (spec.name()?, None),
                                Form::List(l) if !l.is_empty() && l.len() <= 2 => {
                                    (l[0].name()?, l.get(1))
                                }
                                _ => return Err("bad let binding".into()),
                            };
                            if matches!(name.source(), Some("nil" | "t")) {
                                return Err("cannot bind a constant".into());
                            }
                            let value = if let Some(f) = init {
                                self.expr(b, f)?
                            } else {
                                let r = b.reg();
                                b.emit(Op::Const(r, Value::NIL));
                                r
                            };
                            let reg = b.reg();
                            b.emit(Op::Move(reg, value));
                            let cell = !self.binds_dynamically(b, &name, true);
                            if cell {
                                b.emit(Op::Cell(reg));
                            }
                            let bind = (name, Binding { reg, cell });
                            if head == "let*" {
                                dyn_count += self.bind(b, &bind);
                            } else {
                                pending.push(bind);
                            }
                        }
                        for bind in pending {
                            dyn_count += self.bind(b, &bind);
                        }
                        let r = self.progn(b, &rest[1..])?;
                        if dyn_count != 0 {
                            b.emit(Op::Unbind(dyn_count));
                        }
                        b.env = saved;
                        b.local_specials = saved_specials;
                        Ok(r)
                    }
                    _ => {
                        // eval_sub: a special form given too few arguments
                        // (or, for quote and function, too many).
                        if prim::prims::special_forms::special_form_min(head).is_some() {
                            // Fdefvar and Fdefconst refuse a fourth argument themselves.
                            if matches!(head, "defvar" | "defconst") && rest.len() > 3 {
                                let signal = let_forms::malformed_definition(&rest[0])
                                    .unwrap_or_else(|| let_forms::signal("error", vec![Form::String("Too many arguments".into())]));
                                return self.expr(b, &signal);
                            }
                            let data = vec![Form::Sym(head.into()), Form::Int(rest.len() as i64)];
                            return self.expr(b, &let_forms::signal("wrong-number-of-arguments", data));
                        }
                        if matches!(head, "lambda" | "defun") {
                            return Err(format!("invalid or unsupported {head}"));
                        }
                        let (target, args) = if head == "funcall" {
                            if rest.is_empty() {
                                return Err("funcall needs a function".into());
                            }
                            (Some(self.expr(b, &rest[0])?), &rest[1..])
                        } else {
                            (None, rest)
                        };
                        // GNU compiles these calls to a byte code that never reads
                        // the function cell; so does Romacs (sealed.rs, VM2-SEALED1).
                        // byte-opt.el's `byte-optimize-plus' and its siblings:
                        // (OP x y z) of arguments it cannot fold is (OP (OP x y) z),
                        // two byte codes.
                        if target.is_none() && args.len() == 3 && matches!(head, "+" | "-" | "*" | "min" | "max")
                            && sealed::target(head, 2).is_some()
                            && !args.iter().any(|a| matches!(a, Form::Int(_) | Form::Float(_)))
                        {
                            let inner = Form::List(vec![Form::Sym(head.into()), args[0].clone(), args[1].clone()]);
                            return self.expr(b, &Form::List(vec![Form::Sym(head.into()), inner, args[2].clone()]));
                        }
                        let sealed = target.is_none() && sealed::target(head, args.len()).is_some();
                        if target.is_none() && !sealed {
                            let slot = self.slot(head);
                            // Non-eager Lisp bodies may name a macro before its
                            // defining file is loaded. Keep that entire form
                            // unevaluated until reached, not just its arguments.
                            if self.defer_expansion_errors && self.slots[slot].function == Value::UNBOUND {
                                return Err(format!("defer unresolved function {head}"));
                            }
                        }
                        // GNU's loader is its interpreter: a top-level or deferred
                        // form whose head has no function when compiled (a macro
                        // from a file not yet loaded) names that head, not an
                        // argument, when it fails. A call compiled inside a
                        // function keeps the byte code's order (VM2-BCALL1).
                        // Emitted as ops, never as a form: the cell is read by the
                        // sealed `symbol-function' primitive, and a void cell takes
                        // the call itself at once, with no arguments, so that
                        // `direct_generic' raises its (void-function HEAD) before
                        // any argument runs. No name is read and no constant but
                        // the head's symbol is needed (the source cache spells it).
                        let loader_guard = if target.is_none() && !sealed && b.loader_order
                            && self.names.get(head).is_none_or(|&slot| self.slots[slot].function == Value::UNBOUND)
                        {
                            let slot = self.slot(head);
                            let symbol = b.reg();
                            b.emit(Op::Const(symbol, Value::reference(slot + 1, 7)));
                            let cell = b.reg();
                            let reader = self.slot("symbol-function");
                            b.emit(Op::Sealed(cell, reader, symbol, 1));
                            let void = b.emit(Op::Unless(cell, 0));
                            let bound = b.emit(Op::Jump(0));
                            b.code[void] = Op::Unless(cell, b.code.len());
                            let r = b.reg();
                            let cache = self.install_cache();
                            b.emit(Op::Direct(r, slot, cache, b.next, 0));
                            let after = b.emit(Op::Jump(0));
                            b.code[bound] = Op::Jump(b.code.len());
                            Some(after)
                        } else {
                            None
                        };
                        // Reserve consecutive argument registers before compiling nested calls.
                        let start = b.next;
                        b.next += args.len();
                        let oclosure = head == "oclosure--fix-type" && args.len() == 2
                            && matches!(&args[0], Form::List(parts) if matches!(parts.first(), Some(Form::Sym(s)) if s == "ignore"));
                        for (i, f) in args.iter().enumerate() {
                            let r = if oclosure && i == 1 { self.oclosure_argument(b, f)? } else { self.expr(b, f)? };
                            b.emit(Op::Move(start + i, r));
                        }
                        let r = b.reg();
                        if let Some(f) = target {
                            b.emit(Op::Call(r, f, start, args.len()));
                        } else if sealed {
                            b.emit(Op::Sealed(r, self.slot(head), start, args.len()));
                        } else {
                            // GNU's Bcall (bytecode.c): the arguments first, then the
                            // function cell, read through this site's cache.
                            let slot = self.slot(head);
                            let cache = self.install_cache();
                            b.emit(Op::Direct(r, slot, cache, start, args.len()));
                        }
                        if let Some(after) = loader_guard {
                            b.code[after] = Op::Jump(b.code.len());
                        }
                        Ok(r)
                    }
                }
            }
        }
    }
    fn condition_case(&mut self, b: &mut Builder, rest: &[Form]) -> Result<usize> {
        // Macro-generated handler variables carry identity, not an interned name.
        let variable = rest[0].name()?;
        if variable.source() == Some("t") {
            return Err("cannot bind a constant".into());
        }
        let mut clauses = Vec::new();
        let mut success = None;
        for clause in &rest[2..] {
            let clause = clause.list()?;
            // internal_lisp_condition_case ignores a nil handler.
            if clause.is_empty() {
                continue;
            }
            if matches!(&clause[0], Form::Sym(name) if name == ":success") {
                success.get_or_insert(clause);
            } else {
                let conditions = self.literal(&clause[0])?;
                clauses.push((clause, conditions));
            }
        }
        let dest = b.reg();
        let payload = b.reg();
        let mut handlers = Vec::new();
        for (_, conditions) in clauses.iter().rev() {
            handlers.push(b.emit(Op::Condition(payload, *conditions, 0)));
        }
        let value = self.expr(b, &rest[1])?;
        b.emit(Op::Move(dest, value));
        b.emit(Op::PopHandlers(clauses.len()));
        if let Some(clause) = success {
            b.emit(Op::Move(payload, value));
            self.condition_arm(b, &variable, payload, &clause[1..], dest)?;
        }
        let mut ends = vec![b.emit(Op::Jump(0))];
        for ((clause, conditions), handler) in clauses.into_iter().zip(handlers.into_iter().rev()) {
            b.code[handler] = Op::Condition(payload, conditions, b.code.len());
            // All clauses belong to one dynamic scope; the unwinder removes its
            // siblings before entering this arm, not just the matching clause.
            self.condition_arm(b, &variable, payload, &clause[1..], dest)?;
            ends.push(b.emit(Op::Jump(0)));
        }
        let end = b.code.len();
        for jump in ends {
            b.code[jump] = Op::Jump(end);
        }
        Ok(dest)
    }
    fn condition_arm(
        &mut self,
        b: &mut Builder,
        variable: &Name,
        payload: usize,
        body: &[Form],
        dest: usize,
    ) -> Result<()> {
        let saved = b.env.clone();
        let dynamic = if variable.source() == Some("nil") {
            0
        } else {
            let reg = b.reg();
            b.emit(Op::Move(reg, payload));
            let cell = !self.binds_dynamically(b, variable, false);
            if cell {
                b.emit(Op::Cell(reg));
            }
            self.bind(
                b,
                &(variable.clone(), Binding { reg, cell }),
            )
        };
        let result = self.progn(b, body)?;
        b.emit(Op::Move(dest, result));
        if dynamic != 0 {
            b.emit(Op::Unbind(dynamic));
        }
        b.env = saved;
        Ok(())
    }
    /// Whether a binding of NAME is dynamic: every binding of a symbol
    /// under dynamic scope, an uninterned one's too, as GNU's specbind binds
    /// the symbol object; otherwise a special variable's, B's local specials
    /// counting when LOCAL. A compiler temporary is no Lisp symbol, so it
    /// stays a local and never takes a symbol slot.
    fn binds_dynamically(&self, b: &Builder, name: &Name, local: bool) -> bool {
        match name {
            Name::Generated(id) => self.dynamic_scope && self.generated_symbols.contains_key(id),
            Name::Source(n) => self.dynamic_scope || self.specials.contains(n.as_str()) || self.pending_specials.contains(n.as_str())
                || (local && b.local_specials.contains(n.as_str())),
        }
    }
    /// The symbol slot of NAME.
    fn name_slot(&mut self, name: &Name) -> usize {
        match name {
            Name::Source(source) => self.slot(source),
            Name::Generated(id) => self.generated_symbol(*id).index(7).expect("a generated symbol") - 1,
        }
    }
    fn bind(&mut self, b: &mut Builder, (name, binding): &(Name, Binding)) -> usize {
        if self.binds_dynamically(b, name, true) {
            let slot = self.name_slot(name);
            b.emit(Op::Bind(slot, binding.reg));
            1
        } else {
            b.env.insert(name.clone(), *binding);
            0
        }
    }
    /// Read compiled data only: never enters Lisp, executes code, or warms caches.
    pub fn dump_ir(&self, entry: usize) -> Result<String> {
        use std::fmt::Write;
        self.require_live_function(entry)?;
        let mut out = format!("entry f{entry}\n");
        let mut symbols: Vec<_> = self.names.iter().collect();
        symbols.sort_by_key(|(_, slot)| **slot);
        for (name, slot) in symbols {
            writeln!(&mut out, "symbol s{slot} = {name}").unwrap();
        }
        for (id, f) in self.functions.iter().enumerate() {
            let decoded;
            let f = if f.mapped_code.is_some() {
                decoded = self.decode_image_function(id)?;
                &decoded
            } else if f.pending.is_some() {
                decoded = self.decode_pending(id)?;
                &decoded
            } else {
                f
            };
            writeln!(
                &mut out,
                "fn f{id} args={} required={} rest={} captures={} registers={}",
                f.arity, f.required, f.rest, f.captures, f.regs
            )
            .unwrap();
            for (pc, op) in f.code.iter().enumerate() {
                let line = match *op {
                    Op::Const(d, v) => format!("r{d} = const {}", self.display(v)?),
                    Op::Move(d, s) => format!("r{d} = r{s}"),
                    Op::Cell(r) => format!("r{r} = new-cell r{r}"),
                    Op::ReadCell(d, s) => format!("r{d} = cell-value r{s}"),
                    Op::WriteCell(d, s) => format!("cell-value r{d} = r{s}"),
                    Op::Global(d, s) => format!("r{d} = value s{s}"),
                    Op::Sealed(d, s, a, n) => format!("r{d} = sealed s{s} r{a}..+{n}"),
                    Op::SetGlobal(s, r) => format!("value s{s} = r{r}"),
                    Op::Bind(s, r) => format!("bind-special s{s} r{r}"),
                    Op::Unbind(n) => format!("unbind {n}"),
                    Op::Jump(to) => format!("jump {to:04}"),
                    Op::Unless(r, to) => format!("unless r{r} jump {to:04}"),
                    Op::Closure(d, function, site) => {
                        format!("r{d} = closure f{function} captures {:?}", f.sites[site])
                    }
                    Op::Define(s, r) => format!("function s{s} = r{r}"),
                    Op::DefineMacro(s, r) => format!("macro s{s} = r{r}"),
                    Op::DeclareSpecial(s) => format!("declare-special s{s}"),
                    Op::Call(d, t, start, n) => {
                        format!("r{d} = funcall r{t} args r{start} count {n}")
                    }
                    Op::Direct(d, s, cache, start, n) => {
                        format!("r{d} = call s{s} cache c{cache} args r{start} count {n}")
                    }
                    Op::Return(r) => format!("return r{r}"),
                    Op::Catch(d, tag, to) => format!("catch r{tag} -> r{d} resume {to:04}"),
                    Op::Condition(d, conditions, to) => format!(
                        "condition {} -> r{d} resume {to:04}",
                        self.display(conditions)?
                    ),
                    Op::PopHandlers(n) => format!("pop-handlers {n}"),
                    Op::Protect(d, cleanup, end) => {
                        format!("protect r{d} cleanup {cleanup:04} resume {end:04}")
                    }
                    Op::LeaveProtect(r) => format!("leave-protect r{r}"),
                    Op::ResumeExit => "resume-exit".into(),
                };
                writeln!(&mut out, "  {pc:04} {line}").unwrap();
            }
        }
        Ok(out)
    }
    /// Execute a live zero-argument function. Invalid or retired IDs return an
    /// `"invalid function"` error; IDs returned by `compile` remain live.
    pub(crate) fn execute_raw(&mut self, function: usize) -> Result<Value> {
        self.execute_rooted(function, &mut [])
    }
    /// Keep embedder-held Values alive and rewrite them across automatic GC.
    /// Values omitted here must not be used after a collecting execution.
    pub(crate) fn execute_rooted(&mut self, function: usize, roots: &mut [Value]) -> Result<Value> {
        self.require_live_function(function)?;
        let f = &self.functions[function];
        if f.arity != 0 || f.captures != 0 {
            return Err("entry function needs arguments or captures".into());
        }
        let frame = self.root_frame(roots);
        let outcome = threaded::execute(self, function);
        for (i, root) in roots.iter_mut().enumerate() {
            *root = self.rooted(&frame, i);
        }
        self.drop_frame(frame);
        outcome
    }
    /// Declare that this embedder renders the echo area: printing to `t`
    /// in an interactive session then goes there, as GNU's does on a
    /// terminal (print.c PRINTPREPARE), not to standard output.
    pub fn set_echo_area_host(&mut self, rendered: bool) {
        let slot = self.slot(prim::ECHO_AREA_HOST);
        self.slots[slot].value = if rendered { Value::TRUE } else { Value::NIL };
    }
    /// The echo area's message for `current-message`, as an embedder that
    /// renders the echo area itself shows or clears it.
    pub fn set_echo_area_message(&mut self, text: Option<&str>) {
        let value = text.map_or(Value::NIL, |t| self.make_lisp_string(t.as_bytes().to_vec(), !t.is_ascii()));
        let slot = self.slot(prim::ECHO_AREA_MESSAGE);
        self.slots[slot].value = value;
    }
    /// Unwinding pops every binding; one whose restore signals keeps its
    /// value, as GNU's popped entry does. That signal is dropped and the
    /// escape already in flight kept (see `unbind_one`).
    fn unwind(&mut self, n: usize) {
        for _ in 0..n {
            let pending = (self.propagated_escape.take(), std::mem::take(&mut self.propagated_escape_hooked), self.propagated_eval_depth.take());
            let _ = self.unbind_one();
            (self.propagated_escape, self.propagated_escape_hooked, self.propagated_eval_depth) = pending;
        }
    }
    /// GNU `unbind_to` on normal completion: a restore that signals
    /// propagates, and the bindings under it unwind with that escape.
    fn unbind(&mut self, n: usize) -> Result<()> {
        for left in (0..n).rev() {
            if let Err(error) = self.unbind_one() {
                self.unwind(left);
                return Err(error);
            }
        }
        Ok(())
    }
    /// Leave a region that finished with RESULT, popping its N bindings:
    /// unbind on normal completion, unwind under an escape.
    fn unbind_after<T>(&mut self, n: usize, result: Result<T>) -> Result<T> {
        match result {
            Ok(value) => self.unbind(n).map(|()| value),
            Err(error) => { self.unwind(n); Err(error) }
        }
    }
    /// GNU `unbind_to` for the innermost binding: pop it, then notify its
    /// watchers and restore (`do_one_unbind`). A watcher's signal, left in
    /// propagated_escape, aborts the restore. A plain variable reports
    /// `unlet'; a localized default (SPECPDL_LET_DEFAULT, or a SPECPDL_LET
    /// whose variable became local) restores through set_default_internal,
    /// which reports `set'.
    pub(crate) fn unbind_one(&mut self) -> Result<()> {
        let (slot, value, owner, let_default) = self.dynamic.pop().expect("compiler balances bindings");
        let symbol = self.backtrace.bindings.last().filter(|(at, _)| *at == self.dynamic.len()).map(|(_, symbol)| *symbol);
        self.backtrace.unbind(self.dynamic.len());
        let base = symbol.and_then(|symbol| symbol.index_if(7)).map(|index| index - 1);
        // A `let' moves a fenced value back unread; a watcher would see it.
        if let Some(forwarded::Fwd::Fence(cell)) = self.native_forwarding(slot) {
            self.fence_unbound(cell as usize);
            self.fence_watched(cell as usize);
            return self.unfenced(cell as usize, |vm| vm.restore_binding(slot, base, value, owner, let_default));
        }
        self.restore_binding(slot, base, value, owner, let_default)
    }
    /// `unbind_one`'s restore of the popped binding of SLOT (BASE its
    /// variable, when the backtrace names one) to VALUE.
    fn restore_binding(&mut self, slot: usize, base: Option<usize>, value: Value, owner: Value, let_default: bool) -> Result<()> {
        // A local let restores the owner's current local binding, if it
        // still has one (GNU do_one_unbind, SPECPDL_LET_LOCAL): the local
        // may have been killed and made again inside the let.
        // The popped value and owner are no longer traced as a binding: a
        // watcher sees them from a root frame, and the restore reads them back.
        // GNU do_one_unbind (SPECPDL_LET_LOCAL, eval.c:3871-3882) restores
        // only if OWNER still has its local binding.
        if let Some(base) = base && self.local_binding_slots.contains(&slot) {
            if self.owner_local_binding(base, owner).is_none() {
                return Ok(());
            }
            let (mut value, mut owner) = (value, owner);
            if self.variable_trapped(base) {
                let frame = self.root_frame(&[value, owner]);
                let notified = self.notify_watchers_at(base, value, "unlet", Some(owner));
                [value, owner] = self.release_frame(frame);
                notified?;
            }
            // set_internal finds OWNER's binding after the watchers (data.c:
            // 1730-1760): one may have killed it, made another or let its
            // private slot be reused. With none left the default takes it.
            let target = self.owner_local_binding(base, owner).unwrap_or(base);
            // set-buffer-local-toplevel-value may have saved any object; an
            // integer binding refuses a non-integer and keeps its value.
            let value = match self.localized_forwarded(base) {
                Some(forwarded::Fwd::Int(_)) if value.integer_value().is_none() => {
                    let (predicate, condition) = (self.intern("integerp"), self.intern("wrong-type-argument"));
                    let data = self.list_value(&[predicate, value]);
                    self.propagated_escape = Some(crate::threaded::Escape::Signal(condition, data));
                    return Err(forwarded::fault_text(crate::prim::Fault::Wrong("integerp", value)));
                }
                Some(forwarded::Fwd::Bool(_)) => if value == Value::NIL { Value::NIL } else { Value::TRUE },
                _ => value,
            };
            self.slots[target].value = value;
            return Ok(());
        }
        if let Some(fwd) = self.native_forwarding(slot) {
            return self.unbind_forwarded(slot, fwd, value, owner, let_default);
        }
        let mut value = value;
        if let Some(base) = base && self.variable_trapped(base) {
            let frame = self.root_frame(&[value]);
            let notified = if self.local_table(base)?.is_some() { self.notify_watchers_at(base, value, "set", Some(Value::NIL)) }
                else { self.notify_watchers(base, value, "unlet") };
            [value] = self.release_frame(frame);
            notified?;
        }
        if slot != self.buffer_slot || value == Value::NIL || self.live_buffer_value(value) {
            self.slots[slot].value = value;
        }
        Ok(())
    }
    /// The private slot of OWNER's local binding of BASE, if OWNER is live
    /// and still has one.
    fn owner_local_binding(&mut self, base: usize, owner: Value) -> Option<usize> {
        if !self.live_buffer_value(owner) {
            return None;
        }
        // A watcher may have dropped the whole table: no local binding then.
        let table = self.slots[base].local;
        let local = self.declaration_hash_get(table, owner).ok()??;
        Some(local.index(7).expect("private binding") - 1)
    }
    #[inline(never)]
    fn primitive(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        use Primitive::*;
        if matches!(p, ClosureMetadata | ClosureDocument | MakeInterpretedClosure) { return self.interpreted_closure_primitive(p, start, n); }
        if p == TempBuffer { return self.eval_buffer_primitive(p, start, n); }
        if matches!(p, DeepEqual | Min | Append) { return self.eval_data_primitive(p, start, n); }
        if matches!(p, GetBuffer | ForwardLine | SwitchToBuffer | EmacsLispMode | MenuBarMode | ForceModeLineUpdate | LocateLibrary | Daemonp | DaemonInitialized) { return self.eval_buffer_primitive(p, start, n); }
        if (p as usize) >= SymbolValue as usize {
            return self.variable_primitive(p, start, n);
        }
        if matches!(p, MakeObsolete | MakeObsoleteVariable | SetAdvertisedCallingConvention) {
            return self.metadata_primitive(p, start, n);
        }
        if (p as usize) >= DefineKey as usize {
            return self.keymap_primitive(p, start, n);
        }
        if matches!(p, Consp | CarSafe | Symbolp | Integerp | Fixnump | Bignump) {
            return self.surface_primitive(p, start, n);
        }
        if regexp::is_regexp_primitive(p) {
            return self.regexp_primitive_result(p, start, n);
        }
        if hash::is_hash_primitive(p) {
            return self.hash_primitive_result(p, start, n);
        }
        if matches!(
            p,
            KillNew
                | CurrentKill
                | YankPart
                | YankFinalize
                | YankPropertyRun
                | YankFontLockFace
                | YankCategory
                | InsertInherit
                | Get
                | Put
                | RawPropertiesAt
                | GetProperty
                | GetCharProperty
                | BufferSubstring
        ) {
            return self.annotation_primitive(p, start, n);
        }
        if matches!(
            p,
            Car | Cdr | Null | Eq | Memq | Assq | Nth | Setcar | Setcdr
        ) {
            return self.traversal_primitive(p, start, n);
        }
        if matches!(p, Add | Sub | Mul | Div | Abs | Less | Equal | Inc | Dec) {
            return self.numeric_primitive(p, start, n);
        }
        if matches!(p, CacheStats | CacheClear | CacheEntries) {
            return self.cache_primitive(p, n);
        }
        if matches!(p, Load | Require | Provide | Featurep | Eval | Macroexpand | AutoloadDoLoad | ReadHostEvent | FormatModeLine) {
            let args = self.stack[start..start + n].to_vec();
            let result = self.loader_primitive(p, &args);
            // Only the reentrant engine path can re-raise an eval escape.
            self.propagated_escape = None;
            return result;
        }
        if p == Message {
            // The echo area holds the last message; (message nil) clears it.
            let echo = self.slot(prim::ECHO_AREA_MESSAGE);
            if n >= 1 && self.stack[start] == Value::NIL {
                self.slots[echo].value = Value::NIL;
                return Ok(Value::NIL);
            }
            let value = self.format_string(start, n)?;
            self.slots[echo].value = value;
            // GNU writes the bytes it formatted: a unibyte string's as they
            // are, a raw-byte character as its byte.
            let view = self.str_view(value).ok_or("message: format made no string")?;
            let mut line = Vec::with_capacity(view.bytes().len() + 1);
            if view.is_multibyte() {
                for code in view.chars() {
                    match char::from_u32(code) {
                        Some(c) => line.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
                        None => line.push(code as u8),
                    }
                }
            } else {
                line.extend_from_slice(view.bytes());
            }
            line.push(b'\n');
            let _ = std::io::Write::write_all(&mut std::io::stderr().lock(), &line);
            return Ok(value);
        }
        if p == Fset || (List as usize..=Memq as usize).contains(&(p as usize)) {
            return self.bootstrap_primitive(p, start, n);
        }
        if p == ProcessLines {
            return self.process_lines(start, n);
        }
        if p == Format {
            return self.format_string(start, n);
        }
        if matches!(
            p,
            Vector | Aref | Aset | CopySequence | Assq | Setcar | Setcdr
        ) {
            return self.data_primitive(p, start, n);
        }
        if (p as usize) >= Primitive::DisplayDefineFace as usize {
            return self.display_primitive(p, start, n);
        }
        if (p as usize) >= Primitive::Remainder as usize {
            return self.buffer_primitive(p, start, n);
        }
        let a = &self.stack[start..start + n];
        let arity = match p {
            Inc | Dec | Car | Cdr | Length | Nreverse => Some(1),
            Cons => Some(2),
            _ => None,
        };
        if arity.is_some_and(|arity| n != arity) || matches!(p, Sub | Less | Equal) && n == 0 {
            return Err("wrong primitive arity".into());
        }
        match p {
            Cons => {
                let pair = (a[0], a[1]);
                let i = self.conses.len();
                self.conses.push(pair);
                Ok(Value::reference(i, 3))
            }
            Length => {
                if a[0].tag() == 4 && !self.closures[a[0].index(4)?].metadata.is_empty() { return Value::integer(self.closures[a[0].index(4)?].metadata.len() as i64); }
                if a[0] != Value::NIL && a[0].tag() == 0 {
                    // Three sources: mapped UTF-8 strings, heap strings/vectors,
                    // and byte strings. Text counts scalars; byte strings count
                    // octets. Buffers are not sequences.
                    return Value::integer(if Self::image_string(a[0]) {
                        self.string_value(a[0])?.chars().count()
                    } else {
                        match self.object(a[0])? {
                            data::Object::String(s) | data::Object::RawString(s) => s.len(),
                            data::Object::Vector(v) | data::Object::Record(v) => v.len(),
                            data::Object::BoolVector(v) => v.len,
                            data::Object::CharTable(_) => char_table::CharTable::LEN,
                            data::Object::Bytes(v) => v.len(),
                            data::Object::SubCharTable(_)
                            | data::Object::MapKeymapCursor(_)
                            | data::Object::Float(_)
                            | data::Object::Bignum(_)
                            | data::Object::Buffer(_)
                            | data::Object::ByteBuffer(_)
                            | data::Object::Marker(_)
                            | data::Object::Overlay(_)
                            | data::Object::SyntaxTable(_)
                            | data::Object::Window(_)
                            | data::Object::Obarray(_)
                            | data::Object::Frame(_)
                            | data::Object::Terminal(_)
                            | data::Object::WindowConfiguration(_)
                            | data::Object::Sqlite(_)
                            | data::Object::Treesit(_)
                            | data::Object::Finalizer(_)
                            | data::Object::Process(_)
                            | data::Object::HashTable(_) => {
                                return Err("sequence required".into());
                            }
                        }
                    } as i64);
                }
                // fns.c list_length: GNU's FOR_EACH_TAIL; data.rs maps the
                // failure to GNU's condition (tails.rs core_condition).
                match self.gnu_tails(a[0]) {
                    tails::TailEnd::Proper(len) => Value::integer(len as i64),
                    tails::TailEnd::Dotted(_) => Err("list required".into()),
                    tails::TailEnd::Circular(_) => Err("circular list".into()),
                }
            }
            Nreverse => self.core_nreverse(a[0]),
            _ => Err("primitive requires executor adapter".into()),
        }
    }
    pub(crate) fn display(&self, v: Value) -> Result<String> {
        self.display_depth(v, 0)
    }
    fn display_depth(&self, v: Value, depth: usize) -> Result<String> {
        if depth > 100 {
            return Err("display depth exceeded".into());
        }
        if v != Value::NIL && v.tag() == 0 && !Self::image_string(v) {
            return match self.object(v)? {
                data::Object::Float(n) => Ok(numbers::print_float(*n)),
                data::Object::Bignum(n) => Ok(n.to_string()),
                data::Object::BoolVector(v) => Ok(v.printed()),
                data::Object::String(s) => Ok(serde_json::to_string(s.as_str()).unwrap()),
                data::Object::RawString(_) => Err("not-unicode-string: JSON display".into()),
                data::Object::Buffer(_) => Ok("#<buffer>".into()),
                data::Object::Window(w) => Ok(format!("#<window {}>", w.id)),
                data::Object::Frame(_) => Ok("#<frame initial>".into()),
                data::Object::Terminal(_) => Ok("#<terminal initial>".into()),
                data::Object::WindowConfiguration(_) => Ok("#<window-configuration>".into()),
                data::Object::Sqlite(h) => Ok(h.describe()),
                data::Object::Treesit(t) => Ok(crate::treesit::describe(self, t)),
                // print.c PVEC_FINALIZER.
                data::Object::Finalizer(function) => Ok(if *function == Value::NIL { "#<finalizer used>" } else { "#<finalizer>" }.into()),
                data::Object::Process(p) => Ok(format!("#<process {}>", self.process_name_text(p))),
                data::Object::Marker(_) => Ok(self.marker_text(v, false).unwrap_or_else(|| "#<marker>".into())),
                data::Object::Overlay(_) => Ok("#<overlay>".into()),
                data::Object::ByteBuffer(_) => Ok("#<byte-buffer>".into()),
                data::Object::SyntaxTable(_) => Ok("#<syntax-table>".into()),
                data::Object::CharTable(_) => Ok("#<char-table>".into()),
                data::Object::SubCharTable(_) => Ok("#<sub-char-table>".into()),
                data::Object::MapKeymapCursor(_) => Err("private traversal cursor".into()),
                data::Object::HashTable(t) => self.display_table(t, depth),
                data::Object::Obarray(t) => Ok(format!("#<obarray n={}>", t.len())),
                data::Object::Bytes(v) => Ok(format!(
                    "#u8({})",
                    v.iter().map(u8::to_string).collect::<Vec<_>>().join(" ")
                )),
                object @ (data::Object::Vector(values) | data::Object::Record(values)) => Ok(format!(
                    "{}{}{}",
                    if matches!(object, data::Object::Record(_)) { "#s(" } else { "[" },
                    values
                        .iter()
                        .map(|&v| self.display_depth(v, depth + 1))
                        .collect::<Result<Vec<_>>>()?
                        .join(" "),
                    if matches!(object, data::Object::Record(_)) { ")" } else { "]" }
                )),
            };
        }
        if v.tag() == 3 {
            let mut tail = v;
            let mut parts = Vec::new();
            while tail.tag() == 3 {
                // Mapped conses are not in self.conses, and the limit must count
                // the image too or a purely mapped list looks circular.
                let pair = if Self::image_value(tail) {
                    self.cons_pair(tail)?
                } else {
                    *self
                        .conses
                        .get(tail.index(3)?)
                        .ok_or("invalid cons reference")?
                };
                parts.push(self.display_depth(pair.0, depth + 1)?);
                tail = pair.1;
                if parts.len() > self.list_limit() {
                    return Err("circular list".into());
                }
            }
            if tail != Value::NIL {
                parts.push(format!(". {}", self.display_depth(tail, depth + 1)?));
            }
            return Ok(format!("({})", parts.join(" ")));
        }
        if v == Value::NIL {
            Ok("nil".into())
        } else if v == Value::TRUE {
            Ok("t".into())
        } else if Self::image_string(v) {
            Ok(serde_json::to_string(self.string_value(v)?).unwrap())
        } else if v.tag() == 1 {
            Ok(v.as_integer()?.to_string())
        } else if v.tag() == 4 {
            self.closures.get(v.index(4)?).ok_or("invalid closure reference")?;
            Ok("#<interpreted-function>".into())
        } else if v.tag() == 6 {
            let name = prim::name(v).ok_or("invalid primitive")?;
            Ok(format!("#<subr {name}>"))
        } else if v.tag() == 7 && v != Value::UNBOUND {
            let slot = v.index(7)? - 1;
            self.symbol_names
                .get(slot)
                .cloned()
                .ok_or_else(|| "invalid symbol".into())
        } else {
            Err("S0 CLI prints only integer/boolean/symbol results".into())
        }
    }
}

/// Local destination forwarding, with whole-function use counts and branch barriers.
/// Only dead pure stores disappear; calls/loads retain their errors and effects.
fn optimize(mut code: Vec<Op>, sites: &[Vec<usize>], regs: usize) -> Vec<Op> {
    loop {
        let mut reads = vec![0usize; regs];
        let mut writes = vec![0usize; regs];
        let mut targets = HashSet::default();
        for &op in &code {
            let mut read = |r: usize| reads[r] += 1;
            match op {
                Op::Const(d, _) | Op::Global(d, _) => writes[d] += 1,
                Op::Move(d, s) | Op::ReadCell(d, s) => {
                    read(s);
                    writes[d] += 1;
                }
                Op::Cell(r) => {
                    read(r);
                    writes[r] += 1;
                }
                Op::WriteCell(d, s) => {
                    read(d);
                    read(s);
                }
                Op::SetGlobal(_, r)
                | Op::Bind(_, r)
                | Op::Define(_, r)
                | Op::DefineMacro(_, r)
                | Op::Return(r) => read(r),
                Op::Unless(r, target) => {
                    read(r);
                    targets.insert(target);
                }
                Op::Jump(target) => {
                    targets.insert(target);
                }
                Op::Closure(d, _, site) => {
                    for &r in &sites[site] {
                        read(r);
                    }
                    writes[d] += 1;
                }
                Op::Call(d, t, start, n) => {
                    read(t);
                    for r in start..start + n {
                        read(r);
                    }
                    writes[d] += 1;
                }
                Op::Direct(d, _, _, start, n) | Op::Sealed(d, _, start, n) => {
                    for r in start..start + n {
                        read(r);
                    }
                    writes[d] += 1;
                }
                Op::Catch(d, tag, target) => {
                    writes[d] += 1;
                    read(tag);
                    targets.insert(target);
                }
                Op::Condition(d, _, target) => {
                    writes[d] += 1;
                    targets.insert(target);
                }
                Op::Protect(d, cleanup, end) => {
                    writes[d] += 1;
                    targets.insert(cleanup);
                    targets.insert(end);
                }
                Op::LeaveProtect(r) => read(r),
                Op::Unbind(_) | Op::PopHandlers(_) | Op::ResumeExit | Op::DeclareSpecial(_) => {}
            }
        }
        let mut live: Vec<Option<Op>> = code.iter().copied().map(Some).collect();
        let mut changed = false;
        for (i, &op) in code.iter().enumerate() {
            if matches!(op, Op::Const(d, _) | Op::Move(d, _) if reads[d] == 0)
                || matches!(op, Op::Move(d, s) if d == s)
            {
                live[i] = None;
                changed = true;
                continue;
            }
            if i == 0 || targets.contains(&i) {
                continue;
            }
            if let Op::Move(d, s) = op {
                if reads[s] != 1 || writes[s] != 1 || d == s {
                    continue;
                }
                let replacement = match live[i - 1] {
                    Some(Op::Const(out, v)) if out == s => Some(Op::Const(d, v)),
                    Some(Op::Move(out, r)) if out == s => Some(Op::Move(d, r)),
                    Some(Op::ReadCell(out, r)) if out == s => Some(Op::ReadCell(d, r)),
                    Some(Op::Global(out, slot)) if out == s => Some(Op::Global(d, slot)),
                    Some(Op::Closure(out, f, site)) if out == s => Some(Op::Closure(d, f, site)),
                    Some(Op::Call(out, t, a, n)) if out == s => Some(Op::Call(d, t, a, n)),
                    Some(Op::Direct(out, slot, cache, a, n)) if out == s => {
                        Some(Op::Direct(d, slot, cache, a, n))
                    }
                    Some(Op::Sealed(out, slot, a, n)) if out == s => Some(Op::Sealed(d, slot, a, n)),
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    live[i - 1] = Some(replacement);
                    live[i] = None;
                    changed = true;
                }
            }
        }
        if !changed {
            return code;
        }
        let mut remap = vec![0; code.len() + 1];
        let mut next = 0;
        for (i, op) in live.iter().enumerate() {
            remap[i] = next;
            next += usize::from(op.is_some());
        }
        remap[code.len()] = next;
        code = live
            .into_iter()
            .flatten()
            .map(|op| match op {
                Op::Jump(t) => Op::Jump(remap[t]),
                Op::Unless(r, t) => Op::Unless(r, remap[t]),
                Op::Catch(d, tag, t) => Op::Catch(d, tag, remap[t]),
                Op::Condition(d, conditions, t) => Op::Condition(d, conditions, remap[t]),
                Op::Protect(d, cleanup, end) => Op::Protect(d, remap[cleanup], remap[end]),
                other => other,
            })
            .collect();
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    #[test]
    fn sibling_closures_share_one_mutable_cell_through_compaction() {
        let mut vm = Vm::new();
        let pair = vm
            .eval_source_raw("(let ((x 7)) (cons (lambda () (setq x (1+ x))) (lambda () x)))")
            .unwrap();
        let mut roots = [pair];
        vm.collect(&mut roots);
        let (writer, reader) = vm.cons_pair(roots[0]).unwrap();
        let w = &vm.closures[writer.index(4).unwrap()].captures;
        let r = &vm.closures[reader.index(4).unwrap()].captures;
        assert_eq!(w, r);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].tag(), 5);
        assert_eq!(vm.cells.len(), 1);
        vm.declare_special_raw("survivor", reader);
        vm.collect(&mut []);
        assert_eq!(
            vm.cells.len(),
            1,
            "reader keeps the shared binding alive after writer dies"
        );
        let value = vm.eval_source_raw("(funcall survivor)").unwrap();
        assert_eq!(value.as_integer().unwrap(), 7);
    }

    #[test]
    fn immutable_captures_are_inline_values_not_shared_cells() {
        let mut vm = Vm::new();
        let value = vm.eval_source_raw("(let ((a 7)) (lambda () a))").unwrap();
        let captures = &vm.closures[value.index(4).unwrap()].captures;
        assert_eq!(
            captures,
            &[Value::integer(7).unwrap()],
            "immutable binding should not be boxed"
        );
        assert!(vm.cells.is_empty());
    }

    #[test]
    fn macro_expanded_mutation_shares_the_binder_not_a_snapshot() {
        let mut vm = Vm::new();
        let value = vm.eval_source_raw("(defmacro bump () '(setq a (1+ a))) (let ((a 0)) (let ((writer (lambda () (bump))) (reader (lambda () (bump)))) (funcall writer) (funcall reader)))").unwrap();
        assert_eq!(
            value.as_integer().unwrap(),
            2,
            "macro-introduced writes need a shared lexical cell"
        );
    }

    #[test]
    fn unused_lexicals_are_not_retained_by_a_closure() {
        let mut vm = Vm::new();
        let value = vm
            .eval_source_raw("(let ((a (cons 1 nil)) (unused (cons 2 nil))) (lambda () a))")
            .unwrap();
        let mut roots = [value];
        vm.collect(&mut roots);
        assert_eq!(
            vm.conses.len(),
            1,
            "closure must not retain the unused cons"
        );
        assert_eq!(vm.closures[roots[0].index(4).unwrap()].captures.len(), 1);
    }

    #[test]
    fn captures_only_compiled_free_variables() {
        let mut vm = Vm::new();
        for (source, count) in [
            ("(let ((a 1) (b 2) (c 3) (d 4)) (lambda (x) (+ a x)))", 1),
            ("(let ((a 1) (b 2)) (lambda (a) a))", 0),
            ("(let ((a 1) (b 2)) (lambda () 'a))", 0),
            ("(let ((a 1) (b 2)) (lambda () (lambda () b)))", 1),
            (
                "(defmacro read-a () 'a) (let ((a 1) (b 2)) (lambda () (read-a)))",
                1,
            ),
        ] {
            let value = vm.eval_source_raw(source).unwrap();
            let closure = &vm.closures[value.index(4).unwrap()];
            assert_eq!(closure.captures.len(), count, "{source}");
            assert_eq!(vm.functions[closure.function].captures, count);
        }
    }
}
mod orderless;
mod fzf;
pub use fzf::advance_completion_generation;
