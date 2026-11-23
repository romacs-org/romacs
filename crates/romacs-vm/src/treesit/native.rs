//! The tree-sitter runtime behind GNU treesit.c's primitives, through the
//! tree-sitter crate's safe API. Grammars are dlopened as GNU does
//! (`dynlib.c`). A Lisp node stores the runtime's `TSNode` beside the
//! owner of its generation, so a method can borrow the node for a call.
//! Type names are read raw because the binding panics on a missing name.
//! The Rust query engine borrows this module's trees for structural search.
use std::cell::{OnceCell, RefCell};
use std::ffi::{CStr, CString};
use std::rc::Rc;
use tree_sitter::ffi;

#[cfg(test)]
#[path = "query_oracle_tests.rs"]
mod query_oracle;

pub(crate) use ffi::{TSInputEdit, TSPoint};
use ffi::TSNode;
pub(crate) use super::query::{Match, PredicateStep, Query};

pub(crate) const ABI_VERSION: u32 = tree_sitter::LANGUAGE_VERSION as u32;
pub(crate) const MIN_ABI_VERSION: u32 = tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION as u32;
pub(crate) const POINT_1_0: TSPoint = TSPoint { row: 1, column: 0 };

/// A grammar: the `TSLanguage` of a library that is never closed (GNU
/// keeps every grammar for the session), or one linked in.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Language(tree_sitter::Language);

/// A grammar library, open for the rest of the session: GNU never closes
/// one, even when its symbol is missing.
#[derive(Clone, Copy)]
pub(crate) struct Library(&'static libloading::os::unix::Library);

/// dynlib_open (`RTLD_LAZY | RTLD_GLOBAL`): the library, or dlerror's message.
pub(crate) fn open_library(path: &[u8]) -> Result<Library, String> {
    use libloading::os::unix::{Library as Dl, RTLD_GLOBAL, RTLD_LAZY};
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path).map_err(|_| "invalid file name".to_string())?;
    // SAFETY: loading a grammar runs its initializers, as GNU's dlopen of it
    // does; a tree-sitter grammar library has none that need more than the
    // C runtime this process already has.
    match unsafe { Dl::open(Some(std::ffi::OsStr::from_bytes(path.as_bytes())), RTLD_LAZY | RTLD_GLOBAL) } {
        Ok(library) => Ok(Library(Box::leak(Box::new(library)))),
        Err(libloading::Error::DlOpenUnknown) => Err("dlopen failed".into()),
        Err(error) => Err(error.to_string()),
    }
}

/// dynlib_open of each name GNU tries for STEMS, in its order: the
/// first library that opens and its file name, else None with every
/// attempt's message in ERRORS.
pub(crate) fn open_first(stems: &[Vec<u8>], suffixes: &[Vec<u8>], errors: &mut Vec<String>) -> Option<(Library, Vec<u8>)> {
    for stem in stems {
        // GNU conses each suffix's names before the last's.
        for suffix in suffixes.iter().rev() {
            let mut plain = stem.clone();
            plain.extend_from_slice(suffix);
            let mut candidates = vec![plain.clone()];
            for version in (MIN_ABI_VERSION..=ABI_VERSION).rev() {
                candidates.push([&plain[..], format!(".{version}.0").as_bytes()].concat());
            }
            candidates.push([&plain[..], b".0"].concat());
            candidates.push([&plain[..], b".0.0"].concat());
            for candidate in candidates {
                match open_library(&candidate) {
                    Ok(handle) => return Some((handle, candidate)),
                    Err(message) => errors.push(message),
                }
            }
        }
    }
    None
}

/// dynlib_sym of the grammar function NAME in LIBRARY, called: its language
/// and the library's file name (dynlib_addr), or dlerror's message.
pub(crate) fn language_symbol(library: Library, name: &[u8]) -> Result<(Language, Option<String>), String> {
    let name = CString::new(name).map_err(|_| "invalid symbol name".to_string())?;
    // SAFETY: the symbol is read as an address only, never called through
    // this type.
    let symbol: *mut std::ffi::c_void = match unsafe { library.0.get::<*mut std::ffi::c_void>(name.as_bytes_with_nul()) } {
        Ok(symbol) => *symbol,
        Err(libloading::Error::DlSymUnknown) => std::ptr::null_mut(),
        Err(error) => return Err(error.to_string()),
    };
    if symbol.is_null() {
        return Err(format!("undefined symbol: {}", name.to_string_lossy()));
    }
    let filename = library_file(symbol);
    // SAFETY: a grammar library's `tree_sitter_LANG' takes nothing and
    // returns its static TSLanguage, and the library is never closed.
    let function = unsafe { tree_sitter_language::LanguageFn::from_raw(std::mem::transmute::<*mut std::ffi::c_void, unsafe extern "C" fn() -> *const ()>(symbol)) };
    Ok((Language::from_function(function), filename))
}

