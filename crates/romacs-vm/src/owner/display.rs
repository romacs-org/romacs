//! An owner's logical windows as its master's panes (ownership O1b §2.1,
//! §4.1): each master window an owner shows is the root window of a child
//! frame of the owner's own frame (`lisp/vm/owner-window.el`), laid out
//! here as a frame of its own and returned as a `PanePicture`.
//!
//! The host gives the engine (`OwnerDisplay`): romacs-vm holds no layout
//! engine. What an owner keeps beside its VM, the display and each
//! window's last layout, lives with the host's other hosts on its thread
//! (`crate::ui`), never in the VM.
use super::{Result, Vm};
use crate::layout_host::LayoutHost;
use romacs_display_protocol::font::FontCatalogGeneration;
use romacs_display_protocol::pane_picture::{PaneFrame, PaneHead, PanePicture};
use romacs_display_protocol::submission::SubmittedWindow;
use romacs_display_protocol::text_animation::TextAnimation;
use romacs_display_protocol::{Color, DisplayWindowId, FrameGlyphBuffer};
use romacs_layout_input::{FrameChrome, FrameSnapshot, PlacedWindowSnapshot, WindowEnd};
use std::collections::{HashMap, HashSet};

/// The host's half of an owner's display: the engine that lays its
/// windows out, which the layout host its queries measure with shares.
pub trait OwnerDisplay {
    /// Begin as owner slot SLOT: a fresh engine whose fonts intern from
    /// the slot's base, and the layout host that measures with it.
    ///
    /// # Errors
    ///
    /// The engine could not be made: no such slot, or no fonts.
    fn start(&mut self, slot: u8) -> std::result::Result<Box<dyn LayoutHost>, String>;
    /// FRAME laid out with CURSOR (the selected window's id and the
    /// cursor's colour), as a paint lays it out: its glyphs and its
    /// windows' ends.
    ///
    /// # Errors
    ///
    /// Not started, or the layout failed.
    fn lay_out(&mut self, frame: &FrameSnapshot, cursor: Option<(u64, Color)>)
        -> std::result::Result<(FrameGlyphBuffer, Vec<WindowEnd>), String>;
}

/// What a request named of a window: the master's window id and view,
/// the request's revision and the master's font catalog, and whether the
/// window is the master's selected one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaneRequest {
    pub(crate) master: u64,
    pub(crate) view: u64,
    pub(crate) revision: u64,
    pub(crate) catalog: u64,
    pub(crate) selected: bool,
}

/// An owner's display on its host's thread: the host's engine, each own
/// window's last picture (its ids the master's), and the last visual seq
/// a picture took.
pub(crate) struct OwnerUi {
    display: Box<dyn OwnerDisplay>,
    laid: HashMap<u64, PanePicture>,
    v: u64,
}

impl Vm {
    /// Attach (Some) or detach (None) the host's owner display. Until
    /// `owner-window--start` starts it, it lays nothing out.
    pub fn set_owner_display(&mut self, display: Option<Box<dyn OwnerDisplay>>) {
        let owner = display.map(|display| OwnerUi { display, laid: HashMap::new(), v: 0 });
        self.ui_set(owner, |ui| &mut ui.owner);
    }

    /// Whether the host attached an owner display: a worker with none has
    /// no window side.
    pub(crate) fn has_owner_display(&self) -> bool {
        self.ui_ref(|ui| ui.owner.is_some()).unwrap_or(false)
    }

    /// Lay out as owner slot SLOT from here: the display's engine, its
    /// fonts the slot's, measures this VM's windows, its frames show
    /// fringes as its master's display does, and no window is laid.
    pub(crate) fn owner_display_start(&mut self, slot: u8) -> Result<()> {
        let host = self.ui(|ui| ui.owner.as_mut().map(|owner| {
            owner.laid.clear();
            owner.display.start(slot)
        })).flatten().ok_or("no owner display")??;
        self.set_layout_host(Some(host));
        self.window_state.laid_for_display = true;
        Ok(())
    }

    /// The canonical character cell, LINE by COLUMN px: every window's
    /// lines and columns count in it. The selected top-level frame keeps
    /// its size, grown to two cells each way where it was smaller; a cell
    /// two of which do not fit a frame is refused.
    pub(crate) fn owner_set_cell(&mut self, line: u32, column: u32) -> Result<()> {
        let too_large = || format!("cell {line} by {column} px is too large");
        let (lines, columns) = (line.checked_mul(2).ok_or_else(too_large)?, column.checked_mul(2).ok_or_else(too_large)?);
        self.ensure_windows()?;
        let root = self.selected_top_frame();
        let native = self.top_geometry(root)?;
        self.set_top_frame_geometry(root, native.width.max(columns), native.height.max(lines), line, column)
    }

    /// What a redisplay runs before it snapshots, once for every window
    /// laid out after it; the layouts of windows no longer live forgotten.
    pub(crate) fn panes_prepare(&mut self) -> Result<()> {
        self.prepare_redisplay()?;
        let mut live = HashSet::new();
        for frame in self.frames() {
            live.extend(self.leaf_ids(self.frame(frame)?.root)?);
        }
        self.ui(|ui| ui.owner.as_mut().map(|owner| owner.laid.retain(|id, _| live.contains(id))));
        Ok(())
    }

