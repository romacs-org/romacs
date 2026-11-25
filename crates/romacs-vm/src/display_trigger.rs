//! The redisplay trigger: after Lisp the command loop ran between keys (a
//! timer, process output, a background callback), whether the frame on
//! screen went stale, asked before a redisplay cycle is entered at all.
//! GNU's redisplay after a timer finds most windows up to date and leaves
//! early (xdisp.c redisplay_internal's no-change exit); here the question is
//! asked of what the published frame read, so a callback that changed
//! nothing shown costs no cycle. The display version (`version`) is no
//! stamp: every buffer's edit moves it, a hidden one's too, and so does
//! each publication.
//!
//! The walk reads, for each window the published frame shows, the record
//! its last snapshot left (`Shown`: its `ShownKey`, point and owner
//! picture) and compares it with the window now, as reuse does
//! (`Vm::reuse_shown`): scalars and ids, no text, no glyphs. The trigger
//! expands no band: a window that wants one of its own (a mode line, a
//! header line) is stale and cycles, as before K2; with the
//! status bar, which holds the mode line, an ordinary window has none. A
//! window that wants none is current only while its last publication
//! painted none either. The frame's own state (its explicit
//! invalidations, which `force-window-update` and font changes advance, the
//! frame parameters' and faces' revisions, its windows and child frames,
//! its status and tab bars) is compared with the frame accepted
//! (`DisplayAcceptance`) and the frame itself. Anything the walk cannot
//! vouch for reads as stale: the trigger only ever asks for a cycle.
use super::status_bar::{BarKey, BarPlan};
use crate::*;
use romacs_layout_input::FrameSnapshot;

/// What a published frame stood for beyond its windows' own records,
/// taken when the host accepts it (`Vm::display_acceptance`): its frame's
/// stamps, and the key the status bar it painted was expanded under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayAcceptance {
    stamps: FrameStamps,
    bar: Option<BarKey>,
}

/// The top-level frame shown, the explicit invalidations so far
/// (`DisplayState::invalidate`), and the frame parameters' and faces'
/// revisions (`Vm::frame_stamps`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrameStamps {
    frame: u64,
    invalidations: u64,
    frames: u64,
    faces: u64,
}

impl FrameStamps {
    /// What of ACCEPTED this differs in, the first found.
    fn differs(&self, accepted: &Self) -> Option<&'static str> {
        [(self.frame != accepted.frame, "top frame"), (self.invalidations != accepted.invalidations, "invalidated"),
         (self.frames != accepted.frames, "frame parameters"), (self.faces != accepted.faces, "faces")]
            .into_iter().find_map(|(differs, why)| differs.then_some(why))
    }
}

/// What a trigger check found (`Vm::display_stale`): the published frame
/// still current, or stale with the first input found changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Staleness {
    Current,
    Outdated(&'static str),
}

impl Staleness {
    pub fn is_stale(self) -> bool { self != Staleness::Current }
}

/// The first of CHECKS that finds a change, in order.
fn first_stale(found: Option<&'static str>) -> Staleness {
    found.map_or(Staleness::Current, Staleness::Outdated)
}

