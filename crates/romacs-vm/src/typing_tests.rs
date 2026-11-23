//! The edit entry's bounds, a step's span across the steps inside it, and
//! what a receipt is to a candidate. Real command steps: tests/typing_receipts.rs.
use super::*;
use romacs_display_protocol::frame_time::observe_platform_now;

fn selected(origin: u64, bounds: Bounds, revision: u64, len: usize) -> Selected {
    Selected { view: View { window: 4, shows: 2 }, origin, bounds, revision, point: 3, len }
}

/// The written span of ENTRY's writes, in a text of LEN bytes.
fn span(entry: &mut EditEntry, len: usize) -> Edit {
    entry.take().1.edit(len)
}

#[test]
fn the_entry_bounds_every_write_since_its_take() {
    let mut entry = EditEntry::default();
    assert_eq!(span(&mut entry, 0), Edit::Unknown, "a new body's history is unknown");
    assert_eq!(span(&mut entry, 0), Edit::Clean, "a take starts over");
    // "hello" -> "heXllo" -> "heXllo!": the box from the first to the last.
    entry.record(2, 1, 6);
    entry.record(6, 1, 7);
    assert_eq!(span(&mut entry, 7), Edit::Span(2..7));
    // A deletion is the point it left: "abcdef" less "cd".
    entry.record(2, 0, 4);
    assert_eq!(span(&mut entry, 4), Edit::Span(2..2));
}

#[test]
fn each_write_takes_a_new_revision() {
    let (mut a, b) = (EditEntry::default(), EditEntry::default());
    let before = a.revision();
    a.record(0, 1, 1);
    assert!(a.revision() > before && a.revision() > b.revision() && a.origin != b.origin);
}

/// A step inside another takes the body's entry as it begins and ends: the
/// writes it takes are the outer step's too, so the outer span keeps them.
#[test]
fn a_nested_command_keeps_the_outer_edit_span() {
    let mut typing = Typing::default();
    typing.begin(Some(selected(9, Bounds::CLEAN, 1, 20)));
    typing.read(observe_platform_now());
    // The outer step wrote bytes 5..6 of 20 before the inner one began.
    typing.begin(Some(selected(9, Bounds { beg: 5, end: 14, unknown: false }, 2, 20)));
    typing.read(observe_platform_now());
    let inner = typing.ending(true).expect("the inner step read a key");
    typing.end(inner, Some(selected(9, Bounds { beg: 12, end: 4, unknown: false }, 3, 20)));
    let outer = typing.ending(true).expect("the outer step read a key");
    typing.end(outer, Some(selected(9, Bounds::CLEAN, 3, 20)));
    let pending = typing.pending.as_ref().expect("the outer candidate");
    assert_eq!(pending.edit, Edit::Span(5..16));
    assert_eq!(typing.take_settled(), vec!["superseded"], "the inner candidate never showed");
}

/// A step thrown out of hands its keys to the step around it; a body that
/// is not the one the step began on is an unknown edit.
#[test]
fn a_thrown_step_hands_its_keys_out() {
    let mut typing = Typing::default();
    typing.begin(Some(selected(9, Bounds::CLEAN, 1, 20)));
    typing.read(observe_platform_now());
    typing.begin(Some(selected(9, Bounds::CLEAN, 1, 20)));
    typing.read(observe_platform_now());
    assert!(typing.ending(false).is_none(), "its keys went out");
    let outer = typing.ending(true).expect("the outer step's keys");
    assert_eq!(outer.keys.len(), 2);
    typing.end(outer, Some(selected(10, Bounds::CLEAN, 1, 20)));
    assert_eq!(typing.pending.as_ref().map(|c| c.edit.clone()), Some(Edit::Unknown));
}

fn receipt(window: Option<SubmittedWindow>) -> SubmissionReceipt {
    SubmissionReceipt { presentation: 3, submitted_at: observe_platform_now(), drawn_at: None, window }
}

fn painted(visible: Vec<Range<usize>>, cursor: Option<usize>) -> Option<SubmittedWindow> {
    Some(SubmittedWindow { window_id: 4, view_generation: 2, text_revision: 6, visible_text_bytes: visible, cursor_byte: cursor })
}

