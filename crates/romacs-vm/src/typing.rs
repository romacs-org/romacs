//! Typing receipts: the native keys each command step consumed, and the
//! first submitted frame that shows the step's result.
//!
//! Every native key Lisp reads (`host_input::finish`) is claimed by the
//! innermost command step (`romacs--typing-step`, lisp/vm/keyboard.el), or
//! settled at once when no step is running. A step that ends proposes one
//! candidate: its window's view, text revision and point, and the bytes it
//! edited (`EditEntry`, kept by each text body). The next redisplay settles
//! the candidate's keys exactly once: `rendered` when a submitted frame shows
//! that very state, its edit among the bytes the frame painted; `unchanged`
//! when the redisplay submitted nothing; else `unattributed` with a reason.
//! The host drains the outcomes (`Vm::take_typing_outcomes`).
use crate::Vm;
use romacs_display_protocol::frame_time::EventTime;
use romacs_display_protocol::submission::{SubmissionReceipt, SubmittedWindow};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

/// The process's one text revision counter: each body write takes the next,
/// so a body's revision names its text in this process alone.
static TEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn next_text_revision() -> u64 {
    TEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}


/// GNU's unchanged bounds of the writes since the entry was last taken:
/// BEG bytes at the text's start and END bytes at its end left alone, or
/// UNKNOWN when the writes are not known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, romacs_snapshot::Snapshot)]
pub(crate) struct Bounds {
    beg: usize,
    end: usize,
    unknown: bool,
}

impl Bounds {
    const CLEAN: Self = Self { beg: usize::MAX, end: usize::MAX, unknown: false };

    /// Both periods' writes, by bounding box: the bytes unchanged in each.
    fn merge(self, other: Self) -> Self {
        Self { beg: self.beg.min(other.beg), end: self.end.min(other.end), unknown: self.unknown || other.unknown }
    }

    /// The written span in a text of LEN bytes.
    fn edit(self, len: usize) -> Edit {
        match self {
            Self { unknown: true, .. } => Edit::Unknown,
            Self { beg: usize::MAX, .. } => Edit::Clean,
            Self { beg, end, .. } => Edit::Span(beg..len.saturating_sub(end).max(beg)),
        }
    }
}

/// A text body's open edit entry: its revision, and the bounds of the
/// writes since it was last taken. ORIGIN names the body.
#[derive(Clone, Debug, romacs_snapshot::Snapshot)]
pub(crate) struct EditEntry {
    origin: u64,
    revision: u64,
    bounds: Bounds,
}

impl Default for EditEntry {
    /// A new body: nothing is known of what its text replaced.
    fn default() -> Self {
        let revision = next_text_revision();
        Self { origin: revision, revision, bounds: Bounds { unknown: true, ..Bounds::CLEAN } }
    }
}

impl EditEntry {
    /// Bytes from START, replaced by INSERTED bytes, leaving a text of LEN.
    pub(crate) fn record(&mut self, start: usize, inserted: usize, len: usize) {
        self.revision = next_text_revision();
        let written = Bounds { beg: start, end: len.saturating_sub(start + inserted), unknown: false };
        self.bounds = self.bounds.merge(written);
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// The body's name and the bounds since the last take, which start over.
    fn take(&mut self) -> (u64, Bounds) {
        (self.origin, std::mem::replace(&mut self.bounds, Bounds::CLEAN))
    }
}

/// A window showing one buffer: its id and `shows' generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    window: u64,
    shows: u64,
}

/// What a step's command did to its window's buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Edit {
    /// The step left the view it began in: the new view is its effect.
    Moved,
    Clean,
    Span(Range<usize>),
    Unknown,
}

/// The selected window as a step ends or begins.
struct Selected {
    view: View,
    origin: u64,
    bounds: Bounds,
    revision: u64,
    point: usize,
    len: usize,
}

/// A running command step.
struct Active {
    began: Option<(View, u64)>,
    bounds: Bounds,
    keys: Vec<EventTime>,
}

/// A finished step, until the next redisplay settles it.
#[derive(Debug)]
struct Candidate {
    keys: Vec<EventTime>,
    view: View,
    revision: u64,
    point: usize,
    edit: Edit,
}

/// Why a key's step was charged no frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unattributed {
    /// A later step's result replaced it before any frame showed it.
    Superseded,
    /// The frame submitted after it did not show its window.
    NotShown,
    /// The frame showed its state but not the bytes it edited.
    Offscreen,
    /// Its buffer's text was replaced: what it edited is unknown.
    EditUnknown,
    /// Read with no command step running (a timer's or a filter's read).
    OutsideCommand,
}

impl Unattributed {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Superseded => "superseded",
            Self::NotShown => "not-shown",
            Self::Offscreen => "offscreen",
            Self::EditUnknown => "edit-unknown",
            Self::OutsideCommand => "outside-command",
        }
    }
}

/// What became of one native key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// This submission was the first to show its step's result.
    Rendered(SubmissionReceipt),
    /// The redisplay after its step submitted no frame.
    Unchanged,
    Unattributed(Unattributed),
}

