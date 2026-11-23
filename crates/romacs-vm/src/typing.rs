//! Evaluator ownership of native commands and their first submitted display.
use crate::{Value, Vm};
use romacs_display_protocol::frame_time::EventTime;
use romacs_display_protocol::submission::{SubmissionReceipt, SubmittedWindow};
use std::{ops::Range, time::Duration};

#[derive(Clone, Default, romacs_snapshot::Snapshot)]
pub(crate) struct TextEdit {
    pub revision: u64,
    pub bytes: Range<usize>,
}

impl TextEdit {
    pub(crate) fn record(&mut self, start: usize, inserted: usize) {
        self.revision += 1;
        self.bytes = start..start + inserted;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    window: u64,
    generation: u64,
    revision: u64,
    point: usize,
}

struct Active {
    ingress: EventTime,
    before: Option<View>,
}

#[derive(Clone)]
pub(crate) struct Candidate {
    ingress: EventTime,
    after: View,
    edit: Option<Range<usize>>,
}

/// A command whose resulting cursor and edit were in a submitted frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypingMatch {
    pub ingress: EventTime,
    pub presentation: u64,
    pub submitted_at: EventTime,
}

#[derive(Default)]
pub(crate) struct Typing {
    pub(crate) ingress: Option<EventTime>,
    epoch: Option<EventTime>,
    active: Vec<Active>,
    pending: Option<Candidate>,
    matched: Option<TypingMatch>,
    completed: Option<EventTime>,
}

impl Typing {
    pub(crate) fn take_ingress(&mut self) -> Option<i64> {
        let at = self.ingress.take()?;
        let epoch = *self.epoch.get_or_insert(at);
        if at < epoch {
            return None;
        }
        at.saturating_since(epoch).as_nanos().try_into().ok()
    }

    pub(crate) fn candidate(&self) -> Option<&Candidate> {
        self.pending.as_ref()
    }

    fn submitted(&mut self, receipt: &SubmissionReceipt) -> Option<TypingMatch> {
        self.pending.take()?.matched(receipt)
    }
}

impl Candidate {
    pub(crate) fn window(&self) -> u64 {
        self.after.window
    }

    fn matched(&self, receipt: &SubmissionReceipt) -> Option<TypingMatch> {
        let window = receipt
            .windows
            .iter()
            .find(|window| window.window_id == self.after.window)?;
        self.in_frame(window, receipt)
    }

    fn in_frame(
        &self,
        window: &SubmittedWindow,
        receipt: &SubmissionReceipt,
    ) -> Option<TypingMatch> {
        let View {
            generation,
            revision,
            point,
            ..
        } = self.after;
        if window.view_generation != generation
            || window.text_revision != revision
            || window.cursor_byte != Some(point)
            || receipt.submitted_at < self.ingress
        {
            return None;
        }
        if let Some(edit) = &self.edit
            && !window.visible_text_bytes.iter().any(|visible| {
                if edit.is_empty() {
                    visible.contains(&edit.start) || (point == edit.start && visible.end == edit.start)
                } else {
                    edit.start < visible.end && visible.start < edit.end
                }
            })
        {
            return None;
        }
        Some(TypingMatch {
            ingress: self.ingress,
            presentation: receipt.presentation,
            submitted_at: receipt.submitted_at,
        })
    }
}

/// The buffer coordinates frozen for the one window a command may have changed.
pub struct TypingPicture {
    candidate: Candidate,
    window_id: u64,
    view_generation: u64,
    text_revision: u64,
    base: usize,
    segments: Vec<crate::display::Segment>,
    point: usize,
}

impl TypingPicture {
    pub(crate) fn new(
        candidate: Candidate,
        view_generation: u64,
        text_revision: u64,
        base: usize,
        segments: Vec<crate::display::Segment>,
        point: usize,
    ) -> Self {
        Self {
            window_id: candidate.window(),
            candidate,
            view_generation,
            text_revision,
            base,
            segments,
            point,
        }
    }

    /// Project the layout's runs back into the originating buffer.
    pub fn painted(&self, end: &crate::WindowEnd, cursor_painted: bool) -> Option<SubmittedWindow> {
        if self.window_id != end.id {
            return None;
        }
        let visible_text_bytes = if self.segments.is_empty() {
            end.visible_text_bytes
                .iter()
                .map(|r| self.base + r.start..self.base + r.end)
                .collect()
        } else {
            end.visible_text_bytes
                .iter()
                .flat_map(|range| {
                    self.segments.iter().filter_map(move |s| {
                        let start = range.start.max(s.display);
                        let end = range.end.min(s.display + s.len);
                        (!s.string && start < end)
                            .then(|| s.buffer + start - s.display..s.buffer + end - s.display)
                    })
                })
                .collect()
        };
        Some(SubmittedWindow {
            window_id: end.id,
            view_generation: self.view_generation,
            text_revision: self.text_revision,
            visible_text_bytes,
            cursor_byte: cursor_painted.then_some(self.point),
        })
    }

