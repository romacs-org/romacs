//! A parser's reparse on a thread of its own, ahead of the frame that will
//! ask for the tree.
//!
//! A key's frame shows the edit before its parse (`Vm::defer_parse`). The
//! parse then runs here, from a copy of the parser's edited tree over a
//! frozen copy of the text, and the frames meanwhile keep the faces the
//! text has: typing never waits for a parser. The frame the landing wakes
//! takes the tree as the parser's own parse would have left it, so Lisp
//! sees one reparse, with its changed ranges, as GNU's `treesit_ensure_parsed`
//! gives. Anything that asks for the tree sooner waits for the thread, which
//! is no longer than parsing itself.

use super::{Change, Core, EarlySlot, Reader, native, note_early_landed};
use crate::text::{FrozenText, TextBody};
use crate::{Value, Vm, data};
use std::rc::Rc;
use std::sync::mpsc;

/// The most edits a tree catches up with after its parse; more, and the
/// parser parses for itself.
///
/// A parse that lands behind the text starts again from its tree for as
/// long as keys come faster than the parser, as an editor that parses off
/// its main thread does: the text typed meanwhile waits for its faces,
/// never a key for a parse. Parsing on the evaluator instead costs the
/// same parse, and blocks every key behind it (a file whose reparse takes
/// a fifth of a second, typed at forty keys a second).
const CHANGES_LIMIT: usize = 256;

/// One reparse on its thread, and the text's changes since it started.
pub(crate) struct Ahead {
    /// The region the tree saw when the parse started (1-based bytes).
    visible: (i64, i64),
    changes: Vec<Change>,
    result: mpsc::Receiver<Option<Landed>>,
    /// What the thread sent, once `ready` saw it.
    landed: Option<Option<Landed>>,
    restarts: u8,
}

/// What the thread sends: the tree, and the byte ranges where it differs
/// from the tree it started from (`Tree::changed_since`), which are the
/// parser's changed ranges when the text has not changed since.
type Landed = (native::Loose, Vec<(u32, u32)>);

/// What a parser needs of the thread: its language, its included ranges
/// and its tree, edited up to the text the thread reads.
pub(crate) struct Job {
    pub language: native::Language,
    pub ranges: Vec<(u32, u32)>,
    pub old: native::Loose,
    pub visible: (i64, i64),
}

impl Ahead {
    /// The parse of JOB over TEXT in SLOT, calling WAKE when it ends.
    fn start(slot: EarlySlot, job: Job, text: FrozenText, restarts: u8, wake: crate::process_jobs::Wake) -> Option<Self> {
        let (sender, result) = mpsc::channel();
        let visible = job.visible;
        std::thread::Builder::new().name("treesit-reparse".into()).spawn(move || {
            // The slot outlives the text.
            let _slot = slot;
            let text = text;
            let old = job.old.tree();
            let mut parser = native::Parser::new();
            let tree = (parser.set_language(&job.language) && parser.set_included_ranges(&job.ranges)).then(|| {
                let reader = Reader::new(Some(&text), ((visible.0 - 1) as usize, (visible.1 - 1) as usize));
                parser.parse(Some(&old), &reader).map(|new| {
                    let changed = new.changed_since(&old);
                    (new.loose(), changed)
                })
            }).flatten();
            // The copy shares the parser's subtrees: release it here.
            drop(old);
            let _ = sender.send(tree);
            note_early_landed();
            wake();
        }).ok()?;
        Some(Self { visible, changes: Vec::new(), result, landed: None, restarts })
    }
    /// Whether the thread has sent its result.
    pub(crate) fn ready(&mut self) -> bool {
        if self.landed.is_none() {
            self.landed = self.result.try_recv().ok();
        }
        self.landed.is_some()
    }
    /// A change of the text since the parse started. False past the limit:
    /// the parse is of no more use.
    pub(crate) fn record(&mut self, change: Change) -> bool {
        self.changes.push(change);
        self.changes.len() <= CHANGES_LIMIT
    }
    /// The tree, waiting for the thread, edited by the text's changes since
    /// the parse started, with the region those left it, whether there were
    /// any (such a tree is an old one, nearer the text than the parser's
    /// own) and, when it is the parser's next tree, its changed ranges.
    pub(crate) fn finish(mut self, text: &TextBody) -> Option<(native::Tree, (i64, i64), bool, Option<Vec<(u32, u32)>>)> {
        let (tree, changed) = self.landed.take().or_else(|| self.result.recv().ok()).flatten()?;
        let tree = tree.tree();
        let mut core = Core::new(self.visible, None);
        core.tree = Some(Rc::new(tree));
        for change in &self.changes {
            core.record(Change { linecols: None, ..*change }, text);
        }
        // A restarted parse began from a tree behind the parser's own: its
        // ranges are not the parser's.
        let edited = !self.changes.is_empty();
        let changed = (!edited && self.restarts == 0).then_some(changed);
        Rc::into_inner(core.tree.take()?).map(|tree| (tree, core.visible, edited, changed))
    }
}