/// The file of the loaded library holding ADDRESS (dladdr, which
/// libloading lacks).
fn library_file(address: *const std::ffi::c_void) -> Option<String> {
    let mut info = std::mem::MaybeUninit::<libc::Dl_info>::zeroed();
    // SAFETY: dladdr fills INFO when it returns nonzero; its file name is
    // the loader's string for a library that is never closed.
    unsafe {
        if libc::dladdr(address, info.as_mut_ptr()) == 0 { return None; }
        let name = info.assume_init().dli_fname;
        (!name.is_null()).then(|| CStr::from_ptr(name).to_string_lossy().into_owned())
    }
}

impl Language {
    pub(super) fn binding(&self) -> &tree_sitter::Language {
        &self.0
    }
    /// The language a grammar's `tree_sitter_LANG` returns.
    pub(crate) fn from_function(function: tree_sitter_language::LanguageFn) -> Self {
        Language(tree_sitter::Language::new(function))
    }
    pub(crate) fn abi_version(&self) -> u32 {
        self.0.abi_version() as u32
    }
    /// The loaded grammar's `TSLanguage` address: its identity for the
    /// session, since no grammar library is ever closed (`Library`), so no
    /// other grammar can come to occupy it.
    pub(crate) fn address(&self) -> usize {
        // A static grammar's copy is itself: no count is kept for it.
        self.0.clone().into_raw() as usize
    }
    /// Whether this runtime can parse with it (GNU's throwaway parser check).
    pub(crate) fn compatible(&self) -> bool {
        Parser::new().set_language(self)
    }
}

/// A parse tree, deleted with its last handle. One `Tree` is one tree
/// generation: every node of it holds this handle, so `ts_node_eq`, which
/// compares the tree pointer, is stable for its nodes (§2.1). The second
/// field is the tree's pinned copy once it shares its subtrees: a copy
/// nothing edits. An edit of a tree copies the subtrees it shares instead of
/// changing them in place, and a node of the tree points into those
/// subtrees: the pinned copy keeps them for as long as any tree sharing
/// them, and so any node of one, lives.
pub(crate) struct Tree {
    // CELL(Plain)
    inner: RefCell<Editable>,
    // CELL(Plain)
    pinned: OnceCell<Rc<tree_sitter::Tree>>,
    parsed_from_scratch: bool,
}

struct Editable {
    tree: tree_sitter::Tree,
    revision: Revision,
    comparison: Option<Rc<Comparison>>,
}

/// Copies of a parse share an origin when the parsers belong to the same
/// text's registry, which applies each edit to the whole group. A copy
/// across registries or a parser's private view edit starts a new origin.
#[derive(Clone)]
pub(crate) struct Revision {
    origin: Rc<()>,
    edits: u64,
}

struct Comparison {
    old: Revision,
    included: Vec<(u32, u32)>,
    changed: Vec<(u32, u32)>,
}

impl Revision {
    fn new() -> Self {
        Self { origin: Rc::new(()), edits: 0 }
    }
}

/// A tree no thread holds yet, on its way to or from the one that parses.
pub(crate) struct Loose {
    tree: tree_sitter::Tree,
    parsed_from_scratch: bool,
}

impl Loose {
    pub(crate) fn tree(self) -> Tree {
        Tree::new(self.tree, self.parsed_from_scratch)
    }
}

fn point(p: TSPoint) -> tree_sitter::Point {
    tree_sitter::Point { row: p.row as usize, column: p.column as usize }
}

impl Tree {
    fn new(tree: tree_sitter::Tree, parsed_from_scratch: bool) -> Self {
        let revision = Revision::new();
        Self { inner: RefCell::new(Editable { tree, revision, comparison: None }), pinned: OnceCell::new(), parsed_from_scratch }
    }

    pub(crate) fn parsed_from_scratch(&self) -> bool {
        self.parsed_from_scratch
    }

    pub(crate) fn revision(&self) -> Revision {
        self.inner.borrow().revision.clone()
    }

    pub(crate) fn forget_revision(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.revision = Revision::new();
        inner.comparison = None;
    }