    /// Whether this submission carried the command for the prompt stage trace.
    pub fn matched(&self, receipt: &SubmissionReceipt) -> Option<TypingMatch> {
        self.candidate.matched(receipt)
    }
}

impl Vm {
    fn typing_view(&mut self) -> Option<(View, Range<usize>)> {
        let window = self.selected_window_value().ok()?;
        let w = self.window(window).ok()?;
        let crate::data::Object::Buffer(buffer) = self.object_if(w.buffer)? else {
            return None;
        };
        let edit = &buffer.body().last_edit;
        Some((
            View {
                window: w.id,
                generation: w.shows,
                revision: edit.revision,
                point: buffer.point.0,
            },
            edit.bytes.clone(),
        ))
    }

    pub(crate) fn typing_begin(&mut self, ticks: i64) -> bool {
        let Some(ingress) = self
            .typing
            .epoch
            .and_then(|epoch| Some(epoch.plus(Duration::from_nanos(ticks.try_into().ok()?))))
        else {
            return false;
        };
        let before = self.typing_view().map(|(view, _)| view);
        if let Some(outer) = self.typing.active.last_mut() {
            outer.before = None;
        }
        self.typing.active.push(Active { ingress, before });
        true
    }

    pub(crate) fn typing_end(&mut self, success: Value) {
        let Some(Active { ingress, before }) = self.typing.active.pop() else {
            return;
        };
        if success == Value::NIL {
            return;
        }
        let Some(before) = before else { return };
        self.typing.completed = Some(ingress);
        let Some((after, bytes)) = self.typing_view() else {
            return;
        };
        if (before.window, before.generation) != (after.window, after.generation) {
            return;
        }
        self.typing.pending = match after.revision.checked_sub(before.revision) {
            Some(0) if before.point != after.point => Some(Candidate {
                ingress,
                after,
                edit: None,
            }),
            Some(1) => Some(Candidate {
                ingress,
                after,
                edit: Some(bytes),
            }),
            _ => None,
        };
    }

    /// Consume the ingress of the latest native sequence that ran a command.
    pub fn take_completed_key(&mut self) -> Option<EventTime> {
        self.typing.completed.take()
    }

    /// Attribute a successful submission and retain its match for the host.
    pub fn typing_submitted(&mut self, receipt: &SubmissionReceipt) {
        if let Some(matched) = self.typing.submitted(receipt) {
            self.typing.matched = Some(matched);
        }
    }

    /// Take the most recent match from an ordinary or prompt submission.
    pub fn take_typing_match(&mut self) -> Option<TypingMatch> {
        self.typing.matched.take()
    }

    /// An ordinary redisplay confirmed there was no submission.
    pub fn typing_unchanged(&mut self) {
        self.typing.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romacs_display_protocol::frame_time::observe_platform_now;

    fn command(edit: Option<Range<usize>>) -> Typing {
        let ingress = observe_platform_now();
        let after = View {
            window: 4,
            generation: 2,
            revision: 6,
            point: 12,
        };
        Typing {
            pending: Some(Candidate {
                ingress,
                after,
                edit,
            }),
            ..Typing::default()
        }
    }

    fn receipt(typing: &Typing, presentation: u64, cursor: usize) -> SubmissionReceipt {
        let submitted_at = typing
            .pending
            .as_ref()
            .expect("command")
            .ingress
            .plus(Duration::from_millis(1));
        SubmissionReceipt {
            presentation,
            submitted_at,
            windows: vec![SubmittedWindow {
                window_id: 4,
                view_generation: 2,
                text_revision: 6,
                visible_text_bytes: vec![8..20],
                cursor_byte: Some(cursor),
            }],
        }
    }

    #[test]
    fn a_visible_edit_matches_its_first_submission() {
        let mut typing = command(Some(11..12));
        let receipt = receipt(&typing, 3, 12);
        assert_eq!(typing.submitted(&receipt).map(|m| m.presentation), Some(3));
        assert!(typing.submitted(&receipt).is_none());
    }

    #[test]
    fn a_mode_line_submission_does_not_present_an_offscreen_edit() {
        let mut typing = command(Some(41..42));
        let receipt = receipt(&typing, 3, 12);
        assert!(typing.submitted(&receipt).is_none());
    }

    #[test]
    fn the_cursor_from_a_coalesced_command_does_not_match() {
        let mut typing = command(None);
        let receipt = receipt(&typing, 3, 13);
        assert!(typing.submitted(&receipt).is_none());
    }
}
