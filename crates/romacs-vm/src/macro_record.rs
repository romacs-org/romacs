//! Which function cells each compiled function expanded through: the macro
//! record (redesign/macro-invalidation §3.1). Compiles record the
//! `(slot, version)` of every macro cell, alias link and compiler-macro
//! handler they expand through into one stack of scopes; a function keeps
//! its scope's uses as its record (`Function::macros`). The queries read
//! the records: who expanded a cell (`romacs-who-macroexpands`) and what a
//! write to it would make stale. A write to a recorded cell is logged
//! (`define_function`) and changes nothing yet; what it would make stale is
//! scanned when the log is read.
use crate::{HashSet, Op, Value, Vm};

/// A function cell a compile expanded through: its symbol's slot, and the
/// cell's version (`Slot::version`) as the expansion read it.
pub(crate) type MacroUse = (usize, u64);

/// How many writes `Invalidations::writes` keeps.
const LOG_LIMIT: usize = 64;

/// The open scopes' uses, each scope's after its start, and what the
/// records name.
#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct MacroRecord {
    uses: Vec<MacroUse>,
    scopes: Vec<Scope>,
    /// `expand_macro`'s expansions since the runaway count was last reset:
    /// its own counter, which the deduplicated uses cannot be.
    pub(crate) expansions: usize,
    /// By slot: whether an installed record has named it. A hint, never
    /// cleared: a write to a slot it leaves clear makes nothing stale.
    named: Vec<bool>,
    /// How many open top-level scopes are pre-expanded: while none is, a
    /// function's record is its own uses alone.
    pre_expanded: usize,
    /// The uses of the request compile that closed last (`Vm::compile_entry`),
    /// sorted: the source cache's guard for it.
    pub(crate) request: Vec<MacroUse>,
    /// The cells Lisp expanders are about to run through, innermost last
    /// (`romacs--macro-capture`).
    captures: Vec<Capture>,
    pub(crate) invalidations: Invalidations,
}

/// The cells a call about to run resolves through: those of SYMBOLS, read
/// when the capture opened and again after each autoload a call of one of
/// them loads, so they are the ones that run (`Vm::refresh_macro_captures`).
#[derive(Clone, Debug, romacs_snapshot::Snapshot)]
struct Capture {
    symbols: Vec<usize>,
    cells: Vec<MacroUse>,
}

#[derive(Clone, Copy, Debug, romacs_snapshot::Snapshot)]
struct Scope {
    start: usize,
    kind: ScopeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
enum ScopeKind {
    /// One function's compile. An eager nested lambda's closes into its
    /// parent's.
    Function,
    /// One request compile (`Vm::compile_entry`). Its root function, which
    /// runs once, keeps no record: its uses close into this scope, which
    /// keeps them as the request's (`MacroRecord::request`). DETACHED when it
    /// is an expander's own compile or one inside a function's: no scope
    /// above it then reaches past it.
    Request { detached: bool },
    /// A compile of its own inside another (a forced lazy body, a nested
    /// load): it records nothing, and no scope above it reaches past it.
    Detached,
    /// One top-level load form while it is expanded and run. It records
    /// only while HEADING, as its head is expanded (`Vm::set_heading`), so
    /// it holds that form's own expansions, never its functions' or what its
    /// run expands. Pre-expanded once it records an expansion made while an
    /// expander deeper than EXPANDING ran (a `cl-macrolet` expanding its
    /// body): the functions compiled from it then take its uses too.
    Toplevel { expanding: usize, pre_expanded: bool, heading: bool },
}

/// The writes to named slots: how many, and the latest `LOG_LIMIT`, oldest
/// first.
#[derive(Clone, Debug, Default, romacs_snapshot::Snapshot)]
pub(crate) struct Invalidations {
    pub(crate) events: u64,
    writes: Vec<Write>,
}

/// One write to a named slot: the version it replaced, and its symbol as it
/// was then, which a collected symbol's reused slot no longer names.
#[derive(Clone, Debug, romacs_snapshot::Snapshot)]
struct Write {
    slot: usize,
    version: u64,
    symbol: Named,
}

/// A logged symbol: by its slot while that slot can only ever name it (the
/// initial obarray's, which no collection reuses), else by its name alone.
#[derive(Clone, Debug, romacs_snapshot::Snapshot)]
pub(crate) enum Named {
    Interned(usize),
    Name(String),
}

/// What a panic leaves to undo (`Vm::catching`): the scopes and uses open
/// when it began, and the runaway count.
pub(crate) struct RecordMark {
    scopes: usize,
    uses: usize,
    expansions: usize,
    captures: usize,
}

/// One logged write as it reads now: the live functions it would make
/// stale, through how many rounds, the macros that joined it (their
/// expanders being stale themselves), and how long that scan took (0 where
/// no clock runs).
#[derive(Clone, Debug)]
pub(crate) struct Invalidation {
    pub(crate) symbol: Named,
    pub(crate) functions: usize,
    pub(crate) rounds: usize,
    pub(crate) joined: Vec<usize>,
    pub(crate) ns: u64,
}

/// A reusable walk over code trees (`Vm::reaches`): by function id, the
/// walk that last saw it.
struct Walk {
    seen: Vec<u32>,
    stamp: u32,
    functions: Vec<usize>,
    values: Vec<Value>,
    /// The closures this walk's cell reached, by index.
    closures: Vec<usize>,
}

impl Walk {
    fn new(functions: usize) -> Self {
        Self { seen: vec![0; functions], stamp: 0, functions: Vec::new(), values: Vec::new(), closures: Vec::new() }
    }
}

/// What a write to a slot reaches: the stale functions, sorted, the macro
/// slots that joined because a stale function is their expander or is
/// reachable from it, and the scan rounds.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Staleness {
    pub(crate) stale: Vec<usize>,
    pub(crate) joined: Vec<usize>,
    pub(crate) rounds: usize,
}