fn candidate(edit: Edit) -> Candidate {
    Candidate { keys: Vec::new(), view: View { window: 4, shows: 2 }, revision: 6, point: 12, edit }
}

fn judged(edit: Edit, window: Option<SubmittedWindow>) -> Outcome {
    candidate(edit).judge(&receipt(window))
}

#[test]
fn a_frame_is_charged_only_on_an_exact_match() {
    let shown = || painted(vec![8..20], Some(12));
    assert!(matches!(judged(Edit::Span(11..12), shown()), Outcome::Rendered(_)));
    assert!(matches!(judged(Edit::Span(12..12), shown()), Outcome::Rendered(_)), "a deletion at a painted byte");
    assert_eq!(judged(Edit::Span(41..42), shown()), Outcome::Unattributed(Unattributed::Offscreen));
    assert_eq!(judged(Edit::Span(20..20), shown()), Outcome::Unattributed(Unattributed::Offscreen));
    assert_eq!(judged(Edit::Unknown, shown()), Outcome::Unattributed(Unattributed::EditUnknown));
    assert_eq!(judged(Edit::Clean, painted(vec![8..20], Some(13))), Outcome::Unattributed(Unattributed::Superseded));
    assert_eq!(judged(Edit::Clean, painted(vec![8..20], None)), Outcome::Unattributed(Unattributed::Superseded), "no cursor drawn");
    assert_eq!(judged(Edit::Moved, None), Outcome::Unattributed(Unattributed::NotShown));
    assert!(matches!(judged(Edit::Moved, shown()), Outcome::Rendered(_)));
}

impl Typing {
    /// The outcome names settled so far, taken.
    fn take_settled(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.settled).into_iter().map(|key| match key.outcome {
            Outcome::Rendered(_) => "rendered",
            Outcome::Unchanged => "unchanged",
            Outcome::Unattributed(why) => why.name(),
        }).collect()
    }
}

/// A picture of a window showing buffer bytes from 100 on, point at
/// display byte 5, its rows starting at display bytes 0, 10 and 20.
fn picture(clips: bool) -> TypingPicture {
    TypingPicture::new((4, 2, 6), (100, Vec::new()), (5, 105), clips)
}

fn end(visible_end: usize, point: Option<romacs_layout_input::PointPlace>) -> crate::WindowEnd {
    let rows = [(0, -4.0), (10, 16.0), (20, 36.0)].map(|(start, y)| romacs_layout_input::Row { start, y, height: 20.0 }).to_vec();
    crate::WindowEnd { id: 4, end: 30, visible_end, body_height: 60.0, rows, origin_y: 0.0, point }
}

#[test]
fn coverage_is_only_what_the_rows_surely_painted() {
    let painted = |clips, point| picture(clips).painted(&end(25, point), true).expect("the window").visible_text_bytes;
    // A top row scrolled partly out, a bottom row partly in: neither counts.
    assert_eq!(painted(false, None), vec![110..125]);
    // Rows that clip at their sides: point's row only, on its whole sides.
    let place = |left, right| Some(romacs_layout_input::PointPlace { truncated_left: left, truncated_right: right, ..Default::default() });
    assert_eq!(painted(true, place(false, false)), Vec::<Range<usize>>::new(), "point's row is the partly shown top one");
    let point_row = TypingPicture::new((4, 2, 6), (100, Vec::new()), (15, 115), true);
    let shown = |left, right| point_row.painted(&end(25, place(left, right)), true).expect("the window").visible_text_bytes;
    assert_eq!(shown(false, false), vec![110..115, 115..120]);
    assert_eq!(shown(true, false), vec![115..120]);
    assert_eq!(shown(true, true), Vec::<Range<usize>>::new());
}

#[test]
fn coverage_leaves_out_strings() {
    let segments = vec![
        crate::display::Segment { display: 0, buffer: 100, len: 12, string: false },
        crate::display::Segment { display: 12, buffer: 112, len: 4, string: true },
        crate::display::Segment { display: 16, buffer: 112, len: 14, string: false },
    ];
    let picture = TypingPicture::new((4, 2, 6), (0, segments), (5, 105), false);
    let painted = picture.painted(&end(25, None), true).expect("the window");
    assert_eq!(painted.visible_text_bytes, vec![110..112, 112..121]);
}
