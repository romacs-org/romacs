//! Run the structural query over one borrowed tree generation.
use super::model::{
    Alternative, CaptureId, ERROR, Field, Operation, Program, Start, Step, StepId, Symbol, WILDCARD,
};
use std::cmp::Ordering;

pub(super) trait Node: Copy + Eq {
    fn symbol(self) -> Symbol;
    fn named(self) -> bool;
    fn missing(self) -> bool;
    fn span(self) -> Span;
    fn has_field(self, field: Field) -> bool;
}

pub(super) trait Cursor {
    type Node: Node;

    fn node(&self) -> Self::Node;
    fn parent_node(&self) -> Option<Self::Node>;
    fn status(&self) -> Status;
    fn repetition(&self) -> Option<Symbol>;
    fn first_child(&mut self) -> Movement;
    fn next_sibling(&mut self) -> Movement;
    fn parent(&mut self) -> bool;
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    fn overlaps(self, range: Self) -> bool {
        self.start < range.end
            && (self.end > range.start || (self.end == range.start && self.start == self.end))
    }
}

pub(super) enum Movement {
    None,
    Visible,
    Hidden,
}

pub(super) struct Status {
    pub field: Option<Field>,
    pub later: bool,
    pub later_named: bool,
    pub later_field: bool,
    pub supertypes: [Symbol; 8],
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct Capture<N> {
    node: N,
    id: CaptureId,
    span: Span,
}

pub(crate) struct Match<N> {
    pub pattern: u32,
    pub captures: Vec<(N, CaptureId)>,
}

#[derive(Clone, Copy)]
struct State {
    start_depth: u32,
    pattern: u32,
    step: StepId,
    parent: Option<StepId>,
    captures: Option<usize>,
    seeking_immediate: bool,
    skipped_repeat: bool,
    has_longer_alternative: bool,
}

struct Lists<N> {
    slots: Vec<Vec<Capture<N>>>,
    free: Vec<usize>,
}

impl<N: Copy> Lists<N> {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    fn alloc(&mut self) -> usize {
        if let Some(slot) = self.free.pop() {
            return slot;
        }
        let slot = self.slots.len();
        self.slots.push(Vec::new());
        slot
    }

    fn release(&mut self, slot: Option<usize>) {
        if let Some(slot) = slot {
            self.slots[slot].clear();
            self.free.push(slot);
        }
    }

    fn get(&self, slot: Option<usize>) -> &[Capture<N>] {
        slot.map_or(&[], |slot| self.slots[slot].as_slice())
    }

    fn duplicate(&mut self, slot: Option<usize>) -> Option<usize> {
        let slot = slot?;
        let copy = self.alloc();
        let mut buffer = std::mem::take(&mut self.slots[copy]);
        buffer.extend_from_slice(&self.slots[slot]);
        self.slots[copy] = buffer;
        Some(copy)
    }

    fn push(&mut self, state: &mut State, capture: Capture<N>) {
        let slot = *state.captures.get_or_insert_with(|| self.alloc());
        self.slots[slot].push(capture);
    }
}

struct Walker<'a, C: Cursor, N, Project> {
    program: &'a Program,
    cursor: C,
    depth: u32,
    visible: bool,
    range: Span,
    unrestricted: bool,
    wildcards: usize,
    rootless: bool,
    active: Vec<State>,
    branches: Vec<State>,
    lists: Lists<C::Node>,
    output: Vec<Match<N>>,
    project: Project,
}

