//! Bounded exploration of a grammar's productions for query validation.
use super::model::{
    Action, ERROR, Error, ErrorKind, Field, GrammarTables, Operation, Program, StateId, StepId,
    Symbol, WILDCARD,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

const PREDECESSORS: usize = 256;
const DEPTH: usize = 8;
const ITERATIONS: usize = 256;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct ProductionState {
    state: StateId,
    child: u16,
    done: bool,
    production: u16,
}

#[derive(Default)]
struct ProductionGraph {
    starts: Vec<StateId>,
    nodes: Vec<ProductionState>,
}

type Graphs = BTreeMap<Symbol, ProductionGraph>;

#[derive(Clone, Copy)]
struct Edge {
    symbol: Symbol,
    next: StateId,
    extra: bool,
}

#[derive(Default)]
struct Transitions {
    edges: Vec<Edge>,
    ends: Vec<usize>,
}

impl Transitions {
    fn row(&self, state: StateId) -> &[Edge] {
        let state = usize::from(state);
        &self.edges[self.ends[state]..self.ends[state + 1]]
    }

    fn append(&mut self, state: StateId, row: &[(Symbol, Action)]) {
        for (index, &(symbol, action)) in row.iter().enumerate() {
            if row.get(index + 1).is_some_and(|&(next, _)| next == symbol) {
                continue;
            }
            let edge = match action {
                Action::Goto(next) | Action::Shift(next) => Edge {
                    symbol,
                    next,
                    extra: false,
                },
                Action::Extra => Edge {
                    symbol,
                    next: state,
                    extra: true,
                },
                _ => continue,
            };
            self.edges.push(edge);
        }
        self.ends.push(self.edges.len());
    }
}

pub(super) fn analyze(
    grammar: &impl GrammarTables,
    program: &mut Program,
    offsets: &[(StepId, usize)],
) -> Result<(), Error> {
    let parents = classify(grammar, program, offsets)?;
    let rootless: Vec<_> = program
        .starts
        .iter()
        .copied()
        .filter(|start| !start.rooted && start.symbol != WILDCARD)
        .collect();
    if parents.is_empty() && rootless.is_empty() {
        return Ok(());
    }
    let mut graphs: Graphs = parents
        .iter()
        .map(|&id| program.steps[id].symbol)
        .chain(
            grammar
                .nonterminals()
                .filter(|&symbol| !grammar.symbol_info(symbol).visible),
        )
        .map(|symbol| (symbol, ProductionGraph::default()))
        .collect();
    let transitions = collect_graphs(grammar, &mut graphs);
    let mut analysis = Analysis {
        transitions,
        ..Analysis::default()
    };
    for parent in parents {
        validate_parent(grammar, program, offsets, &graphs, &mut analysis, parent)?;
    }
    for start in rootless {
        let initial = graphs
            .iter()
            .filter(|&(symbol, _)| {
                let info = grammar.symbol_info(*symbol);
                !info.visible && !info.named
            })
            .flat_map(|(&symbol, graph)| {
                graph
                    .starts
                    .iter()
                    .map(move |&state| Explore::new(start.step, symbol, state))
            });
        explore(grammar, program, &graphs, initial, &mut analysis);
        program
            .repetitions
            .extend(analysis.finished.iter().copied());
    }
    program.repetitions.sort_unstable();
    program.repetitions.dedup();
    Ok(())
}

fn classify(
    grammar: &impl GrammarTables,
    program: &mut Program,
    offsets: &[(StepId, usize)],
) -> Result<Vec<StepId>, Error> {
    let mut parents = Vec::new();
    for index in 0..program.steps.len() {
        let (depth, symbol, supertype) = {
            let step = &program.steps[index];
            (step.depth, step.symbol, step.supertype)
        };
        let mut captures = !program.steps[index].captures.is_empty();
        let mut has_children = false;
        for child in index + 1..program.steps.len() {
            let step = &mut program.steps[child];
            if matches!(step.operation, Operation::Accept) || step.depth <= depth {
                break;
            }
            has_children = true;
            captures |= !step.captures.is_empty();
            if symbol != WILDCARD {
                step.parent_guaranteed = true;
            } else if let Some(supertype) = supertype
                && grammar.abi() >= 15
                && step.depth == depth + 1
                && step.symbol != WILDCARD
                && !grammar.is_subtype(supertype, step.symbol)
            {
                return Err(structural_error(offsets, child));
            }
        }
        program.steps[index].contains_captures = captures;
        if has_children && symbol != WILDCARD && symbol != ERROR {
            parents.push(index);
        }
    }
    Ok(parents)
}

fn structural_error(offsets: &[(StepId, usize)], step: StepId) -> Error {
    let found = offsets.partition_point(|&(index, _)| index < step);
    Error {
        offset: offsets[found.min(offsets.len() - 1)].1,
        kind: ErrorKind::Structure,
    }
}

fn collect_graphs(grammar: &impl GrammarTables, graphs: &mut Graphs) -> Transitions {
    let (transitions, predecessors) = collect_rows(grammar, graphs);
    graphs.retain(|_, graph| !graph.nodes.is_empty());
    for graph in graphs.values_mut() {
        extend_graph(graph, &predecessors);
    }
    transitions
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "grammar state IDs are 16-bit"
)]
fn collect_rows(
    grammar: &impl GrammarTables,
    graphs: &mut Graphs,
) -> (Transitions, Vec<Vec<StateId>>) {
    let mut predecessors = vec![Vec::new(); grammar.state_count() as usize];
    let mut aliases = Vec::new();
    let mut actions = Vec::new();
    let mut transitions = Transitions {
        ends: vec![0, 0],
        ..Transitions::default()
    };
    for index in 1..grammar.state_count() {
        let state = index as StateId;
        grammar.actions(state, &mut actions);
        transitions.append(state, &actions);
        let primary = grammar.primary(state);
        for &(symbol, action) in &actions {
            match action {
                Action::Goto(next) => {
                    if next != state {
                        predecessor(&mut predecessors, next, state);
                    }
                    if !primary {
                        continue;
                    }
                    grammar.aliases(symbol, &mut aliases);
                    register_start(graphs, &aliases, state);
                }
                Action::Shift(next) => predecessor(&mut predecessors, next, state),
                Action::Reduce {
                    symbol,
                    production,
                    children,
                } => {
                    grammar.aliases(symbol, &mut aliases);
                    register_reduction(
                        graphs,
                        &aliases,
                        ProductionState {
                            state,
                            child: children,
                            done: true,
                            production,
                        },
                    );
                }
                Action::Extra | Action::Other => {}
            }
        }
    }
    (transitions, predecessors)
}

