//! The reuse check (`romacs-display-check`): each time a window's snapshot
//! is answered from its last one (`Vm::reuse_shown`), the window's text is
//! also built in full from the same state, and the two are compared: text,
//! face runs, start, point, stretches, images and line numbers. The bands
//! are no part of it: reuse expands them again. Nor is what lays the text
//! out (tab width, wrap, scroll, chrome), which a snapshot reads apart. A difference is an input
//! the window's key (`ShownKey`) does not hold. It is counted
//! (`romacs--display-stats`, `check-mismatches`) and told once per window
//! and cause as a display diagnostic.
//!
//! The full build here is the one a snapshot makes, less what runs Lisp:
//! point visibility and fontification come before a window's text and
//! change the state it reads, so the check compares against the state as
//! it stands, which is also what the reused snapshot claims to show.
use super::ShownText;
use crate::{Result, Value, Vm};
use romacs_layout_input::{Extend, Face, FaceRun, WindowSnapshot};

/// What a reused snapshot and the full build can differ in, in the order
/// told; a cause's index is its bit in `DisplayState::check_told`.
const CAUSES: [&str; 10] = ["text", "raw bytes", "face runs", "line ends", "start", "point", "stretches", "images", "line numbers", "full build"];
const NO_BUILD: usize = 9;

/// At most this much of a text or a list is quoted in a diagnostic.
const QUOTED: usize = 48;

/// TEXT from byte AT, QUOTED bytes at most, cut at character boundaries.
fn excerpt(text: &str, at: usize) -> &str {
    let mut from = at.min(text.len());
    while !text.is_char_boundary(from) { from -= 1; }
    let mut to = (from + QUOTED).min(text.len());
    while !text.is_char_boundary(to) { to -= 1; }
    &text[from..to]
}

/// The first place lists A and B differ, with each one's item there.
fn first_difference<T: PartialEq + std::fmt::Debug>(a: &[T], b: &[T]) -> Option<String> {
    let at = a.iter().zip(b).position(|(a, b)| a != b).or((a.len() != b.len()).then_some(a.len().min(b.len())))?;
    let item = |list: &[T]| list.get(at).map_or("nothing".into(), |item| { let mut text = format!("{item:?}"); text.truncate(4 * QUOTED); text });
    Some(format!("item {at} of {} and {}: reused {}, rebuilt {}", a.len(), b.len(), item(a), item(b)))
}

/// Face runs as the faces they paint in: a handle means its table's face.
fn painted<'a>(runs: &'a [FaceRun], faces: &'a [Face]) -> Vec<(std::ops::Range<usize>, Option<&'a Face>)> {
    runs.iter().map(|run| (run.bytes.clone(), faces.get(run.face.0 as usize))).collect()
}

fn extended<'a>(extends: &'a [Extend], faces: &'a [Face]) -> Vec<(u32, Option<&'a Face>)> {
    extends.iter().map(|extend| (extend.byte, faces.get(extend.face.0 as usize))).collect()
}

/// How REBUILT, a window's text built in full with the faces FACES, differs
/// from REUSED, the snapshot reuse answered: each cause's index in CAUSES
/// with where the two part.
fn differences(reused: &WindowSnapshot, rebuilt: &ShownText, faces: &[Face]) -> Vec<(usize, String)> {
    let (old, new) = (&reused.text, &rebuilt.text);
    let mut out = vec![];
    if old.utf8 != new.utf8 {
        let at = old.utf8.bytes().zip(new.utf8.bytes()).position(|(a, b)| a != b).unwrap_or(old.utf8.len().min(new.utf8.len()));
        out.push((0, format!("display byte {at} of {} and {}: reused {:?}, rebuilt {:?}", old.utf8.len(), new.utf8.len(), excerpt(&old.utf8, at), excerpt(&new.utf8, at))));
    }
    let listed = [
        (1, first_difference(&old.raw, &new.raw)),
        (2, first_difference(&painted(&old.face_runs, &reused.faces), &painted(&new.face_runs, faces))),
        (3, first_difference(&extended(&old.extends, &reused.faces), &extended(&new.extends, faces))),
        (4, (reused.start_byte != rebuilt.start_byte).then(|| format!("reused {}, rebuilt {}", reused.start_byte, rebuilt.start_byte))),
        (5, (reused.point_byte != rebuilt.point_byte).then(|| format!("reused {}, rebuilt {}", reused.point_byte, rebuilt.point_byte))),
        (6, first_difference(&reused.stretches, &rebuilt.stretches)),
        (7, first_difference(&reused.images, &rebuilt.images)),
        (8, (reused.wrap_flips != rebuilt.wrap_flips || reused.line_numbers != rebuilt.line_numbers)
            .then(|| format!("reused {:?} {:?}, rebuilt {:?} {:?}", reused.wrap_flips, reused.line_numbers, rebuilt.wrap_flips, rebuilt.line_numbers))),
    ];
    out.extend(listed.into_iter().filter_map(|(cause, detail)| detail.map(|detail| (cause, detail))));
    out
}