    /// ts_tree_edit of this generation in place, as GNU edits its tree:
    /// nodes made before the edit keep their old offsets.
    pub(crate) fn edit(&self, edit: &TSInputEdit) {
        let mut inner = self.inner.borrow_mut();
        inner.comparison = None;
        if let Some(edits) = inner.revision.edits.checked_add(1) {
            inner.revision.edits = edits;
        } else {
            inner.revision = Revision::new();
        }
        inner.tree.edit(&tree_sitter::InputEdit {
            start_byte: edit.start_byte as usize,
            old_end_byte: edit.old_end_byte as usize,
            new_end_byte: edit.new_end_byte as usize,
            start_position: point(edit.start_point),
            old_end_position: point(edit.old_end_point),
            new_end_position: point(edit.new_end_point),
        });
    }
    pub(crate) fn root(&self) -> Node {
        self.inner.borrow().tree.root_node().into()
    }
    pub(super) fn binding(&self) -> std::cell::Ref<'_, tree_sitter::Tree> {
        std::cell::Ref::map(self.inner.borrow(), |inner| &inner.tree)
    }
    /// This tree, new from a parse, to send to another thread.
    pub(crate) fn loose(self) -> Loose {
        Loose { tree: self.inner.into_inner().tree, parsed_from_scratch: self.parsed_from_scratch }
    }
    /// `ts_tree_copy` for another thread to parse from: a tree of its own,
    /// sharing this one's subtrees. No node is made of it here.
    pub(crate) fn share(&self) -> Loose {
        Loose { tree: self.inner.borrow().tree.clone(), parsed_from_scratch: self.parsed_from_scratch }
    }
    /// ts_tree_copy: a tree of its own to edit, sharing this one's subtrees
    /// until the edits part them, so O(1) in the tree's size. Only an
    /// unedited tree: the nodes its pinned copy keeps are the ones it had.
    pub(crate) fn duplicate(&self) -> Tree {
        let inner = self.inner.borrow();
        let pinned = self.pinned.get_or_init(|| Rc::new(inner.tree.clone()));
        let copy = Editable { tree: inner.tree.clone(), revision: inner.revision.clone(), comparison: inner.comparison.clone() };
        Tree { inner: RefCell::new(copy), pinned: OnceCell::from(pinned.clone()), parsed_from_scratch: self.parsed_from_scratch }
    }
    /// ts_tree_get_changed_ranges from OLD to this tree, as byte ranges.
    pub(crate) fn changed_since(&self, old: &Tree) -> Vec<(u32, u32)> {
        COMPARISONS.with(|n| n.set(n.get() + 1));
        old.inner.borrow().tree.changed_ranges(&self.inner.borrow().tree)
            .map(|r| (r.start_byte as u32, r.end_byte as u32)).collect()
    }

    pub(crate) fn publish_comparison(&mut self, old: Revision, included: Vec<(u32, u32)>, changed: Vec<(u32, u32)>) {
        self.inner.get_mut().comparison = Some(Rc::new(Comparison { old, included, changed }));
    }

    /// The published comparison belongs to this new tree. A handle with
    /// different input still needs its own ranges for its notifiers.
    pub(crate) fn compare(&mut self, old: &Tree, included: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
        let revision = old.revision();
        if let Some(published) = self.inner.get_mut().comparison.as_deref()
            && Rc::ptr_eq(&published.old.origin, &revision.origin)
            && published.old.edits == revision.edits && published.included == included
        {
            return published.changed.clone();
        }
        let changed = self.changed_since(old);
        if self.inner.get_mut().comparison.is_none() && self.included_ranges() == included {
            self.publish_comparison(revision, included, changed.clone());
        }
        changed
    }

    pub(crate) fn included_ranges(&self) -> Vec<(u32, u32)> {
        byte_ranges(&self.inner.borrow().tree.included_ranges())
    }
}

fn byte_ranges(ranges: &[tree_sitter::Range]) -> Vec<(u32, u32)> {
    let byte = |offset: usize| u32::try_from(offset).unwrap_or(u32::MAX);
    ranges.iter().map(|range| (byte(range.start_byte), byte(range.end_byte))).collect()
}

/// One read of a parse: a run of the text, or a raw byte's two internal
/// bytes (`mod.rs` `Reader`).
pub(crate) enum Chunk<'a> {
    Text(&'a [u8]),
    Raw([u8; 2]),
}
impl AsRef<[u8]> for Chunk<'_> {
    fn as_ref(&self) -> &[u8] {
        match self { Chunk::Text(bytes) => bytes, Chunk::Raw(bytes) => bytes }
    }
}

