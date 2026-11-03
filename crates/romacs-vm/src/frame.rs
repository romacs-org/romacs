//! Lisp identity and ownership for the initial display device and its
//! child frames. No TTY is claimed: the initial terminal has no native TTY
//! top frame. The host frame is the one the display shows; every other
//! frame is a child frame (`parent-frame'), painted over its parent and
//! never resizing its windows (DIVERGENCES.md VM2-CHILD-FRAME).
use crate::{data::Object, *};
pub(super) const SELECTED_FRAME: &str = "#:vm2-frame";
/// The frame the host displays: the initial frame.
const HOST_FRAME: &str = "#:vm2-host-frame";
/// The fringe width a frame parameter of nil or a non-integer gives
/// (frame.c gui_set_left_fringe).
pub(crate) const DEFAULT_FRINGE: u32 = 8;
/// Live frames, newest first: GNU's `Vframe_list' (frame.c:1438).
const FRAME_LIST: &str = "#:vm2-frame-list";
#[derive(Clone)]
pub(super) struct Frame {
    pub root: Value,
    pub selected: Value,
    pub minibuffer: Value,
    pub terminal: Value,
    pub parameters: Vec<(Value, Value)>,
    pub live: bool,
    /// The display's identity for this frame: 1 for the host frame.
    pub id: u64,
    /// The parent frame, nil for the host frame.
    pub parent: Value,
    /// A child frame's outer position in its parent's native area and its
    /// native size, in pixels. The host frame's size is the window state's.
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// GNU `FRAME_INTERNAL_BORDER_WIDTH': the border a child frame draws
    /// (`child-frame-border-width'), or the host frame's
    /// `internal-border-width' around its text area (frame.h:1796).
    pub border: u32,
    pub visible: bool,
    /// Stacking among siblings: a higher one is drawn above a lower one.
    pub z: i64,
    /// GNU `f->focus_frame' (`redirect-frame-focus'), nil when unredirected.
    pub focus: Value,
    /// What the `alpha-background' setter last applied (frame.c:6098).
    pub alpha_background: f32,
    /// Whether this child frame hosts the minibuffer window
    /// (`romacs--host-minibuffer'): the window sits on the top edge of its
    /// text area and its root window takes the rest.
    pub hosts_minibuffer: bool,
    /// GNU `FRAME_LEFT_FRINGE_WIDTH' and `FRAME_RIGHT_FRINGE_WIDTH': the
    /// pixels a window without its own fringe widths gives each fringe on
    /// a window-system frame, 8 until `left-fringe' or `right-fringe' sets
    /// them (frame.c gui_set_left_fringe).
    pub fringes: [u32; 2],
    /// GNU `FRAME_TAB_BAR_LINES': the lines of the tab bar above the root
    /// window, as `tab-bar-lines' set them (pgtkfns.c pgtk_set_tab_bar_lines).
    pub tab_bar_lines: u32,
    /// GNU `FRAME_RIGHT_DIVIDER_WIDTH' and `FRAME_BOTTOM_DIVIDER_WIDTH':
    /// the pixels `right-divider-width' and `bottom-divider-width' give
    /// the dividers between its windows (frame.c gui_set_right_divider_width).
    pub dividers: [u32; 2],
    /// The lines of the status bar between the root window and the
    /// mini-window, as `romacs-status-bar-lines' set them (0 or 1), like
    /// the tab bar's (VM2-STATUS-BAR).
    pub status_bar_lines: u32,
    /// Whether the status bar is at the top, under the tab bar, as
    /// `romacs-status-bar-position' `top' puts it; else above the
    /// mini-window (VM2-STATUS-BAR).
    pub status_bar_top: bool,
}
romacs_snapshot::snapshot_struct!(Frame { root, selected, minibuffer, terminal, parameters, live, id, parent, left, top, width, height, border, visible, z, focus, alpha_background, hosts_minibuffer, fringes, tab_bar_lines, dividers, status_bar_lines, status_bar_top });