/// One native key, by the instant the platform delivered it, and its outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyOutcome {
    pub ingress: EventTime,
    pub outcome: Outcome,
}

/// The evaluator's typing receipts (module doc). Holds no Lisp value.
#[derive(Default)]
pub(crate) struct Typing {
    active: Vec<Active>,
    pending: Option<Candidate>,
    settled: Vec<KeyOutcome>,
}

impl Typing {
    fn settle(&mut self, keys: Vec<EventTime>, outcome: &Outcome) {
        self.settled.extend(keys.into_iter().map(|ingress| KeyOutcome { ingress, outcome: outcome.clone() }));
    }

    /// A native key Lisp read: the innermost step's.
    pub(crate) fn read(&mut self, ingress: EventTime) {
        match self.active.last_mut() {
            Some(step) => step.keys.push(ingress),
            None => self.settle(vec![ingress], &Outcome::Unattributed(Unattributed::OutsideCommand)),
        }
    }

    /// Writes to body ORIGIN taken from its entry: each running step on it wrote them too.
    fn taken(&mut self, origin: u64, bounds: Bounds) {
        for step in self.active.iter_mut().filter(|step| step.began.is_some_and(|(_, body)| body == origin)) {
            step.bounds = step.bounds.merge(bounds);
        }
    }

    fn begin(&mut self, selected: Option<Selected>) {
        let began = selected.map(|s| {
            self.taken(s.origin, s.bounds);
            (s.view, s.origin)
        });
        self.active.push(Active { began, bounds: Bounds::CLEAN, keys: Vec::new() });
    }

    /// The step that began last ends; LOCAL unless it was thrown out of. A
    /// step thrown out of hands its keys to the step around it. Whether the
    /// step needs its selected window (`end').
    fn ending(&mut self, local: bool) -> Option<Active> {
        let step = self.active.pop()?;
        if step.keys.is_empty() {
            return None;
        }
        if !local && let Some(outer) = self.active.last_mut() {
            outer.keys.extend(step.keys);
            return None;
        }
        Some(step)
    }

    fn end(&mut self, step: Active, selected: Option<Selected>) {
        let Some(s) = selected else {
            return self.settle(step.keys, &Outcome::Unattributed(Unattributed::NotShown));
        };
        self.taken(s.origin, s.bounds);
        let edit = match step.began {
            Some((view, _)) if view != s.view => Edit::Moved,
            Some((_, origin)) if origin == s.origin => step.bounds.merge(s.bounds).edit(s.len),
            _ => Edit::Unknown,
        };
        let candidate = Candidate { keys: step.keys, view: s.view, revision: s.revision, point: s.point, edit };
        if let Some(old) = self.pending.replace(candidate) {
            self.settle(old.keys, &Outcome::Unattributed(Unattributed::Superseded));
        }
    }

    fn submitted(&mut self, receipt: &SubmissionReceipt) {
        if let Some(candidate) = self.pending.take() {
            let outcome = candidate.judge(receipt);
            self.settle(candidate.keys, &outcome);
        }
    }

    fn unchanged(&mut self) {
        if let Some(candidate) = self.pending.take() {
            self.settle(candidate.keys, &Outcome::Unchanged);
        }
    }
}

impl Candidate {
    /// What RECEIPT's frame is to this step: its first frame only when the
    /// frame shows exactly the state the step left and the bytes it edited.
    fn judge(&self, receipt: &SubmissionReceipt) -> Outcome {
        let Some(window) = receipt.window.as_ref().filter(|w| w.window_id == self.view.window) else {
            return Outcome::Unattributed(Unattributed::NotShown);
        };
        if (window.view_generation, window.text_revision, window.cursor_byte) != (self.view.shows, self.revision, Some(self.point)) {
            return Outcome::Unattributed(Unattributed::Superseded);
        }
        match &self.edit {
            Edit::Moved | Edit::Clean => Outcome::Rendered(receipt.clone()),
            Edit::Span(span) if covered(span, &window.visible_text_bytes) => Outcome::Rendered(receipt.clone()),
            Edit::Span(_) => Outcome::Unattributed(Unattributed::Offscreen),
            Edit::Unknown => Outcome::Unattributed(Unattributed::EditUnknown),
        }
    }
}

/// Whether SPAN, or the deletion point an empty SPAN is, lies in VISIBLE.
fn covered(span: &Range<usize>, visible: &[Range<usize>]) -> bool {
    visible.iter().any(|v| if span.is_empty() { v.contains(&span.start) } else { span.start < v.end && v.start < span.end })
}

/// The candidate window's publication in a frame: what maps the layout's
/// display bytes back to its buffer (`Vm::typing_picture`).
pub struct TypingPicture {
    window_id: u64,
    view_generation: u64,
    text_revision: u64,
    base: usize,
    segments: Vec<crate::display::Segment>,
    /// Point, in display and in buffer bytes.
    point: (usize, usize),
    /// Rows can hide bytes at their sides (truncated lines, hscroll).
    clips: bool,
}

