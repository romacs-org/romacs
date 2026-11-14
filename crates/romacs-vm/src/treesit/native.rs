//! The tree-sitter runtime behind GNU treesit.c's primitives, through the
//! tree-sitter crate's safe API. Grammars are dlopened as GNU does
//! (`dynlib.c`). Three things stay raw, each for a stated reason: a node is
//! kept as the runtime's `TSNode` beside an owner that keeps its tree alive
//! (a Lisp value cannot hold the crate's borrowed `Node<'tree>`), so one
//! conversion gives it back its lifetime; a node's type name is read raw,
//! since the crate panics where the runtime has none; and queries are the
//! runtime's own, since the crate's `Query` evaluates `#match?` and `#eq?`
//! itself, with Rust's regex syntax, where GNU evaluates every predicate in
//! Lisp. `prim/leaf/treesit.rs` uses the safe types below.
use std::cell::{OnceCell, RefCell};
use std::ffi::{CStr, CString};
use std::rc::Rc;
use tree_sitter::ffi;

pub(crate) use ffi::{TSInputEdit, TSNode, TSPoint};

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
    /// The language a grammar's `tree_sitter_LANG` returns.
    pub(crate) fn from_function(function: tree_sitter_language::LanguageFn) -> Self {
        Language(tree_sitter::Language::new(function))
    }
    pub(crate) fn abi_version(&self) -> u32 {
        self.0.abi_version() as u32
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
pub(crate) struct Tree(RefCell<tree_sitter::Tree>, OnceCell<Rc<tree_sitter::Tree>>);

/// A tree no thread holds yet, on its way from the one that parsed it.
pub(crate) struct Loose(tree_sitter::Tree);
impl Loose {
    pub(crate) fn tree(self) -> Tree {
        Tree(RefCell::new(self.0), OnceCell::new())
    }
}

fn point(p: TSPoint) -> tree_sitter::Point {
    tree_sitter::Point { row: p.row as usize, column: p.column as usize }
}

impl Tree {
    /// ts_tree_edit of this generation in place, as GNU edits its tree:
    /// nodes made before the edit keep their old offsets.
    pub(crate) fn edit(&self, edit: &TSInputEdit) {
        self.0.borrow_mut().edit(&tree_sitter::InputEdit {
            start_byte: edit.start_byte as usize,
            old_end_byte: edit.old_end_byte as usize,
            new_end_byte: edit.new_end_byte as usize,
            start_position: point(edit.start_point),
            old_end_position: point(edit.old_end_point),
            new_end_position: point(edit.new_end_point),
        });
    }
    pub(crate) fn root(&self) -> TSNode {
        self.0.borrow().root_node().into_raw()
    }
    /// This tree, new from a parse, to send to another thread.
    pub(crate) fn loose(self) -> Loose {
        Loose(self.0.into_inner())
    }
    /// ts_tree_copy: a tree of its own to edit, sharing this one's subtrees
    /// until the edits part them, so O(1) in the tree's size. Only an
    /// unedited tree: the nodes its pinned copy keeps are the ones it had.
    pub(crate) fn duplicate(&self) -> Tree {
        let tree = self.0.borrow();
        let pinned = self.1.get_or_init(|| Rc::new(tree.clone()));
        Tree(RefCell::new(tree.clone()), OnceCell::from(pinned.clone()))
    }
    /// ts_tree_get_changed_ranges from OLD to this tree, as byte ranges.
    pub(crate) fn changed_since(&self, old: &Tree) -> Vec<(u32, u32)> {
        old.0.borrow().changed_ranges(&self.0.borrow()).map(|r| (r.start_byte as u32, r.end_byte as u32)).collect()
    }
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
    /// Whether its included ranges show all of a text of LENGTH bytes: the
    /// runtime's default, or one range that covers it.
    pub(crate) fn sees_all(&self, length: u32) -> bool {
        matches!(self.0.included_ranges()[..], [range] if range.start_byte == 0 && range.end_byte >= length as usize)
    }
    /// ts_parser_parse of SOURCE, reusing OLD's unchanged subtrees.
    pub(crate) fn parse(&mut self, old: Option<&Tree>, source: &dyn Source) -> Option<Tree> {
        let old = old.map(|tree| tree.0.borrow());
        let tree = self.0.parse_with_options(&mut |offset, _| source.read(offset), old.as_deref(), None)?;
        Some(Tree(RefCell::new(tree), OnceCell::new()))
    }
}

/// A node of a live tree generation. Copy, as the runtime's TSNode is; the
/// caller keeps its `Tree` alive (`TreeOwner` in `treesit::Node`).
#[derive(Clone, Copy)]
pub(crate) struct Node(pub(crate) TSNode);

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

/// A compiled query, deleted when dropped.
pub(crate) struct Query(*mut ffi::TSQuery);
impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: owned since ts_query_new.
        unsafe { ffi::ts_query_delete(self.0) };
    }
}

