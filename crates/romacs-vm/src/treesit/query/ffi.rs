//! The temporary adapter from C grammar tables and cursors to Rust queries.
use super::execute::{Cursor, Movement, Node, Span, Status};
use super::model::{Action, Field, Grammar, GrammarTables, StateId, Symbol, SymbolInfo};
use std::marker::PhantomData;
use tree_sitter::{Language, ffi};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RowAction {
    lookahead: u16,
    kind: u16,
    symbol_or_state: u16,
    production: u16,
    child_count: u16,
}

#[repr(C)]
struct CursorStatus {
    field: u16,
    flags: u16,
    supertypes: [u16; 8],
}

#[repr(C)]
struct ByteSpan {
    start: u32,
    end: u32,
}

// UNSAFE(the grammar and cursor accessors are defined in the remaining C runtime)
// SAFETY: these declarations use the C runtime's public handles and the
// adapter's fixed-width records. The wrapper borrows the language and tree.
unsafe extern "C" {
    fn romacs_query_actions(
        language: *const ffi::TSLanguage,
        state: u16,
        out: *mut RowAction,
        capacity: u32,
    ) -> u32;
    fn romacs_query_primary_state(language: *const ffi::TSLanguage, state: u16) -> bool;
    fn romacs_query_nonterminal_start(language: *const ffi::TSLanguage) -> u32;
    fn romacs_query_nonterminal_end(language: *const ffi::TSLanguage) -> u32;
    fn romacs_query_symbol_flags(language: *const ffi::TSLanguage, symbol: u16) -> u16;
    fn ts_language_public_symbol(language: *const ffi::TSLanguage, symbol: u16) -> u16;
    fn romacs_query_symbol_aliases(
        language: *const ffi::TSLanguage,
        symbol: u16,
        out: *mut u16,
        capacity: u32,
    ) -> u32;
    fn romacs_query_production_child(
        language: *const ffi::TSLanguage,
        production: u16,
        child: u16,
        alias: *mut u16,
        field: *mut u16,
    );
    fn romacs_query_cursor_status(cursor: *const ffi::TSTreeCursor) -> CursorStatus;
    fn romacs_query_cursor_repetition(cursor: *const ffi::TSTreeCursor) -> u16;
    fn romacs_query_node_span(node: ffi::TSNode) -> ByteSpan;
    fn ts_tree_cursor_parent_node(cursor: *const ffi::TSTreeCursor) -> ffi::TSNode;
    fn ts_tree_cursor_goto_first_child_internal(cursor: *mut ffi::TSTreeCursor) -> u32;
    fn ts_tree_cursor_goto_next_sibling_internal(cursor: *mut ffi::TSTreeCursor) -> u32;
}

pub(super) struct CGrammar<'a> {
    language: &'a Language,
    raw: *const ffi::TSLanguage,
    // CELL(Plain)
    actions: std::cell::RefCell<Vec<RowAction>>,
}

impl<'a> CGrammar<'a> {
    pub(super) fn new(language: &'a Language) -> Self {
        Self {
            language,
            raw: language.as_raw(),
            actions: std::cell::RefCell::new(vec![RowAction::default(); 128]),
        }
    }
}

impl Grammar for CGrammar<'_> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the binding widens a u32 ABI version"
    )]
    fn abi(&self) -> u32 {
        self.language.abi_version() as u32
    }

    fn symbol(&self, name: &[u8], named: bool) -> Option<Symbol> {
        let len = u32::try_from(name.len()).ok()?;
        // UNSAFE(symbol lookup accepts length-delimited query bytes)
        // SAFETY: the grammar is live and the query byte slice is valid.
        nonzero(unsafe {
            ffi::ts_language_symbol_for_name(self.raw, name.as_ptr().cast(), len, named)
        })
    }

    fn field(&self, name: &[u8]) -> Option<Field> {
        self.language
            .field_id_for_name(name)
            .map(std::num::NonZero::get)
    }

    fn is_supertype(&self, symbol: Symbol) -> bool {
        // UNSAFE(ABI 14 supertype metadata is not exposed by the safe binding)
        // SAFETY: symbol came from this grammar or is ERROR.
        unsafe { romacs_query_symbol_flags(self.raw, symbol) & 4 != 0 }
    }

    fn is_subtype(&self, supertype: Symbol, symbol: Symbol) -> bool {
        self.language
            .subtypes_for_supertype(supertype)
            .contains(&symbol)
    }
}