pub(super) fn matches<C: Cursor, N>(
    program: &Program,
    cursor: C,
    (start, end): (u32, u32),
    mut stop: Option<&mut dyn FnMut() -> bool>,
    project: impl Fn(C::Node) -> N,
) -> Option<Vec<Match<N>>> {
    let mut walker = cursor_for_range(program, cursor, start, end, project);
    let mut work = 0;
    let mut entering = true;
    loop {
        if let Some(stop) = &mut stop {
            work += 1 + walker.active.len();
            if work >= 100 {
                work = 0;
                if stop() {
                    return None;
                }
            }
        }
        entering = if entering {
            walker.enter()
        } else {
            match walker.exit() {
                Some(entering) => entering,
                None => break,
            }
        };
    }
    if stop.is_some_and(|stop| stop()) {
        return None;
    }
    Some(walker.output)
}

fn cursor_for_range<C: Cursor, N, Project>(
    program: &Program,
    cursor: C,
    start: u32,
    end: u32,
    project: Project,
) -> Walker<'_, C, N, Project> {
    let end = if end == 0 { u32::MAX } else { end };
    let range = if start <= end {
        Span { start, end }
    } else {
        Span {
            start: 0,
            end: u32::MAX,
        }
    };
    let unrestricted =
        range.start == 0 && range.end == u32::MAX && cursor.node().span().end < u32::MAX;
    Walker {
        program,
        cursor,
        range,
        depth: 0,
        visible: true,
        unrestricted,
        wildcards: program
            .starts
            .partition_point(|entry| entry.symbol == WILDCARD),
        rootless: program.starts.iter().any(|start| !start.rooted),
        active: Vec::new(),
        branches: Vec::new(),
        lists: Lists::new(),
        output: Vec::new(),
        project,
    }
}