/// One step of a pattern's predicates.
pub(crate) enum PredicateStep<'a> {
    /// A capture, by its index in the query.
    Capture(u32),
    String(&'a [u8]),
    Done,
}

/// A match: its pattern and its captures (node, capture index), in order.
pub(crate) struct Match {
    pub pattern: u32,
    pub captures: Vec<(Node, u32)>,
}

/// The LENGTH items at POINTER, which the runtime leaves NULL when there
/// are none.
///
/// # Safety
/// A non-null POINTER is valid for LENGTH reads for the caller's borrow.
unsafe fn raw_slice<'a, T>(pointer: *const T, length: u32) -> &'a [T] {
    if pointer.is_null() || length == 0 {
        return &[];
    }
    // SAFETY: the caller's contract, for a non-null POINTER.
    unsafe { std::slice::from_raw_parts(pointer, length as usize) }
}

impl Query {
    /// ts_query_new of SOURCE for LANGUAGE, or the error's offset and
    /// GNU's description of its type.
    pub(crate) fn new(language: &Language, source: &[u8]) -> Result<Rc<Self>, (u32, &'static str)> {
        let (mut offset, mut kind) = (0, 0);
        // A static grammar's copy is itself: no count is kept for it.
        let language = language.0.clone().into_raw();
        // SAFETY: SOURCE is read for its length; the language is loaded.
        let query = unsafe { ffi::ts_query_new(language, source.as_ptr().cast(), source.len() as u32, &mut offset, &mut kind) };
        if !query.is_null() {
            return Ok(Rc::new(Self(query)));
        }
        let message = match kind {
            ffi::TSQueryErrorNone => "None",
            ffi::TSQueryErrorSyntax => "Syntax error at",
            ffi::TSQueryErrorNodeType => "Node type error at",
            ffi::TSQueryErrorField => "Field error at",
            ffi::TSQueryErrorCapture => "Capture error at",
            ffi::TSQueryErrorStructure => "Structure error at",
            _ => "Unknown error",
        };
        Err((offset, message))
    }
    pub(crate) fn pattern_count(&self) -> u32 {
        // SAFETY: the query is live.
        unsafe { ffi::ts_query_pattern_count(self.0) }
    }
    pub(crate) fn capture_name(&self, index: u32) -> &[u8] {
        let mut length = 0;
        // SAFETY: names live as long as the query.
        unsafe { raw_slice(ffi::ts_query_capture_name_for_id(self.0, index, &mut length).cast(), length) }
    }
    fn string_value(&self, index: u32) -> &[u8] {
        let mut length = 0;
        // SAFETY: strings live as long as the query.
        unsafe { raw_slice(ffi::ts_query_string_value_for_id(self.0, index, &mut length).cast(), length) }
    }
    pub(crate) fn predicates(&self, pattern: u32) -> Vec<PredicateStep<'_>> {
        let mut length = 0;
        // SAFETY: the steps live as long as the query.
        let steps = unsafe { raw_slice(ffi::ts_query_predicates_for_pattern(self.0, pattern, &mut length), length) };
        steps.iter().map(|step| match step.type_ {
            ffi::TSQueryPredicateStepTypeCapture => PredicateStep::Capture(step.value_id),
            ffi::TSQueryPredicateStepTypeString => PredicateStep::String(self.string_value(step.value_id)),
            _ => PredicateStep::Done,
        }).collect()
    }
    /// Every match of the query under NODE whose span overlaps the byte
    /// RANGE, in ts_query_cursor_next_match order.
    pub(crate) fn matches(&self, node: Node, range: (u32, u32)) -> Vec<Match> {
        // SAFETY: the cursor is ours; NODE's tree is live (see `Node`); a
        // match's captures are read before the next step.
        unsafe {
            let cursor = ffi::ts_query_cursor_new();
            ffi::ts_query_cursor_set_byte_range(cursor, range.0, range.1);
            ffi::ts_query_cursor_exec(cursor, self.0, node.0);
            let mut out = Vec::new();
            let mut found: ffi::TSQueryMatch = std::mem::zeroed();
            while ffi::ts_query_cursor_next_match(cursor, &mut found) {
                let captures = raw_slice(found.captures, u32::from(found.capture_count));
                out.push(Match { pattern: u32::from(found.pattern_index), captures: captures.iter().map(|c| (Node(c.node), c.index)).collect() });
            }
            ffi::ts_query_cursor_delete(cursor);
            out
        }
    }
}
