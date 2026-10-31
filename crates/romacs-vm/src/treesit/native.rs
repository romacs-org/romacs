//! The tree-sitter runtime behind GNU treesit.c's primitives. This module is
//! the only unsafe boundary: grammars are dlopened as GNU does
//! (`dynlib.c`), and parsers, trees, nodes, cursors and queries are the
//! runtime's own objects behind owning handles. `prim/leaf/treesit.rs`
//! uses the safe types below.
use std::ffi::{CStr, CString, c_char, c_void};
use std::rc::Rc;
use tree_sitter::ffi;

pub(crate) use ffi::{TSInputEdit, TSNode, TSPoint, TSRange};

pub(crate) const ABI_VERSION: u32 = ffi::TREE_SITTER_LANGUAGE_VERSION;
pub(crate) const MIN_ABI_VERSION: u32 = ffi::TREE_SITTER_MIN_COMPATIBLE_LANGUAGE_VERSION;
pub(crate) const POINT_1_0: TSPoint = TSPoint { row: 1, column: 0 };

/// The LENGTH items at POINTER, which the runtime leaves NULL when there
/// are none.
///
/// # Safety
/// A non-null POINTER is valid for LENGTH reads for the caller's borrow.
unsafe fn raw_slice<'a, T>(pointer: *const T, length: u32) -> &'a [T] {
    if pointer.is_null() || length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(pointer, length as usize) } }
}

fn dlerror() -> Option<String> {
    // SAFETY: dlerror returns NULL or a NUL-terminated message.
    let message = unsafe { libc::dlerror() };
    (!message.is_null()).then(|| unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned())
}

/// A grammar: the `TSLanguage` of a library that is never closed (GNU
/// keeps every grammar for the session).
#[derive(Clone, Copy)]
pub(crate) struct Language(*const ffi::TSLanguage);
// SAFETY: a TSLanguage is immutable static data of its library.
unsafe impl Send for Language {}

/// dynlib_open (`RTLD_LAZY | RTLD_GLOBAL`): the handle, or dlerror's message.
pub(crate) fn open_library(path: &[u8]) -> Result<*mut c_void, String> {
    let path = CString::new(path).map_err(|_| "invalid file name".to_string())?;
    dlerror();
    // SAFETY: `path` is NUL-terminated.
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL) };
    match dlerror() {
        Some(message) => Err(message),
        None if handle.is_null() => Err("dlopen failed".into()),
        None => Ok(handle),
    }
}

/// dynlib_sym of the grammar function NAME in HANDLE, called: its language
/// and the library's file name (dynlib_addr), or dlerror's message.
pub(crate) fn language_symbol(handle: *mut c_void, name: &[u8]) -> Result<(Language, Option<String>), String> {
    let name = CString::new(name).map_err(|_| "invalid symbol name".to_string())?;
    dlerror();
    // SAFETY: `handle` came from dlopen and is never closed.
    let symbol = unsafe { libc::dlsym(handle, name.as_ptr()) };
    if let Some(message) = dlerror() {
        return Err(message);
    }
    if symbol.is_null() {
        return Err(format!("undefined symbol: {}", name.to_string_lossy()));
    }
    // SAFETY: a grammar library's `tree_sitter_LANG` returns its static TSLanguage.
    let function: extern "C" fn() -> *const ffi::TSLanguage = unsafe { std::mem::transmute(symbol) };
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: `symbol` is an address in a loaded library.
    let filename = (unsafe { libc::dladdr(symbol, &mut info) } != 0 && !info.dli_fname.is_null())
        .then(|| unsafe { CStr::from_ptr(info.dli_fname) }.to_string_lossy().into_owned());
    Ok((Language(function()), filename))
}

impl Language {
    pub(crate) fn abi_version(self) -> u32 {
        // SAFETY: a loaded grammar.
        unsafe { ffi::ts_language_abi_version(self.0) }
    }
    /// Whether this runtime can parse with it (GNU's throwaway parser check).
    pub(crate) fn compatible(self) -> bool {
        Parser::new().set_language(self)
    }
}

/// A parse tree, deleted with its last handle. One `Tree` is one tree
/// generation: every node of it holds this handle, so `ts_node_eq`, which
/// compares the tree pointer, is stable for its nodes (§2.1).
pub(crate) struct Tree(*mut ffi::TSTree);
impl Drop for Tree {
    fn drop(&mut self) {
        // SAFETY: owned since ts_parser_parse returned it.
        unsafe { ffi::ts_tree_delete(self.0) };
    }
}
impl Tree {
    /// ts_tree_edit of this generation in place, as GNU edits its tree:
    /// nodes made before the edit keep their old offsets.
    pub(crate) fn edit(&self, edit: &TSInputEdit) {
        // SAFETY: the tree is live; the VM is single-threaded.
        unsafe { ffi::ts_tree_edit(self.0, edit) };
    }
    pub(crate) fn root(&self) -> TSNode {
        // SAFETY: the tree is live.
        unsafe { ffi::ts_tree_root_node(self.0) }
    }
    /// ts_tree_get_changed_ranges from OLD to this tree, as byte ranges.
    pub(crate) fn changed_since(&self, old: &Tree) -> Vec<(u32, u32)> {
        let mut length = 0;
        // SAFETY: both trees are live; the array is the runtime's malloc.
        unsafe {
            let ranges = ffi::ts_tree_get_changed_ranges(old.0, self.0, &mut length);
            let out = raw_slice(ranges, length).iter().map(|r| (r.start_byte, r.end_byte)).collect();
            libc::free(ranges.cast());
            out
        }
    }
}