impl<C: Cursor, N, Project: Fn(C::Node) -> N> Walker<'_, C, N, Project> {
    fn enter(&mut self) -> bool {
        let parent = (!self.unrestricted || (self.visible && self.rootless))
            .then(|| self.cursor.parent_node())
            .flatten();
        let node = self.cursor.node();
        let parent_overlaps =
            self.unrestricted || parent.is_none_or(|node| node.span().overlaps(self.range));
        let overlaps = self.unrestricted || (parent_overlaps && node.span().overlaps(self.range));
        if self.visible {
            let symbol = node.symbol();
            let exact = if symbol == WILDCARD {
                0..0
            } else {
                self.program.exact_starts(symbol)
            };
            let has_wildcards = self.wildcards != 0 && symbol != ERROR;
            let has_exact = !exact.is_empty();
            if !self.active.is_empty() || has_wildcards || has_exact {
                let status = self.cursor.status();
                let rootless = parent_overlaps && parent.is_none_or(|node| node.symbol() != ERROR);
                self.start(symbol, exact, &status, overlaps, rootless);
                if !self.active.is_empty() {
                    self.advance(node, parent, symbol, &status);
                    self.reduce();
                }
            }
        }
        let descend = overlaps || self.pending_descendant() || self.rootless_repetition();
        if descend { self.descend() } else { false }
    }

    fn descend(&mut self) -> bool {
        match self.cursor.first_child() {
            Movement::None => return false,
            Movement::Visible => {
                self.visible = true;
                self.depth += 1;
            }
            Movement::Hidden => self.visible = false,
        }
        true
    }

    fn pending_descendant(&self) -> bool {
        self.active.iter().any(|state| {
            let step = &self.program.steps[state.step];
            !matches!(step.operation, Operation::Accept)
                && state.start_depth + step.depth > self.depth
        })
    }

    fn rootless_repetition(&self) -> bool {
        !self.visible
            && self
                .cursor
                .repetition()
                .is_none_or(|symbol| self.program.repetitions.binary_search(&symbol).is_ok())
    }

    fn exit(&mut self) -> Option<bool> {
        if self.visible {
            self.release_finished_descendants();
        }
        match self.cursor.next_sibling() {
            Movement::Visible => {
                if !self.visible {
                    self.depth += 1;
                }
                self.visible = true;
                Some(true)
            }
            Movement::Hidden => {
                if self.visible {
                    self.depth -= 1;
                }
                self.visible = false;
                Some(true)
            }
            Movement::None => {
                if !self.cursor.parent() {
                    return None;
                }
                self.depth = self.depth.saturating_sub(1);
                Some(false)
            }
        }
    }

    fn release_finished_descendants(&mut self) {
        let mut kept = 0;
        for index in 0..self.active.len() {
            let state = self.active[index];
            let step = &self.program.steps[state.step];
            if matches!(step.operation, Operation::Accept)
                && (state.start_depth > self.depth || self.depth == 0)
            {
                self.finish(state);
            } else if !matches!(step.operation, Operation::Accept)
                && state.start_depth + step.depth > self.depth
            {
                self.lists.release(state.captures);
            } else {
                if kept != index {
                    self.active[kept] = state;
                }
                kept += 1;
            }
        }
        self.active.truncate(kept);
    }

    fn start(
        &mut self,
        symbol: Symbol,
        exact: std::ops::Range<usize>,
        status: &Status,
        overlaps: bool,
        rootless: bool,
    ) {
        let starts = &self.program.starts;
        let wildcards = self.wildcards;
        if symbol != ERROR {
            for &start in &starts[..wildcards] {
                if self.can_start(start, status, overlaps, rootless) {
                    self.add_state(start);
                }
            }
        }
        for &start in &starts[exact] {
            if self.can_start(start, status, overlaps, rootless) {
                self.add_state(start);
            }
        }
    }

    fn can_start(&self, start: Start, status: &Status, overlaps: bool, rootless: bool) -> bool {
        let step = &self.program.steps[start.step];
        (if start.rooted { overlaps } else { rootless })
            && (self.depth >= step.depth || matches!(step.operation, Operation::Accept))
            && step.field.is_none_or(|field| status.field == Some(field))
            && (step.symbol != WILDCARD || step.supertype.is_none() || status.supertypes[0] != 0)
    }

    fn add_state(&mut self, start: Start) {
        let step = &self.program.steps[start.step];
        // A predicate-only pattern has already completed. The C cursor
        // orders that empty match after states starting at this node.
        let start_depth = if matches!(step.operation, Operation::Accept) {
            self.depth + 1
        } else {
            self.depth - step.depth
        };
        let mut index = self.active.len();
        while index > 0 {
            let state = self.active[index - 1];
            if state.start_depth == start_depth
                && state.pattern == start.pattern
                && state.step == start.step
            {
                return;
            }
            if (state.start_depth, state.pattern) <= (start_depth, start.pattern) {
                break;
            }
            index -= 1;
        }
        self.active.insert(
            index,
            State {
                start_depth,
                pattern: start.pattern,
                step: start.step,
                parent: start.parent,
                captures: None,
                seeking_immediate: true,
                skipped_repeat: false,
                has_longer_alternative: false,
            },
        );
    }

    fn advance(&mut self, node: C::Node, parent: Option<C::Node>, symbol: Symbol, status: &Status) {
        let mut index = 0;
        while index < self.active.len() {
            self.active[index].has_longer_alternative = false;
            let state = self.active[index];
            let step = &self.program.steps[state.step];
            if matches!(step.operation, Operation::Accept)
                || state.start_depth + step.depth != self.depth
            {
                index += 1;
                continue;
            }
            let later = later_sibling(state, step, node, status);
            if !self.node_matches(step, node, symbol, status) {
                if later {
                    index += 1;
                } else {
                    self.remove(index);
                }
                continue;
            }
            let sibling = (later && (step.contains_captures || self.fallible(state.step)))
                .then(|| self.copy(state));
            index += self.advance_match(index, state, node, parent, status, sibling);
        }
    }

    fn node_matches(&self, step: &Step, node: C::Node, symbol: Symbol, status: &Status) -> bool {
        let kind_matches = if step.symbol == WILDCARD {
            if step.missing {
                node.missing()
            } else {
                symbol != ERROR && (!step.named || node.named())
            }
        } else {
            step.symbol == symbol && (!step.missing || node.missing())
        };
        kind_matches
            && (!step.last || !status.later_named)
            && step
                .supertype
                .is_none_or(|symbol| status.supertypes.contains(&symbol))
            && step.field.is_none_or(|field| status.field == Some(field))
            && self.program.absent[step.absent.clone()]
                .iter()
                .all(|&field| !node.has_field(field))
    }

    fn advance_match(
        &mut self,
        index: usize,
        mut state: State,
        node: C::Node,
        parent: Option<C::Node>,
        status: &Status,
        sibling: Option<State>,
    ) -> usize {
        if let Some(step) = state.parent.take() {
            let Some(parent) = parent else {
                self.remove(index);
                if let Some(sibling) = sibling {
                    self.active.insert(index, sibling);
                    return 1;
                }
                return 0;
            };
            self.capture(&mut state, step, parent);
        }
        let matched = state.step;
        self.capture(&mut state, matched, node);
        let step = &self.program.steps[matched];
        state.step += 1;
        let next = &self.program.steps[state.step];
        state.seeking_immediate = step.symbol == WILDCARD && !step.named && next.adjacent;
        state.skipped_repeat = false;
        if next.alternative.is_none() {
            self.active[index] = state;
            if let Some(sibling) = sibling {
                self.active.insert(index + 1, sibling);
                return 2;
            }
            return 1;
        }
        self.follow_alternatives(state, status.later_named);
        self.branches.extend(sibling);
        let count = self.branches.len();
        self.active.splice(index..=index, self.branches.drain(..));
        count
    }

    fn follow_alternatives(&mut self, state: State, later_named: bool) {
        self.branches.push(state);
        let mut index = 0;
        while index < self.branches.len() {
            let mut state = self.branches[index];
            let step = &self.program.steps[state.step];
            let Some(link) = step.alternative else {
                index += 1;
                continue;
            };
            let repeat = matches!(step.operation, Operation::Repeat);
            if matches!(step.operation, Operation::Jump) {
                self.branches[index].step = link.step;
                continue;
            }
            if repeat {
                state.step += 1;
                self.branches[index].step = state.step;
            }
            if !(link.skips_repeat && step.last && later_named) {
                let extra = self.follow_branch(state, step, link, repeat);
                self.branches.insert(index + 1, extra);
            }
            if !repeat {
                index += 1;
            }
        }
    }

    fn follow_branch(
        &mut self,
        state: State,
        from: &Step,
        link: Alternative,
        repeat: bool,
    ) -> State {
        let mut copy = self.copy(state);
        copy.step = link.step;
        if repeat {
            copy.seeking_immediate = true;
        }
        if link.skips_repeat {
            let target = &self.program.steps[link.step];
            if !from.adjacent {
                copy.skipped_repeat = target.depth == from.depth;
            } else if state.step > 0 && self.program.steps[state.step - 1].depth < from.depth {
                copy.seeking_immediate = true;
            }
        }
        copy
    }

    fn fallible(&self, step: StepId) -> bool {
        let original = &self.program.steps[step];
        self.program.steps[step + 1..]
            .iter()
            .find(|step| !matches!(step.operation, Operation::Repeat))
            .is_some_and(|step| {
                !matches!(step.operation, Operation::Accept)
                    && (step.depth > original.depth
                        || (step.depth == original.depth && step.adjacent))
                    && (!step.parent_guaranteed || original.symbol == WILDCARD)
            })
    }

    fn capture(&mut self, state: &mut State, step: StepId, node: C::Node) {
        let captures = &self.program.steps[step].captures;
        if captures.is_empty() {
            return;
        }
        let span = node.span();
        for &id in captures {
            self.lists.push(state, Capture { node, id, span });
        }
    }

    fn copy(&mut self, state: State) -> State {
        State {
            captures: self.lists.duplicate(state.captures),
            ..state
        }
    }

    fn reduce(&mut self) {
        self.order_states();
        let mut index = 0;
        while index < self.active.len() {
            if !self.longest(index) {
                continue;
            }
            let state = self.active[index];
            if !state.has_longer_alternative
                && matches!(self.program.steps[state.step].operation, Operation::Accept)
            {
                self.active.remove(index);
                self.finish(state);
            } else {
                index += 1;
            }
        }
    }

    fn order_states(&mut self) {
        for current in 1..self.active.len() {
            let state = self.active[current];
            let key = self.state_order(state);
            let mut index = current;
            while index > 0 && self.state_order(self.active[index - 1]) > key {
                self.active[index] = self.active[index - 1];
                index -= 1;
            }
            self.active[index] = state;
        }
    }

    fn state_order(&self, state: State) -> (u32, u32, Option<u32>) {
        let first = self
            .lists
            .get(state.captures)
            .first()
            .map(|capture| capture.span.start);
        (state.start_depth, state.pattern, first)
    }

    fn longest(&mut self, index: usize) -> bool {
        let mut other_index = index + 1;
        while let Some(&other) = self.active.get(other_index) {
            let state = self.active[index];
            if (state.start_depth, state.pattern) != (other.start_depth, other.pattern) {
                break;
            }
            let captures = self.lists.get(state.captures);
            let others = self.lists.get(other.captures);
            if spans_disjoint(captures, others) {
                break;
            }
            let (left, right) = contains(captures, others);
            if left {
                if state.step == other.step && (other.seeking_immediate || !state.seeking_immediate)
                {
                    self.remove(other_index);
                    continue;
                }
                self.active[other_index].has_longer_alternative = true;
            }
            if right {
                if state.step == other.step && (state.seeking_immediate || !other.seeking_immediate)
                {
                    self.remove(index);
                    return false;
                }
                self.active[index].has_longer_alternative = true;
            }
            other_index += 1;
        }
        true
    }

    fn remove(&mut self, index: usize) {
        let state = self.active.remove(index);
        self.lists.release(state.captures);
    }

    fn finish(&mut self, state: State) {
        let captures = self
            .lists
            .get(state.captures)
            .iter()
            .map(|capture| ((self.project)(capture.node), capture.id))
            .collect();
        self.output.push(Match {
            pattern: state.pattern,
            captures,
        });
        self.lists.release(state.captures);
    }
}