/// Whether RECORD, sorted, names SLOT (at VERSION when given).
fn names(record: &[MacroUse], slot: usize, version: Option<u64>) -> bool {
    let at = record.partition_point(|&(s, _)| s < slot);
    record[at..].iter().take_while(|&&(s, _)| s == slot).any(|&(_, v)| version.is_none_or(|version| v == version))
}

/// A and B, each sorted and free of duplicates, as one such record.
pub(crate) fn union(a: &[MacroUse], b: &[MacroUse]) -> Box<[MacroUse]> {
    if b.is_empty() {
        return a.into();
    }
    let mut all: Vec<MacroUse> = a.iter().chain(b).copied().collect();
    all.sort_unstable();
    all.dedup();
    all.into_boxed_slice()
}

/// Move SORTED's distinct entries to its front, in order: how many there are.
fn dedup_sorted(sorted: &mut [MacroUse]) -> usize {
    let mut kept = 0;
    for i in 0..sorted.len() {
        if kept == 0 || sorted[i] != sorted[kept - 1] {
            sorted[kept] = sorted[i];
            kept += 1;
        }
    }
    kept
}

impl MacroRecord {
    #[inline]
    fn open(&mut self, kind: ScopeKind) -> usize {
        self.scopes.push(Scope { start: self.uses.len(), kind });
        self.scopes.len() - 1
    }
    /// Close the scope at DEPTH and any left above it: a function's uses
    /// stay for its parent function, every other scope's go.
    #[inline]
    fn close(&mut self, depth: usize) {
        let Some(scope) = self.scopes.get(depth).copied() else { return };
        if self.pre_expanded != 0 {
            let flagged = self.scopes[depth..].iter().filter(|s| matches!(s.kind, ScopeKind::Toplevel { pre_expanded: true, .. })).count();
            self.pre_expanded -= flagged;
        }
        self.scopes.truncate(depth);
        let merges = scope.kind == ScopeKind::Function
            && self.scopes.last().is_some_and(|parent| matches!(parent.kind, ScopeKind::Function | ScopeKind::Request { .. }));
        if !merges {
            if matches!(scope.kind, ScopeKind::Request { .. }) {
                let tail = &mut self.uses[scope.start..];
                tail.sort_unstable();
                let kept = dedup_sorted(tail);
                self.request.clear();
                self.request.extend_from_slice(&self.uses[scope.start..scope.start + kept]);
            }
            self.uses.truncate(scope.start);
        }
    }
    /// Whether the innermost scope is a request's root function's.
    fn compiles_request_root(&self) -> bool {
        let n = self.scopes.len();
        n >= 2 && self.scopes[n - 1].kind == ScopeKind::Function && matches!(self.scopes[n - 2].kind, ScopeKind::Request { .. })
    }
    /// Record ENTRY in the innermost scope, EXPANDING expanders deep
    /// (`Vm::expanding`): nowhere in a detached scope or none.
    #[inline]
    fn push(&mut self, entry: MacroUse, expanding: usize) {
        match self.scopes.last_mut() {
            Some(Scope { kind: ScopeKind::Function, .. }) => self.uses.push(entry),
            Some(Scope { kind: ScopeKind::Toplevel { expanding: base, pre_expanded, heading: true }, .. }) => {
                if !*pre_expanded && expanding > *base {
                    *pre_expanded = true;
                    self.pre_expanded += 1;
                }
                self.uses.push(entry);
            }
            Some(Scope { kind: ScopeKind::Detached | ScopeKind::Request { .. } | ScopeKind::Toplevel { heading: false, .. }, .. })
            | None => {}
        }
    }
    /// The innermost function scope's uses, sorted and deduplicated in
    /// place; empty when the innermost scope is no function's.
    fn own_uses(&mut self) -> &[MacroUse] {
        let Some(&Scope { start, kind: ScopeKind::Function }) = self.scopes.last() else { return &[] };
        let tail = &mut self.uses[start..];
        if tail.len() > 1 {
            tail.sort_unstable();
            let kept = dedup_sorted(tail);
            self.uses.truncate(start + kept);
        }
        &self.uses[start..]
    }
    /// The uses of the pre-expanded top-level forms around the innermost
    /// scope, out to the nearest detached one.
    fn inherited_uses(&self) -> Vec<MacroUse> {
        let mut inherited = Vec::new();
        if self.pre_expanded == 0 {
            return inherited;
        }
        for (i, scope) in self.scopes.iter().enumerate().rev() {
            match scope.kind {
                ScopeKind::Detached | ScopeKind::Request { detached: true } => break,
                ScopeKind::Toplevel { pre_expanded: true, .. } => {
                    let end = self.scopes.get(i + 1).map_or(self.uses.len(), |next| next.start);
                    inherited.extend_from_slice(&self.uses[scope.start..end]);
                }
                ScopeKind::Toplevel { .. } | ScopeKind::Function | ScopeKind::Request { detached: false } => {}
            }
        }
        inherited
    }
    fn mark_named(&mut self, record: &[MacroUse]) {
        for &(slot, _) in record {
            if self.named.len() <= slot {
                self.named.resize(slot + 1, false);
            }
            self.named[slot] = true;
        }
    }
    #[inline]
    pub(crate) fn named(&self, slot: usize) -> bool {
        self.named.get(slot).copied().unwrap_or(false)
    }
    /// Log a write to SLOT, SYMBOL's, which replaced VERSION.
    fn log_write(&mut self, slot: usize, version: u64, symbol: Named) {
        let log = &mut self.invalidations;
        log.events += 1;
        if log.writes.len() >= LOG_LIMIT {
            log.writes.remove(0);
        }
        log.writes.push(Write { slot, version, symbol });
    }
    fn set_heading(&mut self, on: bool) {
        if let Some(Scope { kind: ScopeKind::Toplevel { heading, .. }, .. }) = self.scopes.last_mut() {
            *heading = on;
        }
    }
    pub(crate) fn mark(&self) -> RecordMark {
        RecordMark { scopes: self.scopes.len(), uses: self.uses.len(), expansions: self.expansions, captures: self.captures.len() }
    }
    /// Back to MARK, dropping the scopes a panic left open.
    pub(crate) fn restore(&mut self, mark: &RecordMark) {
        self.scopes.truncate(mark.scopes);
        self.uses.truncate(mark.uses);
        self.expansions = mark.expansions;
        self.captures.truncate(mark.captures);
        self.pre_expanded = self.scopes.iter().filter(|s| matches!(s.kind, ScopeKind::Toplevel { pre_expanded: true, .. })).count();
    }
}