/// The reader of one parse: the bytes after a tree offset, or empty at the
/// end (GNU `treesit_read_buffer`).
pub(crate) trait Source {
    fn read(&mut self, offset: usize) -> &[u8];
}

unsafe extern "C" fn read_source(payload: *mut c_void, byte: u32, _: TSPoint, read: *mut u32) -> *const c_char {
    // SAFETY: `payload` is the `&mut dyn Source` of `Parser::parse`, live for the call.
    let source = unsafe { &mut **payload.cast::<&mut dyn Source>() };
    let bytes = source.read(byte as usize);
    unsafe { *read = bytes.len() as u32 };
    bytes.as_ptr().cast()
}

pub(crate) struct Parser(*mut ffi::TSParser);
impl Drop for Parser {
    fn drop(&mut self) {
        // SAFETY: owned since ts_parser_new.
        unsafe { ffi::ts_parser_delete(self.0) };
    }
}
impl Parser {
    pub(crate) fn new() -> Self {
        // SAFETY: allocates a parser.
        Self(unsafe { ffi::ts_parser_new() })
    }
    pub(crate) fn language(&self) -> Language {
        // SAFETY: the parser is live; its language was set at creation.
        Language(unsafe { ffi::ts_parser_language(self.0) })
    }
    pub(crate) fn set_language(&mut self, language: Language) -> bool {
        // SAFETY: the parser is live and the language loaded.
        unsafe { ffi::ts_parser_set_language(self.0, language.0) }
    }
    /// ts_parser_set_included_ranges; an empty list means the whole text.
    pub(crate) fn set_included_ranges(&mut self, ranges: &[(u32, u32)]) -> bool {
        let zero = TSPoint { row: 0, column: 0 };
        let ranges: Vec<TSRange> = ranges.iter().map(|&(start_byte, end_byte)| TSRange { start_point: zero, end_point: zero, start_byte, end_byte }).collect();
        // SAFETY: the array is read for its length during the call.
        unsafe { ffi::ts_parser_set_included_ranges(self.0, if ranges.is_empty() { std::ptr::null() } else { ranges.as_ptr() }, ranges.len() as u32) }
    }
    /// ts_parser_parse of SOURCE, reusing OLD's unchanged subtrees.
    pub(crate) fn parse(&mut self, old: Option<&Tree>, mut source: &mut dyn Source) -> Option<Tree> {
        let input = ffi::TSInput {
            payload: (&mut source as *mut &mut dyn Source).cast(),
            read: Some(read_source),
            encoding: ffi::TSInputEncodingUTF8,
            decode: None,
        };
        // SAFETY: `input.payload` outlives the call; OLD is live.
        let tree = unsafe { ffi::ts_parser_parse(self.0, old.map_or(std::ptr::null(), |t| t.0), input) };
        (!tree.is_null()).then(|| Tree(tree))
    }
}

/// A node of a live tree generation. Copy, as the runtime's TSNode is; the
/// caller keeps its `Tree` alive (`TreeOwner` in `treesit::Node`).
#[derive(Clone, Copy)]
pub(crate) struct Node(pub(crate) TSNode);

macro_rules! node_flag {
    ($($name:ident = $f:ident),* $(,)?) => {$(
        pub(crate) fn $name(self) -> bool {
            // SAFETY: the node's tree is live (see the type).
            unsafe { ffi::$f(self.0) }
        }
    )*};
}
macro_rules! node_step {
    ($($name:ident = $f:ident),* $(,)?) => {$(
        pub(crate) fn $name(self) -> Option<Node> {
            // SAFETY: the node's tree is live (see the type).
            Node(unsafe { ffi::$f(self.0) }).present()
        }
    )*};
}