/// The reader of one parse: the bytes after a tree offset, or empty at the
/// end (GNU `treesit_read_buffer`).
pub(crate) trait Source {
    fn read(&self, offset: usize) -> Chunk<'_>;
}

thread_local! {
    /// The parses this thread ran (`parses_on_this_thread`).
    static PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The changed-range comparisons this thread ran.
    static COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// How many changed-range comparisons the calling thread has run.
#[must_use]
pub fn comparisons_on_this_thread() -> usize {
    COMPARISONS.with(std::cell::Cell::get)
}
/// The parses every thread of the process has run.
static PARSES_IN_PROCESS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// How many parses the calling thread has run: what tells a parse the
/// evaluator paid for from one a parse thread ran for it.
pub fn parses_on_this_thread() -> usize {
    PARSES.with(std::cell::Cell::get)
}
/// How many parses the process has run on any thread: what tells that a
/// buffer was not parsed at all.
pub fn parses_in_process() -> usize {
    PARSES_IN_PROCESS.load(std::sync::atomic::Ordering::Relaxed)
}

pub(crate) struct Parser(tree_sitter::Parser);
impl Parser {
    pub(crate) fn new() -> Self {
        Self(tree_sitter::Parser::new())
    }
    /// Its language, None before one is set.
    pub(crate) fn language(&self) -> Option<Language> {
        self.0.language().map(|language| Language((*language).clone()))
    }
    pub(crate) fn set_language(&mut self, language: &Language) -> bool {
        self.0.set_language(&language.0).is_ok()
    }
    /// ts_parser_set_included_ranges; an empty list means the whole text.
    pub(crate) fn set_included_ranges(&mut self, ranges: &[(u32, u32)]) -> bool {
        let zero = tree_sitter::Point { row: 0, column: 0 };
        let ranges: Vec<tree_sitter::Range> = ranges.iter().map(|&(start, end)| tree_sitter::Range {
            start_byte: start as usize, end_byte: end as usize, start_point: zero, end_point: zero,
        }).collect();
        self.0.set_included_ranges(&ranges).is_ok()
    }
    /// `ts_parser_included_ranges`, as `set_included_ranges` takes them.
    pub(crate) fn included_ranges(&self) -> Vec<(u32, u32)> {
        byte_ranges(&self.0.included_ranges())
    }
    /// Whether its included ranges show all of a text of LENGTH bytes: the
    /// runtime's default, or one range that covers it.
    pub(crate) fn sees_all(&self, length: u32) -> bool {
        matches!(self.0.included_ranges()[..], [range] if range.start_byte == 0 && range.end_byte >= length as usize)
    }
    /// ts_parser_parse of SOURCE, reusing OLD's unchanged subtrees.
    pub(crate) fn parse(&mut self, old: Option<&Tree>, source: &dyn Source) -> Option<Tree> {
        self.parse_until(old, source, &crate::quit::NativeStop::never()).expect("never stopped")
    }
    /// `parse`, asking STOP as it goes (the runtime's progress callback):
    /// Err once STOP stopped it, the parser reset so that the next parse
    /// starts afresh rather than resuming this one.
    pub(crate) fn parse_until(&mut self, old: Option<&Tree>, source: &dyn Source, stop: &crate::quit::NativeStop)
                              -> Result<Option<Tree>, crate::quit::Stopped> {
        let old = old.map(|tree| tree.inner.borrow());
        PARSES.with(|n| n.set(n.get() + 1));
        PARSES_IN_PROCESS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (mut stopped, mut calls) = (false, 0u32);
        let mut progress = |_: &tree_sitter::ParseState| {
            calls = calls.wrapping_add(1);
            stopped = calls.is_multiple_of(64) && stop.due();
            if stopped { std::ops::ControlFlow::Break(()) } else { std::ops::ControlFlow::Continue(()) }
        };
        let options = stop.armed().then(|| tree_sitter::ParseOptions::new().progress_callback(&mut progress));
        let tree = self.0.parse_with_options(&mut |offset, _| source.read(offset), old.as_ref().map(|old| &old.tree), options);
        if stopped {
            self.0.reset();
            return Err(crate::quit::Stopped);
        }
        Ok(tree.map(|tree| Tree::new(tree, old.is_none())))
    }
}

/// A node of a live tree generation. Copy, as the runtime's TSNode is; the
/// caller keeps its `Tree` alive (`TreeOwner` in `treesit::Node`).
#[derive(Clone, Copy)]
pub(crate) struct Node(TSNode);

macro_rules! node_flag {
    ($($name:ident),* $(,)?) => {$(
        pub(crate) fn $name(self) -> bool { self.ts().$name() }
    )*};
}
macro_rules! node_step {
    ($($name:ident),* $(,)?) => {$(
        pub(crate) fn $name(self) -> Option<Node> { self.ts().$name().map(Node::from) }
    )*};
}

impl From<tree_sitter::Node<'_>> for Node {
    fn from(node: tree_sitter::Node<'_>) -> Self {
        Node(node.into_raw())
    }
}