impl Vm {
    /// Whether `romacs-display-check` is on: its value outside any buffer.
    fn display_check(&mut self) -> bool {
        let slot = self.slot("romacs-display-check");
        !matches!(self.slots[slot].value, Value::NIL | Value::UNBOUND)
    }
    /// Under `romacs-display-check`, compare REUSED, the snapshot reuse
    /// answered for WINDOW at HEIGHT, with the window's text built in full
    /// now: a difference is counted, and each cause told once per window.
    /// Runs no Lisp, and leaves what the window last showed as recorded;
    /// the build may add the faces it merges to the window's table, as any
    /// build does.
    pub(super) fn check_reuse(&mut self, window: Value, height: u32, reused: &WindowSnapshot) {
        if !self.display_check() { return; }
        let found = match self.full_text(window, height) {
            Ok(rebuilt) => differences(reused, &rebuilt, &rebuilt.faces.published()),
            Err(error) => vec![(NO_BUILD, error)],
        };
        if found.is_empty() { return; }
        self.display_state.stats.mismatches += 1;
        let told = self.display_state.check_told.entry(reused.id).or_default();
        let news: Vec<_> = found.into_iter().filter(|(cause, _)| *told & (1 << cause) == 0).collect();
        for (cause, _) in &news { *told |= 1 << cause; }
        if news.is_empty() { return; }
        let place = self.check_place(window);
        for (cause, detail) in news {
            self.display_diagnostic(format!("display-check: window {} {place}: {}: {detail}", reused.id, CAUSES[cause]));
        }
    }
    /// WINDOW's text as a full snapshot publishes it: the mini-window's
    /// with its overlay strings.
    fn full_text(&mut self, window: Value, height: u32) -> Result<ShownText> {
        let w = self.window(window)?;
        let (buffer, mini) = (w.buffer, w.mini);
        let mut shown = self.window_text(window, height, None)?;
        if mini { self.splice_mini_window(window, buffer, &mut shown)?; }
        Ok(shown)
    }
    /// What a diagnostic says of WINDOW besides its id: its buffer, and
    /// the counters a key holds for it, to look for the input that moved
    /// beside them.
    fn check_place(&mut self, window: Value) -> String {
        let Ok(w) = self.window(window) else { return "(deleted)".into() };
        let (buffer, start) = (w.buffer, w.start);
        let state = self.buffer_state(buffer).ok();
        let name = state.and_then(|state| state.name.clone()).unwrap_or_default();
        let modiff = state.map_or(0, |state| state.modiff);
        let start = self.marker_position_value(start).ok().and_then(Value::integer_value).unwrap_or(0);
        let point = self.window_point_value(window).ok().and_then(Value::integer_value).unwrap_or(0);
        format!("of {name:?} (start {start}, point {point}, modiff {modiff}, overlays {}, collections {})",
            self.display_state.overlays, self.gc_stats.collections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame whose window shows three lines, snapshot once, the check
    /// set to CHECK where given.
    fn shown(check: Option<&str>) -> (Vm, u64) {
        let mut vm = Vm::new();
        vm.set_frame_geometry(800, 280, 20, 10).unwrap();
        if let Some(value) = check { vm.eval_source(&format!("(setq romacs-display-check {value})")).unwrap(); }
        vm.eval_source("(progn (insert \"one\\ntwo\\nthree\\n\") (goto-char 1))").unwrap();
        vm.frame_snapshot(800, 280).unwrap();
        let id = vm.selected_window_id().unwrap();
        (vm, id)
    }
    fn count(vm: &mut Vm, name: &str) -> u64 {
        vm.eval_source(&format!("(cdr (assq '{name} (romacs--display-stats)))")).unwrap().display().unwrap().parse().unwrap()
    }
    /// Record the selected window's key as it is now for its last snapshot:
    /// what a key that missed the input changed since would amount to.
    fn forge_key(vm: &mut Vm, id: u64) {
        let window = vm.selected_window_value().unwrap();
        let (width, height) = vm.window_body_size(window).unwrap();
        let key = vm.shown_key(window, width, height).unwrap();
        vm.display_state.shown.get_mut(&id).unwrap().key = key;
    }

    #[test]
    fn a_right_reuse_passes_the_check() {
        let (mut vm, _) = shown(Some("t"));
        vm.frame_snapshot(800, 280).unwrap();
        assert!(count(&mut vm, "windows-reused") >= 1, "the second snapshot reuses");
        assert_eq!(count(&mut vm, "check-mismatches"), 0);
        assert_eq!(vm.take_display_diagnostics(), Vec::<String>::new());
    }

    #[test]
    fn a_stale_reuse_is_counted_and_each_cause_told_once() {
        let (mut vm, id) = shown(Some("t"));
        vm.eval_source("(save-excursion (goto-char (point-max)) (insert \"four\\n\"))").unwrap();
        forge_key(&mut vm, id);
        let stale = vm.frame_snapshot(800, 280).unwrap();
        assert!(!stale.windows[0].snapshot.text.utf8.contains("four"), "the reuse is stale");
        assert_eq!(count(&mut vm, "check-mismatches"), 1);
        let told = vm.take_display_diagnostics();
        // The stale text is shorter, and so is the run of its face.
        assert_eq!(told.len(), 2, "{told:?}");
        assert!(told[0].starts_with(&format!("display-check: window {id} of \"*scratch*\" (start 1, point 1, modiff ")), "{told:?}");
        assert!(told[0].contains("): text: display byte 14 of 14 and 19: reused \"\", rebuilt \"four\\n\""), "{told:?}");
        assert!(told[1].contains("): face runs: item 0 of 1 and 1: reused (0..14, Some("), "{told:?}");
        // The same staleness again: counted, not told again.
        vm.frame_snapshot(800, 280).unwrap();
        assert_eq!(count(&mut vm, "check-mismatches"), 2);
        assert_eq!(vm.take_display_diagnostics(), Vec::<String>::new());
        // A stretch the key missed is another cause, told beside the known ones.
        vm.eval_source("(put-text-property 1 2 'display '(space :width 3))").unwrap();
        forge_key(&mut vm, id);
        vm.frame_snapshot(800, 280).unwrap();
        let told = vm.take_display_diagnostics();
        assert!(told.iter().any(|line| line.contains("): stretches: item 0 of 0 and 1: reused nothing, rebuilt Stretch")), "{told:?}");
        assert!(!told.iter().any(|line| line.contains("): text: ") || line.contains("): face runs: ")), "{told:?}");
    }

    #[test]
    fn the_check_is_off_unless_set() {
        for check in [None, Some("nil")] {
            let (mut vm, id) = shown(check);
            vm.eval_source("(save-excursion (goto-char (point-max)) (insert \"four\\n\"))").unwrap();
            forge_key(&mut vm, id);
            vm.frame_snapshot(800, 280).unwrap();
            assert_eq!(count(&mut vm, "check-mismatches"), 0, "{check:?}");
            assert_eq!(vm.take_display_diagnostics(), Vec::<String>::new(), "{check:?}");
        }
    }

    #[test]
    fn differences_name_where_two_lists_part() {
        assert_eq!(first_difference(&[1, 2], &[1, 2]), None);
        assert_eq!(first_difference(&[1, 2], &[1, 3]).unwrap(), "item 1 of 2 and 2: reused 2, rebuilt 3");
        assert_eq!(first_difference(&[1], &[1, 3]).unwrap(), "item 1 of 1 and 2: reused nothing, rebuilt 3");
        assert_eq!(excerpt("aΩb", 2), "Ωb");
    }
}