impl Vm {
    /// Open the scope of one function's compile (`Vm::function`).
    #[inline]
    pub(super) fn open_function_scope(&mut self) -> usize {
        self.macro_record.open(ScopeKind::Function)
    }
    /// Open a compile's scope that nothing around it sees.
    pub(super) fn open_detached_scope(&mut self) -> usize {
        self.macro_record.open(ScopeKind::Detached)
    }
    /// Open a request compile's scope (`compile_entry`): detached unless it
    /// is a load form's own compile.
    pub(super) fn open_request_scope(&mut self) -> usize {
        let own = matches!(self.macro_record.scopes.last(),
            Some(Scope { kind: ScopeKind::Toplevel { expanding, .. }, .. }) if *expanding == self.mutator.expanding.len());
        self.macro_record.open(ScopeKind::Request { detached: !own })
    }
    /// Open one top-level load form's scope (`eval_toplevel`).
    pub(super) fn open_toplevel_scope(&mut self) -> usize {
        let kind = ScopeKind::Toplevel { expanding: self.mutator.expanding.len(), pre_expanded: false, heading: false };
        self.macro_record.open(kind)
    }
    /// Whether the innermost top-level form's head is being expanded now:
    /// only then does its scope record.
    pub(super) fn set_heading(&mut self, on: bool) {
        self.macro_record.set_heading(on);
    }
    /// A write to SLOT's cell, which replaced VERSION, that a record names
    /// (`MacroRecord::named`): logged, its symbol kept as it is now.
    pub(super) fn log_macro_write(&mut self, slot: usize, version: u64) {
        let symbol = if self.in_initial_obarray(slot) { Named::Interned(slot) } else { Named::Name(self.symbol_names[slot].clone()) };
        self.macro_record.log_write(slot, version, symbol);
    }
    #[inline]
    pub(super) fn close_scope(&mut self, depth: usize) {
        self.macro_record.close(depth);
    }
    /// Record that the compile in the innermost scope expanded through
    /// SLOT's cell as it is now.
    pub(super) fn record_macro_use(&mut self, slot: usize) {
        self.record_macro_uses(&[(slot, self.slots[slot].version)]);
    }
    /// Record that the compile in the innermost scope expanded through each
    /// of USES, the cells and versions it read.
    pub(super) fn record_macro_uses(&mut self, uses: &[MacroUse]) {
        for &entry in uses {
            self.macro_record.push(entry, self.mutator.expanding.len());
        }
    }
    /// Open a capture of the cells a call through SYMBOLS (slots) resolves
    /// through: its depth, for `close_macro_capture`.
    pub(crate) fn open_macro_capture(&mut self, symbols: Vec<usize>) -> usize {
        let cells = self.capture_cells(&symbols);
        self.macro_record.captures.push(Capture { symbols, cells });
        self.macro_record.captures.len() - 1
    }
    /// Close the capture at DEPTH and any left above it; when RECORD, the
    /// compile now running expanded through its cells.
    pub(crate) fn close_macro_capture(&mut self, depth: usize, record: bool) {
        let Some(capture) = self.macro_record.captures.get(depth) else { return };
        let cells = if record { capture.cells.clone() } else { Vec::new() };
        self.macro_record.captures.truncate(depth);
        self.record_macro_uses(&cells);
    }
    /// A call of FUNCTION has just autoloaded its definition and is about
    /// to run it: each open capture through FUNCTION reads its cells again.
    pub(crate) fn refresh_macro_captures(&mut self, function: Value) {
        if self.macro_record.captures.is_empty() {
            return;
        }
        let Some(slot) = function.index_if(7).and_then(|i| i.checked_sub(1)) else { return };
        for i in 0..self.macro_record.captures.len() {
            if self.macro_record.captures[i].symbols.contains(&slot) {
                let cells = self.capture_cells(&self.macro_record.captures[i].symbols);
                self.macro_record.captures[i].cells = cells;
            }
        }
    }
    fn capture_cells(&self, symbols: &[usize]) -> Vec<MacroUse> {
        symbols.iter().flat_map(|&slot| self.alias_cells(slot)).collect()
    }
    /// The record of the function the innermost scope compiles, as it is
    /// installed: its own uses and those of the pre-expanded forms it came from.
    pub(super) fn function_record(&mut self) -> Box<[MacroUse]> {
        if self.macro_record.compiles_request_root() {
            return Box::default();
        }
        let inherited = self.macro_record.inherited_uses();
        let record = union(self.macro_record.own_uses(), &inherited);
        if record.is_empty() {
            return record;
        }
        self.macro_record.mark_named(&record);
        record
    }
    /// The record of a lazily held body installed now: only what the
    /// pre-expanded forms it came from expanded, until it is forced.
    pub(super) fn lazy_record(&mut self) -> Box<[MacroUse]> {
        let mut inherited = self.macro_record.inherited_uses();
        inherited.sort_unstable();
        inherited.dedup();
        let record = inherited.into_boxed_slice();
        self.macro_record.mark_named(&record);
        record
    }
    /// The ids of the functions whose record names one of SLOTS, each at
    /// its version when it has one.
    fn functions_naming(&self, slots: &[(usize, Option<u64>)]) -> Vec<usize> {
        self.functions.iter().enumerate()
            .filter(|(_, f)| !f.macros.is_empty() && slots.iter().any(|&(slot, version)| names(&f.macros, slot, version)))
            .map(|(id, _)| id)
            .collect()
    }
    /// Whether VALUE's code reaches a function of TARGETS: a closure's
    /// function, the functions its code makes closures of, and the same
    /// through the closures it captures (an advice wrapper's original).
    fn reaches(&self, value: Value, targets: &HashSet<usize>, walk: &mut Walk) -> bool {
        walk.stamp += 1;
        walk.values.clear();
        walk.closures.clear();
        walk.values.push(value);
        while let Some(value) = walk.values.pop() {
            let Some(index) = value.index_if(4) else { continue };
            let Some(closure) = self.closures.get(index) else { continue };
            if walk.closures.contains(&index) {
                continue;
            }
            walk.closures.push(index);
            walk.functions.clear();
            walk.functions.push(closure.function);
            while let Some(id) = walk.functions.pop() {
                if id >= walk.seen.len() || walk.seen[id] == walk.stamp {
                    continue;
                }
                walk.seen[id] = walk.stamp;
                if targets.contains(&id) {
                    return true;
                }
                let f = &self.functions[id];
                walk.functions.extend(f.code.iter().filter_map(|op| match *op { Op::Closure(_, child, _) => Some(child), _ => None }));
                walk.functions.extend(f.pending.iter().flat_map(|body| body.children.iter().copied()));
            }
            walk.values.extend(closure.captures.iter().copied());
        }
        false
    }
    /// What a write to SLOT reaches: the functions whose record names it
    /// (at VERSION when given), then, round by round, those of each macro
    /// whose expander reaches a function found stale.
    pub(crate) fn staleness(&self, slot: usize, version: Option<u64>) -> Staleness {
        let mut stale: HashSet<usize> = HashSet::default();
        let (mut joined, mut rounds, mut frontier) = (vec![slot], 0, vec![(slot, version)]);
        let mut walk = None;
        while !frontier.is_empty() {
            rounds += 1;
            let before = stale.len();
            stale.extend(self.functions_naming(&frontier));
            if stale.len() == before {
                break;
            }
            let walk = walk.get_or_insert_with(|| Walk::new(self.functions.len()));
            frontier = self.macros.iter().copied()
                .filter(|m| !joined.contains(m))
                .filter(|&m| self.reaches(self.slots[m].function, &stale, walk))
                .map(|m| (m, Some(self.slots[m].version)))
                .collect();
            joined.extend(frontier.iter().map(|&(m, _)| m));
        }
        joined.remove(0);
        joined.sort_unstable();
        let mut stale: Vec<usize> = stale.into_iter().collect();
        stale.sort_unstable();
        Staleness { stale, joined, rounds }
    }
    /// The logged writes (`Invalidations`), each with what it would make
    /// stale now.
    pub(crate) fn macro_invalidations(&self) -> (u64, Vec<Invalidation>) {
        let log = &self.macro_record.invalidations;
        // Newest first, so the latest write's scan is timed cold.
        let resolved = log.writes.iter().rev().map(|&Write { slot, version, ref symbol }| {
            let started = crate::host::stat_clock();
            let reach = self.staleness(slot, Some(version));
            // HOST(Instant::now): the scan's own clock, running only once
            // `host::stat_clock` started it, which no world builder does
            let ns = started.map_or(0, |at| u64::try_from(std::time::Instant::now().duration_since(at).as_nanos()).unwrap_or(u64::MAX));
            Invalidation { symbol: symbol.clone(), functions: reach.stale.len(), rounds: reach.rounds, joined: reach.joined, ns }
        });
        let mut resolved: Vec<Invalidation> = resolved.collect();
        resolved.reverse();
        (log.events, resolved)
    }
    /// The slots of the symbols whose function cell reaches a function that
    /// expanded through SLOT's cell, or, when TRANSITIVE, one that a write
    /// to SLOT would make stale; sorted by name.
    pub(crate) fn who_macroexpands(&self, slot: usize, transitive: bool) -> Vec<usize> {
        let stale: HashSet<usize> = if transitive {
            self.staleness(slot, None).stale.into_iter().collect()
        } else {
            self.functions_naming(&[(slot, None)]).into_iter().collect()
        };
        if stale.is_empty() {
            return Vec::new();
        }
        let mut walk = Walk::new(self.functions.len());
        let mut callers: Vec<usize> = (0..self.slots.len())
            .filter(|&s| self.slots[s].function.index_if(4).is_some())
            .filter(|&s| self.reaches(self.slots[s].function, &stale, &mut walk))
            .collect();
        callers.sort_by(|a, b| self.symbol_names[*a].cmp(&self.symbol_names[*b]));
        callers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_finds_a_slot_at_any_or_one_version() {
        let record = [(1, 4), (3, 1), (3, 2), (7, 0)];
        assert!(names(&record, 3, None));
        assert!(names(&record, 3, Some(2)));
        assert!(!names(&record, 3, Some(5)));
        assert!(!names(&record, 2, None));
        assert!(!names(&[], 2, None));
    }

    #[test]
    fn a_function_scope_closes_into_its_parent_function_only() {
        let mut record = MacroRecord::default();
        let outer = record.open(ScopeKind::Function);
        record.push((1, 0), 0);
        let inner = record.open(ScopeKind::Function);
        record.push((2, 0), 0);
        record.push((2, 0), 0);
        assert_eq!(record.own_uses(), &[(2, 0)]);
        record.close(inner);
        assert_eq!(record.own_uses(), &[(1, 0), (2, 0)]);
        let detached = record.open(ScopeKind::Detached);
        let forced = record.open(ScopeKind::Function);
        record.push((9, 0), 0);
        record.close(forced);
        record.close(detached);
        assert_eq!(record.own_uses(), &[(1, 0), (2, 0)]);
        record.close(outer);
        assert!(record.uses.is_empty() && record.scopes.is_empty());
    }

    #[test]
    fn a_top_level_form_records_only_while_its_head_expands() {
        let mut record = MacroRecord::default();
        record.open(ScopeKind::Toplevel { expanding: 0, pre_expanded: false, heading: false });
        record.push((1, 0), 0);
        record.set_heading(true);
        record.push((2, 0), 1);
        record.set_heading(false);
        record.push((3, 0), 1);
        assert_eq!(record.uses, vec![(2, 0)], "its run records nothing");
        assert_eq!(record.pre_expanded, 1);
    }

    #[test]
    fn a_panic_leaves_the_record_as_it_was() {
        let mut vm = Vm::new();
        let caught = vm.catching(|vm| {
            vm.open_function_scope();
            vm.macro_record.push((1, 0), 0);
            vm.macro_record.expansions = 7;
            std::panic::panic_any("in a compile")
        });
        assert!(caught.is_err());
        assert!(vm.macro_record.scopes.is_empty() && vm.macro_record.uses.is_empty() && vm.macro_record.expansions == 0);
    }

    #[test]
    fn a_request_keeps_its_root_s_uses_as_its_own() {
        let mut record = MacroRecord::default();
        let request = record.open(ScopeKind::Request { detached: true });
        record.push((4, 0), 0);
        let root = record.open(ScopeKind::Function);
        assert!(record.compiles_request_root());
        record.push((3, 1), 0);
        let lambda = record.open(ScopeKind::Function);
        assert!(!record.compiles_request_root());
        record.push((2, 0), 0);
        record.close(lambda);
        record.close(root);
        record.close(request);
        assert_eq!(record.request, vec![(2, 0), (3, 1)], "the request's own scope records nothing");
        assert!(record.uses.is_empty() && record.scopes.is_empty());
    }

    #[test]
    fn only_pre_expanded_top_level_forms_are_inherited() {
        let mut record = MacroRecord::default();
        let plain = record.open(ScopeKind::Toplevel { expanding: 0, pre_expanded: false, heading: true });
        record.push((5, 0), 0);
        record.open(ScopeKind::Toplevel { expanding: 0, pre_expanded: false, heading: true });
        record.push((6, 1), 1);
        record.open(ScopeKind::Function);
        record.push((7, 0), 1);
        assert_eq!(record.inherited_uses(), vec![(6, 1)], "only the form an expander's expansion ran in");
        record.open(ScopeKind::Detached);
        record.push((8, 0), 1);
        assert!(record.inherited_uses().is_empty());
        record.close(plain);
        assert!(record.uses.is_empty() && record.scopes.is_empty() && record.pre_expanded == 0);
    }
}
