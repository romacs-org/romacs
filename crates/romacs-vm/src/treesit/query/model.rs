//! Query bytecode shared by compilation, grammar analysis, and execution.
use std::ops::Range;

pub(super) type Symbol = u16;
pub(super) type Field = u16;
pub(super) type CaptureId = u32;
pub(super) type StepId = usize;

pub(super) const WILDCARD: Symbol = 0;
pub(super) const ERROR: Symbol = u16::MAX;

// The compiler bounds the source to u32 bytes, and every entry in these
// tables consumes source bytes.
#[expect(
    clippy::cast_possible_truncation,
    reason = "source bounds query offsets and IDs"
)]
pub(super) fn query_index(index: usize) -> u32 {
    index as u32
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ErrorKind {
    Syntax,
    NodeType,
    Field,
    Capture,
    Structure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Error {
    pub offset: usize,
    pub kind: ErrorKind,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Predicate {
    Capture(CaptureId),
    String(u32),
    Done,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Operation {
    Match,
    Repeat,
    Jump,
    Accept,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Alternative {
    pub step: StepId,
    pub skips_repeat: bool,
}

#[derive(Clone, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a step carries independent query and analysis flags"
)]
pub(super) struct Step {
    pub operation: Operation,
    pub offset: usize,
    pub depth: u32,
    pub symbol: Symbol,
    pub supertype: Option<Symbol>,
    pub field: Option<Field>,
    pub absent: Range<usize>,
    pub captures: Vec<CaptureId>,
    pub alternative: Option<Alternative>,
    pub named: bool,
    pub missing: bool,
    pub adjacent: bool,
    pub last: bool,
    pub inside_choice: bool,
    pub contains_captures: bool,
    pub parent_guaranteed: bool,
}

impl Step {
    pub(super) fn new(offset: usize, depth: u32, operation: Operation) -> Self {
        Self {
            operation,
            offset,
            depth,
            symbol: WILDCARD,
            supertype: None,
            field: None,
            absent: 0..0,
            captures: Vec::new(),
            alternative: None,
            named: false,
            missing: false,
            adjacent: false,
            last: false,
            inside_choice: false,
            contains_captures: false,
            parent_guaranteed: false,
        }
    }
}

#[derive(Debug)]
pub(super) struct Pattern {
    pub predicates: Range<usize>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Start {
    pub symbol: Symbol,
    pub step: StepId,
    pub pattern: u32,
    pub rooted: bool,
    pub parent: Option<StepId>,
}

#[derive(Default)]
pub(super) struct Program {
    pub steps: Vec<Step>,
    pub patterns: Vec<Pattern>,
    pub starts: Vec<Start>,
    starts_by_symbol: Vec<Range<usize>>,
    error_starts: Range<usize>,
    pub names: Vec<Box<[u8]>>,
    pub operands: Vec<Box<[u8]>>,
    pub predicates: Vec<Predicate>,
    pub absent: Vec<Field>,
    pub repetitions: Vec<Symbol>,
}

impl Program {
    pub(super) fn index_starts(&mut self) {
        self.starts
            .sort_by_key(|entry| (entry.symbol, entry.pattern));
        let error = self.starts.partition_point(|entry| entry.symbol < ERROR);
        self.error_starts = error..self.starts.len();
        let plain = &self.starts[..error];
        let bound = plain
            .last()
            .map_or(0, |entry| usize::from(entry.symbol) + 1);
        if bound > plain.len().saturating_mul(8) {
            return;
        }
        self.starts_by_symbol.resize(bound, 0..0);
        let mut at = 0;
        for group in plain.chunk_by(|left, right| left.symbol == right.symbol) {
            let next = at + group.len();
            self.starts_by_symbol[usize::from(group[0].symbol)] = at..next;
            at = next;
        }
    }

    pub(super) fn exact_starts(&self, symbol: Symbol) -> Range<usize> {
        if symbol == ERROR {
            return self.error_starts.clone();
        }
        if !self.starts_by_symbol.is_empty() {
            return self
                .starts_by_symbol
                .get(usize::from(symbol))
                .cloned()
                .unwrap_or(0..0);
        }
        let begin = self.starts.partition_point(|entry| entry.symbol < symbol);
        let end = begin + self.starts[begin..].partition_point(|entry| entry.symbol == symbol);
        begin..end
    }
}

pub(super) trait Grammar {
    fn abi(&self) -> u32;
    fn symbol(&self, name: &[u8], named: bool) -> Option<Symbol>;
    fn field(&self, name: &[u8]) -> Option<Field>;
    fn is_supertype(&self, symbol: Symbol) -> bool;
    fn is_subtype(&self, supertype: Symbol, symbol: Symbol) -> bool;
}

pub(super) type StateId = u16;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Goto(StateId),
    Shift(StateId),
    Extra,
    Reduce {
        symbol: Symbol,
        production: u16,
        children: u16,
    },
    Other,
}

#[derive(Clone, Copy)]
pub(super) struct SymbolInfo {
    pub public: Symbol,
    pub named: bool,
    pub visible: bool,
}

pub(super) trait GrammarTables: Grammar {
    fn actions(&self, state: StateId, into: &mut Vec<(Symbol, Action)>);
    fn primary(&self, state: StateId) -> bool;
    fn state_count(&self) -> u32;
    fn nonterminals(&self) -> Range<Symbol>;
    fn symbol_info(&self, symbol: Symbol) -> SymbolInfo;
    fn aliases(&self, symbol: Symbol, into: &mut Vec<Symbol>);
    fn child(&self, production: u16, child: u16) -> (Option<Symbol>, Option<Field>);
}