impl Vm {
    /// What the frame the host just accepted (painted, or found equal to
    /// the one painted) stood for, for `display_stale` to compare;
    /// SUBMITTED, the frame painted, when it was.
    pub fn display_acceptance(&mut self, submitted: Option<&FrameSnapshot>) -> Result<DisplayAcceptance> {
        if let Some(frame) = submitted { self.name_built_pictures(frame); }
        Ok(DisplayAcceptance { stamps: self.frame_stamps()?, bar: self.status_bar_expanded() })
    }
    /// FRAME was submitted: the pictures the snapshots of the windows it
    /// carries, its own and its child frames', were built with are the ones
    /// it names. A window it does not carry, and a frame found equal to the
    /// one on screen, name nothing new.
    fn name_built_pictures(&mut self, frame: &FrameSnapshot) {
        let children = frame.children.iter().flat_map(|child| child.windows.iter());
        for id in frame.windows.iter().chain(children).map(|w| w.snapshot.id) {
            if let Some(shown) = self.display_state.shown.get_mut(&id) && let Some(built) = shown.built.take() { shown.picture = Some(built); }
        }
    }
    fn frame_stamps(&mut self) -> Result<FrameStamps> {
        // An in-place edit of the alternative families' alist bypasses its
        // setter: observing it invalidates, as a snapshot would.
        if self.font_alternatives_moved() { self.font_alternative_state()?; }
        self.ensure_windows()?;
        let frame = self.frame(self.selected_top_frame())?.id;
        let state = &self.display_state;
        Ok(FrameStamps { frame, invalidations: state.invalidations, frames: state.frame_revision, faces: state.face_revision })
    }
    /// Whether SHOWN, the frame on screen, accepted as ACCEPTED, is stale:
    /// what a redisplay would build now may differ. Runs
    /// `pre-redisplay-function` when it has work; never a band's Lisp. The
    /// host brackets its whole decision with `trigger_open` and
    /// `trigger_close`, which count it.
    pub fn display_stale(&mut self, shown: &FrameSnapshot, accepted: &DisplayAcceptance) -> Result<Staleness> {
        self.stale_input(shown, accepted).map(first_stale)
    }
    /// A check found current whose frame, built anyway, changed: an input
    /// the trigger does not read (`romacs-display-check`).
    pub fn trigger_missed(&mut self, why: &str) {
        self.display_state.stats.trigger_misses += 1;
        self.display_diagnostic(format!("display-trigger: a frame found current changed: {why}"));
    }
    /// Whether the host should build the frame a check found current, to
    /// verify it (`romacs-display-check`, display_check.rs).
    pub fn trigger_verified(&mut self) -> bool {
        self.display_check()
    }
    /// What the walk finds stale; when it finds nothing, the
    /// `pre-redisplay-function' a redisplay would run first, if it has
    /// work, then the walk again over what it left. A stale frame's cycle
    /// runs that Lisp and the window change functions itself; a current
    /// one leaves the window change functions to the next redisplay,
    /// since the windows the frame shows are as it painted them.
    fn stale_input(&mut self, shown: &FrameSnapshot, accepted: &DisplayAcceptance) -> Result<Option<&'static str>> {
        if let Some(why) = self.stale_shown(shown, accepted)? { return Ok(Some(why)); }
        if !self.pre_redisplay_lisp_ran()? { return Ok(None); }
        self.stale_shown(shown, accepted)
    }
    /// Run `pre-redisplay-function' as a redisplay would, when it has work
    /// (`Vm::pre_redisplay_call`); whether its Lisp ran.
    fn pre_redisplay_lisp_ran(&mut self) -> Result<bool> {
        if self.redisplay_inhibited()? || self.treesit_deferred.is_some() { return Ok(false); }
        let ran = self.run_pre_redisplay()?;
        if ran { self.display_state.hooked = None; }
        Ok(ran)
    }
    /// The frame's stamps and windows, then every window's record, then
    /// the bars. A window that paints a band of its own is left to the
    /// cycle: the trigger expands no band.
    fn stale_shown(&mut self, shown: &FrameSnapshot, accepted: &DisplayAcceptance) -> Result<Option<&'static str>> {
        if let Some(why) = self.frame_stamps()?.differs(&accepted.stamps) { return Ok(Some(why)); }
        let root = self.frame_of_id(accepted.stamps.frame)?;
        let mut leaves = vec![];
        if let Some(why) = self.stale_frame(root, shown, &mut leaves)? { return Ok(Some(why)); }
        for leaf in leaves {
            if let Some(why) = self.stale_window(leaf)? { return Ok(Some(why)); }
        }
        self.stale_bars(accepted, root, shown)
    }
    /// What of top-level frame ROOT's own state SHOWN no longer paints:
    /// its windows, its child frames' places, its fonts. LEAVES gets the
    /// windows found, the frame's and then its child frames'.
    fn stale_frame(&mut self, root: Value, shown: &FrameSnapshot, leaves: &mut Vec<Leaf>) -> Result<Option<&'static str>> {
        for id in self.frame_window_ids(root)? {
            if let Some(found) = self.shown_leaf(id)? { leaves.push(Leaf::of(id, found)); }
        }
        if !leaves.iter().map(|leaf| leaf.id).eq(shown.windows.iter().map(|w| w.snapshot.id)) { return Ok(Some("windows")); }
        if self.shown_children(root, leaves)? != children_of(shown) { return Ok(Some("child frames")); }
        let fontsets = self.fontset_state();
        if !std::sync::Arc::ptr_eq(&fontsets, &shown.fontsets) && fontsets != shown.fontsets { return Ok(Some("fontsets")); }
        Ok((self.no_font_display() != shown.no_font).then_some("no-font display"))
    }
    /// Top-level frame ROOT's visible child frames, lowest first: each one's
    /// id, place and size, and the windows it shows, which LEAVES gets.
    fn shown_children(&mut self, root: Value, leaves: &mut Vec<Leaf>) -> Result<Vec<ShownChild>> {
        let mut children = vec![];
        for frame in self.shown_child_frames(root) {
            let (x, y) = self.frame_origin(frame)?;
            let f = self.frame(frame)?;
            let (id, width, height, hosts, child_root) = (f.id, f.width, f.height, f.hosts_minibuffer, f.root);
            let mut windows = self.leaf_ids(child_root)?;
            if hosts { windows.push(self.window(self.mini_window())?.id); }
            let mut shown = vec![];
            for window in windows {
                if let Some(found) = self.shown_leaf(window)? { shown.push(window); leaves.push(Leaf::of(window, found)); }
            }
            children.push(ShownChild { id, x, y, width, height, windows: shown });
        }
        Ok(children)
    }
    /// What window LEAF now shows differently from its last snapshot, which
    /// the published frame painted: its record (`ShownKey`), point,
    /// cursor and owner picture; and any band at all, which the cycle
    /// expands, as before K2, as the trigger expands none.
    fn stale_window(&mut self, leaf: Leaf) -> Result<Option<&'static str>> {
        let Leaf { id, window, width, height } = leaf;
        let w = self.window(window)?;
        let (forced, cursor_off, buffer, mini) = (w.force_start, w.cursor_off, w.buffer, w.mini);
        let revisions = w.pane_revisions.clone();
        if !self.display_state.shown.contains_key(&id) || !self.display_state.previous.contains_key(&id) {
            return Ok(Some("unpublished window"));
        }
        if forced { return Ok(Some("forced start")); }
        if !mini && self.window_wants_band(window)? { return Ok(Some("bands")); }
        let published = &self.display_state.previous[&id];
        if cursor_off != published.cursor_off { return Ok(Some("cursor")); }
        let point = self.display_state.shown[&id].point;
        if let Some(revisions) = revisions && self.pane_pending(id, &revisions) { return Ok(Some("owner picture")); }
        if self.window_point_value(window)?.as_integer()? != point { return Ok(Some("point")); }
        let key = self.shown_key(window, width, height)?;
        if self.display_state.shown.get(&id).is_none_or(|shown| shown.key != key) { return Ok(Some("window")); }
        let cursor = self.window_cursor_kinds(window, buffer)?;
        if self.display_state.previous.get(&id).is_none_or(|published| published.cursor != cursor) { return Ok(Some("cursor")); }
        // No band now: one the frame painted must go.
        let painted = self.display_state.previous.get(&id).is_some_and(|p| p.mode_line.is_some() || p.header_line.is_some());
        Ok(painted.then_some("bands"))
    }
    /// Whether window ID's owner published a picture its frame does not
    /// name, which the pane path has not composed: one atomic load of each
    /// of REVISIONS (O1 §3.3). One composed already needs no root frame,
    /// and is taken as named.
    fn pane_pending(&mut self, id: u64, revisions: &std::sync::Arc<romacs_display_protocol::pane_revisions::PaneRevisions>) -> bool {
        let (picture, composed) = (revisions.picture(), revisions.composed());
        let Some(shown) = self.display_state.shown.get_mut(&id) else { return true };
        // Another pair is a fresh view or incarnation: it named none.
        let named = shown.picture.as_ref().filter(|(held, _)| std::sync::Arc::ptr_eq(held, revisions)).map_or(0, |&(_, picture)| picture);
        match pane_work(named, picture, composed) {
            PaneWork::Settled => false,
            PaneWork::Composed => { shown.picture = Some((std::sync::Arc::clone(revisions), picture)); false }
            PaneWork::Pending => true,
        }
    }
    /// Whether WINDOW wants a band of its own, a header line or a mode line,
    /// as the window functions decide them. They intern the formats' names,
    /// as every redisplay already does reading them for the record key
    /// (`DISPLAY_VARIABLES`), so the check adds no observable effect. Romacs
    /// paints no tab line, so a wanted one changes no picture; whoever
    /// paints tab lines adds them here.
    fn window_wants_band(&mut self, window: Value) -> Result<bool> {
        let header = crate::window::Windows::new(self).header_line_format(window).map_err(crate::forwarded::fault_text)?;
        Ok(header != Value::NIL || self.window_wants_mode_line(window)?)
    }
    /// Whether the frame's status bar or tab bar would paint differently
    /// from SHOWN's. The bar is expanded again only when what its line
    /// depends on moved (`Vm::bar_key`): it stands while that key is the
    /// one ACCEPTED's bar was expanded under, in the same place. The tab
    /// bar's Lisp runs each time, as each redisplay runs it.
    fn stale_bars(&mut self, accepted: &DisplayAcceptance, root: Value, shown: &FrameSnapshot) -> Result<Option<&'static str>> {
        // As `status_bar_shown' keeps it: only while its row has room.
        let room = self.top_geometry(root).is_ok_and(|g| g.status_reserved > 0);
        let plan = if room { self.status_bar_plan(accepted.stamps.frame)? } else { None };
        let placed = |plan: &BarPlan, bar: &romacs_layout_input::StatusBarSnapshot| plan.place == (bar.x, bar.y, bar.width, bar.height);
        let current = match (&plan, &shown.status_bar) {
            (None, None) => true,
            (Some(plan), Some(bar)) => accepted.bar.as_ref() == Some(&plan.key) && placed(plan, bar),
            _ => false,
        };
        if !current { return Ok(Some("status bar")); }
        Ok((self.tab_bar_snapshot(accepted.stamps.frame)? != shown.tab_bar).then_some("tab bar"))
    }
}

