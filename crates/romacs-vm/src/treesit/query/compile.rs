//! Parse query bytes into structural steps and predicate metadata.
use super::model::{
    Alternative, Error, ErrorKind, Field, GrammarTables, Operation, Pattern, Predicate, Program,
    Start, Step, StepId, Symbol, WILDCARD, query_index,
};

#[derive(Clone, Copy, Default)]
struct Context {
    depth: u32,
    adjacent: bool,
    inside_choice: bool,
}

#[derive(Clone, Copy)]
struct Mark {
    offset: usize,
    step: StepId,
    context: Context,
}

enum Frame {
    Field {
        mark: Mark,
        name: std::ops::Range<usize>,
    },
    Choice {
        mark: Mark,
        branches: Vec<StepId>,
    },
    Sequence {
        mark: Mark,
        node: bool,
        previous: Option<StepId>,
        absent: Vec<Field>,
    },
}

#[derive(Clone, Copy)]
enum Next {
    Child(Context),
    Complete(Mark),
    Predicate,
}

struct Parser<'a, G> {
    source: &'a [u8],
    grammar: &'a G,
    offset: usize,
    program: Program,
    step_offsets: Vec<(StepId, usize)>,
}

pub(super) fn compile(source: &[u8], grammar: &impl GrammarTables) -> Result<Program, Error> {
    if u32::try_from(source.len()).is_err() {
        return Err(Error {
            offset: 0,
            kind: ErrorKind::Syntax,
        });
    }
    let mut parser = Parser {
        source,
        grammar,
        offset: 0,
        program: Program::default(),
        step_offsets: Vec::new(),
    };
    parser.skip_space();
    while parser.offset < source.len() {
        parser.top_level()?;
    }
    parser.program.index_starts();
    super::analysis::analyze(grammar, &mut parser.program, &parser.step_offsets)?;
    Ok(parser.program)
}