fn register_start(graphs: &mut Graphs, aliases: &[Symbol], state: StateId) {
    for alias in aliases {
        let Some(graph) = graphs.get_mut(alias) else {
            continue;
        };
        if graph.starts.last() != Some(&state) {
            graph.starts.push(state);
        }
    }
}

fn register_reduction(graphs: &mut Graphs, aliases: &[Symbol], reduction: ProductionState) {
    for alias in aliases {
        let Some(graph) = graphs.get_mut(alias) else {
            continue;
        };
        if graph
            .nodes
            .last()
            .is_none_or(|node| node.state != reduction.state)
        {
            graph.nodes.push(reduction);
        }
    }
}

fn predecessor(predecessors: &mut [Vec<StateId>], next: StateId, state: StateId) {
    let list = &mut predecessors[next as usize];
    if list.last() != Some(&state) && list.len() < PREDECESSORS {
        list.push(state);
    }
}

fn extend_graph(graph: &mut ProductionGraph, predecessors: &[Vec<StateId>]) {
    let mut pending = graph.nodes.clone();
    while let Some(node) = pending.pop() {
        if node.child < 2 {
            continue;
        }
        for &state in &predecessors[node.state as usize] {
            let previous = ProductionState {
                state,
                child: node.child - 1,
                done: false,
                production: node.production,
            };
            if let Err(index) = graph.nodes.binary_search(&previous) {
                graph.nodes.insert(index, previous);
                pending.push(previous);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Frame {
    child: u16,
    symbol: Symbol,
    state: StateId,
    field: Option<Field>,
    done: bool,
}

#[derive(Clone, Copy)]
struct Explore {
    step: StepId,
    root: Symbol,
    len: usize,
    frames: [Frame; DEPTH],
}

impl Explore {
    fn new(step: StepId, symbol: Symbol, state: StateId) -> Self {
        let mut frames = [Frame::default(); DEPTH];
        frames[0] = Frame {
            symbol,
            state,
            ..Frame::default()
        };
        Self {
            step,
            root: symbol,
            len: 1,
            frames,
        }
    }

    fn ordering(&self, other: &Self) -> Ordering {
        if self.len < other.len {
            return Ordering::Greater;
        }
        let key = |frame: &Frame| (frame.child, frame.symbol, frame.state, frame.field);
        for (left, right) in self.frames[..self.len]
            .iter()
            .zip(&other.frames[..other.len])
        {
            match key(left).cmp(&key(right)) {
                Ordering::Equal => {}
                order => return order,
            }
        }
        other
            .len
            .cmp(&self.len)
            .then_with(|| self.step.cmp(&other.step))
    }

    fn recursion(self) -> usize {
        let frames = &self.frames[..self.len];
        frames
            .iter()
            .enumerate()
            .filter(|&(index, frame)| {
                frames[..index]
                    .iter()
                    .any(|previous| previous.symbol == frame.symbol)
            })
            .count()
    }
}

#[derive(Default)]
struct Analysis {
    states: Vec<Explore>,
    next: Vec<Explore>,
    deeper: Vec<Explore>,
    final_steps: BTreeSet<StepId>,
    finished: BTreeSet<Symbol>,
    aborted: bool,
    recursion_limit: usize,
    previous_finals: usize,
    transitions: Transitions,
}

fn validate_parent(
    grammar: &impl GrammarTables,
    program: &mut Program,
    offsets: &[(StepId, usize)],
    graphs: &Graphs,
    analysis: &mut Analysis,
    parent: StepId,
) -> Result<(), Error> {
    let head = &program.steps[parent];
    let depth = head.depth;
    let symbol = head.symbol;
    let first = parent + 1;
    let graph = graphs
        .get(&symbol)
        .ok_or_else(|| structural_error(offsets, first))?;
    let initial = graph
        .starts
        .iter()
        .map(|&state| Explore::new(first, symbol, state));
    explore(grammar, program, graphs, initial, analysis);
    if analysis.aborted {
        for step in program.steps[first..]
            .iter_mut()
            .take_while(|step| step.depth > depth && !matches!(step.operation, Operation::Accept))
        {
            step.parent_guaranteed = false;
        }
        return Ok(());
    }
    if analysis.finished.is_empty() {
        return Err(structural_error(
            offsets,
            analysis.final_steps.last().copied().unwrap_or(parent),
        ));
    }
    for &step in &analysis.final_steps {
        let step = &mut program.steps[step];
        if step.depth > depth && !matches!(step.operation, Operation::Accept | Operation::Jump) {
            step.parent_guaranteed = false;
        }
    }
    Ok(())
}

fn explore(
    grammar: &impl GrammarTables,
    program: &Program,
    graphs: &Graphs,
    initial: impl Iterator<Item = Explore>,
    out: &mut Analysis,
) {
    out.states.clear();
    out.states.extend(initial);
    out.next.clear();
    out.deeper.clear();
    out.final_steps.clear();
    out.finished.clear();
    out.aborted = false;
    out.recursion_limit = 0;
    out.previous_finals = 0;
    for _ in 0..ITERATIONS {
        if out.states.is_empty() {
            if out.deeper.is_empty() || out.final_steps.len() <= out.previous_finals {
                return;
            }
            out.previous_finals = out.final_steps.len();
            out.recursion_limit += 1;
            std::mem::swap(&mut out.states, &mut out.deeper);
        } else {
            advance_states(grammar, program, graphs, out);
        }
    }
    out.aborted = true;
}

fn advance_states(
    grammar: &impl GrammarTables,
    program: &Program,
    graphs: &Graphs,
    out: &mut Analysis,
) {
    out.next.clear();
    for index in 0..out.states.len() {
        let state = out.states[index];
        if let Some(last) = out.next.last() {
            match state.ordering(last) {
                Ordering::Equal => continue,
                Ordering::Greater => {
                    out.next.extend_from_slice(&out.states[index..]);
                    break;
                }
                Ordering::Less => {}
            }
        }
        advance_state(grammar, program, graphs, out, state);
    }
    std::mem::swap(&mut out.states, &mut out.next);
}

fn advance_state(
    grammar: &impl GrammarTables,
    program: &Program,
    graphs: &Graphs,
    out: &mut Analysis,
    state: Explore,
) {
    let frame = state.frames[state.len - 1];
    let Some(graph) = graphs.get(&frame.symbol) else {
        return;
    };
    for index in 0..out.transitions.row(frame.state).len() {
        let Edge {
            symbol,
            next,
            extra,
        } = out.transitions.row(frame.state)[index];
        let child = frame.child + u16::from(!extra);
        let start = graph
            .nodes
            .partition_point(|node| (node.state, node.child) < (next, child));
        for node in graph.nodes[start..]
            .iter()
            .take_while(|node| (node.state, node.child) == (next, child))
        {
            advance_child(grammar, program, out, state, frame, symbol, *node);
        }
    }
}

fn advance_child(
    grammar: &impl GrammarTables,
    program: &Program,
    out: &mut Analysis,
    mut state: Explore,
    frame: Frame,
    symbol: Symbol,
    node: ProductionState,
) {
    let (alias, field) = grammar.child(node.production, frame.child);
    let info = grammar.symbol_info(symbol);
    let visible = alias
        .or_else(|| info.visible.then_some(info.public))
        .filter(|&symbol| symbol != WILDCARD);
    let field = frame.field.or(field);
    let step = &program.steps[state.step];
    let matched = step.symbol == ERROR
        || visible.is_some_and(|symbol| {
            (if step.symbol == WILDCARD {
                !step.named || grammar.symbol_info(symbol).named
            } else {
                step.symbol == symbol
            }) && step.field.is_none_or(|expected| field == Some(expected))
                && step.supertype.is_none_or(|expected| {
                    state.frames[..state.len]
                        .iter()
                        .any(|frame| expected == frame.symbol)
                })
        });
    state.frames[state.len - 1] = Frame {
        state: node.state,
        child: node.child,
        done: node.done,
        ..frame
    };
    if !matched
        && visible.is_none()
        && symbol >= grammar.nonterminals().start
        && !descend(frame, node, out, &mut state, symbol, field)
    {
        return;
    }
    while state.len > 0 && state.frames[state.len - 1].done {
        state.len -= 1;
    }
    if matched {
        state.step += 1;
        while program.steps[state.step].depth > step.depth
            && !matches!(program.steps[state.step].operation, Operation::Accept)
        {
            state.step += 1;
        }
    } else if node.state == frame.state {
        return;
    }
    record_results(program, out, state, step.depth, matched);
}

fn descend(
    frame: Frame,
    node: ProductionState,
    out: &mut Analysis,
    state: &mut Explore,
    symbol: Symbol,
    field: Option<Field>,
) -> bool {
    if !node.done {
        if state.len + 1 >= DEPTH {
            out.aborted = true;
            return false;
        }
        state.len += 1;
    }
    state.frames[state.len - 1] = Frame {
        state: frame.state,
        symbol,
        field,
        ..Frame::default()
    };
    if state.recursion() > out.recursion_limit {
        insert(&mut out.deeper, *state);
        return false;
    }
    true
}

fn record_results(
    program: &Program,
    out: &mut Analysis,
    mut state: Explore,
    depth: u32,
    matched: bool,
) {
    loop {
        let step = &program.steps[state.step];
        if matches!(step.operation, Operation::Repeat) {
            state.step += 1;
            continue;
        }
        if !matches!(step.operation, Operation::Jump) {
            if step.depth != depth || matches!(step.operation, Operation::Accept) {
                out.finished.insert(state.root);
            } else if state.len == 0 {
                out.final_steps.insert(state.step);
            } else {
                insert(&mut out.next, state);
            }
        }
        match step.alternative {
            Some(link) if matched && link.step > state.step => state.step = link.step,
            _ => break,
        }
    }
}

fn insert(states: &mut Vec<Explore>, state: Explore) {
    if let Err(index) = states.binary_search_by(|item| item.ordering(&state)) {
        states.insert(index, state);
    }
}