    /// Lay live WINDOW out for REQUEST, as a frame of its own box, and keep
    /// it as the window's last picture: its glyphs, with the cursor of the
    /// request's selection and the request's font catalog, its text
    /// animations placed on its rows, and what it shows of its buffer
    /// under the request's view, every id the master's window's. Its
    /// window ends are applied, so the window's own queries answer from
    /// this layout.
    pub(crate) fn pane_lay_out(&mut self, window: crate::Value, request: PaneRequest) -> Result<()> {
        let id = self.window(window)?.id;
        let frame = self.pane_snapshot(id)?;
        // No window has id 0: an unselected pane's cursor is inactive.
        let cursor = Some((if request.selected { id } else { 0 }, cursor_colour(&frame)));
        let (mut glyphs, ends) = self.ui(|ui| ui.owner.as_mut().map(|owner| owner.display.lay_out(&frame, cursor)))
            .flatten().ok_or("no owner display")??;
        self.display_apply_layout(&ends)?;
        let own: Vec<_> = self.text_animations()?.into_iter().filter(|(_, animation)| animation.window == id).collect();
        let animations = placed_animations(&own, &ends);
        let painted = glyphs.window_cursors.iter().any(|c| c.active && c.window_id == DisplayWindowId::new(id.cast_signed()));
        let submitted = self.display_picture(&frame, id)
            .and_then(|picture| ends.iter().find_map(|end| picture.painted(end, painted)))
            .map(|shown| SubmittedWindow { view_generation: request.view, ..shown });
        glyphs.font_catalog_generation = FontCatalogGeneration::from_raw(request.catalog);
        // The incarnation and committed seq are the publication's (O1b-1b),
        // the visual seq the picture's when taken.
        let head = PaneHead { owner: 0, window: request.master, view: request.view, request: request.revision, v: 0, c: 0 };
        let picture = PanePicture::from_frame(head, PaneFrame { frame: glyphs, animations, submitted }, DisplayWindowId::new(id.cast_signed()));
        self.ui(|ui| ui.owner.as_mut().map(|owner| owner.laid.insert(id, picture)));
        Ok(())
    }

    /// Own window ID's last picture's bytes, under the next visual seq;
    /// None before any.
    pub(crate) fn pane_picture(&mut self, id: u64) -> Result<Option<Vec<u8>>> {
        self.ui(|ui| ui.owner.as_mut().and_then(|OwnerUi { laid, v, .. }| {
            let picture = laid.get_mut(&id)?;
            *v += 1;
            picture.head.v = *v;
            Some(picture.encode().map_err(|error| error.to_string()))
        })).flatten().transpose()
    }

    /// Live window ID as a one-window frame of its box: its text area
    /// placed in the box as a frame's snapshot places it, its cursor shown
    /// whatever a blink left it, no bars, no child frames and no chrome,
    /// which its master draws around the pane; its top-level frame's cursor
    /// colour and style. The top-level frame keeps the size it was last
    /// displayed at. The window's mode line runs Lisp: what is read after
    /// it is found again by id.
    fn pane_snapshot(&mut self, id: u64) -> Result<FrameSnapshot> {
        let window = self.live_window(id).ok_or("the pane's window is not live")?;
        let w = self.window(window)?;
        let (width, height, top) = (w.width, w.height, self.frame(self.top_frame(w.frame))?.id);
        let displayed = (self.display_state.width, self.display_state.height);
        let leaves = self.snapshot_leaves(vec![id]);
        (self.display_state.width, self.display_state.height) = displayed;
        let (x, y, mut snapshot) = leaves?.pop().ok_or("the pane's window has no lines")?;
        snapshot.cursor_off = false;
        let root = self.frame_of_id(top)?;
        let shown = self.pane_chrome(root)?;
        let chrome = FrameChrome { cursor: shown.cursor, style: shown.style, selected: shown.selected, minibuffer: shown.minibuffer, ..FrameChrome::default() };
        Ok(FrameSnapshot {
            font_selection: self.display_state.font_selection_order,
            font_alternatives: self.font_alternative_state()?,
            fontsets: self.fontset_state(),
            width,
            height,
            windows: vec![PlacedWindowSnapshot { x, y, snapshot }],
            background_alpha: self.frame_background_alpha(root)?,
            children: Vec::new(),
            tab_bar: None,
            chrome,
            status_bar: None,
            no_font: self.no_font_display(),
        })
    }
}

/// The cursor's colour: the frame's `cursor-color`, which SNAPSHOT carries.
#[must_use]
pub fn cursor_colour(snapshot: &FrameSnapshot) -> Color {
    let [r, g, b] = snapshot.chrome.cursor.unwrap_or_default();
    Color::from_pixel(u32::from_be_bytes([0, r, g, b]))
}

/// ANIMATIONS on the rows ENDS laid their windows out in: an anchored one
/// (a working thread) on the row its anchor is laid on, and dropped where
/// no laid row shows it; any other as it is.
#[must_use]
pub fn placed_animations(animations: &[(u64, TextAnimation)], ends: &[WindowEnd]) -> Vec<(u64, TextAnimation)> {
    animations.iter().filter_map(|(id, animation)| {
        if animation.anchor.is_none() { return Some((*id, animation.clone())); }
        let end = ends.iter().find(|end| end.id == animation.window)?;
        let rows: Vec<(usize, f32, f32)> = end.rows.iter().map(|row| (row.start, row.y, row.height)).collect();
        animation.placed(&rows, end.end, end.origin_y).map(|placed| (*id, placed))
    }).collect()
}