impl<G: GrammarTables> Parser<'_, G> {
    fn error(&self, kind: ErrorKind) -> Error {
        Error {
            offset: self.offset,
            kind,
        }
    }

    fn byte(&self) -> Option<u8> {
        self.source.get(self.offset).copied()
    }

    fn take(&mut self, byte: u8) -> bool {
        if self.byte() != Some(byte) {
            return false;
        }
        self.offset += 1;
        true
    }

    fn skip_space(&mut self) {
        loop {
            match self.byte() {
                Some(byte) if byte.is_ascii_whitespace() || byte == 0x0b => self.offset += 1,
                Some(b';') => {
                    self.offset += 1;
                    self.offset = self.source[self.offset..]
                        .iter()
                        .position(|&byte| byte == b'\n' || byte == 0)
                        .map_or(self.source.len(), |length| self.offset + length);
                }
                _ => return,
            }
        }
    }

    // The current C runtime tokenizes outside string literals in the C locale.
    fn identifier(&mut self) -> Result<std::ops::Range<usize>, Error> {
        let start = self.offset;
        if !self.byte().is_some_and(is_identifier) {
            return Err(self.error(ErrorKind::Syntax));
        }
        self.offset += 1;
        while self
            .byte()
            .is_some_and(|byte| is_identifier(byte) || byte == b'.')
        {
            self.offset += 1;
        }
        Ok(start..self.offset)
    }

    fn literal(&mut self) -> Result<Vec<u8>, Error> {
        let start = self.offset;
        self.offset += 1;
        let mut bytes = Vec::new();
        while let Some(byte) = self.byte() {
            self.offset += 1;
            match byte {
                b'"' => return Ok(bytes),
                b'\n' => break,
                b'\\' => {
                    let Some(escaped) = self.byte() else { break };
                    self.offset += 1;
                    bytes.push(match escaped {
                        b't' => b'\t',
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b'0' => 0,
                        other => other,
                    });
                }
                other => bytes.push(other),
            }
        }
        Err(Error {
            offset: start,
            kind: ErrorKind::Syntax,
        })
    }

    fn top_level(&mut self) -> Result<(), Error> {
        let start = self.program.steps.len();
        let predicates = self.program.predicates.len();
        self.pattern()?;
        self.program
            .steps
            .push(Step::new(self.offset, u32::MAX, Operation::Accept));
        let end = self.program.steps.len();
        let pattern = query_index(self.program.patterns.len());
        self.program.patterns.push(Pattern {
            predicates: predicates..self.program.predicates.len(),
        });
        self.add_starts(pattern, start, end);
        self.redirect_branch_repeats(start, end);
        Ok(())
    }

    fn pattern(&mut self) -> Result<(), Error> {
        let mut frames = Vec::new();
        let mut next = Next::Child(Context::default());
        loop {
            match next {
                Next::Child(context) => next = self.read_pattern(context, &mut frames)?,
                Next::Complete(mark) => {
                    self.modifiers(mark)?;
                    let Some(frame) = frames.pop() else {
                        return Ok(());
                    };
                    next = self.finish_child(frame, Some(mark.step), &mut frames)?;
                }
                Next::Predicate => {
                    let Some(frame) = frames.pop() else {
                        return Ok(());
                    };
                    next = self.finish_child(frame, None, &mut frames)?;
                }
            }
        }
    }

    fn read_pattern(&mut self, context: Context, frames: &mut Vec<Frame>) -> Result<Next, Error> {
        let mark = Mark {
            offset: self.offset,
            step: self.program.steps.len(),
            context,
        };
        if self
            .step_offsets
            .last()
            .is_none_or(|(step, _)| *step != mark.step)
        {
            self.step_offsets.push((mark.step, mark.offset));
        }
        match self.byte() {
            Some(b'[') => {
                self.offset += 1;
                self.skip_space();
                let mut child = context;
                child.inside_choice = true;
                frames.push(Frame::Choice {
                    mark,
                    branches: Vec::new(),
                });
                Ok(Next::Child(child))
            }
            Some(b'(') => self.parenthesized(mark, frames),
            Some(b'_') => {
                self.offset += 1;
                self.program.steps.push(new_match(mark));
                Ok(Next::Complete(mark))
            }
            Some(b'"') => {
                let mut step = new_match(mark);
                step.symbol = self.anonymous(mark.offset + 1)?;
                self.program.steps.push(step);
                Ok(Next::Complete(mark))
            }
            _ => self.field_pattern(mark, frames),
        }
    }

    fn parenthesized(&mut self, mark: Mark, frames: &mut Vec<Frame>) -> Result<Next, Error> {
        self.offset += 1;
        self.skip_space();
        match self.byte() {
            Some(b'(' | b'[' | b'"') => self.sequence(mark, false, None, Vec::new(), frames),
            Some(b'.' | b'#') => {
                self.offset += 1;
                self.predicate()?;
                Ok(Next::Predicate)
            }
            _ => {
                let step = self.named_node(mark)?;
                self.program.steps.push(step);
                self.sequence(mark, true, None, Vec::new(), frames)
            }
        }
    }

    fn named_node(&mut self, mark: Mark) -> Result<Step, Error> {
        let name = self.identifier()?;
        let mut step = new_match(mark);
        match &self.source[name.clone()] {
            b"_" => step.named = true,
            b"MISSING" => {
                step.missing = true;
                self.skip_space();
                step.symbol = match self.byte() {
                    Some(b')') => WILDCARD,
                    Some(b'"') => self.anonymous(self.offset + 1)?,
                    _ => {
                        let name = self.identifier()?;
                        self.named(name)?
                    }
                };
                step.named = step.symbol == WILDCARD;
            }
            _ => step.symbol = self.named(name)?,
        }
        if step.symbol != WILDCARD && self.grammar.is_supertype(step.symbol) {
            step.supertype = Some(step.symbol);
            step.symbol = WILDCARD;
        }
        if self.take(b'/') {
            let invalid = Error {
                offset: mark.offset,
                kind: ErrorKind::Structure,
            };
            let supertype = step.supertype.ok_or(invalid)?;
            step.symbol = self.subtype()?;
            if self.grammar.abi() >= 15 && !self.grammar.is_subtype(supertype, step.symbol) {
                return Err(invalid);
            }
        }
        self.skip_space();
        Ok(step)
    }

    fn subtype(&mut self) -> Result<Symbol, Error> {
        if self.byte() == Some(b'"') {
            return self.anonymous(self.offset);
        }
        let name = self.identifier()?;
        self.named(name)
    }

    fn field_pattern(&mut self, mark: Mark, frames: &mut Vec<Frame>) -> Result<Next, Error> {
        let name = self.identifier()?;
        self.skip_space();
        if !self.take(b':') {
            return Err(Error {
                offset: mark.offset,
                kind: ErrorKind::Syntax,
            });
        }
        self.skip_space();
        frames.push(Frame::Field { mark, name });
        Ok(Next::Child(mark.context))
    }

    fn finish_child(
        &mut self,
        frame: Frame,
        child: Option<StepId>,
        frames: &mut Vec<Frame>,
    ) -> Result<Next, Error> {
        match frame {
            Frame::Field { mark, name } => {
                let field = self.field(name)?;
                if child.is_none() {
                    return Err(Error {
                        offset: mark.offset,
                        kind: ErrorKind::Syntax,
                    });
                }
                self.for_branch_starts(mark.step, |step| step.field = Some(field));
                Ok(Next::Complete(mark))
            }
            Frame::Choice { mark, mut branches } => {
                branches.push(child.unwrap_or(self.program.steps.len()));
                Ok(self.choice(mark, branches, frames))
            }
            Frame::Sequence {
                mark,
                node,
                previous,
                absent,
            } => self.sequence(mark, node, child.or(previous), absent, frames),
        }
    }

    fn sequence(
        &mut self,
        mark: Mark,
        node: bool,
        previous: Option<StepId>,
        mut absent: Vec<Field>,
        frames: &mut Vec<Frame>,
    ) -> Result<Next, Error> {
        while node && self.take(b'!') {
            self.skip_space();
            let name = self.identifier()?;
            let field = self.field(name)?;
            if absent.len() < 8 {
                absent.push(field);
            }
            self.skip_space();
        }
        let anchor = self.offset;
        let adjacent = self.take(b'.');
        self.skip_space();
        if self.take(b')') {
            self.finish_sequence(mark, node, previous, absent, adjacent.then_some(anchor))?;
            return Ok(Next::Complete(mark));
        }
        frames.push(Frame::Sequence {
            mark,
            node,
            previous,
            absent,
        });
        Ok(Next::Child(Context {
            depth: mark.context.depth + u32::from(node),
            adjacent: adjacent || (!node && previous.is_none() && mark.context.adjacent),
            inside_choice: mark.context.inside_choice,
        }))
    }

    fn finish_sequence(
        &mut self,
        mark: Mark,
        node: bool,
        previous: Option<StepId>,
        absent: Vec<Field>,
        anchor: Option<usize>,
    ) -> Result<(), Error> {
        if let Some(offset) = anchor {
            let step = previous.filter(|_| node).ok_or(Error {
                offset: if node { self.offset - 1 } else { offset },
                kind: ErrorKind::Syntax,
            })?;
            self.for_branch_starts(step, |step| step.last = true);
        }
        if node && !absent.is_empty() {
            let start = self.program.absent.len();
            self.program.absent.extend(absent);
            self.program.steps[mark.step].absent = start..self.program.absent.len();
        }
        Ok(())
    }

    fn choice(&mut self, mark: Mark, branches: Vec<StepId>, frames: &mut Vec<Frame>) -> Next {
        if self.take(b']') {
            let end = self.program.steps.len();
            for adjacent in branches.windows(2) {
                let [start, next] = adjacent else {
                    unreachable!()
                };
                self.program.steps[*start].alternative = Some(branch(*next));
                let jump = &mut self.program.steps[*next - 1];
                jump.operation = Operation::Jump;
                jump.alternative = Some(branch(end));
            }
            return Next::Complete(mark);
        }
        self.program
            .steps
            .push(Step::new(self.offset, mark.context.depth, Operation::Jump));
        frames.push(Frame::Choice { mark, branches });
        Next::Child(Context {
            inside_choice: true,
            ..mark.context
        })
    }

    fn modifiers(&mut self, mark: Mark) -> Result<(), Error> {
        let (mut optional, mut repeated) = (false, false);
        loop {
            self.skip_space();
            match self.byte() {
                Some(b'?') => {
                    optional = true;
                    self.offset += 1;
                }
                Some(b'+') => {
                    repeated = true;
                    self.offset += 1;
                }
                Some(b'*') => {
                    optional = true;
                    repeated = true;
                    self.offset += 1;
                }
                Some(b'@') => {
                    self.offset += 1;
                    let name = self.identifier()?;
                    let id = intern(&mut self.program.names, &self.source[name]);
                    self.for_branch_starts(mark.step, |step| step.captures.push(id));
                }
                _ => break,
            }
        }
        self.quantify(mark, optional, repeated);
        Ok(())
    }

    fn quantify(&mut self, mark: Mark, optional: bool, repeated: bool) {
        let end = self.program.steps.len();
        if repeated {
            let mut step = Step::new(mark.offset, mark.context.depth, Operation::Repeat);
            step.alternative = Some(branch(mark.step));
            step.inside_choice = mark.context.inside_choice;
            self.program.steps.push(step);
        }
        let last_branch = optional
            .then(|| self.branch_indices(mark.step, end).last())
            .flatten();
        if let Some(last) = last_branch {
            self.program.steps[last].alternative = Some(Alternative {
                step: self.program.steps.len(),
                skips_repeat: true,
            });
        }
    }

    fn predicate(&mut self) -> Result<(), Error> {
        let name = self.identifier()?;
        if !self.take(b'?') && !self.take(b'!') {
            return Err(self.error(ErrorKind::Syntax));
        }
        let id = intern(
            &mut self.program.operands,
            &self.source[name.start..self.offset],
        );
        self.program.predicates.push(Predicate::String(id));
        loop {
            self.skip_space();
            if self.take(b')') {
                self.skip_space();
                self.program.predicates.push(Predicate::Done);
                return Ok(());
            }
            let operand = self.operand()?;
            self.program.predicates.push(operand);
        }
    }

    fn operand(&mut self) -> Result<Predicate, Error> {
        if self.take(b'@') {
            let name = self.identifier()?;
            return self
                .program
                .names
                .iter()
                .position(|existing| **existing == self.source[name.clone()])
                .map(|id| Predicate::Capture(query_index(id)))
                .ok_or(Error {
                    offset: name.start,
                    kind: ErrorKind::Capture,
                });
        }
        let id = if self.byte() == Some(b'"') {
            let bytes = self.literal()?;
            intern(&mut self.program.operands, &bytes)
        } else {
            let name = self.identifier()?;
            intern(&mut self.program.operands, &self.source[name])
        };
        Ok(Predicate::String(id))
    }

    fn named(&self, name: std::ops::Range<usize>) -> Result<Symbol, Error> {
        self.grammar
            .symbol(&self.source[name.clone()], true)
            .ok_or(Error {
                offset: name.start,
                kind: ErrorKind::NodeType,
            })
    }

    fn anonymous(&mut self, offset: usize) -> Result<Symbol, Error> {
        let bytes = self.literal()?;
        self.grammar.symbol(&bytes, false).ok_or(Error {
            offset,
            kind: ErrorKind::NodeType,
        })
    }

    fn field(&self, name: std::ops::Range<usize>) -> Result<Field, Error> {
        self.grammar.field(&self.source[name.clone()]).ok_or(Error {
            offset: name.start,
            kind: ErrorKind::Field,
        })
    }

    fn branch_indices(&self, at: StepId, end: StepId) -> impl Iterator<Item = StepId> + '_ {
        std::iter::successors((at < end).then_some(at), move |&index| {
            let alternative = self.program.steps.get(index)?.alternative?;
            (alternative.step > index && alternative.step < end).then_some(alternative.step)
        })
    }

    fn for_branch_starts(&mut self, mut index: StepId, mut change: impl FnMut(&mut Step)) {
        let len = self.program.steps.len();
        while let Some(step) = self.program.steps.get_mut(index) {
            change(step);
            match step.alternative {
                Some(next) if next.step > index && next.step < len => index = next.step,
                _ => break,
            }
        }
    }

    fn add_starts(&mut self, pattern: u32, start: StepId, end: StepId) {
        let mut index = Some(start);
        let mut wildcard_alternative = None;
        while let Some(at) = index.filter(|&at| at < end) {
            let (step, parent) = self.select_start(at);
            let head = &self.program.steps[step];
            if parent.is_some() {
                wildcard_alternative = self.program.steps[at].alternative;
            }
            if matches!(head.operation, Operation::Match | Operation::Accept) {
                let rooted = head.depth == 0
                    && self.program.steps[step + 1..end]
                        .iter()
                        .take_while(|step| !matches!(step.operation, Operation::Jump))
                        .all(|step| step.depth != head.depth);
                self.program.starts.push(Start {
                    symbol: head.symbol,
                    step,
                    pattern,
                    rooted,
                    parent,
                });
            }
            index = head
                .alternative
                .or_else(|| wildcard_alternative.take())
                .map(|link| link.step);
        }
    }

    fn select_start(&self, index: StepId) -> (StepId, Option<StepId>) {
        let head = &self.program.steps[index];
        if head.symbol == WILDCARD
            && head.depth == 0
            && head.field.is_none()
            && head.absent.is_empty()
            && let Some(child) = self.program.steps.get(index + 1)
            && child.symbol != WILDCARD
            && child.depth == 1
            && !child.adjacent
        {
            return (index + 1, Some(index));
        }
        (index, None)
    }

    fn redirect_branch_repeats(&mut self, start: StepId, end: StepId) {
        for index in start..end {
            let step = &self.program.steps[index];
            let Some(link) = step.alternative else {
                continue;
            };
            if !step.inside_choice
                || !matches!(step.operation, Operation::Repeat)
                || link.step >= index
            {
                continue;
            }
            let Some(target_alt) = self.program.steps[link.step].alternative else {
                continue;
            };
            if target_alt.step <= link.step || target_alt.step >= end - 1 {
                continue;
            }
            let mut copy = self.program.steps[link.step].clone();
            copy.alternative = None;
            let mut jump = Step::new(copy.offset, copy.depth, Operation::Jump);
            jump.alternative = Some(branch(link.step + 1));
            self.program.steps[index].alternative = Some(branch(self.program.steps.len()));
            self.program.steps.extend([copy, jump]);
        }
    }
}

fn is_identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn new_match(mark: Mark) -> Step {
    let mut step = Step::new(mark.offset, mark.context.depth, Operation::Match);
    step.adjacent = mark.context.adjacent;
    step
}

fn branch(step: StepId) -> Alternative {
    Alternative {
        step,
        skips_repeat: false,
    }
}

fn intern(values: &mut Vec<Box<[u8]>>, value: &[u8]) -> u32 {
    if let Some(id) = values.iter().position(|bytes| **bytes == *value) {
        return query_index(id);
    }
    let id = query_index(values.len());
    values.push(value.into());
    id
}
