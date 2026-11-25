//! Pane layers: owners' pane pictures over the primary frame (design O1b.md
//! rev 3, §6.2; typing-modes o1b3/DESIGN.md).
//!
//! Each render round takes the pane mailbox, then drains the command
//! channel, then ingests what it took against the root now current, then
//! draws (`process_commands_with_waker`). So whatever the ownership side
//! sent on the channel before it posted a picture (the root naming its
//! request, its image loads) is applied before the picture is checked.
//!
//! A picture is checked against the root's `ForeignPaneEntry` for its
//! window ([`admit`]); it is dropped, held until the root names its
//! request, or swapped in. Every picture is settled once, when the pane
//! path is done with it: taken for composition, superseded, or dropped.
//! A layer whose entry is gone, or names another (owner, view), is torn
//! down with its animations.
use super::page_scenes::PageScene;
use super::text_animations::TextAnimations;
use crate::pane_mailbox::{Posted, Taken};
use romacs_display_protocol::FrameGlyphBuffer;
use romacs_display_protocol::font::FontCatalogGeneration;
use romacs_display_protocol::foreign_pane::ForeignPaneEntry;
use romacs_display_protocol::pane_picture::{ImageIdSpace, PaneHead, PanePicture, owner_font_slot};
use romacs_display_protocol::pane_sink::PaneState;
use romacs_display_protocol::RootSurfaceRect;
use romacs_display_protocol::frame_time::FrameSample;
use romacs_renderer_wgpu::WgpuRenderer;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// What becomes of a picture checked against its window's entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Admit {
    Drop,
    /// Its request is newer than the root names: kept until the root does.
    Hold,
    Swap,
}

/// What becomes of PICTURE under the root whose entry for its window is
/// ENTRY and whose font catalog is CATALOG.
pub(crate) fn admit(entry: Option<&ForeignPaneEntry>, catalog: FontCatalogGeneration, picture: &PanePicture) -> Admit {
    let head = &picture.head;
    let Some(entry) = entry.filter(|entry| (entry.owner, entry.view) == (head.owner, head.view)) else {
        return Admit::Drop;
    };
    match head.request.cmp(&entry.request) {
        Ordering::Less => Admit::Drop,
        Ordering::Greater => Admit::Hold,
        Ordering::Equal if entry.size() != (picture.width, picture.height) || picture.catalog != catalog => Admit::Drop,
        Ordering::Equal => Admit::Swap,
    }
}

/// The picture a pane draws.
pub(crate) struct Shown {
    pub head: PaneHead,
    pub frame: Arc<FrameGlyphBuffer>,
    /// Its text animations: each swap names the set.
    pub animations: TextAnimations,
}

#[derive(Default)]
pub(crate) struct PaneLayer {
    pub shown: Option<Shown>,
    held: Option<Posted>,
}

impl PaneLayer {
    /// The (owner, view) the layer draws for.
    fn identity(&self) -> Option<(u64, u64)> {
        let head = self.shown.as_ref().map(|shown| shown.head).or_else(|| self.held.as_ref().map(|held| held.picture.head))?;
        Some((head.owner, head.view))
    }

    /// Swap PICTURE in, its animations halted unless the window is FOCUSED
    /// and not OCCLUDED.
    fn show(&mut self, mut picture: PanePicture, focused: bool, occluded: bool) {
        let animations = std::mem::take(&mut picture.animations);
        let head = picture.head;
        let frame = Arc::new(picture.into_frame());
        match self.shown.as_mut() {
            Some(shown) => {
                shown.head = head;
                shown.frame = frame;
                shown.animations.replace(animations);
            }
            None => {
                let mut fresh = TextAnimations::default();
                fresh.focus(focused);
                fresh.occlude(occluded);
                fresh.replace(animations);
                self.shown = Some(Shown { head, frame, animations: fresh });
            }
        }
    }

    /// The layer goes: its held picture settled through SETTLE, its
    /// animations' scenes into SCENES for the renderer to free.
    fn retire(self, settle: &mut dyn FnMut(u64, PaneHead), scenes: &mut Vec<PageScene>) {
        if let Some(held) = self.held {
            settle(held.owner, held.picture.head);
        }
        if let Some(mut shown) = self.shown {
            shown.animations.replace(Vec::new());
            scenes.extend(shown.animations.take_retired());
        }
    }
}

/// One pane as this frame drew it, for the over-pane hook.
#[cfg_attr(not(test), expect(dead_code, reason = "read by O1c-4's over-pane hook"))]
pub(crate) struct DrawnPane {
    pub owner: u64,
    pub window: u64,
    /// The entry's rect, in the root's logical px: the hook draws within it.
    pub rect: RootSurfaceRect,
    /// The head of the picture drawn; None while the pane waits for one.
    pub picture: Option<PaneHead>,
    /// The owner's last `PaneSink::state`; None before one.
    pub state: Option<PaneState>,
}