impl Frame {
    pub fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) {
        for v in [&mut self.root, &mut self.selected, &mut self.minibuffer,
                  &mut self.terminal, &mut self.parent, &mut self.focus] { f(v); }
        for (k, v) in &mut self.parameters { f(k); f(v); }
    }
    pub fn storage_bytes(&self) -> usize {
        size_of::<Self>() + self.parameters.capacity() * size_of::<(Value, Value)>()
    }
}
/// A child frame's placement and size request, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChildGeometry {
    pub left: i32,
    pub top: i32,
    /// The text area's size.
    pub width: u32,
    pub height: u32,
    pub border: u32,
}
#[derive(Clone)]
pub(super) struct Terminal {
    pub frame: Value,
    pub live: bool,
    /// Coding system names of TERMINAL_TERMINAL_CODING and
    /// TERMINAL_KEYBOARD_CODING (terminal.c create_terminal defaults).
    pub terminal_coding: Value,
    pub keyboard_coding: Value,
    /// The parameter alist (terminal.c `param_alist').
    pub params: Value,
}
romacs_snapshot::snapshot_struct!(Terminal { frame, live, terminal_coding, keyboard_coding, params });
impl Terminal {
    pub fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) {
        f(&mut self.frame); f(&mut self.terminal_coding); f(&mut self.keyboard_coding); f(&mut self.params);
    }
    pub fn storage_bytes(&self) -> usize { size_of::<Self>() }
}
impl Vm {
    pub(super) fn frame_value_peek(&self) -> Value {
        self.names.get(SELECTED_FRAME).map_or(Value::UNBOUND, |&i| self.slots[i].value)
    }
    /// The frame the host displays, unbound before `ensure_windows'.
    pub(super) fn host_frame_peek(&self) -> Value {
        self.names.get(HOST_FRAME).map_or(Value::UNBOUND, |&i| self.slots[i].value)
    }
    pub(super) fn selected_frame_value(&mut self) -> Result<Value> {
        self.ensure_windows()?;
        Ok(self.frame_value_peek())
    }
    pub(super) fn frame(&self, value: Value) -> Result<&Frame> {
        match self.object_if(value) {
            Some(Object::Frame(f)) => Ok(f),
            _ => Err("frame required".into()),
        }
    }
    /// The host frame's background opacity, as painting presents it.
    pub(crate) fn frame_background_alpha(&mut self) -> Result<f32> {
        self.ensure_windows()?;
        Ok(self.frame(self.host_frame_peek())?.alpha_background)
    }
    pub(super) fn frame_mut(&mut self, value: Value) -> Result<&mut Frame> {
        match self.objects.get_mut(value.index(0)?.saturating_sub(1)) {
            Some(Object::Frame(f)) => Ok(f),
            _ => Err("frame required".into()),
        }
    }
    /// Live frames, newest first.
    pub(super) fn frames(&self) -> Vec<Value> {
        let mut out = vec![];
        let mut tail = self.names.get(FRAME_LIST).map_or(Value::NIL, |&i| self.slots[i].value);
        while let Ok((frame, rest)) = self.cons_pair(tail) {
            out.push(frame);
            tail = rest;
        }
        out
    }
    fn set_frames(&mut self, frames: &[Value]) {
        let list = self.list_value(frames);
        let slot = self.slot(FRAME_LIST);
        self.slots[slot].value = list;
    }
    /// FRAME's parent chain, nearest first.
    pub(super) fn frame_ancestors(&self, mut frame: Value) -> Vec<Value> {
        let mut out = vec![];
        while let Ok(f) = self.frame(frame) {
            if f.parent == Value::NIL { break; }
            out.push(f.parent);
            frame = f.parent;
        }
        out
    }
    pub(super) fn allocate_initial_frame(&mut self, root: Value, minibuffer: Value) -> Result<()> {
        let (undecided, no_conversion) = (self.intern("undecided"), self.intern("no-conversion"));
        let terminal = self.allocate_object(Object::Terminal(Box::new(Terminal {
            frame: Value::NIL, live: true, terminal_coding: undecided, keyboard_coding: no_conversion, params: Value::NIL,
        })));
        // make_initial_frame (frame.c:1441) names the initial frame F1.
        let name = (self.intern("name"), self.make_lisp_string(b"F1".to_vec(), false));
        let frame = self.allocate_object(Object::Frame(Box::new(Frame {
            root, selected: root, minibuffer, terminal,
            parameters: vec![name], live: true, id: 1, parent: Value::NIL,
            left: 0, top: 0, width: 0, height: 0, border: 0, visible: true, z: 0, focus: Value::NIL,
            alpha_background: 1.0, hosts_minibuffer: false, fringes: [DEFAULT_FRINGE; 2], tab_bar_lines: 0, dividers: [0; 2], status_bar_lines: 0, status_bar_top: false,
        })));
        let Object::Terminal(t) = self.object_mut(terminal)? else { unreachable!() };
        t.frame = frame;
        for name in [SELECTED_FRAME, HOST_FRAME, "#:vm2-last-nonminibuffer-frame"] {
            let slot = self.slot(name);
            self.slots[slot].value = frame;
        }
        self.set_frames(&[frame]);
        self.window_mut(root)?.frame = frame;
        self.window_mut(minibuffer)?.frame = frame;
        Ok(())
    }
    /// A new invisible child frame of PARENT showing BUFFER in one window,
    /// sharing PARENT's minibuffer window (pgtkfns.c x-create-frame with a
    /// `parent-frame'); PARAMETERS are stored as given, newest first.
    pub(super) fn allocate_child_frame(&mut self, parent: Value, buffer: Value, geometry: ChildGeometry,
                                       parameters: Vec<(Value, Value)>) -> Result<Value> {
        self.ensure_windows()?;
        let parent_frame = self.frame(parent)?;
        let (minibuffer, terminal) = (parent_frame.minibuffer, parent_frame.terminal);
        let root = self.allocate_window(buffer)?;
        self.window_state.next_frame += 1;
        let id = self.window_state.next_frame;
        let frame = self.allocate_object(Object::Frame(Box::new(Frame {
            root, selected: root, minibuffer, terminal, parameters, live: true, id, parent,
            left: 0, top: 0, width: 0, height: 0, border: 0, visible: false, z: id as i64, focus: Value::NIL, alpha_background: 1.0,
            hosts_minibuffer: false, fringes: [DEFAULT_FRINGE; 2], tab_bar_lines: 0, dividers: [0; 2], status_bar_lines: 0, status_bar_top: false,
        })));
        self.window_mut(root)?.frame = frame;
        let mut frames = self.frames();
        frames.insert(0, frame);
        self.set_frames(&frames);
        self.set_child_geometry(frame, geometry)?;
        Ok(frame)
    }
    /// Place and size child FRAME, then lay its windows out in the text
    /// area inside its border, which its fringes widen as they do in GNU
    /// (FRAME_TEXT_TO_PIXEL_WIDTH). Its parent's windows do not change. A frame
    /// hosting the minibuffer window gives it its lines at its edge, placed
    /// in the host frame, whose window it stays; the root window takes the
    /// rest, possibly nothing.
    pub(super) fn set_child_geometry(&mut self, frame: Value, geometry: ChildGeometry) -> Result<()> {
        let (line, column) = (self.window_state.line.max(1), self.window_state.column.max(1));
        let ChildGeometry { left, top, width, height, border } = geometry;
        let hosts = self.frame(frame)?.hosts_minibuffer;
        let mini_height = if hosts { self.window_state.mini_lines.max(1) * line } else { 0 };
        // A child frame keeps at least one line of two columns.
        let [left_fringe, right_fringe] = self.frame_fringes(frame)?;
        let (width, height) = (width.max(2 * column) + left_fringe + right_fringe, height.max(line).max(mini_height));
        let root = {
            let f = self.frame_mut(frame)?;
            (f.left, f.top, f.border) = (left, top, border);
            (f.width, f.height) = (width + 2 * border, height + 2 * border);
            f.root
        };
        let mut geometry = vec![];
        let rest = height - mini_height;
        let (root_y, mini_y) = (border + mini_height, border);
        if rest >= line || !hosts {
            self.plan_window_geometry(root, border, root_y, width, rest, (0, 0), line, column, &mut geometry)?;
        } else if self.window(root)?.live() {
            geometry.push((root, border, root_y, width, 0));
        } else {
            return Err("a split child frame cannot give the minibuffer all its lines".into());
        }
        if hosts {
            let (x, y) = self.frame_origin(frame)?;
            let to = |n: i32, by: u32| u32::try_from(i64::from(n) + i64::from(by)).unwrap_or(0);
            geometry.push((self.mini_window(), to(x, border), to(y, mini_y), width, mini_height));
        }
        for (v, x, y, w, h) in geometry {
            let window = self.window_mut(v)?;
            if (window.x, window.y, window.width, window.height) != (x, y, w, h) {
                window.end_valid = false;
            }
            (window.x, window.y, window.width, window.height) = (x, y, w, h);
        }
        self.display_state.version += 1;
        Ok(())
    }
    /// FRAME's fringe widths, none on a frame without a window system.
    pub(crate) fn frame_fringes(&self, frame: Value) -> Result<[u32; 2]> {
        Ok(if self.window_system_open() { self.frame(frame)?.fringes } else { [0, 0] })
    }
    /// A child frame's geometry request as it stands.
    pub(super) fn child_geometry(&self, frame: Value) -> Result<ChildGeometry> {
        let f = self.frame(frame)?;
        let [left_fringe, right_fringe] = self.frame_fringes(frame)?;
        Ok(ChildGeometry { left: f.left, top: f.top, width: f.width - 2 * f.border - left_fringe - right_fringe,
                           height: f.height - 2 * f.border, border: f.border })
    }
    /// FRAME's native size in pixels.
    pub(super) fn frame_native(&self, frame: Value) -> Result<(u32, u32)> {
        let f = self.frame(frame)?;
        Ok(if f.parent == Value::NIL { (self.window_state.width, self.window_state.height) } else { (f.width, f.height) })
    }
    /// Delete child FRAME and its descendants (frame.c delete_frame): their
    /// windows die, and a deleted selected frame passes the selection to
    /// its nearest live ancestor. The host frame is never deleted here; the
    /// minibuffer window a deleted frame hosted returns to its line.
    pub(super) fn delete_child_frame(&mut self, frame: Value) -> Result<()> {
        if !self.frame(frame)?.live { return Ok(()); }
        if self.frame(frame)?.hosts_minibuffer { self.host_minibuffer(None)?; }
        if self.frame(frame)?.parent == Value::NIL {
            return Err("Attempt to delete the sole visible or iconified frame".into());
        }
        let children: Vec<Value> = self.frames().into_iter().filter(|&f| self.frame(f).is_ok_and(|f| f.parent == frame)).collect();
        for child in children {
            self.delete_child_frame(child)?;
        }
        let selected = self.frame_value_peek();
        if selected == frame {
            let parent = self.frame(frame)?.parent;
            self.select_frame_value(parent, false)?;
        }
        let root = self.frame(frame)?.root;
        self.retire_window(root)?;
        let f = self.frame_mut(frame)?;
        f.live = false;
        f.visible = false;
        f.root = Value::NIL;
        f.selected = Value::NIL;
        let frames: Vec<Value> = self.frames().into_iter().filter(|&f| f != frame).collect();
        self.set_frames(&frames);
        self.display_state.version += 1;
        Ok(())
    }
    /// Select FRAME and its selected window (frame.c do_switch_frame).
    pub(super) fn select_frame_value(&mut self, frame: Value, record: bool) -> Result<()> {
        let window = self.frame(frame)?.selected;
        self.select_window_value(window, record)?;
        Ok(())
    }
    /// Whether FRAME shows: it and its ancestors are live and visible.
    pub(super) fn frame_shown(&self, frame: Value) -> bool {
        std::iter::once(frame).chain(self.frame_ancestors(frame)).all(|f| self.frame(f).is_ok_and(|f| f.visible && f.live))
    }
    /// FRAME's outer position in the host frame.
    pub(super) fn frame_origin(&self, frame: Value) -> Result<(i32, i32)> {
        std::iter::once(frame).chain(self.frame_ancestors(frame))
            .try_fold((0, 0), |(x, y), f| self.frame(f).map(|f| (x + f.left, y + f.top)))
    }
    /// Live child frames in stacking order, lowest first: a frame's
    /// children above it, siblings by creation unless raised or lowered.
    /// GNU leaves stacking to the window manager; `z-group' is not read.
    pub(crate) fn child_frames_bottom_up(&self) -> Vec<Value> {
        let host = self.host_frame_peek();
        let mut out: Vec<Value> = self.frames().into_iter()
            .filter(|&f| f != host && self.frame(f).is_ok_and(|f| f.live)).collect();
        let key = |vm: &Self, f: Value| (vm.frame_ancestors(f).len(), vm.frame(f).map_or(0, |f| f.z));
        out.sort_by_key(|&f| key(self, f));
        out
    }
    /// The live child frame hosting the minibuffer window.
    pub(crate) fn minibuffer_host(&self) -> Option<Value> {
        self.frames().into_iter().find(|&f| self.frame(f).is_ok_and(|x| x.live && x.hosts_minibuffer))
    }
    /// Let child frame HOST show the minibuffer window on its top edge, or
    /// with None give the window back its line at the host frame's bottom.
    /// Either way the root frame's windows are laid out again: a hosted
    /// minibuffer takes none of their lines.
    pub(super) fn host_minibuffer(&mut self, host: Option<Value>) -> Result<()> {
        let old = self.minibuffer_host();
        if old == host { return Ok(()); }
        for f in self.frames() { self.frame_mut(f)?.hosts_minibuffer = false; }
        if let Some(frame) = host { self.frame_mut(frame)?.hosts_minibuffer = true; }
        for frame in old.into_iter().chain(host) {
            let geometry = self.child_geometry(frame)?;
            self.set_child_geometry(frame, geometry)?;
        }
        let state = &self.window_state;
        let (width, height, line, column) = (state.width, state.height, state.line, state.column);
        self.set_frame_geometry(width, height, line, column)?;
        self.display_state.version += 1;
        Ok(())
    }
    /// Keep a hosting FRAME's top in place as the minibuffer window's lines
    /// change: the frame grows or shrinks by the difference downward.
    pub(super) fn refit_minibuffer_host(&mut self, frame: Value) -> Result<()> {
        let line = self.window_state.line.max(1);
        let wanted = i64::from(self.window_state.mini_lines.max(1) * line);
        let delta = wanted - i64::from(self.window(self.mini_window())?.height);
        let mut geometry = self.child_geometry(frame)?;
        geometry.height = u32::try_from(i64::from(geometry.height) + delta).unwrap_or(0);
        self.set_child_geometry(frame, geometry)
    }
    /// Raise (or lower) child FRAME above (below) its siblings.
    pub(super) fn restack_frame(&mut self, frame: Value, raise: bool) -> Result<()> {
        self.window_state.next_frame += 1;
        let z = self.window_state.next_frame as i64;
        self.frame_mut(frame)?.z = if raise { z } else { -z };
        self.display_state.version += 1;
        Ok(())
    }
}
type Out = std::result::Result<Value, crate::prim::Fault>;
use crate::prim::Fault;
fn internal(e: String) -> Fault { Fault::Error(e) }
/// The frame surface `prims/frame.rs' declares beyond window.rs's: child
/// frames (GNU frame.c and pgtkfns.c x-create-frame).
impl crate::window::Windows<'_> {
    /// A live child frame, or GNU's error for the host frame CALLER cannot
    /// change.
    fn child(&mut self, frame: Value) -> std::result::Result<Option<Value>, Fault> {
        let frame = self.decode_live_frame(frame)?;
        Ok((self.vm.frame(frame).map_err(internal)?.parent != Value::NIL).then_some(frame))
    }
    pub(crate) fn create_child_frame(&mut self, parent: Value, geometry: ChildGeometry, parameters: Vec<(Value, Value)>) -> Out {
        let parent = self.decode_live_frame(parent)?;
        // make_frame: the current buffer, unless it is a minibuffer.
        let current = self.vm.slots[self.vm.buffer_slot].value;
        let buffer = if self.vm.minibuffer_window_buffer() == Some(current) {
            self.vm.named_buffer("*scratch*").unwrap_or(current)
        } else { current };
        self.vm.allocate_child_frame(parent, buffer, geometry, parameters).map_err(internal)
    }
    pub(crate) fn frame_minibuffer_window(&mut self, frame: Value) -> Out {
        let frame = self.decode_live_frame(frame)?;
        Ok(self.vm.frame(frame).map_err(internal)?.minibuffer)
    }
    pub(crate) fn frame_parent(&mut self, frame: Value) -> Out {
        let frame = self.decode_live_frame(frame)?;
        Ok(self.vm.frame(frame).map_err(internal)?.parent)
    }
    pub(crate) fn frame_ancestor_p(&mut self, ancestor: Value, descendant: Value) -> Out {
        let (a, d) = (self.decode_live_frame(ancestor)?, self.decode_live_frame(descendant)?);
        Ok(if self.vm.frame_ancestors(d).contains(&a) { Value::TRUE } else { Value::NIL })
    }
    /// A child frame's geometry, nil for the host frame.
    pub(crate) fn child_geometry(&mut self, frame: Value) -> std::result::Result<Option<ChildGeometry>, Fault> {
        match self.child(frame)? {
            Some(frame) => Ok(Some(self.vm.child_geometry(frame).map_err(internal)?)),
            None => Ok(None),
        }
    }
    /// Place or size child FRAME; the host frame's geometry is the host
    /// window's, which Lisp does not change.
    pub(crate) fn set_child_geometry(&mut self, frame: Value, geometry: ChildGeometry) -> std::result::Result<(), Fault> {
        if let Some(frame) = self.child(frame)? {
            self.vm.set_child_geometry(frame, geometry).map_err(internal)?;
        }
        Ok(())
    }
    pub(crate) fn frame_visible(&mut self, frame: Value) -> std::result::Result<bool, Fault> {
        let frame = self.decode_live_frame(frame)?;
        Ok(self.vm.frame(frame).map_err(internal)?.visible)
    }
    /// Show or hide child FRAME; the host frame stays visible (VM2-FRAME-VIS).
    pub(crate) fn set_frame_visible(&mut self, frame: Value, visible: bool) -> std::result::Result<(), Fault> {
        if let Some(frame) = self.child(frame)? {
            let f = self.vm.frame_mut(frame).map_err(internal)?;
            if f.visible != visible {
                f.visible = visible;
                self.vm.display_state.version += 1;
            }
        }
        Ok(())
    }
    /// frame.c:3652: the visible frames, oldest first.
    pub(crate) fn visible_frames(&mut self) -> Out {
        self.decode_live_frame(Value::NIL)?;
        let frames: Vec<Value> = self.vm.frames().into_iter().rev().filter(|&f| self.vm.frame(f).is_ok_and(|f| f.visible)).collect();
        Ok(self.vm.list_value(&frames))
    }
    pub(crate) fn restack(&mut self, frame: Value, raise: bool) -> std::result::Result<(), Fault> {
        if let Some(frame) = self.child(frame)? {
            self.vm.restack_frame(frame, raise).map_err(internal)?;
        }
        Ok(())
    }
    pub(crate) fn redirect_frame_focus(&mut self, frame: Value, focus: Value) -> Out {
        let frame = self.decode_live_frame(frame)?;
        if focus != Value::NIL && !self.is_frame_value(focus) { return Err(Fault::Wrong("framep", focus)); }
        self.vm.frame_mut(frame).map_err(internal)?.focus = focus;
        Ok(Value::NIL)
    }
    pub(crate) fn frame_focus(&mut self, frame: Value) -> Out {
        let frame = self.decode_live_frame(frame)?;
        Ok(self.vm.frame(frame).map_err(internal)?.focus)
    }
    fn is_frame_value(&self, v: Value) -> bool {
        matches!(self.vm.object_if(v), Some(Object::Frame(_)))
    }
    /// `romacs--host-minibuffer': child FRAME shows the minibuffer window
    /// on the top edge of its text area, or with FRAME nil the window
    /// returns to its line at the frame's bottom.
    pub(crate) fn host_minibuffer(&mut self, frame: Value) -> Out {
        let host = if frame == Value::NIL { None } else {
            Some(self.child(frame)?.ok_or_else(|| Fault::Error("Only a child frame hosts the minibuffer window".into()))?)
        };
        self.vm.host_minibuffer(host).map_err(internal)?;
        Ok(frame)
    }
    /// `romacs--minibuffer-host': the child frame hosting the minibuffer
    /// window, or nil.
    pub(crate) fn minibuffer_host(&mut self) -> Out {
        Ok(self.vm.minibuffer_host().unwrap_or(Value::NIL))
    }
    /// Whether FRAME (nil, the selected one) is a live child frame.
    pub(crate) fn is_child_frame(&mut self, frame: Value) -> std::result::Result<bool, Fault> {
        Ok(self.child(frame)?.is_some())
    }
    /// frame.c delete_frame's deletion, once `delete-frame-functions' ran:
    /// FRAME and its descendants die. A frame already dead is left alone;
    /// the host frame is the sole frame GNU would refuse to delete.
    pub(crate) fn delete_frame(&mut self, frame: Value, force: Value) -> Out {
        let frame = if frame == Value::NIL { self.decode_live_frame(frame)? } else { frame };
        if !self.is_frame_value(frame) { return Err(Fault::Wrong("framep", frame)); }
        if !self.vm.frame(frame).map_err(internal)?.live { return Ok(Value::NIL); }
        if self.vm.frame(frame).map_err(internal)?.parent == Value::NIL {
            return Err(Fault::Error(if force == Value::NIL { "Attempt to delete the sole visible or iconified frame" }
                                    else { "Attempt to delete the only frame" }.into()));
        }
        self.vm.delete_child_frame(frame).map_err(internal)?;
        Ok(Value::NIL)
    }
}
