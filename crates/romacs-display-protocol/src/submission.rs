//! What a submitted scene showed of the window a command step left, and
//! when: the receipt romacs-vm's typing.rs settles a step's keys by.
//!
//! The timestamps, on the one platform clock (`frame_time::EventTime`):
//! - a key's ingress: the render thread, as the platform delivers the key
//!   press (`input_progress::InputDelivery::ingress`);
//! - `submitted_at`: the evaluator thread, as `submit_scene` returns Ok;
//! - `drawn_at`: the render thread, as the GPU queue returns from presenting
//!   the frame, which is not when the compositor shows it.
//!
//! The receipt covers the keys the evaluator reads as Lisp input. A key
//! the terminal passthrough takes (measured by `key-ingress`) and a C-g the
//! render thread turns into a quit never reach it.
use crate::frame_time::EventTime;
use std::ops::Range;

/// One submitted scene, stamped by the evaluator thread that submitted it,
/// the only one that sees the submission succeed. Process-local.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionReceipt {
    pub presentation: u64,
    /// When `submit_scene` returned Ok, on the evaluator thread.
    pub submitted_at: EventTime,
    /// When the render thread's present call for this presentation, or the
    /// first later one, returned: None until the evaluator sees it drawn
    /// (`PaintDiagnostic::drawn_at`), which it fills in then.
    pub drawn_at: Option<EventTime>,
    /// The pending step's window as painted, or None when the scene did not
    /// show it: the one window a receipt describes.
    pub window: Option<SubmittedWindow>,
}

/// A window's painted text and cursor, in buffer bytes at its text revision.
/// `visible_text_bytes` holds only bytes the scene surely painted.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SubmittedWindow {
    pub window_id: u64,
    pub view_generation: u64,
    pub text_revision: u64,
    pub visible_text_bytes: Vec<Range<usize>>,
    pub cursor_byte: Option<usize>,
}