/// What draws over each pane after its picture, every frame, whether or
/// not it has one: ownership's veil (O1c-4). It keeps its own state between
/// calls. Draws over PANE into VIEW for the frame SAMPLE dates; whether it
/// still animates, asking for another frame.
pub(crate) type OverPane = Box<dyn FnMut(&mut WgpuRenderer, &wgpu::TextureView, &DrawnPane, FrameSample) -> bool + Send>;

/// The over-pane hook, the render app's: installed once for the app's life
/// (`over_pane`), and shared with each window's pane pass, so a window
/// hidden and reopened draws with it too.
pub(crate) type SharedOverPane = Arc<Mutex<OverPane>>;

/// The hook the app starts with: it draws nothing. Ownership's veil
/// (O1c-4) is installed here.
pub(crate) fn over_pane() -> SharedOverPane {
    let none: OverPane = Box::new(|_, _, _, _| false);
    Arc::new(Mutex::new(none))
}

/// Owners' states (`PaneSink::state`): the render app's, not a window's, so
/// they outlive a hidden window and take updates while none exists. Each
/// owner's last state is kept until it is dead and nothing of it is drawn.
/// A window's pane layers mirror them for the over-pane hook.
#[derive(Clone, Default)]
pub(crate) struct PaneStates {
    map: HashMap<u64, PaneState>,
    /// Bumped on each change: a mirror of another revision is copied anew.
    revision: u64,
    /// The owners whose death the command channel delivered.
    dead: HashSet<u64>,
}

impl PaneStates {
    /// Take POSTED, then forget each dead owner DRAWN does not show:
    /// whether a posted state is of an owner DRAWN shows.
    pub(crate) fn update(&mut self, posted: Vec<(u64, PaneState)>, drawn: impl Fn(u64) -> bool) -> bool {
        let dead = |state: &PaneState| state.kind() == PaneState::DEAD;
        if posted.is_empty() && !self.map.values().any(dead) {
            return false;
        }
        let shown = posted.iter().any(|&(owner, _)| drawn(owner));
        let before = (posted.is_empty(), self.map.len());
        self.map.extend(posted);
        self.map.retain(|&owner, state| !dead(state) || drawn(owner));
        if !before.0 || self.map.len() != before.1 {
            self.revision += 1;
        }
        shown
    }

    /// OWNER's last state.
    pub(crate) fn get(&self, owner: u64) -> Option<PaneState> {
        self.map.get(&owner).copied()
    }
}

/// The primary frame's pane layers, by master window id.
#[derive(Default)]
pub(crate) struct PaneLayers {
    layers: BTreeMap<u64, PaneLayer>,
    /// The app's owner states, as last mirrored.
    states: PaneStates,
    /// Scenes of torn-down layers' animations, for the renderer to free.
    retired: Vec<PageScene>,
    /// The app's over-pane hook, as last shared: none before a round.
    over_pane: Option<SharedOverPane>,
    /// The window's focus and occlusion, which halt every pane's motion.
    unfocused: bool,
    occluded: bool,
}

impl PaneLayers {
    /// The layer of master window WINDOW.
    #[cfg(test)]
    pub(crate) fn layer(&self, window: u64) -> Option<&PaneLayer> {
        self.layers.get(&window)
    }

    /// The picture master window WINDOW draws.
    pub(crate) fn shown_mut(&mut self, window: u64) -> Option<&mut Shown> {
        self.layers.get_mut(&window).and_then(|layer| layer.shown.as_mut())
    }

    /// OWNER's last posted state.
    pub(crate) fn state(&self, owner: u64) -> Option<PaneState> {
        self.states.get(owner)
    }

    /// Mirror STATES, the app's, when they changed since the last mirror.
    pub(crate) fn mirror(&mut self, states: &PaneStates) {
        if self.states.revision != states.revision {
            self.states = states.clone();
        }
    }

    /// Share HOOK, the app's over-pane hook, with this window's pane pass.
    pub(crate) fn share(&mut self, hook: &SharedOverPane) {
        if !self.over_pane.as_ref().is_some_and(|shared| Arc::ptr_eq(shared, hook)) {
            self.over_pane = Some(Arc::clone(hook));
        }
    }

    /// The over-pane hook this window's pane pass calls.
    pub(crate) fn over_pane(&self) -> Option<&SharedOverPane> {
        self.over_pane.as_ref()
    }