/// The reparses of a text's parsers, as a frame finds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// None runs and none has a tree to hand over.
    None,
    /// One is still parsing: the frame shows the text as it stands.
    Running,
    /// Every one landed: the frame's parsers take the trees.
    Landed,
}

impl Vm {
    /// The parsers of BUFFER's text and their shared states.
    fn treesit_cores(&mut self, buffer: Value) -> crate::Result<Vec<(Value, Rc<std::cell::RefCell<Core>>)>> {
        Ok(self.treesit_text_of(buffer)?.map_or_else(Vec::new, |text| text.treesit.parsers.clone()))
    }
    /// Whether a reparse of BUFFER's text is still on its thread.
    pub(crate) fn treesit_ahead_running(&mut self, buffer: Value) -> crate::Result<bool> {
        Ok(self.treesit_cores(buffer)?.iter().any(|(_, core)| core.borrow_mut().ahead.as_mut().is_some_and(|ahead| !ahead.ready())))
    }
    /// Bring the reparses of BUFFER's text up to the frame about to be
    /// built: with START, one begins for each parser an edit left to
    /// reparse and that has none; one that landed behind the text begins
    /// again from its tree.
    pub(crate) fn treesit_run_ahead(&mut self, buffer: Value, start: bool) -> crate::Result<State> {
        let Some(wake) = self.process_jobs.wake() else { return Ok(State::None) };
        let cores = self.treesit_cores(buffer)?;
        let mut state = State::None;
        for (parser, core) in cores {
            let landed = core.borrow_mut().ahead.as_mut().map(|ahead| ahead.ready());
            let restarts = match landed {
                Some(false) => {
                    state = State::Running;
                    continue;
                }
                Some(true) => {
                    let ahead = core.borrow().ahead.as_ref().map(|ahead| (ahead.changes.is_empty(), ahead.restarts));
                    match ahead {
                        Some((false, restarts)) => restarts.saturating_add(1),
                        _ => {
                            if state == State::None { state = State::Landed; }
                            continue;
                        }
                    }
                }
                None if start => 0,
                None => continue,
            };
            if self.treesit_start_ahead(buffer, parser, &core, restarts, wake.clone())? {
                state = State::Running;
            } else if core.borrow().ahead.is_some() && state == State::None {
                state = State::Landed;
            }
        }
        Ok(state)
    }
    /// Start PARSER's reparse on a thread, from the tree of a reparse that
    /// landed behind the text if CORE holds one, else from its own. False
    /// when it has nothing to reparse, tracks lines and columns (an edit
    /// replayed on the thread's tree has no points), or no thread is free.
    fn treesit_start_ahead(&mut self, buffer: Value, parser: Value, core: &Rc<std::cell::RefCell<Core>>, restarts: u8,
                           wake: crate::process_jobs::Wake) -> crate::Result<bool> {
        let (language, ranges) = match self.object(parser)? {
            data::Object::Treesit(object) => match &**object {
                super::Object::Parser(p) if !p.deleted && !p.within_reparse => {
                    let native = p.native.borrow();
                    let Some(language) = native.language() else { return Ok(false) };
                    (language, native.included_ranges())
                }
                _ => return Ok(false),
            },
            _ => return Ok(false),
        };
        let Some(text) = self.treesit_text_of(buffer)? else { return Ok(false) };
        let behind = core.borrow_mut().ahead.take();
        let mut c = core.borrow_mut();
        if !c.need_reparse || c.linecols.is_some() {
            return Ok(false);
        }
        let old = match behind.and_then(|ahead| ahead.finish(text)) {
            Some((tree, region, ..)) if region == c.visible => tree.loose(),
            _ => match &c.tree {
                Some(tree) => tree.share(),
                None => return Ok(false),
            },
        };
        let Some(slot) = EarlySlot::take() else { return Ok(false) };
        let job = Job { language, ranges, old, visible: c.visible };
        c.ahead = Ahead::start(slot, job, text.frozen(), restarts, wake);
        Ok(c.ahead.is_some())
    }
}