impl GrammarTables for CGrammar<'_> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the grammar row counts and fills at most u32 records"
    )]
    fn actions(&self, state: StateId, into: &mut Vec<(Symbol, Action)>) {
        let mut buffer = self.actions.borrow_mut();
        let read = |buffer: &mut [RowAction]| {
            // UNSAFE(the safe lookahead iterator omits parse actions)
            // SAFETY: state refers to this live grammar, and the adapter writes
            // at most the supplied number of initialized records.
            unsafe {
                romacs_query_actions(self.raw, state, buffer.as_mut_ptr(), buffer.len() as u32)
                    as usize
            }
        };
        let size = read(&mut buffer);
        if size > buffer.len() {
            buffer.resize(size, RowAction::default());
            let _ = read(&mut buffer);
        }
        into.clear();
        into.extend(
            buffer[..size]
                .iter()
                .map(|entry| (entry.lookahead, entry.action())),
        );
    }

    fn primary(&self, state: StateId) -> bool {
        // UNSAFE(grammar primary-state metadata is private to the C tables)
        // SAFETY: state is in bounds for this live grammar.
        unsafe { romacs_query_primary_state(self.raw, state) }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the binding widens a u32 state count"
    )]
    fn state_count(&self) -> u32 {
        self.language.parse_state_count() as u32
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "symbols and their bounds fit the grammar ABI"
    )]
    fn nonterminals(&self) -> std::ops::Range<Symbol> {
        // UNSAFE(the public node-kind count includes appended aliases)
        // SAFETY: the adapter reads only this live language's header.
        unsafe {
            romacs_query_nonterminal_start(self.raw) as u16
                ..romacs_query_nonterminal_end(self.raw) as u16
        }
    }

    fn symbol_info(&self, symbol: Symbol) -> SymbolInfo {
        // UNSAFE(query analysis needs visible and named independently)
        // SAFETY: symbol came from a parse row in this live grammar.
        unsafe {
            let flags = romacs_query_symbol_flags(self.raw, symbol);
            SymbolInfo {
                public: ts_language_public_symbol(self.raw, symbol),
                named: flags & 2 != 0,
                visible: flags & 1 != 0,
            }
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the adapter counts and fills at most u32 aliases"
    )]
    fn aliases(&self, symbol: Symbol, into: &mut Vec<Symbol>) {
        let read = |buffer: &mut [Symbol]| {
            // UNSAFE(parse-symbol aliases are private to the C tables)
            // SAFETY: symbol came from this live grammar's parse row. The
            // adapter writes at most the supplied output length.
            unsafe {
                romacs_query_symbol_aliases(
                    self.raw,
                    symbol,
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                ) as usize
            }
        };
        into.resize(into.capacity().max(16), 0);
        let len = read(into);
        if len > into.len() {
            into.resize(len, 0);
            let _ = read(into);
        }
        into.truncate(len);
    }

    fn child(&self, production: u16, child: u16) -> (Option<Symbol>, Option<Field>) {
        let (mut alias, mut field) = (0, 0);
        // UNSAFE(production aliases and direct fields are stored in C tables)
        // SAFETY: production and child come from this live grammar's reduce
        // action, and the two output locations are distinct and initialized.
        unsafe {
            romacs_query_production_child(
                self.raw,
                production,
                child,
                &raw mut alias,
                &raw mut field,
            );
        }
        (nonzero(alias), nonzero(field))
    }
}

impl RowAction {
    fn action(self) -> Action {
        match self.kind {
            0 => Action::Goto(self.symbol_or_state),
            1 => Action::Shift(self.symbol_or_state),
            2 => Action::Extra,
            3 => Action::Reduce {
                symbol: self.symbol_or_state,
                production: self.production,
                children: self.child_count,
            },
            _ => Action::Other,
        }
    }
}

fn nonzero(value: u16) -> Option<u16> {
    (value != 0).then_some(value)
}

pub(super) struct CCursor<'tree> {
    raw: ffi::TSTreeCursor,
    tree: PhantomData<tree_sitter::Node<'tree>>,
}