    /// The owners the layers draw for.
    fn owners(&self) -> impl Iterator<Item = u64> + '_ {
        self.layers.values().filter_map(|layer| layer.identity().map(|(owner, _)| owner))
    }

    /// Every drawn pane's picture and animations.
    pub(crate) fn shown_all_mut(&mut self) -> impl Iterator<Item = &mut Shown> {
        self.layers.values_mut().filter_map(|layer| layer.shown.as_mut())
    }

    pub(crate) fn focus(&mut self, focused: bool) {
        self.unfocused = !focused;
        for shown in self.shown_all_mut() {
            shown.animations.focus(focused);
        }
    }

    pub(crate) fn occlude(&mut self, occluded: bool) {
        self.occluded = occluded;
        for shown in self.shown_all_mut() {
            shown.animations.occlude(occluded);
        }
    }

    /// Advance every pane's animations due at NOW: whether one did, and
    /// when the next is due.
    pub(crate) fn tick(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let (mut advanced, mut due) = (false, None::<Instant>);
        for shown in self.shown_all_mut() {
            advanced |= shown.animations.tick(now);
            due = match (due, shown.animations.deadline(now)) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        (advanced, due)
    }

    /// Scenes to free: torn-down layers' and ended animations'.
    pub(crate) fn take_retired(&mut self) -> Vec<PageScene> {
        let mut retired = std::mem::take(&mut self.retired);
        for shown in self.shown_all_mut() {
            retired.extend(shown.animations.take_retired());
        }
        retired
    }

    /// Tear down every layer of an owner OWNS names (all, as the window
    /// goes; a dead owner's): each held picture settled through SETTLE;
    /// every scene the torn-down panes held, for the renderer to free.
    pub(crate) fn clear(&mut self, owns: impl Fn(u64) -> bool, settle: &mut dyn FnMut(u64, PaneHead)) -> Vec<PageScene> {
        let mut scenes = std::mem::take(&mut self.retired);
        let gone: Vec<u64> = self.layers.iter()
            .filter(|(_, layer)| layer.identity().is_some_and(|(owner, _)| owns(owner))).map(|(&window, _)| window).collect();
        for window in gone {
            if let Some(layer) = self.layers.remove(&window) {
                layer.retire(settle, &mut scenes);
            }
        }
        scenes
    }

    /// Take PICTURES against ROOT, the frame the round draws: settle what
    /// the pane path is done with through SETTLE. Whether what a pane
    /// draws changed.
    pub(crate) fn ingest(&mut self, root: Option<&FrameGlyphBuffer>, pictures: Vec<Posted>, settle: &mut dyn FnMut(u64, PaneHead)) -> bool {
        if pictures.is_empty() && self.layers.is_empty() {
            return false;
        }
        let entries = root.map_or(&[][..], |root| &root.foreign_panes[..]);
        let catalog = root.map(|root| root.font_catalog_generation);
        let mut changed = self.tear_down(entries, settle);
        let held: Vec<Posted> = self.layers.values_mut().filter_map(|layer| layer.held.take()).collect();
        for posted in held.into_iter().chain(pictures) {
            changed |= self.offer(entries, catalog, posted, settle);
        }
        self.layers.retain(|_, layer| layer.shown.is_some() || layer.held.is_some());
        changed
    }

    /// Drop every layer ENTRIES no longer names for its (owner, view);
    /// whether one went.
    fn tear_down(&mut self, entries: &[ForeignPaneEntry], settle: &mut dyn FnMut(u64, PaneHead)) -> bool {
        let named = |window: u64, layer: &PaneLayer| {
            entries.iter().any(|entry| entry.window == window && layer.identity() == Some((entry.owner, entry.view)))
        };
        let gone: Vec<u64> = self.layers.iter().filter(|(window, layer)| !named(**window, layer)).map(|(window, _)| *window).collect();
        for window in &gone {
            if let Some(layer) = self.layers.remove(window) {
                layer.retire(settle, &mut self.retired);
            }
        }
        !gone.is_empty()
    }

    /// Check POSTED against its window's entry; whether it swapped in.
    fn offer(&mut self, entries: &[ForeignPaneEntry], catalog: Option<FontCatalogGeneration>, posted: Posted, settle: &mut dyn FnMut(u64, PaneHead)) -> bool {
        let head = posted.picture.head;
        let entry = entries.iter().find(|entry| entry.window == head.window);
        let verdict = catalog.map_or(Admit::Drop, |catalog| admit(entry, catalog, &posted.picture));
        match verdict {
            Admit::Drop => {
                settle(posted.owner, head);
                false
            }
            Admit::Hold => {
                if let Some(old) = self.layers.entry(head.window).or_default().held.replace(posted) {
                    settle(old.owner, old.picture.head);
                }
                false
            }
            Admit::Swap => {
                posted.revisions.mark_composed(head.v);
                settle(posted.owner, head);
                let (focused, occluded) = (!self.unfocused, self.occluded);
                self.layers.entry(head.window).or_default().show(posted.picture, focused, occluded);
                true
            }
        }
    }
}