fn later_sibling(state: State, step: &Step, node: impl Node, status: &Status) -> bool {
    status.later
        && !state.seeking_immediate
        && (!step.adjacent || !node.named() || state.skipped_repeat)
        && (step.field.is_none_or(|field| status.field != Some(field)) || status.later_field)
}

fn spans_disjoint<N: Node>(left: &[Capture<N>], right: &[Capture<N>]) -> bool {
    left.last()
        .zip(right.first())
        .is_some_and(|(left, right)| left.span.end <= right.span.start)
}

fn contains<N: Node>(mut left: &[Capture<N>], mut right: &[Capture<N>]) -> (bool, bool) {
    let (mut contains_left, mut contains_right) = (true, true);
    while let (Some(l), Some(r)) = (left.first(), right.first()) {
        if l == r {
            left = &left[1..];
            right = &right[1..];
            continue;
        }
        let ordering = compare(l, r);
        if ordering != Ordering::Greater {
            contains_right = false;
            left = &left[1..];
        }
        if ordering != Ordering::Less {
            contains_left = false;
            right = &right[1..];
        }
    }
    (
        contains_left && right.is_empty(),
        contains_right && left.is_empty(),
    )
}

fn compare<N: Node>(left: &Capture<N>, right: &Capture<N>) -> Ordering {
    if left.node == right.node {
        return Ordering::Equal;
    }
    let (left, right) = (left.span, right.span);
    left.start
        .cmp(&right.start)
        .then_with(|| right.end.cmp(&left.end))
}