/// What a foreign pane's pictures ask of the evaluator (O1 §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneWork {
    /// No picture past the one the frame names.
    Settled,
    /// A newer picture the pane path has composed: no root frame.
    Composed,
    /// A newer picture not yet composed: a root frame must name it.
    Pending,
}

/// What PICTURE, the newest picture accepted, and COMPOSED, the newest the
/// render thread took for composition, ask of a frame that names NAMED.
fn pane_work(named: u64, picture: u64, composed: u64) -> PaneWork {
    if picture <= named { PaneWork::Settled } else if composed >= picture { PaneWork::Composed } else { PaneWork::Pending }
}

/// A visible child frame as a published frame shows it
/// (`Vm::shown_children`).
#[derive(Debug, PartialEq)]
struct ShownChild {
    id: u64,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    windows: Vec<u64>,
}

/// A window the published frame shows, found live (`Vm::shown_leaf`): its
/// id, the window, and its text area's width and height.
#[derive(Clone, Copy)]
struct Leaf {
    id: u64,
    window: Value,
    width: u32,
    height: u32,
}

impl Leaf {
    fn of(id: u64, (window, (_, _, width, height)): (Value, (u32, u32, u32, u32))) -> Self {
        Self { id, window, width, height }
    }
}

/// SHOWN's child frames as `Vm::shown_children` reads them.
fn children_of(shown: &FrameSnapshot) -> Vec<ShownChild> {
    shown.children.iter().map(|child| ShownChild {
        id: child.id, x: child.x, y: child.y, width: child.width, height: child.height,
        windows: child.windows.iter().map(|w| w.snapshot.id).collect(),
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A VM whose frame shows `*scratch*`-like text, its frame published
    /// and accepted: the frame on screen and what it stood for.
    fn published(setup: &str) -> (Vm, FrameSnapshot, DisplayAcceptance) {
        let mut vm = Vm::new();
        vm.set_frame_geometry(800, 400, 20, 10).unwrap();
        vm.eval_source(setup).unwrap();
        let (shown, accepted) = publish(&mut vm);
        (vm, shown, accepted)
    }

    fn publish(vm: &mut Vm) -> (FrameSnapshot, DisplayAcceptance) {
        let shown = vm.frame_snapshot(800, 400).unwrap();
        let accepted = vm.display_acceptance(Some(&shown)).unwrap();
        (shown, accepted)
    }

    /// What a check finds after FORM ran, as a timer would run it.
    fn after(vm: &mut Vm, shown: &FrameSnapshot, accepted: &DisplayAcceptance, form: &str) -> Staleness {
        vm.eval_source(form).unwrap();
        vm.display_stale(shown, accepted).unwrap()
    }

    /// Text in a window that paints no band: the bare VM has no status bar
    /// to hold its mode line.
    const TEXT: &str = "(insert \"(defun f ()\\n  (list 1 2))\\n\") (goto-char 3) (setq mode-line-format nil)";

    #[test]
    fn bookkeeping_leaves_the_frame_current() {
        // show-paren's overlay, made once and since deleted: the state its
        // idle timer leaves where point is at no paren.
        let (mut vm, shown, accepted) = published(&format!("{TEXT} (defvar trigger-ov (make-overlay 1 1)) (delete-overlay trigger-ov)"));
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Current, "nothing ran");
        for form in ["(defvar trigger-ticks 0) (setq trigger-ticks (1+ trigger-ticks))",
                     "(with-temp-buffer (insert \"hidden\") (put-text-property 1 3 'face 'bold))",
                     "(save-excursion (goto-char (point-max)))",
                     // show-paren with nothing to show: its overlay, in no buffer, deleted again.
                     "(delete-overlay trigger-ov)"] {
            assert_eq!(after(&mut vm, &shown, &accepted, form), Staleness::Current, "{form}");
        }
    }

    #[test]
    fn each_shown_change_makes_the_frame_stale() {
        let changes = [
            ("(insert \"x\")", "point"),
            ("(save-excursion (goto-char (point-max)) (insert \"x\"))", "window"),
            ("(put-text-property 1 3 'face 'bold)", "window"),
            ("(overlay-put (make-overlay 1 3) 'face 'bold)", "window"),
            ("(goto-char 5)", "point"),
            ("(clear-face-cache)", "invalidated"),
            ("(force-window-update (selected-window))", "invalidated"),
            ("(modify-frame-parameters nil '((romacs-pane-radius . 4)))", "frame parameters"),
            ("(narrow-to-region 1 4)", "window"),
        ];
        for (form, why) in changes {
            let (mut vm, shown, accepted) = published(TEXT);
            assert_eq!(after(&mut vm, &shown, &accepted, form), Staleness::Outdated(why), "{form}");
        }
    }

    #[test]
    fn an_overlay_removed_makes_the_frame_stale() {
        let (mut vm, shown, accepted) = published(&format!("{TEXT} (defvar trigger-ov (make-overlay 1 3)) (overlay-put trigger-ov 'face 'bold)"));
        assert_eq!(after(&mut vm, &shown, &accepted, "(delete-overlay trigger-ov)"), Staleness::Outdated("window"));
    }

    /// A window's mode line hidden (its `mode-line-format' parameter
    /// `none'): the band the frame painted must go.
    #[test]
    fn a_band_that_goes_makes_the_frame_stale() {
        let (mut vm, shown, accepted) = published(&format!("{TEXT} (setq mode-line-format \"KEPT MODE LINE\")"));
        assert_eq!(after(&mut vm, &shown, &accepted, "(set-window-parameter nil 'mode-line-format 'none)"), Staleness::Outdated("bands"));
    }

    /// The alternative families' alist edited in place, past its setter,
    /// as a snapshot would observe it.
    #[test]
    fn an_alternative_family_edited_in_place_makes_the_frame_stale() {
        let setup = format!("{TEXT} (defvar trigger-alist (internal-set-alternative-font-family-alist '((\"K2-Absent\" \"DejaVu Sans Mono\"))))");
        let (mut vm, shown, accepted) = published(&setup);
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Current);
        assert_eq!(after(&mut vm, &shown, &accepted, "(setcar (cdr (car trigger-alist)) 'Monospace)"), Staleness::Outdated("invalidated"));
    }

    /// A property change through an indirect sibling, a buffer no window
    /// shows, changes the text the shown base buffer paints.
    #[test]
    fn a_hidden_indirect_sibling_s_property_change_makes_the_frame_stale() {
        let (mut vm, shown, accepted) = published(&format!("{TEXT} (defvar trigger-sibling (make-indirect-buffer (current-buffer) \"sibling\"))"));
        let form = "(with-current-buffer trigger-sibling (put-text-property 1 3 'face 'bold))";
        assert_eq!(after(&mut vm, &shown, &accepted, form), Staleness::Outdated("window"));
    }

    /// A window that wants a band of its own, a mode line or a header line,
    /// is stale, and the trigger expands neither: the band expansions and
    /// the evaluator entries they make do not move.
    #[test]
    fn a_window_with_a_band_is_stale_and_none_is_expanded() {
        for band in ["(setq mode-line-format \"plain\")", "(setq header-line-format \"head\")"] {
            let (mut vm, shown, accepted) = published(&format!("{TEXT} {band}"));
            let expansions = |vm: &mut Vm| vm.eval_source("(cdr (assq 'band-expansions (romacs--display-stats)))").unwrap().display().unwrap();
            let (before, evals) = (expansions(&mut vm), vm.mode_line_evals());
            assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Outdated("bands"), "{band}");
            assert_eq!((expansions(&mut vm), vm.mode_line_evals()), (before, evals), "{band}: nothing expanded");
        }
    }

    use romacs_display_protocol::pane_revisions::PaneRevisions;
    use std::sync::Arc;

    /// The selected window shows a stand-in: its view's revisions, as
    /// ownership's view slot hands them out.
    fn foreign(vm: &mut Vm) -> Arc<PaneRevisions> {
        let revisions = Arc::new(PaneRevisions::default());
        let window = vm.selected_window_value().unwrap();
        vm.window_mut(window).unwrap().pane_revisions = Some(revisions.clone());
        revisions
    }

    #[test]
    fn the_pane_rule() {
        assert_eq!(pane_work(3, 3, 0), PaneWork::Settled);
        assert_eq!(pane_work(3, 4, 3), PaneWork::Pending);
        assert_eq!(pane_work(3, 4, 4), PaneWork::Composed);
        assert_eq!(pane_work(3, 4, 5), PaneWork::Composed);
    }

    /// An owner's newer picture for a shown window, bumped on the
    /// receiver's thread, is pending pane work, though nothing of the
    /// window's own changed; once the render thread composed it, no root
    /// frame is due.
    #[test]
    fn an_owner_picture_is_pending_until_composed() {
        let (mut vm, _, _) = published(TEXT);
        let revisions = foreign(&mut vm);
        let (shown, accepted) = publish(&mut vm);
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Current, "a fresh pair is not pending");
        let receiver = revisions.clone();
        std::thread::spawn(move || receiver.bump_picture(1)).join().unwrap();
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Outdated("owner picture"));
        revisions.mark_composed(1);
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Current, "composed: no root frame");
        revisions.bump_picture(2);
        // A frame built with it but found equal to the one on screen, so
        // never submitted, names nothing new.
        let built = vm.frame_snapshot(800, 400).unwrap();
        let unsubmitted = vm.display_acceptance(None).unwrap();
        assert_eq!(vm.display_stale(&built, &unsubmitted).unwrap(), Staleness::Outdated("owner picture"), "built, not submitted");
        let (shown, accepted) = publish(&mut vm);
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Current, "a frame submitted since names it");
    }

    /// A frame built with a window's newer picture but found equal, so not
    /// submitted; then a submitted frame that does not carry that window:
    /// no frame on screen names the picture, so it is still pending.
    #[test]
    fn a_submitted_frame_names_only_the_pictures_it_carries() {
        let split = "(let* ((a (selected-window)) (h (/ (window-pixel-height a) 2))) (set-window-new-pixel a h) (split-window-internal a h nil 0.5))";
        let (mut vm, _, _) = published(&format!("{TEXT} {split}"));
        let revisions = foreign(&mut vm);
        let (shown, _) = publish(&mut vm);
        revisions.bump_picture(1);
        let built = vm.frame_snapshot(800, 400).unwrap();
        vm.display_acceptance(None).unwrap();
        let selected = vm.selected_window_value().unwrap();
        let id = vm.window(selected).unwrap().id;
        let mut without = built.clone();
        without.windows.retain(|w| w.snapshot.id != id);
        assert_eq!(without.windows.len() + 1, built.windows.len(), "the frame submitted leaves the window out");
        let accepted = vm.display_acceptance(Some(&without)).unwrap();
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Outdated("owner picture"));
    }

    /// A new view or incarnation is a fresh pair at 0: it names nothing
    /// yet and asks nothing, whatever the old one held.
    #[test]
    fn a_fresh_pair_is_not_pending() {
        let (mut vm, _, _) = published(TEXT);
        foreign(&mut vm).bump_picture(5);
        let (shown, accepted) = publish(&mut vm);
        let fresh = foreign(&mut vm);
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Current);
        fresh.bump_picture(1);
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Outdated("owner picture"));
    }

    /// A window no frame shows requests nothing, whatever its owner
    /// publishes.
    #[test]
    fn a_hidden_window_s_picture_requests_nothing() {
        let (mut vm, shown, accepted) = published(TEXT);
        let selected = vm.selected_window_value().unwrap();
        let buffer = vm.window(selected).unwrap().buffer;
        let hidden = vm.allocate_window(buffer).unwrap();
        let revisions = Arc::new(PaneRevisions::default());
        vm.window_mut(hidden).unwrap().pane_revisions = Some(revisions.clone());
        revisions.bump_picture(3);
        assert_eq!(vm.display_stale(&shown, &accepted).unwrap(), Staleness::Current);
    }

    #[test]
    fn another_frame_size_makes_the_frame_stale() {
        let (mut vm, shown, accepted) = published(TEXT);
        vm.set_frame_geometry(900, 400, 20, 10).unwrap();
        assert!(vm.display_stale(&shown, &accepted).unwrap().is_stale());
    }

    /// Checks are counted, with their instructions where the thread counts
    /// them (`romacs--display-stats`), and run no redisplay cycle.
    #[test]
    fn a_check_is_counted_and_is_no_cycle() {
        let (mut vm, shown, accepted) = published(TEXT);
        vm.eval_source("(romacs--display-stats t)").unwrap();
        for form in ["nil", "(insert \"x\")"] {
            let bracket = vm.trigger_open();
            let stale = after(&mut vm, &shown, &accepted, form).is_stale();
            vm.trigger_close(bracket, stale);
        }
        let count = |vm: &mut Vm, name: &str| vm.eval_source(&format!("(cdr (assq '{name} (romacs--display-stats)))")).unwrap().display().unwrap();
        assert_eq!((count(&mut vm, "trigger-checks"), count(&mut vm, "trigger-stale"), count(&mut vm, "cycles")), ("2".into(), "1".into(), "0".into()));
        if crate::host::instruction_count().is_some() {
            assert!(count(&mut vm, "trigger-instructions").parse::<u64>().unwrap() > 0);
        }
    }
}