impl super::RenderApp {
    /// Tear down the primary window's panes of every owner OWNS names (all,
    /// before the window's state goes; a dead owner's): every held picture
    /// settled, and every scene they hold freed while the renderer lives.
    pub(super) fn tear_down_panes(&mut self, owns: impl Fn(u64) -> bool) {
        let mailbox = &self.panes;
        let Some(window) = self.frame_windows.primary_window_mut() else { return };
        let scenes = window.render.compositor.pane_layers.clear(owns, &mut |owner, head| mailbox.settle(owner, head));
        if let Some(renderer) = self.renderer.as_mut() {
            for scene in scenes {
                scene.free(renderer);
            }
        }
    }

    /// The primary window's state, taken: its panes torn down first.
    pub(super) fn take_primary_window(&mut self) -> Option<super::frame_windows::GuiFrameWindowState> {
        self.tear_down_panes(|_| true);
        self.frame_windows.take_primary_window()
    }

    /// As the render thread exits: close the mailbox, settle what it still
    /// held, and tear the panes down.
    pub(super) fn close_panes(&mut self) {
        let Taken { pictures, superseded, .. } = self.panes.close();
        for posted in pictures {
            self.panes.settle(posted.owner, posted.picture.head);
        }
        for (owner, head) in superseded {
            self.panes.settle(owner, head);
        }
        self.tear_down_panes(|_| true);
    }

    /// Contract A4, as the drain reaches OWNER's death (`OwnerDied`), in
    /// order with the uploads: the owner is dead to the render thread, its
    /// panes are torn down (held pictures settled, scenes freed) so no
    /// later frame draws its picture, and SLOT's fonts and images are
    /// forgotten. The boundary: no picture of a dead owner is composed after
    /// its `OwnerDied` drains; rasterized history a transition fades from
    /// (ring textures, transition snapshots) is not recomposed, and is out
    /// of scope.
    pub(super) fn bury(&mut self, owner: u64, slot: u8) {
        self.pane_states.dead.insert(owner);
        let shown = self.frame_windows.primary_window().is_some_and(|window| window.render.compositor.pane_layers.owners().any(|drawn| drawn == owner));
        self.tear_down_panes(|drawn| drawn == owner);
        self.forget_slot(slot);
        if shown && let Some(window) = self.frame_windows.primary_window_mut() {
            window.render.compositor.current_scene_generation = super::frame_state::next_scene_generation();
            window.render.mark_dirty();
            window.request_redraw();
        }
    }

    /// Forget owner slot SLOT's fonts (the primary window's atlas) and
    /// images (the renderer's cache).
    fn forget_slot(&mut self, slot: u8) {
        let atlas = self.frame_windows.primary_window_mut().and_then(|window| window.render.compositor.glyph_atlas.as_mut());
        if let Some(atlas) = atlas {
            atlas.forget_fonts(|font| owner_font_slot(font.0) == Some(slot));
        }
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.retire_images_where(|image| ImageIdSpace::Owner(slot).contains(image));
        }
    }

    /// Ingest TAKEN: its pictures into the primary window's pane layers,
    /// against its current root (with no primary window, each is dropped),
    /// its states into the app's, mirrored into the layers.
    pub(super) fn ingest_panes(&mut self, taken: Taken) {
        let Taken { pictures, superseded, states } = taken;
        for (owner, head) in superseded {
            self.panes.settle(owner, head);
        }
        // A picture the round took before its owner's death drained.
        let (gone, pictures): (Vec<Posted>, Vec<Posted>) = pictures.into_iter().partition(|posted| self.pane_states.dead.contains(&posted.owner));
        for posted in gone {
            self.panes.settle(posted.owner, posted.picture.head);
        }
        let mailbox = &self.panes;
        let mut settle = |owner, head| mailbox.settle(owner, head);
        let Some(window) = self.frame_windows.primary_window_mut() else {
            for posted in pictures {
                settle(posted.owner, posted.picture.head);
            }
            self.pane_states.update(states, |_| false);
            return;
        };
        let compositor = &mut window.render.compositor;
        let root = compositor.current_frame.as_deref();
        let mut changed = compositor.pane_layers.ingest(root, pictures, &mut settle);
        // A state, DEAD too, changes the frame only for an owner whose pane
        // it shows: a dead owner's torn-down layer was its root entry's.
        let layers = &compositor.pane_layers;
        let drawn = |owner| root.is_some_and(|root| root.foreign_panes.iter().any(|entry| entry.owner == owner)) || layers.owners().any(|shown| shown == owner);
        changed |= self.pane_states.update(states, drawn);
        compositor.pane_layers.mirror(&self.pane_states);
        compositor.pane_layers.share(&self.over_pane);
        if changed {
            compositor.current_scene_generation = super::frame_state::next_scene_generation();
            window.render.mark_dirty();
            window.request_redraw();
        }
    }
}

#[cfg(test)]
#[path = "tests/pane_layers_test.rs"]
mod tests;