impl<'tree> CCursor<'tree> {
    pub(super) fn for_node(
        tree: &'tree tree_sitter::Tree,
        node: super::Node,
    ) -> Result<Self, super::InvalidQueryNode> {
        let raw = node.raw();
        if !std::ptr::eq(raw.tree, tree.as_raw()) || raw.id.is_null() {
            return Err(super::InvalidQueryNode);
        }
        // UNSAFE(the native query cursor walks hidden nodes)
        // SAFETY: the handle comes from this borrowed tree and is non-null.
        let raw = unsafe { ffi::ts_tree_cursor_new(raw) };
        Ok(Self {
            raw,
            tree: PhantomData,
        })
    }
}

impl<'tree> Cursor for CCursor<'tree> {
    type Node = tree_sitter::Node<'tree>;

    fn node(&self) -> Self::Node {
        // UNSAFE(the low-level cursor can visit a hidden node)
        // SAFETY: this cursor and all nodes it returns belong to the live tree.
        unsafe {
            tree_sitter::Node::from_raw(ffi::ts_tree_cursor_current_node(&raw const self.raw))
        }
    }

    #[inline]
    fn parent_node(&self) -> Option<Self::Node> {
        // UNSAFE(the scoped cursor's public parent operation changes position)
        // SAFETY: a non-null result borrows the same tree as this cursor.
        unsafe {
            let node = ts_tree_cursor_parent_node(&raw const self.raw);
            (!node.id.is_null()).then(|| tree_sitter::Node::from_raw(node))
        }
    }

    fn status(&self) -> Status {
        // UNSAFE(effective fields and hidden supertypes are cursor internals)
        // SAFETY: the cursor is live and the adapter returns copied facts.
        let status = unsafe { romacs_query_cursor_status(&raw const self.raw) };
        Status {
            field: nonzero(status.field),
            later: status.flags & 1 != 0,
            later_named: status.flags & 2 != 0,
            later_field: status.flags & 4 != 0,
            supertypes: status.supertypes,
        }
    }

    fn repetition(&self) -> Option<Symbol> {
        // UNSAFE(hidden repetition symbols are cursor internals)
        // SAFETY: the cursor holds a live tree and the result is copied.
        nonzero(unsafe { romacs_query_cursor_repetition(&raw const self.raw) })
    }

    fn first_child(&mut self) -> Movement {
        // UNSAFE(the public child operation skips hidden repetitions)
        // SAFETY: we hold the unique live cursor and borrow its tree.
        movement(unsafe { ts_tree_cursor_goto_first_child_internal(&raw mut self.raw) })
    }

    fn next_sibling(&mut self) -> Movement {
        // UNSAFE(the public sibling operation skips hidden repetitions)
        // SAFETY: we hold the unique live cursor and borrow its tree.
        movement(unsafe { ts_tree_cursor_goto_next_sibling_internal(&raw mut self.raw) })
    }

    fn parent(&mut self) -> bool {
        // UNSAFE(the Rust binding owns a different cursor representation)
        // SAFETY: we hold the unique live cursor and borrow its tree.
        unsafe { ffi::ts_tree_cursor_goto_parent(&raw mut self.raw) }
    }
}

impl Drop for CCursor<'_> {
    fn drop(&mut self) {
        // UNSAFE(the cursor stack was allocated by the C runtime)
        // SAFETY: this wrapper owns the cursor and frees it once.
        unsafe { ffi::ts_tree_cursor_delete(&raw mut self.raw) }
    }
}

impl Node for tree_sitter::Node<'_> {
    fn symbol(self) -> Symbol {
        self.kind_id()
    }
    fn named(self) -> bool {
        self.is_named()
    }
    fn missing(self) -> bool {
        self.is_missing()
    }
    #[inline]
    fn span(self) -> Span {
        // UNSAFE(the adapter reads both byte coordinates across the C boundary once)
        // SAFETY: Node borrows its live tree for this call.
        let ByteSpan { start, end } = unsafe { romacs_query_node_span(self.into_raw()) };
        Span { start, end }
    }
    fn has_field(self, field: Field) -> bool {
        self.child_by_field_id(field).is_some()
    }
}

fn movement(value: u32) -> Movement {
    match value {
        1 => Movement::Hidden,
        2 => Movement::Visible,
        _ => Movement::None,
    }
}
