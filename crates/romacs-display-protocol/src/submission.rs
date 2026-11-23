//! Metadata for windows that contribute to a submitted scene.
use crate::frame_time::EventTime;
use std::ops::Range;

/// The frame the display host submitted. The submitting process stamps
/// this local receipt because only it observes the submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionReceipt {
    pub presentation: u64,
    pub submitted_at: EventTime,
    pub windows: Vec<SubmittedWindow>,
}

/// A window's painted text and cursor, in buffer bytes at its text revision.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SubmittedWindow {
    pub window_id: u64,
    pub view_generation: u64,
    pub text_revision: u64,
    pub visible_text_bytes: Vec<Range<usize>>,
    pub cursor_byte: Option<usize>,
}