impl TypingPicture {
    pub(crate) fn new(
        (window_id, view_generation, text_revision): (u64, u64, u64),
        (base, segments): (usize, Vec<crate::display::Segment>),
        point: (usize, usize),
        clips: bool,
    ) -> Self {
        Self { window_id, view_generation, text_revision, base, segments, point, clips }
    }

    /// The window as END says the layout painted it, CURSOR_PAINTED when its
    /// cursor was drawn: its painted buffer bytes and cursor.
    pub fn painted(&self, end: &crate::WindowEnd, cursor_painted: bool) -> Option<SubmittedWindow> {
        (end.id == self.window_id).then(|| SubmittedWindow {
            window_id: end.id,
            view_generation: self.view_generation,
            text_revision: self.text_revision,
            visible_text_bytes: self.displayed(end).into_iter().flat_map(|range| self.to_buffer(range)).collect(),
            cursor_byte: cursor_painted.then_some(self.point.1),
        })
    }

    /// The display bytes END's rows surely painted: those of its fully
    /// visible rows; where rows clip, only point's row, on each side of
    /// point that is not truncated.
    fn displayed(&self, end: &crate::WindowEnd) -> Vec<Range<usize>> {
        let Some(top) = end.rows.iter().find(|row| row.y >= 0.0) else { return Vec::new() };
        let shown = top.start..end.visible_end.max(top.start);
        if !self.clips {
            return vec![shown];
        }
        let (Some(place), point) = (end.point, self.point.0) else { return Vec::new() };
        let Some(at) = end.rows.iter().rposition(|row| row.start <= point) else { return Vec::new() };
        let row_end = end.rows.get(at + 1).map_or(end.end, |row| row.start);
        let sides = [(!place.truncated_left).then_some(end.rows[at].start..point), (!place.truncated_right).then_some(point..row_end)];
        sides.into_iter().flatten().map(|side| side.start.max(shown.start)..side.end.min(shown.end)).filter(|side| !side.is_empty()).collect()
    }

    /// The buffer bytes display RANGE shows, strings left out.
    fn to_buffer(&self, range: Range<usize>) -> Vec<Range<usize>> {
        if self.segments.is_empty() {
            return vec![self.base + range.start..self.base + range.end];
        }
        self.segments.iter().filter_map(|s| {
            let (start, end) = (range.start.max(s.display), range.end.min(s.display + s.len));
            (!s.string && start < end).then(|| s.buffer + start - s.display..s.buffer + end - s.display)
        }).collect()
    }
}

impl Vm {
    /// After a world restore: the counter past every revision the image's
    /// buffers carry, which another process's counter gave them.
    pub(crate) fn reseed_text_revisions(&self) {
        for object in self.objects.iter() {
            if let crate::data::Object::Buffer(buffer) = object {
                TEXT_REVISION.fetch_max(buffer.body().edits_revision().saturating_add(1), Ordering::Relaxed);
            }
        }
    }

    /// The selected window's view and its buffer's entry, taken.
    fn typing_selected(&mut self) -> Option<Selected> {
        let window = self.selected_window_value().ok()?;
        let (view, buffer) = self.window(window).ok().map(|w| (View { window: w.id, shows: w.shows }, w.buffer))?;
        let buffer = self.annotated_mut(buffer).ok()?;
        let point = buffer.point.0;
        let len = buffer.len_bytes();
        let entry = buffer.edit_entry();
        let revision = entry.revision();
        let (origin, bounds) = entry.take();
        Some(Selected { view, origin, bounds, revision, point, len })
    }

    /// `romacs--typing-begin': a command step begins.
    pub(crate) fn typing_begin(&mut self) {
        let selected = self.typing_selected();
        self.typing.begin(selected);
    }

    /// `romacs--typing-end': the step that began last ends, LOCAL unless thrown out of.
    pub(crate) fn typing_end(&mut self, local: bool) {
        if let Some(step) = self.typing.ending(local) {
            let selected = self.typing_selected();
            self.typing.end(step, selected);
        }
    }

    /// A native key Lisp read, delivered by the platform at INGRESS.
    pub(crate) fn typing_read(&mut self, ingress: EventTime) {
        self.typing.read(ingress);
    }

    /// The pending candidate's window's publication in the frame being
    /// painted, which the host needs to say what it painted of it.
    pub fn typing_picture(&self, frame: &romacs_layout_input::FrameSnapshot) -> Option<TypingPicture> {
        let id = self.typing.pending.as_ref()?.view.window;
        self.display_picture(frame, id)
    }

    /// A redisplay submitted RECEIPT's frame: settle the pending step.
    pub fn typing_submitted(&mut self, receipt: &SubmissionReceipt) {
        self.typing.submitted(receipt);
    }

    /// A redisplay submitted nothing: the pending step changed nothing shown.
    pub fn typing_unchanged(&mut self) {
        self.typing.unchanged();
    }

    /// The native keys settled since the last call, in the order settled.
    pub fn take_typing_outcomes(&mut self) -> Vec<KeyOutcome> {
        std::mem::take(&mut self.typing.settled)
    }
}

#[cfg(test)]
#[path = "typing_tests.rs"]
mod tests;