impl Node {
    pub(super) fn raw(self) -> TSNode {
        self.0
    }
    /// The crate's view of this node. Its lifetime is the owner's, which
    /// the type cannot name: the view never leaves the method that asks.
    fn ts(self) -> tree_sitter::Node<'static> {
        // SAFETY: a node of a tree its owner keeps alive (see the type),
        // read only while the caller holds the node.
        unsafe { tree_sitter::Node::from_raw(self.0) }
    }
    /// ts_node_is_null: no node, as a step past the last sibling gives.
    pub(crate) fn is_null(self) -> bool {
        self.0.id.is_null()
    }
    node_flag! { is_named, is_missing, is_extra, has_error }
    node_step! { next_sibling, prev_sibling, next_named_sibling, prev_named_sibling }
    /// ts_node_type, None when the runtime has no name for the symbol.
    pub(crate) fn kind(self) -> Option<&'static [u8]> {
        // SAFETY: the node's tree is live (see the type); a non-null name is
        // the grammar's static NUL-terminated string.
        unsafe {
            let name = ffi::ts_node_type(self.0);
            (!name.is_null()).then(|| CStr::from_ptr(name).to_bytes())
        }
    }
    pub(crate) fn start(self) -> u32 {
        self.ts().start_byte() as u32
    }
    pub(crate) fn end(self) -> u32 {
        self.ts().end_byte() as u32
    }
    pub(crate) fn child_count(self, named: bool) -> u32 {
        if named { self.ts().named_child_count() as u32 } else { self.ts().child_count() as u32 }
    }
    pub(crate) fn child(self, index: u32, named: bool) -> Option<Node> {
        if named { self.ts().named_child(index) } else { self.ts().child(index) }.map(Node::from)
    }
    pub(crate) fn field_name_for_child(self, index: u32) -> Option<&'static [u8]> {
        self.ts().field_name_for_child(index).map(str::as_bytes)
    }
    pub(crate) fn child_by_field_name(self, name: &[u8]) -> Option<Node> {
        self.ts().child_by_field_name(name).map(Node::from)
    }
    pub(crate) fn descendant_for_range(self, start: u32, end: u32, named: bool) -> Option<Node> {
        let (start, end) = (start as usize, end as usize);
        if named { self.ts().named_descendant_for_byte_range(start, end) } else { self.ts().descendant_for_byte_range(start, end) }.map(Node::from)
    }
    /// ts_node_string: the S-expression of the subtree.
    pub(crate) fn sexp(self) -> Vec<u8> {
        self.ts().to_sexp().into_bytes()
    }
    /// ts_node_eq: the same tree pointer and node id.
    pub(crate) fn same(self, other: Node) -> bool {
        self.identity() == other.identity()
    }
    /// The identity `ts_node_eq` compares, for `sxhash-equal`.
    pub(crate) fn identity(self) -> (usize, usize) {
        (self.0.tree as usize, self.0.id as usize)
    }
}

/// A tree cursor, deleted when dropped. Like a node, it reads a tree its
/// owner keeps alive.
pub(crate) struct Cursor(tree_sitter::TreeCursor<'static>);
impl Cursor {
    pub(crate) fn new(node: Node) -> Self {
        Self(node.ts().walk())
    }
    pub(crate) fn node(&self) -> Node {
        Node::from(self.0.node())
    }
    pub(crate) fn parent(&mut self) -> bool {
        self.0.goto_parent()
    }
    pub(crate) fn first_child(&mut self) -> bool {
        self.0.goto_first_child()
    }
    pub(crate) fn next_sibling(&mut self) -> bool {
        self.0.goto_next_sibling()
    }
    pub(crate) fn previous_sibling(&mut self) -> bool {
        self.0.goto_previous_sibling()
    }
    /// ts_tree_cursor_goto_first_child_for_byte: the child's index or -1.
    pub(crate) fn first_child_for_byte(&mut self, byte: u32) -> i64 {
        self.0.goto_first_child_for_byte(byte as usize).map_or(-1, |index| index as i64)
    }
}