impl Node {
    fn present(self) -> Option<Node> {
        (!self.is_null()).then_some(self)
    }
    node_flag! {
        is_null = ts_node_is_null, is_named = ts_node_is_named, is_missing = ts_node_is_missing,
        is_extra = ts_node_is_extra, has_error = ts_node_has_error,
    }
    node_step! {
        next_sibling = ts_node_next_sibling, prev_sibling = ts_node_prev_sibling,
        next_named_sibling = ts_node_next_named_sibling, prev_named_sibling = ts_node_prev_named_sibling,
    }
    /// ts_node_type, None when the runtime has no name for the symbol.
    pub(crate) fn kind(self) -> Option<&'static [u8]> {
        // SAFETY: names are the grammar's static strings.
        let name = unsafe { ffi::ts_node_type(self.0) };
        (!name.is_null()).then(|| unsafe { CStr::from_ptr(name) }.to_bytes())
    }
    pub(crate) fn start(self) -> u32 {
        // SAFETY: see the type.
        unsafe { ffi::ts_node_start_byte(self.0) }
    }
    pub(crate) fn end(self) -> u32 {
        // SAFETY: see the type.
        unsafe { ffi::ts_node_end_byte(self.0) }
    }
    pub(crate) fn child_count(self, named: bool) -> u32 {
        // SAFETY: see the type.
        unsafe { if named { ffi::ts_node_named_child_count(self.0) } else { ffi::ts_node_child_count(self.0) } }
    }
    pub(crate) fn child(self, index: u32, named: bool) -> Option<Node> {
        // SAFETY: see the type.
        Node(unsafe { if named { ffi::ts_node_named_child(self.0, index) } else { ffi::ts_node_child(self.0, index) } }).present()
    }
    pub(crate) fn field_name_for_child(self, index: u32) -> Option<&'static [u8]> {
        // SAFETY: field names are the grammar's static strings.
        let name = unsafe { ffi::ts_node_field_name_for_child(self.0, index) };
        (!name.is_null()).then(|| unsafe { CStr::from_ptr(name) }.to_bytes())
    }
    pub(crate) fn child_by_field_name(self, name: &[u8]) -> Option<Node> {
        // SAFETY: NAME is read for its length.
        Node(unsafe { ffi::ts_node_child_by_field_name(self.0, name.as_ptr().cast(), name.len() as u32) }).present()
    }
    pub(crate) fn descendant_for_range(self, start: u32, end: u32, named: bool) -> Option<Node> {
        // SAFETY: see the type.
        Node(unsafe {
            if named { ffi::ts_node_named_descendant_for_byte_range(self.0, start, end) } else { ffi::ts_node_descendant_for_byte_range(self.0, start, end) }
        }).present()
    }
    /// ts_node_string: the S-expression of the subtree.
    pub(crate) fn sexp(self) -> Vec<u8> {
        // SAFETY: the string is the runtime's malloc, freed here.
        unsafe {
            let string = ffi::ts_node_string(self.0);
            let out = CStr::from_ptr(string).to_bytes().to_vec();
            libc::free(string.cast());
            out
        }
    }
    /// ts_node_eq: the same tree pointer and node id.
    pub(crate) fn same(self, other: Node) -> bool {
        // SAFETY: compares the handles only.
        unsafe { ffi::ts_node_eq(self.0, other.0) }
    }
    /// The identity `ts_node_eq` compares, for `sxhash-equal`.
    pub(crate) fn identity(self) -> (usize, usize) {
        (self.0.tree as usize, self.0.id as usize)
    }
}

/// A tree cursor, deleted when dropped.
pub(crate) struct Cursor(ffi::TSTreeCursor);
impl Drop for Cursor {
    fn drop(&mut self) {
        // SAFETY: owned since ts_tree_cursor_new.
        unsafe { ffi::ts_tree_cursor_delete(&mut self.0) };
    }
}
impl Cursor {
    pub(crate) fn new(node: Node) -> Self {
        // SAFETY: see `Node`.
        Self(unsafe { ffi::ts_tree_cursor_new(node.0) })
    }
    pub(crate) fn node(&self) -> Node {
        // SAFETY: the cursor is live.
        Node(unsafe { ffi::ts_tree_cursor_current_node(&self.0) })
    }
    pub(crate) fn parent(&mut self) -> bool {
        // SAFETY: the cursor is live.
        unsafe { ffi::ts_tree_cursor_goto_parent(&mut self.0) }
    }
    pub(crate) fn first_child(&mut self) -> bool {
        // SAFETY: the cursor is live.
        unsafe { ffi::ts_tree_cursor_goto_first_child(&mut self.0) }
    }
    pub(crate) fn next_sibling(&mut self) -> bool {
        // SAFETY: the cursor is live.
        unsafe { ffi::ts_tree_cursor_goto_next_sibling(&mut self.0) }
    }
    pub(crate) fn previous_sibling(&mut self) -> bool {
        // SAFETY: the cursor is live.
        unsafe { ffi::ts_tree_cursor_goto_previous_sibling(&mut self.0) }
    }
    /// ts_tree_cursor_goto_first_child_for_byte: the child's index or -1.
    pub(crate) fn first_child_for_byte(&mut self, byte: u32) -> i64 {
        // SAFETY: the cursor is live.
        unsafe { ffi::ts_tree_cursor_goto_first_child_for_byte(&mut self.0, byte) }
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

impl Query {
    /// ts_query_new of SOURCE for LANGUAGE, or the error's offset and
    /// GNU's description of its type.
    pub(crate) fn new(language: Language, source: &[u8]) -> Result<Rc<Self>, (u32, &'static str)> {
        let (mut offset, mut kind) = (0, 0);
        // SAFETY: SOURCE is read for its length.
        let query = unsafe { ffi::ts_query_new(language.0, source.as_ptr().cast(), source.len() as u32, &mut offset, &mut kind) };
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
        // SAFETY: the cursor is ours; NODE's tree is live (see `Node`).
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
