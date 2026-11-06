//! VM-owned windows. GNU window.c's prepared-size transactions, not Lisp policy.
use crate::{data::Object, *};
#[path = "redisplay.rs"]
mod redisplay;
pub(crate) use redisplay::shown_bytes;
#[path = "window_config.rs"]
mod config;
pub(crate) use config::WindowConfiguration;
pub(crate) const ROOT: &str = "#:vm2-root-window";
const MINI: &str = "#:vm2-minibuffer-window";
const SELECTED: &str = "#:vm2-selected-window";
#[derive(Clone)]
pub(super) struct Window {
    pub id: u64,
    pub frame: Value,
    pub combination_limit: Value,
    pub buffer: Value,
    pub point: Value,
    pub start: Value,
    pub parent: Value,
    pub children: Vec<Value>,
    pub horizontal: bool,
    pub dead: bool,
    pub mini: bool,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub new_pixel: u32,
    pub left_margin: u32,
    pub right_margin: u32,
    /// GNU `w->left_fringe_width' and `w->right_fringe_width' in pixels,
    /// None for the frame's (-1), `w->fringes_outside_margins' and
    /// `w->fringes_persistent' (window.h), as `set-window-fringes' set them.
    pub fringes: [Option<u32>; 2],
    pub fringes_outside_margins: bool,
    pub fringes_persistent: bool,
    /// Per-window vertical scroll in pixels. GNU keeps this on the window as
    /// `w->vscroll` and both `window-vscroll` and `set-window-vscroll` begin by
    /// decoding the WINDOW argument (`src/window.c:8947`, `:8978`). It lived on
    /// `DisplayState` and was published into every window, so one window's
    /// scroll was observable from another. It belongs here, beside the margins,
    /// which are per-window for the same reason and never had that defect.
    pub vscroll_px: u32,
    /// Deferred scroll, applied after the frozen frame paints (D-ML1).
    pub pending_vscroll: Option<u32>,
    /// GNU `w->hscroll`, in columns: how far the window's lines are
    /// scrolled left. Redisplay keeps point in view with it
    /// (`hscroll_window`), never below `min_hscroll`, which `scroll-left'
    /// and `scroll-right' set when called interactively.
    pub hscroll: i64,
    pub min_hscroll: i64,
    /// GNU `w->suspend_auto_hscroll`: an explicit scroll holds automatic
    /// hscrolling until point moves from `old_point`, GNU's
    /// `w->old_pointm`, a marker at the window's point at its last
    /// redisplay, which edits move as they move point.
    pub suspend_auto_hscroll: bool,
    pub old_point: Value,
    pub parameters: Vec<(Value, Value)>,
    /// GNU `w->display_table` (`set-window-display-table`); the display
    /// reads the buffer's and the standard table only (VM2-DISPTAB1).
    pub display_table: Value,
    /// GNU `w->prev_buffers` and `w->next_buffers`: Lisp lists kept for
    /// window.el's buffer history (`window-prev-buffers`).
    pub prev_buffers: Value,
    pub next_buffers: Value,
    /// GNU `w->dedicated`: nil, t (strongly dedicated) or another value
    /// (weakly dedicated), as set-window-dedicated-p left it.
    pub dedicated: Value,
    /// GNU `w->new_total`, `w->new_normal`, `w->normal_lines` and
    /// `w->normal_cols` (window.h): Lisp objects, set by `make_window`
    /// (window.c:4599) to 0, 0, 1.0 and 1.0. `window_resize_apply`
    /// (window.c:4815) installs a numeric `new_normal` as the normal size
    /// along the resized axis. Traced by `visit_values`.
    pub new_total: Value,
    pub new_normal: Value,
    pub normal_lines: Value,
    pub normal_cols: Value,
    /// GNU `w->use_time` (window.c:767): `window_select_count` at the last
    /// recorded selection, 0 for a window never selected that way.
    pub use_time: u64,
    /// GNU `w->cursor_off_p` (dispnew.c internal-show-cursor).
    pub cursor_off: bool,
    pub end_distance: usize,
    pub end_valid: bool,
    /// How many times the window's buffer changed: what a record of its
    /// display compares, as a collection moves the buffer's Value.
    pub(crate) shows: u64,
    pub force_start: bool,
}
romacs_snapshot::snapshot_struct!(Window { shows, id, frame, combination_limit, buffer, point, start, parent, children, horizontal, dead, mini, x, y, width, height, new_pixel, left_margin, right_margin, fringes, fringes_outside_margins, fringes_persistent, vscroll_px, pending_vscroll, hscroll, min_hscroll, suspend_auto_hscroll, old_point, parameters, display_table, prev_buffers, next_buffers, dedicated, new_total, new_normal, normal_lines, normal_cols, use_time, cursor_off, end_distance, end_valid, force_start });
impl Window {
    /// BUFFER is what the window shows now.
    pub(crate) fn show(&mut self, buffer: Value) {
        if self.buffer != buffer { self.shows += 1; }
        self.buffer = buffer;
    }
    fn new(id: u64, buffer: Value) -> Self {
        Self {
            id,
            frame: Value::NIL,
            combination_limit: Value::NIL,
            buffer,
            point: Value::NIL,
            start: Value::NIL,
            parent: Value::NIL,
            children: vec![],
            horizontal: false,
            dead: false,
            mini: false,
            x: 0,
            y: 0,
            width: 640,
            height: 460,
            new_pixel: 0,
            left_margin: 0,
            fringes: [None; 2],
            fringes_outside_margins: false,
            fringes_persistent: false,
            vscroll_px: 0,
            pending_vscroll: None,
            hscroll: 0,
            min_hscroll: 0,
            suspend_auto_hscroll: false,
            old_point: Value::NIL,
            right_margin: 0,
            parameters: vec![],
            display_table: Value::NIL,
            prev_buffers: Value::NIL,
            next_buffers: Value::NIL,
            dedicated: Value::NIL,
            new_total: Value::NIL,
            new_normal: Value::NIL,
            normal_lines: Value::NIL,
            normal_cols: Value::NIL,
            use_time: 0,
            cursor_off: false,
            end_distance: 0,
            end_valid: false,
            shows: 0,
            force_start: false,
        }
    }
    pub fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) {
        for v in [
            &mut self.frame,
            &mut self.combination_limit,
            &mut self.buffer,
            &mut self.point,
            &mut self.start,
            &mut self.old_point,
            &mut self.parent,
            &mut self.new_total,
            &mut self.new_normal,
            &mut self.normal_lines,
            &mut self.normal_cols,
            &mut self.prev_buffers,
            &mut self.next_buffers,
            &mut self.dedicated,
        ] {
            f(v);
        }
        for v in &mut self.children {
            f(v);
        }
        for (k, v) in &mut self.parameters {
            f(k);
            f(v);
        }
        f(&mut self.display_table);
    }
    pub fn storage_bytes(&self) -> usize {
        size_of::<Self>()
            + self.children.capacity() * size_of::<Value>()
            + self.parameters.capacity() * size_of::<(Value, Value)>()
    }
    pub fn live(&self) -> bool {
        !self.dead && self.children.is_empty() && self.buffer != Value::NIL
    }
}
pub(super) struct WindowState {
    next: u64,
    /// GNU `window_select_count` (window.c:136).
    select_count: u64,
    pub width: u32,
    pub height: u32,
    pub line: u32,
    pub column: u32,
    /// The mini-window's height in lines (xdisp.c resize_mini_window).
    pub mini_lines: u32,
    /// The pixels the host frame's last layout left below its root
    /// windows for the mini-window: none while a child frame hosts it.
    pub mini_reserved: u32,
    /// The last frame identity or stacking rank handed out (frame.rs).
    pub next_frame: u64,
    /// The host frame's text area at its last layout, tab bar included.
    pub text_area: [u32; 2],
    /// The status bar's height as the host measured its row (0 before
    /// one did: a canonical line), and the pixels the last layout left
    /// for it between the root window and the mini-window (VM2-STATUS-BAR).
    pub status_height: u32,
    pub status_reserved: u32,
    /// The display's device pixels per logical pixel (a HiDPI output's
    /// scale): images are decoded at it and laid out in logical pixels.
    pub device_scale: f32,
}
romacs_snapshot::snapshot_struct!(WindowState { next, select_count, width, height, line, column, mini_lines, mini_reserved, next_frame, text_area, status_height, status_reserved, device_scale });
impl Default for WindowState {
    fn default() -> Self {
        Self {
            next: 1,
            select_count: 0,
            width: 640,
            height: 480,
            line: 20,
            column: 10,
            mini_lines: 1,
            mini_reserved: 0,
            next_frame: 1,
            text_area: [640, 480],
            status_height: 0,
            status_reserved: 0,
            device_scale: 1.0,
        }
    }
}
impl Vm {
    pub(super) fn window(&self, v: Value) -> Result<&Window> {
        match self.object_if(v) {
            Some(Object::Window(w)) => Ok(w),
            _ => Err("window required".into()),
        }
    }
    pub(super) fn window_mut(&mut self, v: Value) -> Result<&mut Window> {
        match self.objects.get_mut(v.index(0)?.saturating_sub(1)) {
            Some(Object::Window(w)) => Ok(w),
            _ => Err("window required".into()),
        }
    }
    fn window_root(&mut self, name: &str) -> Value {
        self.window_root_peek(name)
    }
    /// Whether a window of a shown frame shows BUFFER.
    pub(super) fn buffer_is_displayed(&self, buffer: Value) -> Result<bool> {
        let mut pending = vec![self.window_root_peek(MINI)];
        for frame in self.frames().into_iter().filter(|&f| self.frame_shown(f)) {
            pending.push(self.frame(frame)?.root);
        }
        while let Some(value) = pending.pop() {
            let window = self.window(value)?;
            if window.dead { continue; }
            if window.live() && window.buffer == buffer && window.width > 0 && window.height > 0 {
                return Ok(true);
            }
            pending.extend(window.children.iter().copied());
        }
        Ok(false)
    }

    /// The buffer the minibuffer window shows (every frame shares it).
    pub(crate) fn minibuffer_window_buffer(&self) -> Option<Value> {
        self.window(self.window_root_peek(MINI)).ok().map(|w| w.buffer)
    }
    /// The host frame's root or minibuffer window, or the selected
    /// frame's selected window, read without interning: the slots exist
    /// once `ensure_windows` has run.
    pub(crate) fn window_root_peek(&self, name: &str) -> Value {
        let frame = if name == SELECTED { self.frame_value_peek() } else { self.host_frame_peek() };
        let Ok(f) = self.frame(frame) else { return Value::UNBOUND; };
        match name { ROOT => f.root, SELECTED => f.selected, MINI => f.minibuffer, _ => unreachable!() }
    }
    /// The minibuffer window, which every frame shares.
    pub(super) fn mini_window(&self) -> Value {
        self.window_root_peek(MINI)
    }
    /// The root window of the frame window V is on.
    pub(super) fn root_of(&self, v: Value) -> Result<Value> {
        Ok(self.frame(self.window(v)?.frame)?.root)
    }
    /// Put window V's old point at POSITION in BUFFER, making its marker if
    /// it has none.
    pub(crate) fn set_window_old_point(&mut self, v: Value, position: Value, buffer: Value) -> Result<()> {
        let marker = self.window(v)?.old_point;
        if marker == Value::NIL {
            let marker = self.window_marker(buffer, position)?;
            self.window_mut(v)?.old_point = marker;
            return Ok(());
        }
        self.set_marker_object(marker, position, buffer).map(drop)
    }
    /// How text inserted at a window's point moves its old point:
    /// `window-point-insertion-type' in the window's BUFFER.
    pub(crate) fn window_point_bias(&mut self, buffer: Value) -> Result<anchors::Bias> {
        let slot = self.slot("window-point-insertion-type");
        Ok(if self.value_in_buffer(slot, buffer)? == Value::NIL { anchors::Bias::Left } else { anchors::Bias::Right })
    }
    fn window_marker(&mut self, buffer: Value, position: Value) -> Result<Value> {
        let v = self.allocate_object(Object::Marker(Box::new(annotations::Marker {
            buffer: Value::NIL,
            anchor: None,
            bias: anchors::Bias::Left,
        })));
        self.set_marker_object(v, position, buffer)?;
        Ok(v)
    }
    pub(super) fn allocate_window(&mut self, buffer: Value) -> Result<Value> {
        let id = self.window_state.next;
        self.window_state.next += 1;
        let mut w = Window::new(id, buffer);
        if self.frame_value_peek() != Value::UNBOUND {
            w.frame = self.frame_value_peek();
        }
        w.new_total = Value::integer(0)?;
        w.new_normal = Value::integer(0)?;
        w.normal_lines = self.allocate_object(Object::Float(1.0));
        w.normal_cols = self.allocate_object(Object::Float(1.0));
        if buffer != Value::NIL {
            w.point = self.window_marker(buffer, Value::integer(1)?)?;
            w.start = self.window_marker(buffer, Value::integer(1)?)?;
            w.old_point = self.window_marker(buffer, Value::integer(1)?)?;
            let bias = self.window_point_bias(buffer)?;
            self.set_marker_bias(w.old_point, bias)?;
        }
        Ok(self.allocate_object(Object::Window(Box::new(w))))
    }
    pub(super) fn ensure_windows(&mut self) -> Result<()> {
        if self.window_root(ROOT) != Value::UNBOUND {
            return Ok(());
        }
        let scratch = self.create_named_buffer("*scratch*", false)?;
        let root = self.allocate_window(scratch)?;
        let mini_buffer = self.create_named_buffer(" *Minibuf-0*", false)?;
        let mini = self.allocate_window(mini_buffer)?;
        self.allocate_initial_frame(root, mini)?;
        // The initial frame's window is selected with NORECORD nil
        // (frame.c make_initial_frame -> Fselect_window), so it starts at 1.
        self.window_state.select_count += 1;
        self.window_mut(root)?.use_time = self.window_state.select_count;
        let (width, height, line) = (
            self.window_state.width,
            self.window_state.height,
            self.window_state.line,
        );
        {
            let w = self.window_mut(root)?;
            w.width = width;
            w.height = height - line;
        }
        {
            let w = self.window_mut(mini)?;
            w.mini = true;
            w.width = width;
            w.height = line;
            w.y = height - line;
        }
        Ok(())
    }
    pub(super) fn selected_window_value(&mut self) -> Result<Value> {
        self.ensure_windows()?;
        Ok(self.window_root(SELECTED))
    }
    pub(super) fn decode_window(&mut self, v: Value, live: bool, dead_ok: bool) -> Result<Value> {
        self.ensure_windows()?;
        let v = if v == Value::NIL {
            self.window_root(SELECTED)
        } else {
            v
        };
        let w = self.window(v)?;
        if (live && !w.live()) || (!dead_ok && w.dead) {
            return Err("live/valid window required".into());
        }
        Ok(v)
    }
    /// decode_live_frame: nil is the selected frame.
    fn frame_arg(&mut self, v: Value) -> Result<Value> {
        let frame = self.selected_frame_value()?;
        match self.object_if(v) {
            _ if v == Value::NIL => Ok(frame),
            Some(Object::Frame(f)) if f.live => Ok(v),
            _ => Err("unknown frame".into()),
        }
    }
    pub(super) fn window_point_value(&mut self, v: Value) -> Result<Value> {
        if v == self.window_root(SELECTED) {
            Value::integer(self.buffer_point_position(self.window(v)?.buffer)?)
        } else {
            self.marker_position_value(self.window(v)?.point)
        }
    }
    pub(super) fn set_buffer_point_for_window(&mut self, v: Value, pos: Value) -> Result<()> {
        let buffer = self.window(v)?.buffer;
        let marker = self.window(v)?.point;
        self.set_marker_object(marker, pos, buffer)?;
        let pos = self.marker_position_value(marker)?.as_integer()?;
        match self.object_mut(buffer)? {
            Object::Buffer(b) => b.point = b.byte_of_char(coordinates::CharOffset((pos - 1) as usize))?,
            Object::ByteBuffer(b) => b.point = crate::coordinates::ByteOffset((pos - 1) as usize),
            _ => return Err("buffer required".into()),
        }
        Ok(())
    }
    /// GNU window_discard_buffer_from_window (ALL false): BUFFER leaves
    /// WINDOW's previous-buffers alist and next-buffers list, spliced out
    /// destructively as GNU does.
    pub(super) fn discard_buffer_from_window(&mut self, buffer: Value, window: Value) -> Result<()> {
        let (prev, next) = (self.window(window)?.prev_buffers, self.window(window)?.next_buffers);
        let prev = self.discard_buffer_from_list(buffer, prev, true)?;
        let next = self.discard_buffer_from_list(buffer, next, false)?;
        let w = self.window_mut(window)?;
        w.prev_buffers = prev;
        w.next_buffers = next;
        Ok(())
    }
    /// LIST without the elements that are BUFFER (or, with ALIST, whose car is).
    fn discard_buffer_from_list(&mut self, buffer: Value, list: Value, alist: bool) -> Result<Value> {
        let (mut head, mut previous, mut tail) = (list, None::<Value>, list);
        while let Ok((element, rest)) = self.cons_pair(tail) {
            let key = if alist { self.cons_pair(element).map_or(Value::NIL, |(k, _)| k) } else { element };
            if key == buffer {
                match previous {
                    None => head = rest,
                    Some(cell) => {
                        self.cons_mut(cell)?.1 = rest;
                    }
                }
            } else {
                previous = Some(tail);
            }
            tail = rest;
        }
        Ok(head)
    }
    /// set_window_buffer: show live BUFFER in live window V (src/window.c).
    pub(super) fn set_window_buffer_value(&mut self, v: Value, b: Value, keep: bool) -> Result<()> {
        let buffer = self.display_buffer(b)?;
        // GNU computes `samebuf` BEFORE assigning, then skips the
        // whole marker/scroll reset when KEEP-MARGINS is set and the
        // buffer is unchanged (src/window.c:4367, the
        // `!(keep_margins_p && samebuf)` guard). Its comment
        // explains why: resetting hscroll/vscroll breaks image-mode
        // and doc-view-mode on every frame resize.
        let samebuf = self.window(v)?.buffer == buffer;
        if !(keep && samebuf) {
            let (point, st) = (self.window(v)?.point, self.window(v)?.start);
            // GNU takes the buffer's own point, `BUF_PT (b)`, not 1
            // (src/window.c:4375). Resetting to 1 loses an
            // unselected window's position: verified on live GNU
            // 32.0.50, assigning a buffer whose point is 7 leaves
            // window-point at 7 and selecting the window puts point
            // at 7.
            let pos = {
                match self.object(buffer)? {
                    Object::Buffer(buf) => Value::integer((buf.char_of_byte(buf.point)?.0 + 1) as i64)?,
                    Object::ByteBuffer(buf) => Value::integer(buf.point.0 as i64 + 1)?,
                    _ => return Err("buffer required".into()),
                }
            };
            self.set_marker_object(point, pos, buffer)?;
            // GNU restores `b->last_window_start` here; VM2 keeps no
            // per-buffer last window start, so it starts at 1. See
            // DIVERGENCES.md VM2-WB1.
            self.set_marker_object(st, Value::integer(1)?, buffer)?;
            self.set_window_old_point(v, pos, buffer)?;
            let w = self.window_mut(v)?;
            (w.hscroll, w.min_hscroll, w.suspend_auto_hscroll) = (0, 0, false);
        }
        // window.c:4395, whatever KEEP-MARGINS kept: the old point inserts
        // as the buffer's window points do.
        let old = self.window(v)?.old_point;
        if old != Value::NIL {
            let bias = self.window_point_bias(buffer)?;
            self.set_marker_bias(old, bias)?;
        }
        self.discard_buffer_from_window(buffer, v)?;
        let w = self.window_mut(v)?;
        w.show(buffer);
        w.end_distance = 0;
        w.end_valid = false;
        self.display_state.version += 1;
        // Without KEEP-MARGINS the window takes the buffer's margin widths
        // (src/window.c:4397), independently of the guard above.
        if !keep {
            let width = |vm: &mut Self, name: &str| -> Result<u32> {
                let slot = vm.slot(name);
                let value = vm.value_in_buffer(slot, buffer)?;
                Ok(value.integer_value().and_then(|n| u32::try_from(n).ok()).unwrap_or(0))
            };
            let (left, right) = (width(self, "left-margin-width")?, width(self, "right-margin-width")?);
            let w = self.window_mut(v)?;
            w.left_margin = left;
            w.right_margin = right;
            // ... and, unless its fringes are persistent, the buffer's
            // fringe widths (window.c:4401).
            if !self.window(v)?.fringes_persistent {
                let value = |vm: &mut Self, name: &str| -> Result<Value> { let slot = vm.slot(name); vm.value_in_buffer(slot, buffer) };
                let widths = [value(self, "left-fringe-width")?, value(self, "right-fringe-width")?];
                let outside = value(self, "fringes-outside-margins")?;
                self.set_window_fringes(v, widths, outside != Value::NIL && outside != Value::UNBOUND, None)?;
            }
        }
        Ok(())
    }
    /// GNU `select_window` (window.c:536): NORECORD nil bumps the use time
    /// even when V is already selected (`record_and_return`).
    pub(crate) fn select_window_value(&mut self, v: Value, record: bool) -> Result<Value> {
        if record {
            self.window_state.select_count += 1;
            let count = self.window_state.select_count;
            self.window_mut(v)?.use_time = count;
        }
        let old = self.window_root(SELECTED);
        if old != v {
            let old_point = self.window_point_value(old)?;
            let old_buffer = self.window(old)?.buffer;
            self.set_marker_object(self.window(old)?.point, old_point, old_buffer)?;
            let new_point = self.marker_position_value(self.window(v)?.point)?;
            self.set_buffer_point_for_window(v, new_point)?;
            // Selecting a window selects its frame (window.c select_window).
            let frame = self.window(v)?.frame;
            let slot = self.slot(crate::frame::SELECTED_FRAME);
            self.slots[slot].value = frame;
            self.frame_mut(frame)?.selected = v;
        }
        let buffer = self.window(v)?.buffer;
        self.slots[self.buffer_slot].value = buffer;
        // window.c select_window's record_and_return: record_buffer moves
        // the buffer to the front of `buffer-list' (buffer.c:2120).
        if record { self.record_buffer_model(buffer); }
        Ok(v)
    }
    fn prepared_check(&self, v: Value, horizontal: bool, omit: Value) -> Result<bool> {
        let w = self.window(v)?;
        if w.children.is_empty() {
            return Ok(w.new_pixel
                >= if horizontal {
                    2 * self.window_state.column
                } else {
                    self.window_state.line
                });
        }
        let mut sum = 0u64;
        for &c in &w.children {
            if c == omit {
                continue;
            }
            if !self.prepared_check(c, horizontal, omit)? {
                return Ok(false);
            }
            let size = self.window(c)?.new_pixel;
            if w.horizontal == horizontal {
                sum += u64::from(size);
            } else if size != w.new_pixel {
                return Ok(false);
            }
        }
        Ok(w.horizontal != horizontal || sum == u64::from(w.new_pixel))
    }
    fn apply_prepared(&mut self, v: Value, horizontal: bool, edge: u32) -> Result<()> {
        let numeric = self.numberp(self.window(v)?.new_normal);
        let w = self.window_mut(v)?;
        if horizontal {
            w.width = w.new_pixel;
            w.x = edge;
            if numeric {
                w.normal_cols = w.new_normal;
            }
        } else {
            w.height = w.new_pixel;
            w.y = edge;
            if numeric {
                w.normal_lines = w.new_normal;
            }
        }
        w.end_valid = false;
        let (children, same) = (w.children.clone(), w.horizontal == horizontal);
        let mut at = edge;
        for c in children {
            self.apply_prepared(c, horizontal, at)?;
            if same {
                at += self.window(c)?.new_pixel;
            }
        }
        Ok(())
    }
    fn replace_tree_node(&mut self, old: Value, new: Value) -> Result<()> {
        let parent = self.window(old)?.parent;
        if parent == Value::NIL {
            let frame = self.window(old)?.frame;
            self.frame_mut(frame)?.root = new;
        } else {
            let p = self.window_mut(parent)?;
            let at = p
                .children
                .iter()
                .position(|v| *v == old)
                .ok_or("broken window parent")?;
            p.children[at] = new;
        }
        self.window_mut(new)?.parent = parent;
        Ok(())
    }
    pub(super) fn retire_window(&mut self, v: Value) -> Result<()> {
        let children = self.window(v)?.children.clone();
        for c in children {
            self.retire_window(c)?;
        }
        // Prune here, not at the deletion site: this is the function that
        // recurses, so a descendant leaf retired as part of a subtree would
        // otherwise keep its published snapshot forever. Window ids are
        // monotonic and never reused, so the map would grow for the life of the
        // VM and hold that window's owned text with it. The publication map is
        // Rust-side state rather than a Lisp heap object, so no heap-object or
        // live-byte assertion can observe this leak.
        let retired_id = self.window(v)?.id;
        self.display_state.forget_window(retired_id);
        let (point, start, old) = (self.window(v)?.point, self.window(v)?.start, self.window(v)?.old_point);
        for marker in [point, start, old] {
            if marker != Value::NIL {
                self.set_marker_object(marker, Value::NIL, Value::NIL)?;
            }
        }
        let w = self.window_mut(v)?;
        w.dead = true;
        w.buffer = Value::NIL;
        w.point = Value::NIL;
        w.start = Value::NIL;
        w.old_point = Value::NIL;
        w.parent = Value::NIL;
        w.children = vec![];
        w.end_valid = false;
        Ok(())
    }
    fn first_leaf(&self, mut v: Value) -> Result<Value> {
        while let Some(&c) = self.window(v)?.children.first() {
            v = c;
        }
        Ok(v)
    }
    fn delete_window_value(&mut self, v: Value) -> Result<Value> {
        let w = self.window(v)?;
        if w.dead {
            return Ok(Value::NIL);
        }
        let parent = w.parent;
        if parent == Value::NIL {
            return Err("Attempt to delete minibuffer or sole ordinary window".into());
        }
        let horizontal = self.window(parent)?.horizontal;
        let root = self.root_of(v)?;
        let root_size = if horizontal {
            self.window(root)?.width
        } else {
            self.window(root)?.height
        };
        if !self.prepared_check(root, horizontal, v)? || self.window(root)?.new_pixel != root_size {
            return Err("Deletion failed".into());
        }
        // Validation precedes mutation: failure has exactly GNU's relinked state.
        self.window_mut(parent)?.children.retain(|c| *c != v);
        // `retain` removes elements but keeps the allocation, and the
        // collector forwards a surviving window without compacting its
        // vectors. A parent that grows to many children and is then reduced
        // would otherwise retain its peak storage for as long as it lives:
        // measured flat at 14 objects while bytes climbed 5136 -> 5392 ->
        // 5904 across peaks of 4, 8 and 16, i.e. 64 bytes per peak child
        // held by a parent whose final topology is identical.
        self.window_mut(parent)?.children.shrink_to_fit();
        let edge = if horizontal {
            self.window(parent)?.x
        } else {
            self.window(parent)?.y
        };
        self.apply_prepared(parent, horizontal, edge)?;
        let selected = self.window_root(SELECTED);
        let selected_point = self.window_point_value(selected)?;
        let selected_buffer = self.window(selected)?.buffer;
        self.set_marker_object(
            self.window(selected)?.point,
            selected_point,
            selected_buffer,
        )?;
        self.retire_window(v)?;
        if self.window(parent)?.children.len() == 1 {
            let sibling = self.window(parent)?.children[0];
            self.replace_tree_node(parent, sibling)?;
            // window.c:5810-5811: SIBLING inherits PARENT's normal sizes.
            let (cols, lines) = {
                let p = self.window(parent)?;
                (p.normal_cols, p.normal_lines)
            };
            let s = self.window_mut(sibling)?;
            s.normal_cols = cols;
            s.normal_lines = lines;
            self.window_mut(parent)?.children.clear();
            self.retire_window(parent)?;
            self.recombine_window(sibling)?;
        }
        let frame = self.window(v)?.frame;
        if !self.window(self.frame(frame)?.selected)?.live() {
            let next = self.first_leaf(self.frame(frame)?.root)?;
            self.frame_mut(frame)?.selected = next;
            if self.window(selected)?.dead {
                self.slots[self.buffer_slot].value = self.window(next)?.buffer;
                let point = self.marker_position_value(self.window(next)?.point)?;
                self.set_buffer_point_for_window(next, point)?;
            }
        }
        Ok(Value::NIL)
    }
    fn recombine_window(&mut self, v: Value) -> Result<()> {
        let w = self.window(v)?;
        let parent = w.parent;
        if parent == Value::NIL
            || w.children.is_empty()
            || w.combination_limit != Value::NIL
            || self.window(parent)?.horizontal != w.horizontal
        {
            return Ok(());
        }
        let children = w.children.clone();
        let p = self.window_mut(parent)?;
        let at = p
            .children
            .iter()
            .position(|c| *c == v)
            .ok_or("broken window parent")?;
        p.children.splice(at..at + 1, children.iter().copied());
        let (horizontal, extent) = (p.horizontal, if p.horizontal { p.width } else { p.height });
        for c in children {
            // window.c:2646-2653: a promoted child's normal size is its
            // share of its new parent.
            let size = { let w = self.window(c)?; if horizontal { w.width } else { w.height } };
            let normal = self.allocate_object(Object::Float(f64::from(size) / f64::from(extent.max(1))));
            let w = self.window_mut(c)?;
            w.parent = parent;
            if horizontal { w.normal_cols = normal; } else { w.normal_lines = normal; }
        }
        self.window_mut(v)?.children = vec![];
        self.retire_window(v)
    }
    fn split_window_value(
        &mut self,
        v: Value,
        pixels: u32,
        side: Value,
        normal: Value,
        refer: Value,
    ) -> Result<Value> {
        if refer != Value::NIL {
            return Err("VM2 split REFER resurrection is unsupported".into());
        }
        // window.c:5378 does not check NORMAL-SIZE: it becomes the new
        // window's new normal size, installed only if it is a number
        // (window_resize_apply, window.c:4830).
        let side_name = self.display(side)?;
        let horizontal = side == Value::TRUE || side_name == "left" || side_name == "right";
        let before = side_name == "left" || side_name == "above";
        let w = self.window(v)?.clone();
        if w.mini {
            return Err("Attempt to split minibuffer window".into());
        }
        let minimum = if horizontal {
            2 * self.window_state.column
        } else {
            self.window_state.line
        };
        let old_size = if horizontal { w.width } else { w.height };
        if pixels < minimum
            || !self.prepared_check(v, horizontal, Value::NIL)?
            || pixels.checked_add(w.new_pixel) != Some(old_size)
        {
            return Err("Sum of sizes of old and new window don't fit".into());
        }
        let reference = if w.live() {
            v
        } else {
            self.window_root(SELECTED)
        };
        let (buffer, left_margin, right_margin, fringes, outside) = {
            let reference = self.window(reference)?;
            (reference.buffer, reference.left_margin, reference.right_margin, reference.fringes, reference.fringes_outside_margins)
        };
        let new = self.allocate_window(buffer)?;
        {
            let n = self.window_mut(new)?;
            n.x = w.x;
            n.y = w.y;
            n.width = w.width;
            n.height = w.height;
            n.new_pixel = pixels;
            n.left_margin = left_margin;
            n.right_margin = right_margin;
            n.fringes = fringes;
            n.fringes_outside_margins = outside;
        }
        if w.live() {
            let point = self.window_point_value(v)?;
            self.set_marker_object(self.window(new)?.point, point, buffer)?;
            let start = self.marker_position_value(w.start)?;
            self.set_marker_object(self.window(new)?.start, start, buffer)?;
        }
        let unit = if horizontal {
            self.window_state.column
        } else {
            self.window_state.line
        };
        let limit_slot = self.slot("window-combination-limit");
        let limit = self.variable_value(limit_slot)? == Value::TRUE;
        let parent = if !limit && w.parent != Value::NIL && self.window(w.parent)?.horizontal == horizontal {
            w.parent
        } else {
            let p = self.allocate_window(Value::NIL)?;
            {
                // make_parent_window copies OLD (window.c:5142); then
                // Fsplit_window_internal gives the parent OLD's pixel and
                // total size and OLD's normal size as its new ones
                // (window.c:5506-5511).
                let node = self.window_mut(p)?;
                node.combination_limit = if limit { Value::TRUE } else { Value::NIL };
                node.x = w.x;
                node.y = w.y;
                node.width = w.width;
                node.height = w.height;
                node.horizontal = horizontal;
                node.new_pixel = old_size;
                node.normal_lines = w.normal_lines;
                node.normal_cols = w.normal_cols;
                node.new_total = Value::integer(i64::from(old_size / unit))?;
                node.new_normal = if horizontal {
                    w.normal_cols
                } else {
                    w.normal_lines
                };
            }
            // replace_window (window.c:2571-2574, SETFLAG true) hands OLD's
            // normal sizes to the parent and resets OLD's own to 1.0.
            let lines = self.allocate_object(Object::Float(1.0));
            let cols = self.allocate_object(Object::Float(1.0));
            {
                let old = self.window_mut(v)?;
                old.normal_lines = lines;
                old.normal_cols = cols;
            }
            self.replace_tree_node(v, p)?;
            self.window_mut(p)?.children.push(v);
            self.window_mut(v)?.parent = p;
            p
        };
        let at = self
            .window(parent)?
            .children
            .iter()
            .position(|c| *c == v)
            .unwrap()
            + usize::from(!before);
        self.window_mut(parent)?.children.insert(at, new);
        self.window_mut(new)?.parent = parent;
        // window.c:5641-5653: the new window's new total is what its
        // siblings' new totals leave of the parent's total; its new normal
        // is NORMAL-SIZE.
        let parent_total = {
            let p = self.window(parent)?;
            i64::from(if horizontal { p.width } else { p.height } / unit)
        };
        let mut sum = 0i64;
        for &c in &self.window(parent)?.children {
            if c != new {
                sum = sum.saturating_add(self.window(c)?.new_total.as_integer().unwrap_or(0));
            }
        }
        {
            let total = Value::integer(parent_total.saturating_sub(sum))?;
            let n = self.window_mut(new)?;
            n.new_total = total;
            n.new_normal = normal;
        }
        let edge = if horizontal { w.x } else { w.y };
        if before {
            self.apply_prepared(new, horizontal, edge)?;
            self.apply_prepared(v, horizontal, edge + pixels)?;
        } else {
            self.apply_prepared(v, horizontal, edge)?;
            self.apply_prepared(new, horizontal, edge + w.new_pixel)?;
        }
        // window_resize_apply (p, horflag) also installs every numeric new
        // normal size in PARENT's subtree along the split axis.
        self.install_normals(parent, horizontal)?;
        Ok(new)
    }
    fn numberp(&self, v: Value) -> bool {
        v.as_integer().is_ok() || matches!(self.object_if(v), Some(Object::Float(_)))
    }
    /// The normal-size half of GNU `window_resize_apply` (window.c:4815).
    fn install_normals(&mut self, v: Value, horizontal: bool) -> Result<()> {
        if self.numberp(self.window(v)?.new_normal) {
            let w = self.window_mut(v)?;
            if horizontal {
                w.normal_cols = w.new_normal;
            } else {
                w.normal_lines = w.new_normal;
            }
        }
        for c in self.window(v)?.children.clone() {
            self.install_normals(c, horizontal)?;
        }
        Ok(())
    }
    /// read_minibuf_unwind (minibuf.c:1197): leaving the outermost
    /// minibuffer gives the mini-window back the height its display takes
    /// now, before the command goes on (a prompt it reads next starts from
    /// there, not from the height the last one grew to). True when the
    /// height changed and the frame was laid out again.
    pub(crate) fn resize_mini_window_now(&mut self) -> Result<bool> {
        let mini = self.window_root(MINI);
        if !self.resize_mini_window(mini)? { return Ok(false); }
        let (width, height) = (self.window_state.width, self.window_state.height);
        self.refit_mini_window(width, height)?;
        Ok(true)
    }
    /// Lay out again after the mini-window's height changed: a hosted
    /// mini-window's host frame takes its new height, else the frame's
    /// windows, WIDTH by HEIGHT, make room for it.
    fn refit_mini_window(&mut self, width: u32, height: u32) -> Result<()> {
        match self.minibuffer_host() {
            Some(frame) => self.refit_minibuffer_host(frame),
            None => self.set_frame_geometry(width, height, self.window_state.line, self.window_state.column),
        }
    }

    /// Freeze leaf publications using stable scalar IDs across collecting mode-line eval.
    pub(super) fn frame_window_snapshots(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<Vec<(u32, u32, romacs_layout_input::WindowSnapshot)>> {
        self.resize_root_window(width, height)?;
        self.set_frame_geometry(
            width,
            height,
            self.window_state.line,
            self.window_state.column,
        )?;
        let root = self.window_root(ROOT);
        let mut ids = vec![];
        self.window_leaf_ids(root, &mut ids)?;
        let mini = self.window_root(MINI);
        // A hosted mini-window is shown with its host's windows.
        let host = self.minibuffer_host();
        if host.is_none() { ids.push(self.window(mini)?.id); }
        // Redisplay sizes the mini-window to its display before laying out.
        if self.resize_mini_window(mini)? { self.refit_mini_window(width, height)?; }
        self.reconcile_terminal_geometry()?;
        // Painted inside the internal border (frame.h:1796).
        let border = self.host_border();
        let outcome = self.snapshot_leaves(ids).map(|leaves| leaves.into_iter().map(|(x, y, s)| (x + border, y + border, s)).collect());
        self.display_state.width = width;
        self.display_state.height = height;
        outcome
    }
    /// GNU WINDOW_LEFT_FRINGE_WIDTH and WINDOW_RIGHT_FRINGE_WIDTH
    /// (window.h): V's own widths, else its frame's; none on a frame
    /// without a window system.
    /// The minibuffer window a child frame hosts has that frame's.
    pub(crate) fn window_fringe_widths(&self, v: Value) -> Result<[u32; 2]> {
        if !self.window_system_open() { return Ok([0, 0]); }
        let w = self.window(v)?;
        let shown_in = match self.minibuffer_host() { Some(host) if w.mini => host, _ => w.frame };
        let frame = self.frame(shown_in).map_or([0, 0], |f| f.fringes);
        Ok([w.fringes[0].unwrap_or(frame[0]), w.fringes[1].unwrap_or(frame[1])])
    }
    /// set_window_fringes (window.c:8544): WIDTHS, each a pixel count or
    /// nil for the frame's (extract_dimension), take effect when the text
    /// area keeps MIN_SAFE_WINDOW_PIXEL_WIDTH; OUTSIDE always does; and
    /// PERSISTENT, when given, unless the widths failed. True when V
    /// changed. Nothing on a frame without a window system.
    pub(crate) fn set_window_fringes(&mut self, v: Value, widths: [Value; 2], outside: bool, persistent: Option<bool>) -> Result<bool> {
        if !self.window_system_open() { return Ok(false); }
        let new = widths.map(|width| width.integer_value().and_then(|n| u32::try_from(n).ok()));
        let frame = { let w = self.window(v)?; self.frame(w.frame).map_or([0, 0], |f| f.fringes) };
        let column = self.window_state.column;
        let w = self.window_mut(v)?;
        let effective = |set: [Option<u32>; 2]| [set[0].unwrap_or(frame[0]), set[1].unwrap_or(frame[1])];
        let old = effective(w.fringes);
        let fits = i64::from(w.width) - i64::from((w.left_margin + w.right_margin) * column)
            - i64::from(effective(new)[0] + effective(new)[1]) >= i64::from(2 * column);
        let mut changed = false;
        if fits {
            w.fringes = new;
            changed = effective(new) != old;
            if let Some(persistent) = persistent { w.fringes_persistent = persistent; }
        }
        if outside != w.fringes_outside_margins {
            w.fringes_outside_margins = outside;
            changed = true;
        }
        if changed { w.end_valid = false; }
        Ok(changed)
    }
    /// The pixels of V left and right of its text area: its margins and
    /// fringes, and its right divider, GNU's window box less TEXT_AREA.
    pub(crate) fn window_text_insets(&self, v: Value) -> Result<[u32; 2]> {
        let fringes = self.window_fringe_widths(v)?;
        let [divider, _] = self.window_dividers(v)?;
        let w = self.window(v)?;
        let column = self.window_state.column;
        Ok([w.left_margin * column + fringes[0], w.right_margin * column + fringes[1] + divider])
    }
    /// GNU WINDOW_RIGHT_DIVIDER_WIDTH and WINDOW_BOTTOM_DIVIDER_WIDTH
    /// (window.h:707-718) of V: its frame's divider widths, but no right
    /// divider on a window at the root window's right edge, and no bottom
    /// divider on the mini-window or, on a frame without its own
    /// mini-window (a child frame), on a window at the root's bottom edge.
    /// Derived from position, never stored.
    pub(crate) fn window_dividers(&self, v: Value) -> Result<[u32; 2]> {
        let w = self.window(v)?;
        let Ok(f) = self.frame(w.frame) else { return Ok([0, 0]) };
        if w.mini || f.dividers == [0, 0] { return Ok([0, 0]); }
        let root = self.window(f.root)?;
        let rightmost = w.x + w.width >= root.x + root.width;
        let bottommost = w.y + w.height >= root.y + root.height;
        let below = !bottommost || f.parent == Value::NIL;
        Ok([if rightmost { 0 } else { f.dividers[0] }, if below { f.dividers[1] } else { 0 }])
    }
    /// The host frame's internal border (`internal-border-width'), between
    /// its native edges and its text area (frame.h:1796-1814).
    pub(crate) fn host_border(&self) -> u32 {
        self.frame(self.host_frame_peek()).map_or(0, |f| f.border)
    }
    /// The host frame's text area for a native WIDTH by HEIGHT, its tab
    /// bar included: what GNU's window coordinates span.
    fn host_inner(&self, width: u32, height: u32) -> Option<(u32, u32)> {
        let border = 2 * self.host_border();
        Some((width.checked_sub(border)?, height.checked_sub(border)?))
    }
    /// The live windows IDS name, frozen and placed in their frame. Lisp
    /// run by one window's redisplay may delete a later one, or its own.
    pub(super) fn snapshot_leaves(&mut self, ids: Vec<u64>) -> Result<Vec<(u32, u32, romacs_layout_input::WindowSnapshot)>> {
        // The kept window list, not a heap walk (live_window_index).
        let live = |vm: &mut Self, id| vm.live_window_index(id).map(|i| Value::reference(i + 1, 0));
        let mut snapshots = vec![];
        for id in ids {
            let Some(v) = live(self, id) else { continue };
            let w = self.window(v)?;
            // A hosting child frame's root window may have no lines.
            if w.height == 0 { continue; }
            let [left, right] = self.window_text_insets(v)?;
            let [_, below] = self.window_dividers(v)?;
            let w = self.window(v)?;
            let (x, y, width, height) = (w.x + left, w.y, w.width.saturating_sub(left + right), w.height.saturating_sub(below));
            match self.snapshot_window(v, width, height) {
                Ok(snapshot) => snapshots.push((x, y, snapshot)),
                Err(_) if live(self, id).is_none() => {}
                Err(error) => return Err(error),
            }
        }
        Ok(snapshots)
    }
    /// The leaf ids of window V's subtree, in tree order.
    pub(super) fn leaf_ids(&self, v: Value) -> Result<Vec<u64>> {
        let mut ids = vec![];
        self.window_leaf_ids(v, &mut ids)?;
        Ok(ids)
    }
    /// The index in `objects` of the live window whose id is ID.
    fn live_window_index(&mut self, id: u64) -> Option<usize> {
        self.referrers();
        let objects = &self.objects;
        self.referrers.as_ref().map_or(&[][..], |r| &r.windows).iter().copied()
            .find(|&i| matches!(&objects[i], Object::Window(w) if w.id == id && w.live()))
    }
    /// The root frame's live windows in tree order, as `window-list' from
    /// its first window lists them.
    pub(crate) fn leaf_windows(&self) -> Result<Vec<Value>> {
        fn walk(vm: &Vm, v: Value, out: &mut Vec<Value>) -> Result<()> {
            let w = vm.window(v)?;
            if w.live() { out.push(v); }
            for &c in &w.children { walk(vm, c, out)?; }
            Ok(())
        }
        let mut out = vec![];
        walk(self, self.window_root_peek(ROOT), &mut out)?;
        Ok(out)
    }
    /// WINDOW's text body in (columns, rows) with a one-line mini-window,
    /// or none while a child frame hosts it: the size it has whenever the
    /// mini-window is not grown, planned without changing any window. A
    /// mode line takes one row.
    pub(crate) fn stable_window_body(&self, window: Value) -> Result<Option<(u16, u16)>> {
        let state = &self.window_state;
        let (line, column) = (state.line, state.column);
        let Some((frame_width, frame_height)) = self.host_inner(state.width, state.height) else { return Ok(None) };
        if line == 0 || column == 0 || frame_height < 2 * line { return Ok(None); }
        let mut geometry = vec![];
        let root = self.window_root_peek(ROOT);
        let now = self.mini_growth()?;
        let reserved = if self.minibuffer_host().is_some() { 0 } else { line };
        let status = self.status_bar_reservation(frame_height, reserved, line);
        let tab_bar = (self.tab_bar_lines() * line).min(frame_height.saturating_sub(reserved + status + line));
        let top = tab_bar + if self.status_bar_top() { status } else { 0 };
        if self.plan_window_geometry(root, 0, top, frame_width, frame_height - reserved - status - tab_bar, (now, 0), line, column, &mut geometry).is_err() {
            return Ok(None);
        }
        let Some(&(_, _, _, width, height)) = geometry.iter().find(|g| g.0 == window) else { return Ok(None) };
        let [left, right] = self.window_text_insets(window)?;
        let [_, below] = self.window_dividers(window)?;
        let width = width.saturating_sub(left + right);
        let columns = (width / column).clamp(2, u32::from(u16::MAX)) as u16;
        let band = u32::from(!self.bar_owns_mode_line(window));
        let rows = (height.saturating_sub(below) / line).saturating_sub(band).clamp(1, u32::from(u16::MAX)) as u16;
        Ok(Some((columns, rows)))
    }
    /// The root window's growth now: minus the rows past one line that
    /// the last layout left the mini-window.
    fn mini_growth(&self) -> Result<i64> {
        Ok((i64::from(self.window_state.line) - i64::from(self.window_state.mini_reserved)).min(0))
    }
    /// GNU `window-normal-size' of V along an axis, when it is a number.
    fn normal_size(&self, v: Value, horizontal: bool) -> Result<Option<f64>> {
        let w = self.window(v)?;
        let normal = if horizontal { w.normal_cols } else { w.normal_lines };
        Ok(match self.object_if(normal) {
            Some(Object::Float(f)) => Some(*f),
            _ => normal.as_integer().ok().map(|n| n as f64),
        })
    }
    fn window_leaf_ids(&self, v: Value, out: &mut Vec<u64>) -> Result<()> {
        let w = self.window(v)?;
        if w.live() {
            out.push(w.id);
        } else {
            for &c in &w.children {
                self.window_leaf_ids(c, out)?;
            }
        }
        Ok(())
    }
    /// After a change to a frame's fringes or tab bar: every window's
    /// layout is stale, and the host frame's windows are placed again at
    /// once, as adjust_frame_size places them.
    pub(crate) fn frame_changed(&mut self) -> Result<()> {
        for v in self.leaf_windows()? {
            self.window_mut(v)?.end_valid = false;
        }
        self.display_state.version += 1;
        let state = &self.window_state;
        let (width, height, line, column) = (state.width, state.height, state.line, state.column);
        if line > 0 && column > 0 && height > 0 {
            self.set_frame_geometry(width, height, line, column)?;
        }
        Ok(())
    }
    /// The host frame's tab bar lines, none on a frame without a window
    /// system; each is the frame's line tall (pgtk_change_tab_bar_height).
    pub(crate) fn tab_bar_lines(&self) -> u32 {
        if !self.window_system_open() { return 0; }
        self.frame(self.host_frame_peek()).map_or(0, |f| f.tab_bar_lines)
    }
    /// Whether the host frame shows a status bar where it fits:
    /// `romacs-status-bar-lines' on a window system.
    /// Whether the host frame's status bar is at its top (`status_bar_top').
    pub(crate) fn status_bar_top(&self) -> bool {
        self.frame(self.host_frame_peek()).is_ok_and(|f| f.status_bar_top)
    }
    pub(crate) fn status_bar_wanted(&self) -> bool {
        self.window_system_open() && self.frame(self.host_frame_peek()).is_ok_and(|f| f.status_bar_lines > 0)
    }
    /// The pixels the host frame's status bar takes above a mini-window
    /// MINI tall in a text area HEIGHT tall (VM2-STATUS-BAR): its measured
    /// row, else a canonical LINE, while it is wanted and the tab bar and
    /// two lines of the root window fit beside it; else none.
    pub(crate) fn status_bar_reservation(&self, height: u32, mini: u32, line: u32) -> u32 {
        let bar = if self.window_state.status_height > 0 { self.window_state.status_height } else { line };
        let needed = [mini, bar, self.tab_bar_lines() * line, 2 * line].map(u64::from).iter().sum::<u64>();
        if !self.status_bar_wanted() || u64::from(height) < needed { 0 } else { bar }
    }
    /// Whether the status bar is WINDOW's mode line (VM2-STATUS-BAR): an
    /// ordinary window of the host frame while the frame has the bar,
    /// which a window system's chrome gives it; such a window draws no
    /// band of its own. Batch frames have none, so GNU's geometry stands.
    pub(crate) fn bar_owns_mode_line(&self, window: Value) -> bool {
        let Ok(w) = self.window(window) else { return false };
        !w.mini && w.frame == self.host_frame_peek() && self.frame(w.frame).is_ok_and(|f| f.status_bar_lines > 0)
    }
    /// GNU window_wants_mode_line (window.c:6037): a live non-minibuffer
    /// window whose `mode-line-format' parameter is not `none', with that
    /// parameter or its buffer's `mode-line-format' non-nil, taller than
    /// one canonical line; and, in Romacs, none while the status bar is
    /// its mode line.
    pub(crate) fn window_wants_mode_line(&mut self, window: Value) -> Result<bool> {
        if self.bar_owns_mode_line(window) { return Ok(false); }
        Ok(self.window_mode_line_format(window)?.is_some_and(|_| self.window(window).is_ok_and(|w| w.height > self.window_state.line)))
    }
    /// The mode-line format WINDOW shows (xdisp.c:28149): its
    /// `mode-line-format' parameter when non-nil, else its buffer's; none
    /// for a mini-window, a parameter `none', or a nil format.
    /// Whether WINDOW shows the default `mode-line-format': no
    /// `mode-line-format' parameter, and its buffer has no local value.
    /// Only that format is Romacs's to replace (a pane label, VM2-STATUS-BAR):
    /// a buffer that sets its own, nil included, shows it as in GNU.
    pub(crate) fn window_shows_default_mode_line(&mut self, window: Value) -> Result<bool> {
        let key = self.intern("mode-line-format");
        let w = self.window(window)?;
        if w.parameters.iter().any(|(k, v)| *k == key && *v != Value::NIL) { return Ok(false); }
        let buffer = w.buffer;
        let slot = self.slot("mode-line-format");
        let slot = self.indirect_variable(slot);
        if let Some(crate::forwarded::Fwd::Buffer(i)) = self.forwarding(slot) {
            let index = buffer.index(0).map_err(|_| "buffer required")? - 1;
            return Ok(!self.per_buffer_local_p(index, i));
        }
        let Some(table) = self.local_table(slot)? else { return Ok(true) };
        Ok(self.declaration_hash_get(table, buffer)?.is_none())
    }
    pub(crate) fn window_mode_line_format(&mut self, window: Value) -> Result<Option<Value>> {
        let (key, none, slot) = (self.intern("mode-line-format"), self.intern("none"), self.slot("mode-line-format"));
        let w = self.window(window)?;
        if w.mini || !w.live() { return Ok(None); }
        let parameter = w.parameters.iter().find(|(k, _)| *k == key).map_or(Value::NIL, |(_, v)| *v);
        if parameter == none { return Ok(None); }
        if parameter != Value::NIL { return Ok(Some(parameter)); }
        let format = self.value_in_buffer(slot, w.buffer)?;
        Ok((format != Value::NIL && format != Value::UNBOUND).then_some(format))
    }
    /// Install canonical font metrics before startup Lisp observes window sizes.
    /// The display's device scale is SCALE (a HiDPI output's 2): images
    /// realize anew at it.  Whether it changed.
    pub fn set_device_scale(&mut self, scale: f32) -> bool {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        if scale == self.window_state.device_scale { return false; }
        self.window_state.device_scale = scale;
        // Every window publishes again, its images decoded at the new scale.
        self.display_state.free_realized_faces();
        true
    }
    pub fn set_frame_geometry(
        &mut self,
        width: u32,
        height: u32,
        line_height: u32,
        column_width: u32,
    ) -> Result<()> {
        self.ensure_windows()?;
        // GNU's windows tile the text area, inside the internal border.
        let (native_width, native_height) = (width, height);
        let (width, height) = self.host_inner(width, height).unwrap_or((0, 0));
        if line_height == 0
            || column_width == 0
            || u64::from(height) < 2 * u64::from(line_height)
            || u64::from(width) < 2 * u64::from(column_width)
        {
            return Err("frame geometry is below safe window minimum".into());
        }
        let root = self.window_root(ROOT);
        // A hosted mini-window takes none of the root windows' lines.
        let hosted = self.minibuffer_host().is_some();
        // A mini-window the root windows cannot make room for keeps one
        // line (window.c grow_mini_window, bug#69140).
        let mini_height = if hosted { 0 } else { self.window_state.mini_lines.saturating_mul(line_height).min(height - line_height) };
        let mut geometry = vec![];
        let now = self.mini_growth()?;
        let wanted = if hosted { 0 } else { i64::from(line_height) - i64::from(mini_height) };
        // The status bar takes a band above the mini-window, and the tab
        // bar the top of the frame (FRAME_TOP_MARGIN), as far as a line is
        // left for the root window.
        let status = self.status_bar_reservation(height, mini_height, line_height);
        let tab_bar = (self.tab_bar_lines() * line_height).min(height.saturating_sub(mini_height + status + line_height));
        // A bar at the top is under the tab bar, and the root window under it.
        let top = tab_bar + if self.status_bar_top() { status } else { 0 };
        let planned = self.plan_window_geometry(root, 0, top, width, height - mini_height - status - tab_bar, (now, wanted), line_height, column_width, &mut geometry);
        let mini_height = if planned.is_err() && mini_height > line_height {
            self.window_state.mini_lines = 1;
            geometry.clear();
            self.plan_window_geometry(root, 0, top, width, height - line_height - status - tab_bar, (now, 0), line_height, column_width, &mut geometry)?;
            line_height
        } else {
            planned?;
            mini_height
        };
        if !hosted {
            let mini = self.window_root(MINI);
            geometry.push((mini, 0, height - mini_height, width, mini_height));
        }
        self.window_state.mini_reserved = mini_height;
        self.window_state.status_reserved = status;
        for (v, x, y, w, h) in geometry {
            let window = self.window_mut(v)?;
            if (window.x, window.y, window.width, window.height) != (x, y, w, h) {
                window.end_valid = false;
            }
            window.x = x;
            window.y = y;
            window.width = w;
            window.height = h;
        }
        self.window_state.width = native_width;
        self.window_state.height = native_height;
        self.window_state.text_area = [width, height];
        self.window_state.line = line_height;
        self.window_state.column = column_width;
        self.display_state.width = native_width;
        self.display_state.height = native_height;
        Ok(())
    }
    /// GNU resize_frame_windows (window.c:5046) where window.el is loaded,
    /// before the host frame is laid out WIDTH by HEIGHT native pixels:
    /// along each axis its text area changed on, an internal root window
    /// resizes its children proportionally by `window--resize-root-window',
    /// with "normal" and then "reasonable" minimum sizes, and keeps the
    /// sizes when they tile it; `set_frame_geometry' then places them. A
    /// leaf root, a runtime without window.el, or sizes that do not fit
    /// are left to the planner. Runs Lisp, so only where the host resizes
    /// the frame, with no heap value held (`frame_window_snapshots', a
    /// host's resize event); a parameter's re-layout uses the planner.
    pub(crate) fn resize_root_window(&mut self, width: u32, height: u32) -> Result<()> {
        self.ensure_windows()?;
        let Some((width, height)) = self.host_inner(width, height) else { return Ok(()) };
        let [old_width, old_height] = self.window_state.text_area;
        self.resize_root_along(i64::from(width) - i64::from(old_width), true)?;
        self.resize_root_along(i64::from(height) - i64::from(old_height), false)
    }
    fn resize_root_along(&mut self, delta: i64, horizontal: bool) -> Result<()> {
        let w = self.window(self.window_root_peek(ROOT))?;
        let size = i64::from(if horizontal { w.width } else { w.height }) + delta;
        let edge = if horizontal { w.x } else { w.y };
        let (Ok(size), false, false) = (u32::try_from(size), delta == 0, w.live()) else { return Ok(()) };
        const RESIZE: &str = "(lambda (root delta horizontal ignore) (and (fboundp 'window--resize-root-window) (progn (window--resize-root-window root delta horizontal ignore t) t)))";
        let axis = if horizontal { Value::TRUE } else { Value::NIL };
        for ignore in [Value::NIL, Value::TRUE] {
            let root = self.window_root_peek(ROOT);
            match self.call_prepared_source_raw(RESIZE, &[root, Value::integer(delta)?, axis, ignore]) {
                Ok(called) if called != Value::NIL => {}
                _ => return Ok(()),
            }
            let root = self.window_root_peek(ROOT);
            if self.window(root)?.new_pixel == size && self.prepared_check(root, horizontal, Value::NIL)? {
                return self.apply_prepared(root, horizontal, edge);
            }
        }
        Ok(())
    }
    /// Plan V's subtree in the rect. Along the vertical axis a window's
    /// size is its stable size (with a one-line mini-window) plus a growth,
    /// the negative share of the rows a grown mini-window takes: GROWTH.0
    /// now, GROWTH.1 planned. A vertical combination recovers its
    /// children's stable sizes, rescales them to its new stable extent,
    /// and shares the planned growth among them by their normal sizes, as
    /// GNU's `window--resize-root-window-vertically' shares a mini-window's
    /// growth without changing normal sizes. A prompt's shrink and the
    /// restore after it so land on the same edges, where rescaling the
    /// shrunk pixel sizes would round a row away per prompt.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_window_geometry(
        &self,
        v: Value,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        growth: (i64, i64),
        line: u32,
        column: u32,
        out: &mut Vec<(Value, u32, u32, u32, u32)>,
    ) -> Result<()> {
        let w = self.window(v)?;
        if width < 2 * column || height < line {
            return Err("frame resize would violate safe window minimum".into());
        }
        out.push((v, x, y, width, height));
        let (now, wanted) = growth;
        let count = w.children.len();
        let shares = if w.horizontal { vec![] } else { self.growth_shares(&w.children)? };
        // The growth of children 0..=I: all of it from the last child.
        let share = |i: usize, growth: i64| if i + 1 == count { growth } else { (shares[i] * growth as f64).round() as i64 };
        let total = i64::from(if w.horizontal { width } else { height });
        let old_total = i64::from(if w.horizontal { w.width } else { w.height });
        let (stable_old, stable_new) = if w.horizontal { (old_total, total) } else { (old_total - now, total - wanted) };
        let (mut old_edge, mut edge, mut shared) = (0i64, 0i64, (0i64, 0i64));
        for (i, &child) in w.children.iter().enumerate() {
            let c = self.window(child)?;
            old_edge += i64::from(if w.horizontal { c.width } else { c.height });
            let (child_growth, next) = if w.horizontal {
                (growth, if i + 1 == count { total } else { old_edge * total / old_total.max(1) })
            } else {
                let upto = (share(i, now), share(i, wanted));
                let stable_edge = old_edge - upto.0;
                let next = if i + 1 == count { total } else { stable_edge * stable_new / stable_old.max(1) + upto.1 };
                let child = (upto.0 - shared.0, upto.1 - shared.1);
                shared = upto;
                (child, next)
            };
            let next = next.clamp(edge, total);
            let size = (next - edge) as u32;
            if w.horizontal {
                self.plan_window_geometry(child, x + edge as u32, y, size, height, child_growth, line, column, out)?;
            } else {
                self.plan_window_geometry(child, x, y + edge as u32, width, size, child_growth, line, column, out)?;
            }
            edge = next;
        }
        Ok(())
    }
    /// The cumulative normal sizes of a vertical combination's CHILDREN,
    /// or none (the last child takes all growth) when one is not a number.
    fn growth_shares(&self, children: &[Value]) -> Result<Vec<f64>> {
        let normals = children.iter().map(|&c| self.normal_size(c, false)).collect::<Result<Option<Vec<_>>>>()?;
        Ok(match normals {
            Some(normals) => normals.iter().scan(0.0, |sum, n| { *sum += n; Some(*sum) }).collect(),
            None => vec![0.0; children.len()],
        })
    }
    pub fn display_apply_layout(&mut self, ends: &[crate::WindowEnd]) -> Result<()> {
        // Validate the complete journal before publishing any per-window end.
        let (mut changes, mut bytes) = (vec![], vec![]);
        for &crate::WindowEnd { id, end, .. } in ends {
            let index = self.live_window_index(id).ok_or("layout journal names an unknown live window")?;
            let Object::Window(w) = &self.objects[index] else { unreachable!("live window") };
            let b = self.shown_text(w.buffer)?;
            // A display with overlay strings ends at a display byte; any
            // other, at a byte of the text it published.
            let end = end + self.display_state.base(id);
            let end = self.display_state.segments(id)
                .map_or(end, |segments| crate::display::shown_end(segments, end, b.len_bytes(), |at| b.newline_at(at)));
            let end_char = b.char_of_byte(coordinates::ByteOffset(end))?.0;
            changes.push((index, b.len_chars() - end_char));
            bytes.push(end);
        }
        self.display_state.hosted = true;
        for (end, &byte) in ends.iter().zip(&bytes) { self.display_state.painted_rows(end, byte); }
        for (index, distance) in changes {
            let Object::Window(w) = &mut self.objects[index] else {
                unreachable!()
            };
            w.end_distance = distance;
            w.end_valid = true;
        }
        Ok(())
    }
    pub(super) fn window_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        let valid = match p {
            SelectedWindow => n == 0,
            SetWindowPoint => n == 2,
            SetWindowStart | SetWindowMargins | SetWindowNewPixel => n >= 2 && n <= 3,
            SetWindowParameter => n == 3,
            SplitWindowInternal => n == 4 || n == 5,
            WindowTotalHeight | WindowTotalWidth | WindowResizeApply => {
                n <= 2
            }
            WindowParameter => n == 2,
            DeleteWindowInternal | Windowp | WindowLiveP => n == 1,
            _ => n <= 1,
        };
        if !valid {
            return Err("wrong primitive arity".into());
        }
        let arg = |i: usize| {
            if i < n {
                self.stack[start + i]
            } else {
                Value::NIL
            }
        };
        let (a, b, c, d, e) = (arg(0), arg(1), arg(2), arg(3), arg(4));
        if matches!(p, Windowp | WindowLiveP) {
            return Ok(
                if matches!(self.object_if(a),Some(Object::Window(w)) if p==Windowp || w.live()) {
                    Value::TRUE
                } else {
                    Value::NIL
                },
            );
        }
        self.ensure_windows()?;
        if matches!(
            p,
            SelectedWindow | FrameSelectedWindow | MinibufferWindow | FrameRootWindow
        ) {
            // frame-root-window and frame-selected-window take FRAME-OR-WINDOW:
            // a valid window stands for its frame (window.c WINDOW_VALID_P).
            let frame = match self.window(a) {
                Ok(w) if !w.dead && matches!(p, FrameSelectedWindow | FrameRootWindow) => w.frame,
                _ if p == SelectedWindow => self.frame_value_peek(),
                _ => self.frame_arg(a)?,
            };
            let f = self.frame(frame)?;
            return Ok(match p { MinibufferWindow => f.minibuffer, FrameRootWindow => f.root, _ => f.selected });
        }
        if p == WindowResizeApply {
            let frame = self.frame_arg(a)?;
            let root = self.frame(frame)?.root;
            let horizontal = b != Value::NIL;
            let size = if horizontal {
                self.window(root)?.width
            } else {
                self.window(root)?.height
            };
            if self.window(root)?.new_pixel != size
                || !self.prepared_check(root, horizontal, Value::NIL)?
            {
                return Ok(Value::NIL);
            }
            self.apply_prepared(root, horizontal, 0)?;
            return Ok(Value::TRUE);
        }
        let live = matches!(
            p,
            SetWindowPoint
                | SetWindowStart
                | WindowPoint
                | WindowStart
                | SetWindowMargins
                | WindowMargins
        );
        let dead_ok = matches!(
            p,
            DeleteWindowInternal | WindowParameter | SetWindowParameter | WindowBuffer
        );
        let v = self.decode_window(a, live, dead_ok)?;
        let value = match p {
            DeleteWindowInternal => self.delete_window_value(v)?,
            SplitWindowInternal => self.split_window_value(
                v,
                u32::try_from(b.as_integer()?).map_err(|_| "positive size required")?,
                c,
                d,
                e,
            )?,
            WindowBuffer => self.window(v)?.buffer,
            WindowPoint => self.window_point_value(v)?,
            SetWindowPoint => {
                if v == self.window_root(SELECTED) {
                    // CHECK_FIXNUM_COERCE_MARKER replaces POS when the
                    // window's buffer is not current, so that POS is returned.
                    let coerced = self.window(v)?.buffer != self.slots[self.buffer_slot].value;
                    let pos = if coerced && matches!(self.object_if(b), Some(Object::Marker(_))) { self.marker_position_value(b)? } else { b };
                    self.set_buffer_point_for_window(v, pos)?;
                    if coerced { pos } else { b }
                } else {
                    self.set_marker_object(self.window(v)?.point, b, self.window(v)?.buffer)?;
                    b
                }
            }
            WindowStart => self.marker_position_value(self.window(v)?.start)?,
            SetWindowStart => {
                self.set_marker_object(self.window(v)?.start, b, self.window(v)?.buffer)?;
                let w = self.window_mut(v)?;
                w.end_valid = false;
                // NOFORCE lets redisplay move the start to keep point visible.
                if c == Value::NIL {
                    w.force_start = true;
                }
                b
            }
            WindowParent => self.window(v)?.parent,
            WindowChild => self
                .window(v)?
                .children
                .first()
                .copied()
                .unwrap_or(Value::NIL),
            WindowNextSibling | WindowPrevSibling => {
                let parent = self.window(v)?.parent;
                if parent == Value::NIL {
                    Value::NIL
                } else {
                    let children = &self.window(parent)?.children;
                    let at = children.iter().position(|c| *c == v).unwrap();
                    let next = if p == WindowNextSibling {
                        at.checked_add(1)
                    } else {
                        at.checked_sub(1)
                    };
                    next.and_then(|i| children.get(i))
                        .copied()
                        .unwrap_or(Value::NIL)
                }
            }
            SetWindowNewPixel => {
                let base = if c == Value::NIL {
                    0
                } else {
                    self.window(v)?.new_pixel as i64
                };
                let pixels = base
                    .checked_add(b.as_integer()?)
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n <= i32::MAX as u32)
                    .ok_or("new pixel size out of range")?;
                self.window_mut(v)?.new_pixel = pixels;
                Value::integer(pixels as i64)?
            }
            WindowNewPixel => Value::integer(self.window(v)?.new_pixel as i64)?,
            WindowPixelHeight => Value::integer(self.window(v)?.height as i64)?,
            WindowPixelWidth => Value::integer(self.window(v)?.width as i64)?,
            WindowTotalHeight | WindowTotalWidth => {
                let pixels = if p == WindowTotalHeight {
                    self.window(v)?.height
                } else {
                    self.window(v)?.width
                };
                let unit = if p == WindowTotalHeight {
                    self.window_state.line
                } else {
                    self.window_state.column
                };
                let ceiling = self.display(b)? == "ceiling";
                Value::integer(if ceiling {
                    pixels.div_ceil(unit)
                } else {
                    pixels / unit
                } as i64)?
            }
            WindowParameter => self
                .window(v)?
                .parameters
                .iter()
                .find(|(k, _)| *k == b)
                .map(|(_, v)| *v)
                .unwrap_or(Value::NIL),
            SetWindowParameter => {
                let w = self.window_mut(v)?;
                if let Some((_, value)) = w.parameters.iter_mut().find(|(k, _)| *k == b) {
                    *value = c;
                } else {
                    w.parameters.insert(0, (b, c));
                }
                c
            }
            WindowParameters => {
                let params = self.window(v)?.parameters.clone();
                let pairs = params
                    .iter()
                    .map(|&(k, v)| {
                        self.alloc_cons(k, v)
                    })
                    .collect::<Vec<_>>();
                self.list_value(&pairs)
            }
            WindowMargins => {
                let w = self.window(v)?;
                let l = if w.left_margin == 0 {
                    Value::NIL
                } else {
                    Value::integer(w.left_margin as i64)?
                };
                let r = if w.right_margin == 0 {
                    Value::NIL
                } else {
                    Value::integer(w.right_margin as i64)?
                };
                self.alloc_cons(l, r)
            }
            SetWindowMargins => {
                let dim = |v: Value| -> Result<u32> {
                    if v == Value::NIL {
                        Ok(0)
                    } else {
                        Ok(u32::try_from(v.as_integer()?)
                            .ok()
                            .filter(|n| *n <= i32::MAX as u32)
                            .ok_or("margin out of range")?)
                    }
                };
                let (l, r) = (dim(b)?, dim(c)?);
                let col = self.window_state.column;
                let fringes = self.window_fringe_widths(v)?;
                let w = self.window_mut(v)?;
                if (u64::from(l) + u64::from(r) + 2) * u64::from(col) + u64::from(fringes[0] + fringes[1]) > u64::from(w.width)
                    || (l, r) == (w.left_margin, w.right_margin)
                {
                    Value::NIL
                } else {
                    w.left_margin = l;
                    w.right_margin = r;
                    w.end_valid = false;
                    Value::TRUE
                }
            }
            _ => return Err("unknown window primitive".into()),
        };
        if matches!(
            p,
            DeleteWindowInternal
                | SplitWindowInternal
                | SetWindowPoint
                | SetWindowStart
                | SetWindowParameter
                | SetWindowMargins
        ) {
            self.display_state.version += 1;
        }
        Ok(value)
    }
}

/// A live window as the display iterator sees it, in columns but for
/// the mode line's pixels and the frame's character size.
#[derive(Clone)]
pub(crate) struct TextArea {
    pub window: Value,
    pub buffer: Value,
    pub columns: i64,
    pub left_margin: i64,
    pub right_margin: i64,
    pub rightmost: bool,
    pub full_width: bool,
    /// GNU `w->hscroll`, and the columns init_iterator hides at each
    /// line's left (first_visible_x): `min_hscroll` under
    /// `auto-hscroll-mode' `current-line', else the hscroll.
    pub hscroll: i64,
    pub first_visible: i64,
    pub mode_line: i64,
    pub header_line: i64,
    pub column: i64,
    pub line: i64,
    /// A host has laid the frame out and measures for it (VM2-WTPS1):
    /// then MODE_LINE is the host's band, and these are the text area's
    /// pixel width and height and its vertical scroll.
    pub hosted: bool,
    pub body_width: i64,
    /// The right fringe's pixels, where a window-system frame shows the
    /// continuation and truncation indicators instead of glyphs.
    pub right_fringe: i64,
    pub body_height: i64,
    pub vscroll: i64,
}
/// The window surface `prims/window.rs` declares as primitives (GNU
/// window.c, frame.c and minibuf.c's `active-minibuffer-window`). Reached only
/// through `Leaf::windows`, so a body never holds `&mut Vm`; nothing here
/// evaluates Lisp or collects, and every Value it returns is already rooted
/// in a window, a slot or a fresh allocation.
pub(crate) struct Windows<'a> {
    pub(super) vm: &'a mut Vm,
}
type Out = std::result::Result<Value, crate::prim::Fault>;
use crate::prim::Fault;
fn internal(e: String) -> Fault {
    Fault::Error(e)
}
impl<'a> Windows<'a> {
    pub(crate) fn new(vm: &'a mut Vm) -> Self {
        Windows { vm }
    }
    fn frame(&mut self) -> Value {
        self.vm.selected_frame_value().expect("initial frame")
    }
    fn is_frame(&mut self, v: Value) -> bool {
        matches!(self.vm.object_if(v), Some(Object::Frame(_)))
    }
    /// GNU `decode_live_frame`: nil is the selected frame, else
    /// `CHECK_LIVE_FRAME` (`wrong-type-argument frame-live-p`).
    fn live_frame(&mut self, v: Value) -> std::result::Result<(), Fault> {
        if v == Value::NIL || matches!(self.vm.object_if(v), Some(Object::Frame(f)) if f.live) {
            Ok(())
        } else {
            Err(Fault::Wrong("frame-live-p", v))
        }
    }
    /// GNU `decode_any_frame`: `CHECK_FRAME` (`wrong-type-argument framep`).
    fn any_frame(&mut self, v: Value) -> std::result::Result<(), Fault> {
        if v == Value::NIL || self.is_frame(v) {
            Ok(())
        } else {
            Err(Fault::Wrong("framep", v))
        }
    }
    /// GNU `decode_valid_window` (nil is the selected window).
    fn valid(&mut self, v: Value) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        let w = if v == Value::NIL { self.vm.window_root(SELECTED) } else { v };
        match self.vm.object_if(w) {
            Some(Object::Window(x)) if !x.dead => Ok(w),
            _ => Err(Fault::Wrong("window-valid-p", v)),
        }
    }
    /// GNU `decode_live_window` (nil is the selected window).
    fn live(&mut self, v: Value) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        let w = if v == Value::NIL { self.vm.window_root(SELECTED) } else { v };
        match self.vm.object_if(w) {
            Some(Object::Window(x)) if x.live() => Ok(w),
            _ => Err(Fault::Wrong("window-live-p", v)),
        }
    }
    fn get(&self, v: Value) -> std::result::Result<&Window, Fault> {
        self.vm.window(v).map_err(internal)
    }
    fn get_mut(&mut self, v: Value) -> std::result::Result<&mut Window, Fault> {
        self.vm.window_mut(v).map_err(internal)
    }
    fn int(n: i64) -> Out {
        Value::integer(n).map_err(internal)
    }
    pub(crate) fn window_valid_p(&mut self, object: Value) -> Value {
        let valid = object != Value::NIL
            && matches!(self.vm.object_if(object), Some(Object::Window(w)) if !w.dead);
        if valid { Value::TRUE } else { Value::NIL }
    }
    pub(crate) fn window_frame(&mut self, window: Value) -> Out {
        let window = self.valid(window)?;
        Ok(self.get(window)?.frame)
    }
    pub(crate) fn window_minibuffer_p(&mut self, window: Value) -> Out {
        let w = self.valid(window)?;
        Ok(if self.get(w)?.mini { Value::TRUE } else { Value::NIL })
    }
    /// `window-top-child` (HORIZONTAL false) / `window-left-child` (true).
    pub(crate) fn first_child(&mut self, window: Value, horizontal: bool) -> Out {
        let v = self.valid(window)?;
        let w = self.get(v)?;
        Ok(match w.children.first() {
            Some(&c) if w.horizontal == horizontal => c,
            _ => Value::NIL,
        })
    }
    pub(crate) fn window_normal_size(&mut self, window: Value, horizontal: Value) -> Out {
        let v = self.valid(window)?;
        let w = self.get(v)?;
        Ok(if horizontal == Value::NIL { w.normal_lines } else { w.normal_cols })
    }
    pub(crate) fn window_new_total(&mut self, window: Value) -> Out {
        let v = self.valid(window)?;
        Ok(self.get(v)?.new_total)
    }
    pub(crate) fn window_new_normal(&mut self, window: Value) -> Out {
        let v = self.valid(window)?;
        Ok(self.get(v)?.new_normal)
    }
    /// window.c:4667: `CHECK_FIXNUM (size)`; ADD sums with the old value.
    pub(crate) fn set_window_new_total(&mut self, window: Value, size: Value, add: Value) -> Out {
        let w = self.valid(window)?;
        let n = size.integer_value().ok_or(Fault::Wrong("fixnump", size))?;
        let total = if add == Value::NIL {
            size
        } else {
            let old = self.get(w)?.new_total.integer_value().unwrap_or(0);
            Self::int(old.checked_add(n).ok_or(Fault::Wrong("fixnump", size))?)?
        };
        self.get_mut(w)?.new_total = total;
        Ok(total)
    }
    pub(crate) fn set_window_new_normal(&mut self, window: Value, size: Value) -> Out {
        let w = self.valid(window)?;
        self.get_mut(w)?.new_normal = size;
        Ok(size)
    }
    /// window.c:5003. VM2 derives a window's total size from its pixel size
    /// (`window-total-height`), so there is no separate total to install:
    /// this validates FRAME and returns t (DIVERGENCES.md VM2-WIN1).
    pub(crate) fn window_resize_apply_total(&mut self, frame: Value) -> Out {
        self.live_frame(frame)?;
        Ok(Value::TRUE)
    }
    pub(crate) fn window_use_time(&mut self, window: Value) -> Out {
        let w = self.live(window)?;
        Self::int(self.get(w)?.use_time as i64)
    }
    pub(crate) fn window_dedicated_p(&mut self, window: Value) -> Out {
        let w = self.live(window)?;
        Ok(self.get(w)?.dedicated)
    }
    /// Fwindow_display_table: the display table of a live WINDOW.
    pub(crate) fn window_display_table(&mut self, window: Value) -> Out {
        let w = self.live(window)?;
        Ok(self.get(w)?.display_table)
    }
    /// Fset_window_display_table: TABLE, stored on a live WINDOW.
    pub(crate) fn set_window_display_table(&mut self, window: Value, table: Value) -> Out {
        let w = self.live(window)?;
        self.vm.window_mut(w).map_err(internal)?.display_table = table;
        Ok(table)
    }
    /// Fset_window_dedicated_p: FLAG, stored on a live WINDOW.
    pub(crate) fn set_window_dedicated_p(&mut self, window: Value, flag: Value) -> Out {
        let w = self.live(window)?;
        self.vm.window_mut(w).map_err(internal)?.dedicated = flag;
        Ok(flag)
    }
    /// GNU window.c:4492 force-window-update: nil repaints every window, a
    /// live window itself, a buffer (or its name) every window showing it;
    /// t when something was marked, nil for anything else (no error).
    pub(crate) fn force_window_update(&mut self, object: Value) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        if object == Value::NIL {
            self.vm.display_state.free_realized_faces();
            return Ok(Value::TRUE);
        }
        if let Some(Object::Window(w)) = self.vm.object_if(object) && w.live() {
            let id = w.id;
            self.vm.display_state.repaint_window(id);
            return Ok(Value::TRUE);
        }
        let buffer = match self.vm.buffer_name_text(object) {
            Some(name) => self.vm.named_buffer(&name).unwrap_or(Value::NIL),
            None => object,
        };
        if !matches!(self.vm.object_if(buffer), Some(Object::Buffer(b)) if b.state.live) { return Ok(Value::NIL); }
        let selected = self.vm.window_root(SELECTED);
        let (_, windows) = self.candidates(selected, Value::TRUE, Value::TRUE)?;
        let ids: Vec<u64> = windows.into_iter().filter_map(|w| self.get(w).ok().filter(|w| w.live() && w.buffer == buffer).map(|w| w.id)).collect();
        for &id in &ids { self.vm.display_state.repaint_window(id); }
        Ok(if ids.is_empty() { Value::NIL } else { Value::TRUE })
    }
    /// GNU dispnew.c:3252 redraw-display: clear and redisplay every frame.
    pub(crate) fn redraw_display(&mut self) -> Out {
        self.vm.display_state.free_realized_faces();
        Ok(Value::NIL)
    }
    /// Decorations VM2 does not draw: tab lines and scroll bars are 0
    /// pixels (window.c:1224-1269 on a live window).
    pub(crate) fn live_zero(&mut self, window: Value) -> Out {
        self.live(window)?;
        Ok(Value::integer(0).unwrap())
    }
    /// window.c:1242/1251: a live WINDOW's right (SIDE 0) or bottom divider.
    pub(crate) fn window_divider_width(&mut self, window: Value, side: usize) -> Out {
        let w = self.live(window)?;
        Self::int(i64::from(self.vm.window_dividers(w).map_err(internal)?[side]))
    }
    /// window.c:1215 `WINDOW_MODE_LINE_HEIGHT` over `window_wants_mode_line`
    /// (window.c:6037): a live non-minibuffer window whose `mode-line-format`
    /// parameter is not `none`, with that parameter or `mode-line-format`
    /// non-nil, taller than one canonical line. `mode-line-format` is the
    /// window's buffer's, as display.rs paints it. VM2 draws a mode line one
    /// canonical line tall (DIVERGENCES.md VM2-WIN3).
    pub(crate) fn window_mode_line_height(&mut self, window: Value) -> Out {
        let w = self.live(window)?;
        let wants = self.vm.window_wants_mode_line(w).map_err(internal)?;
        Self::int(if wants { i64::from(self.vm.window_state.line) } else { 0 })
    }
    /// The header line window_wants_header_line (window.c:6067) gives W, a
    /// live window: its `header-line-format' parameter, else its buffer's
    /// `header-line-format', unless W is a minibuffer window, the parameter
    /// is `none', or W is no taller than its mode line and one canonical
    /// line.  nil when W wants none.
    pub(crate) fn header_line_format(&mut self, w: Value) -> std::result::Result<Value, Fault> {
        let mode_line = self.window_mode_line_height(w)?.integer_value().unwrap_or(0) != 0;
        let (key, none, slot) = (self.vm.intern("header-line-format"), self.vm.intern("none"), self.vm.slot("header-line-format"));
        let line = self.vm.window_state.line;
        let x = self.get(w)?;
        let (mini, height, buffer) = (x.mini, x.height, x.buffer);
        let parameter = x.parameters.iter().find(|(k, _)| *k == key).map_or(Value::NIL, |(_, v)| *v);
        let format = if parameter != Value::NIL { parameter } else { self.vm.value_in_buffer(slot, buffer).map_err(internal)? };
        let tall = height > if mode_line { 2 * line } else { line };
        Ok(if mini || parameter == none || format == Value::UNBOUND || !tall { Value::NIL } else { format })
    }
    /// WINDOW_HEADER_LINE_HEIGHT (window.h): one canonical line, or once a
    /// host lays the frame out, the height it gives the header line.
    pub(crate) fn window_header_line_height(&mut self, window: Value) -> Out {
        let w = self.live(window)?;
        if self.header_line_format(w)? == Value::NIL { return Self::int(0); }
        let height = if self.vm.display_state.hosted && self.vm.has_layout_host() {
            self.vm.host_band_height(w, true).map_err(internal)?
        } else {
            self.vm.window_state.line
        };
        Self::int(i64::from(height))
    }
    /// window_body_width / window_body_height (window.c): WINDOW's text
    /// area in canonical characters (PIXELWISE nil, or `remap' with no face
    /// remapping) or pixels. The width loses the margins, the fringes on a
    /// window system's frame and, on another frame, the column of the
    /// vertical bar a window left of another has; the height loses the mode
    /// line. VM2 has no scroll bars, dividers or tab lines; the height also
    /// loses the header line.
    /// window_pixel_left / window_pixel_top: the window's frame-relative
    /// pixel edge.
    /// The refusal for a question about what a host has laid out, when no
    /// host layout is attached to answer it (VM2-POSVIS1).
    pub(crate) fn displayed_window(caller: &str) -> Fault {
        internal(format!("VM2 {caller} in a displayed window is unsupported"))
    }
    /// Fpos_visible_in_window_p's checks before pos_visible_p: the live
    /// window, the position to ask about (-1 for t) and the window start,
    /// or None when
    /// nothing is visible -- POS above the window start or outside the
    /// accessible text, or the initial frame, which no host lays out.
    /// A displayed window with no host layout refuses as CALLER.
    pub(crate) fn pos_visible(&mut self, pos: Value, window: Value, caller: &str) -> std::result::Result<Option<(Value, i64, i64)>, Fault> {
        let w = self.live(window)?;
        let posint = if pos == Value::TRUE {
            None
        } else if pos == Value::NIL && w == self.vm.window_root(SELECTED) {
            // PT: the current buffer's point, whichever buffer W shows.
            let index = self.vm.buffer_index().map_err(internal)?;
            Some(match &self.vm.objects[index] {
                Object::Buffer(b) => b.char_of_byte(b.point).map_err(internal)?.0 as i64 + 1,
                Object::ByteBuffer(b) => b.point.0 as i64 + 1,
                _ => return Err(internal("buffer required".into())),
            })
        } else if pos == Value::NIL {
            let point = self.get(w)?.point;
            Some(self.vm.marker_position_value(point).map_err(internal)?.as_integer().map_err(internal)?)
        } else {
            Some(match self.vm.object_if(pos) {
                Some(Object::Marker(_)) => {
                    let at = self.vm.marker_position_value(pos).map_err(internal)?;
                    at.as_integer().map_err(|_| internal("Marker does not point anywhere".into()))?
                }
                // fix_position saturates a bignum.
                Some(Object::Bignum(n)) => if n.sign() == num_bigint::Sign::Minus { i64::MIN } else { i64::MAX },
                _ => pos.as_integer().map_err(|_| Fault::Wrong("integer-or-marker-p", pos))?,
            })
        };
        let (start, buffer) = { let x = self.get(w)?; (x.start, x.buffer) };
        let top = self.vm.marker_position_value(start).map_err(internal)?.as_integer().map_err(internal)?;
        // A unibyte buffer's positions are its bytes'.
        let (begv, zv) = match self.vm.object(buffer).map_err(internal)? {
            Object::Buffer(b) => {
                let accessible = b.accessible();
                let char_at = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(internal);
                (char_at(accessible.start)?, char_at(accessible.end)?)
            }
            Object::ByteBuffer(b) => (b.restriction.start.0 as i64 + 1, b.restriction.end.0 as i64 + 1),
            _ => return Err(internal("buffer required".into())),
        };
        if posint.is_some_and(|p| p < top || p > zv) || top < begv || top > zv || !self.vm.display_state.hosted {
            return Ok(None);
        }
        if !self.vm.has_layout_host() { return Err(Self::displayed_window(caller)); }
        Ok(Some((w, posint.unwrap_or(-1), top)))
    }
    /// Fwindow_end (window.c:1782) for WINDOW: the live window, the end its
    /// last layout recorded (Z less the distance left after its last row),
    /// and its start clipped to the accessible region
    /// (CLIP_TEXT_POS_FROM_MARKER).
    pub(crate) fn window_end(&mut self, window: Value) -> std::result::Result<(Value, i64, i64), Fault> {
        let w = self.live(window)?;
        let (start, buffer, distance) = { let x = self.get(w)?; (x.start, x.buffer, x.end_distance as i64) };
        let top = self.vm.marker_position_value(start).map_err(internal)?.as_integer().map_err(internal)?;
        self.vm.mirror_shown(buffer).map_err(internal)?;
        let b = self.vm.shown_text(buffer).map_err(internal)?;
        let accessible = b.accessible();
        let char_at = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(internal);
        let (begv, zv) = (char_at(accessible.start)?, char_at(accessible.end)?);
        Ok((w, b.len_chars() as i64 + 1 - distance, top.clamp(begv, zv)))
    }
    /// decode_live_window and what xdisp.c init_iterator reads of the
    /// window. The terminal frame model holds until a host lays the frame
    /// out; then the host's layout answers, if one is attached (VM2-WTPS1).
    pub(crate) fn text_area(&mut self, window: Value, caller: &str) -> std::result::Result<TextArea, Fault> {
        let w = self.live(window)?;
        let hosted = self.vm.display_state.hosted;
        if hosted && !self.vm.has_layout_host() { return Err(Self::displayed_window(caller)); }
        let buffer = self.get(w)?.buffer;
        self.vm.mirror_shown(buffer).map_err(internal)?;
        let mut mode_line = self.window_mode_line_height(w)?.integer_value().unwrap_or(0);
        if hosted && mode_line != 0 {
            mode_line = i64::from(self.vm.host_band_height(w, false).map_err(internal)?);
        }
        let header_line = self.window_header_line_height(w)?.integer_value().unwrap_or(0);
        let (column, line) = (i64::from(self.vm.window_state.column), i64::from(self.vm.window_state.line));
        let root = self.vm.root_of(w).map_err(internal)?;
        let (root_x, root_width) = self.vm.window(root).map_or((0, 0), |r| (i64::from(r.x), i64::from(r.width)));
        let fringes = self.vm.window_fringe_widths(w).map_err(internal)?;
        let [right_divider, bottom_divider] = self.vm.window_dividers(w).map_err(internal)?.map(i64::from);
        let buffer = self.get(w)?.buffer;
        let current_line = self.vm.hscrolls_current_line(w, buffer).map_err(internal)?;
        let x = self.get(w)?;
        let first_visible = if current_line { x.min_hscroll } else { x.hscroll };
        let (left, width) = (i64::from(x.x), i64::from(x.width));
        let body_width = (width - i64::from(x.left_margin + x.right_margin) * column - i64::from(fringes[0] + fringes[1]) - right_divider).max(0);
        let body_height = (i64::from(x.height) - mode_line - header_line - bottom_divider).max(0);
        Ok(TextArea {
            window: w,
            buffer: x.buffer,
            columns: width / column,
            left_margin: i64::from(x.left_margin),
            right_margin: i64::from(x.right_margin),
            rightmost: left + width >= root_x + root_width,
            full_width: left == root_x && width == root_width,
            hscroll: x.hscroll,
            first_visible,
            mode_line,
            header_line,
            column,
            line,
            hosted,
            body_width,
            body_height,
            right_fringe: i64::from(fringes[1]),
            vscroll: i64::from(x.vscroll_px),
        })
    }
    /// window.c:1017/1029: WINDOW's left column or top line, its pixel
    /// edge in the frame's characters.
    pub(crate) fn window_char_edge(&mut self, window: Value, top: bool) -> Out {
        let w = self.valid(window)?;
        let x = self.get(w)?;
        let (column, line) = (self.vm.window_state.column.max(1), self.vm.window_state.line.max(1));
        Value::integer(i64::from(if top { x.y / line } else { x.x / column })).map_err(internal)
    }
    /// window.c Fwindow_scroll_bars: VM2 windows take the frame's scroll
    /// bars, and VM2 frames draw none, so every live window answers GNU's
    /// (WIDTH COLS VERTICAL-TYPE HEIGHT LINES HORIZONTAL-TYPE PERSISTENT)
    /// for that: (nil 0 t nil 0 t nil).
    pub(crate) fn window_scroll_bars(&mut self, window: Value) -> Out {
        self.live(window)?;
        let zero = Value::integer(0).map_err(internal)?;
        Ok(self.vm.list_value(&[Value::NIL, zero, Value::TRUE, Value::NIL, zero, Value::TRUE, Value::NIL]))
    }
    pub(crate) fn window_pixel_edge(&mut self, window: Value, top: bool) -> Out {
        let w = self.valid(window)?;
        let x = self.get(w)?;
        Value::integer(i64::from(if top { x.y } else { x.x })).map_err(internal)
    }
    pub(crate) fn window_body_size(&mut self, window: Value, pixelwise: Value, width: bool) -> Out {
        let w = self.live(window)?;
        let (column, line) = (i64::from(self.vm.window_state.column), i64::from(self.vm.window_state.line));
        let remap = pixelwise == self.vm.intern("remap");
        if remap {
            let slot = self.vm.slot("face-remapping-alist");
            let alist = self.vm.variable_value(slot).map_err(internal)?;
            if alist != Value::NIL && alist != Value::UNBOUND {
                return Err(Fault::Error("VM2 window body sizes in remapped face characters are unsupported".into()));
            }
        }
        let px = self.text_area_pixels(w, width, true)?;
        let chars = pixelwise == Value::NIL || remap;
        Self::int((if chars { px / if width { column } else { line } } else { px }).max(0))
    }
    /// Fwindow_text_width and Fwindow_text_height (window.c:7459, 7481):
    /// WINDOW's text area (window_box_width, window_box_height) in pixels
    /// with PIXELWISE, else in whole columns or lines of its frame.
    pub(crate) fn window_text_size(&mut self, window: Value, pixelwise: Value, width: bool) -> Out {
        let w = self.live(window)?;
        let unit = i64::from(if width { self.vm.window_state.column } else { self.vm.window_state.line }).max(1);
        let px = self.text_area_pixels(w, width, false)?;
        Self::int((if pixelwise == Value::NIL { px / unit } else { px }).max(0))
    }
    /// W's text area in pixels: its width less its margins, fringes and
    /// right divider, and with BORDER, on a frame without a window system,
    /// the column of the bar right of a window that is not rightmost
    /// (window_body_width, window.c:1095; window_box_width keeps it); or
    /// its height less its mode and header lines and bottom divider.
    fn text_area_pixels(&mut self, w: Value, width: bool, border: bool) -> std::result::Result<i64, Fault> {
        let column = i64::from(self.vm.window_state.column);
        Ok(if width {
            let (right, margins, pixels) = {
                let x = self.get(w)?;
                (i64::from(x.x) + i64::from(x.width), i64::from(x.left_margin + x.right_margin) * column, i64::from(x.width))
            };
            let root = self.vm.root_of(w).map_err(internal)?;
            let rightmost = self.vm.window(root).map_or(true, |r| right >= i64::from(r.x) + i64::from(r.width));
            let bar = if border && !rightmost && !self.vm.window_system_open() { column } else { 0 };
            let fringes = self.vm.window_fringe_widths(w).map_err(internal)?;
            let [divider, _] = self.vm.window_dividers(w).map_err(internal)?;
            pixels - bar - margins - i64::from(fringes[0] + fringes[1] + divider)
        } else {
            let mode_line = self.window_mode_line_height(w)?.integer_value().unwrap_or(0);
            let header_line = self.window_header_line_height(w)?.integer_value().unwrap_or(0);
            let [_, divider] = self.vm.window_dividers(w).map_err(internal)?;
            i64::from(self.get(w)?.height) - mode_line - header_line - i64::from(divider)
        })
    }
    /// window.c:8639: (LEFT RIGHT OUTSIDE-MARGINS PERSISTENT), the widths
    /// the window draws.
    pub(crate) fn window_fringes(&mut self, window: Value) -> Out {
        let w = self.live(window)?;
        let [left, right] = self.vm.window_fringe_widths(w).map_err(internal)?;
        let x = self.get(w)?;
        let flag = |b: bool| if b { Value::TRUE } else { Value::NIL };
        let (outside, persistent) = (flag(x.fringes_outside_margins), flag(x.fringes_persistent));
        Ok(self.vm.list_value(&[Value::integer(i64::from(left)).unwrap(), Value::integer(i64::from(right)).unwrap(), outside, persistent]))
    }
    /// set_window_fringes (window.c:8544) on a live WINDOW: nothing on a
    /// frame without a window system; new widths only when the text area
    /// keeps two columns; t when anything changed.
    pub(crate) fn set_window_fringes(&mut self, window: Value, left: Value, right: Value, outside: Value, persistent: Value) -> Out {
        let w = self.live(window)?;
        Ok(if self.vm.set_window_fringes(w, [left, right], outside != Value::NIL, Some(persistent != Value::NIL)).map_err(internal)? { Value::TRUE } else { Value::NIL })
    }
    /// frame.c:4358/4376: VM2 reports backend LINE_HEIGHT / COLUMN_WIDTH,
    /// even though frame-live-p returns t (DIVERGENCES.md VM2-WIN5).
    pub(crate) fn frame_char_size(&mut self, frame: Value, horizontal: bool) -> Out {
        self.any_frame(frame)?;
        let unit = if horizontal { self.vm.window_state.column } else { self.vm.window_state.line };
        Self::int(i64::from(unit))
    }
    /// frame.c:4531/4538: FRAME's right (SIDE 0) or bottom divider width.
    pub(crate) fn frame_divider_width(&mut self, frame: Value, side: usize) -> Out {
        self.any_frame(frame)?;
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        Self::int(i64::from(self.vm.frame(frame).map_err(internal)?.dividers[side]))
    }
    /// frame.c:4524: the host frame's internal border. A child frame's
    /// windows are placed inside its border already (VM2-CHILD-FRAME), so
    /// it answers 0 as before.
    pub(crate) fn frame_internal_border_width(&mut self, frame: Value) -> Out {
        self.any_frame(frame)?;
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        Self::int(i64::from(if self.is_child_frame(frame)? { 0 } else { self.vm.frame(frame).map_err(internal)?.border }))
    }
    /// gui_set_right_divider_width / gui_set_bottom_divider_width
    /// (frame.c:5862): FRAME's divider on SIDE (0 right) becomes WIDTH and
    /// its windows' bodies are laid out again; the native size stays.
    pub(crate) fn set_frame_divider(&mut self, frame: Value, side: usize, width: u32) -> std::result::Result<(), Fault> {
        let frame = self.decode_live_frame(frame)?;
        let f = self.vm.frame_mut(frame).map_err(internal)?;
        if f.dividers[side] == width { return Ok(()); }
        f.dividers[side] = width;
        self.vm.frame_changed().map_err(internal)
    }
    /// x_set_internal_border_width (xfns.c:1997) on the host frame: its
    /// windows are laid out again inside BORDER, the native size kept as
    /// a tiled or maximized frame keeps it. A child frame applies its
    /// border with its geometry (VM2-CHILD-FRAME).
    pub(crate) fn set_frame_internal_border(&mut self, frame: Value, border: u32) -> std::result::Result<(), Fault> {
        let frame = self.decode_live_frame(frame)?;
        if self.is_child_frame(frame)? { return Ok(()); }
        let old = std::mem::replace(&mut self.vm.frame_mut(frame).map_err(internal)?.border, border);
        if old == border { return Ok(()); }
        // A border its windows cannot fit inside is refused, the old one and
        // a grown mini-window's lines kept (set_frame_geometry runs no Lisp,
        // so FRAME stays valid).
        let mini_lines = self.vm.window_state.mini_lines;
        self.vm.frame_changed().or_else(|error| {
            self.vm.frame_mut(frame).map_err(internal)?.border = old;
            self.vm.window_state.mini_lines = mini_lines;
            self.vm.frame_changed().map_err(internal)?;
            Err(internal(error))
        })
    }
    /// frame.c:327: VM2 returns the initial/termcap type t, but retains
    /// graphical character metrics (documented hybrid, VM2-WIN5).
    /// decode_live_frame: the selected frame for nil, else FRAME if live.
    pub(crate) fn decode_live_frame(&mut self, frame: Value) -> std::result::Result<Value, Fault> {
        self.live_frame(frame)?;
        Ok(if frame == Value::NIL { self.frame() } else { frame })
    }
    /// GNU frame.c:2103 select-frame through do_switch_frame: FRAME's
    /// selected window becomes the selected window.
    pub(crate) fn select_frame(&mut self, frame: Value, norecord: Value) -> Out {
        if self.frame_live_p(frame) == Value::NIL {
            return Err(Fault::Wrong("frame-live-p", frame));
        }
        if frame != self.frame() {
            self.vm.select_frame_value(frame, norecord == Value::NIL).map_err(internal)?;
            self.vm.display_state.version += 1;
        }
        Ok(frame)
    }
    /// frame.c:326: a live frame's type (framep), else nil.
    pub(crate) fn frame_live_p(&mut self, object: Value) -> Value {
        if matches!(self.vm.object_if(object), Some(Object::Frame(f)) if f.live) { self.vm.frame_type() } else { Value::NIL }
    }
    /// The frame's parameter alist entries, most recently added first
    /// (frame.c f->param_alist). FRAME nil is the selected frame.
    pub(crate) fn frame_parameter_entries(&mut self, frame: Value) -> std::result::Result<Vec<(Value, Value)>, Fault> {
        self.any_frame(frame)?;
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        Ok(self.vm.frame(frame).map_err(internal)?.parameters.clone())
    }
    pub(crate) fn check_live_frame(&mut self, frame: Value) -> std::result::Result<(), Fault> {
        self.live_frame(frame)
    }
    /// frame.c:4381 Fframe_native_width and :4401 Fframe_native_height:
    /// the frame's size in the pixels its windows are measured in, as the
    /// host or the batch runner set it (DIVERGENCES.md VM2-WIN5).
    pub(crate) fn frame_native_size(&mut self, frame: Value, horizontal: bool) -> Out {
        self.any_frame(frame)?;
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        let (width, height) = self.vm.frame_native(frame).map_err(internal)?;
        Self::int(i64::from(if horizontal { width } else { height }))
    }
    /// FRAME's text area in pixels: its native size inside a child
    /// frame's border.
    pub(crate) fn frame_text_pixels(&mut self, frame: Value) -> std::result::Result<(u32, u32), Fault> {
        let frame = self.decode_live_frame(frame)?;
        let (width, height) = self.vm.frame_native(frame).map_err(internal)?;
        let border = self.vm.frame(frame).map_err(internal)?.border;
        Ok((width - 2 * border, height - 2 * border))
    }
    /// FRAME_TEXT_WIDTH and FRAME_TEXT_HEIGHT: FRAME's text pixels less
    /// its fringes on a window-system frame and the host frame's tab bar.
    pub(crate) fn frame_text_area(&mut self, frame: Value) -> std::result::Result<(u32, u32), Fault> {
        let (width, height) = self.frame_text_pixels(frame)?;
        let height = if self.is_child_frame(frame)? { height } else { height.saturating_sub(self.vm.tab_bar_lines() * self.vm.window_state.line) };
        let ([left, right], _) = self.frame_chrome(frame)?;
        Ok((if self.vm.window_system_open() { width.saturating_sub(left + right) } else { width }, height))
    }
    /// FRAME_COLS and FRAME_LINES of FRAME's text area.
    pub(crate) fn frame_text_size(&mut self, frame: Value) -> std::result::Result<(i64, i64), Fault> {
        let (width, height) = self.frame_text_area(frame)?;
        let state = &self.vm.window_state;
        Ok((i64::from(width / state.column.max(1)), i64::from(height / state.line.max(1))))
    }
    /// frame.c store_frame_param: replace an existing entry's value, else
    /// push a new entry onto the front.
    pub(crate) fn store_frame_parameter(&mut self, frame: Value, key: Value, value: Value) -> std::result::Result<(), Fault> {
        self.live_frame(frame)?;
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        let parameters = &mut self.vm.frame_mut(frame).map_err(internal)?.parameters;
        match parameters.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => *v = value,
            None => parameters.insert(0, (key, value)),
        }
        Ok(())
    }
    /// gui_set_left_fringe / gui_set_right_fringe (frame.c:5800): FRAME's
    /// fringe on SIDE (0 left) becomes WIDTH pixels, and its windows
    /// without widths of their own are laid out again.
    /// A child frame keeps its text area and widens.
    pub(crate) fn set_frame_fringe(&mut self, frame: Value, side: usize, width: u32) -> std::result::Result<(), Fault> {
        let frame = self.decode_live_frame(frame)?;
        if self.vm.frame(frame).map_err(internal)?.fringes[side] == width { return Ok(()); }
        let child = self.is_child_frame(frame)?.then(|| self.vm.child_geometry(frame)).transpose().map_err(internal)?;
        self.vm.frame_mut(frame).map_err(internal)?.fringes[side] = width;
        match child {
            Some(geometry) => self.vm.set_child_geometry(frame, geometry).map_err(internal),
            None => self.vm.frame_changed().map_err(internal),
        }
    }
    /// pgtk_set_tab_bar_lines (pgtkfns.c:444): the host frame's tab bar
    /// becomes LINES lines tall and its windows are laid out below it. A
    /// child frame keeps the parameter only.
    pub(crate) fn set_frame_tab_bar_lines(&mut self, frame: Value, lines: u32) -> std::result::Result<(), Fault> {
        let frame = self.decode_live_frame(frame)?;
        if self.is_child_frame(frame)? { return Ok(()); }
        let f = self.vm.frame_mut(frame).map_err(internal)?;
        if f.tab_bar_lines == lines { return Ok(()); }
        f.tab_bar_lines = lines;
        self.vm.frame_changed().map_err(internal)
    }
    /// The host frame's status bar becomes LINES (0 or 1) lines and its
    /// windows are laid out above it. A child frame keeps the parameter
    /// only.
    pub(crate) fn set_frame_status_bar_lines(&mut self, frame: Value, lines: u32) -> std::result::Result<(), Fault> {
        let frame = self.decode_live_frame(frame)?;
        if self.is_child_frame(frame)? { return Ok(()); }
        let f = self.vm.frame_mut(frame).map_err(internal)?;
        if f.status_bar_lines == lines { return Ok(()); }
        f.status_bar_lines = lines;
        self.vm.frame_changed().map_err(internal)
    }
    /// The host frame's status bar goes to its top when TOP, else above its
    /// mini-window; the windows are laid out again around it. A child
    /// frame keeps the parameter only.
    pub(crate) fn set_frame_status_bar_top(&mut self, frame: Value, top: bool) -> std::result::Result<(), Fault> {
        let frame = self.decode_live_frame(frame)?;
        if self.is_child_frame(frame)? { return Ok(()); }
        let f = self.vm.frame_mut(frame).map_err(internal)?;
        if f.status_bar_top == top { return Ok(()); }
        f.status_bar_top = top;
        self.vm.frame_changed().map_err(internal)
    }
    /// The host frame's tab bar lines as the display shows them.
    pub(crate) fn tab_bar_lines(&mut self) -> u32 {
        self.vm.tab_bar_lines()
    }
    /// FRAME's fringe widths and tab bar lines.
    pub(crate) fn frame_chrome(&mut self, frame: Value) -> std::result::Result<([u32; 2], u32), Fault> {
        let frame = self.decode_live_frame(frame)?;
        let f = self.vm.frame(frame).map_err(internal)?;
        Ok((f.fringes, f.tab_bar_lines))
    }
    /// FRAME's background opacity becomes ALPHA.
    pub(crate) fn set_frame_background_alpha(&mut self, frame: Value, alpha: f32) -> std::result::Result<(), Fault> {
        self.live_frame(frame)?;
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        self.vm.frame_mut(frame).map_err(internal)?.alpha_background = alpha;
        Ok(())
    }
    /// The window system's frame alpha hook (frame.c:6088) for FRAME:
    /// ALPHA while active and inactive, LIMIT the lower limit it reads.
    pub(crate) fn set_frame_alpha(&mut self, frame: Value, alpha: [f32; 2], limit: f32) -> std::result::Result<(), Fault> {
        self.live_frame(frame)?;
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        let id = self.vm.frame(frame).map_err(internal)?.id;
        if let Some(host) = self.vm.window_system_host() { host.set_frame_alpha(id, alpha, limit); }
        Ok(())
    }
    /// frame.c:286: the frame's type, t on the initial terminal and the
    /// display's backend once the session opened one (display_host.rs).
    pub(crate) fn frame_p(&mut self, object: Value) -> Value {
        if matches!(self.vm.object_if(object), Some(Object::Frame(_))) { self.vm.frame_type() } else { Value::NIL }
    }
    /// frame.c:415 Fwindow_system: the live frame's window system, nil on
    /// the initial terminal.
    pub(crate) fn window_system(&mut self, frame: Value) -> Out {
        self.live_frame(frame)?;
        let kind = self.vm.frame_type();
        Ok(if kind == Value::TRUE { Value::NIL } else { kind })
    }
    /// frame.c:2161: the live frames, newest first.
    pub(crate) fn frame_list(&mut self) -> Out {
        self.frame();
        let frames = self.vm.frames();
        Ok(self.vm.list_value(&frames))
    }
    pub(crate) fn last_nonminibuffer_frame(&mut self) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        let i = self.vm.slot("#:vm2-last-nonminibuffer-frame");
        Ok(self.vm.slots[i].value)
    }
    pub(crate) fn frame_terminal(&mut self, frame: Value) -> Out {
        self.live_frame(frame)?;
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        Ok(self.vm.frame(frame).map_err(internal)?.terminal)
    }
    fn terminal(&mut self, object: Value) -> Option<Value> {
        let object = if object == Value::NIL { self.frame() } else { object };
        let terminal = match self.vm.object_if(object) {
            Some(Object::Frame(f)) => f.terminal,
            Some(Object::Terminal(_)) => object,
            _ => return None,
        };
        matches!(self.vm.object_if(terminal), Some(Object::Terminal(t)) if t.live).then_some(terminal)
    }
    /// decode_live_terminal, then the (terminal, keyboard) coding names.
    pub(crate) fn terminal_codings(&mut self, object: Value) -> std::result::Result<(Value, Value, Value), Fault> {
        self.vm.ensure_windows().map_err(internal)?;
        let terminal = self.terminal(object).ok_or(Fault::Wrong("terminal-live-p", object))?;
        match self.vm.object_if(terminal) {
            Some(Object::Terminal(t)) => Ok((terminal, t.terminal_coding, t.keyboard_coding)),
            _ => Err(Fault::Wrong("terminal-live-p", object)),
        }
    }
    /// decode_live_terminal, then its parameter alist.
    pub(crate) fn terminal_params(&mut self, object: Value) -> std::result::Result<(Value, Value), Fault> {
        self.vm.ensure_windows().map_err(internal)?;
        let terminal = self.terminal(object).ok_or(Fault::Wrong("terminal-live-p", object))?;
        match self.vm.object_if(terminal) {
            Some(Object::Terminal(t)) => Ok((terminal, t.params)),
            _ => Err(Fault::Wrong("terminal-live-p", object)),
        }
    }
    pub(crate) fn set_terminal_params(&mut self, terminal: Value, params: Value) -> std::result::Result<(), Fault> {
        let index = terminal.index(0).map_err(internal)?.saturating_sub(1);
        if let Some(Object::Terminal(t)) = self.vm.objects.get_mut(index) { t.params = params; }
        Ok(())
    }
    pub(crate) fn set_terminal_coding(&mut self, terminal: Value, keyboard: bool, coding: Value) -> std::result::Result<(), Fault> {
        let index = terminal.index(0).map_err(internal)?.saturating_sub(1);
        if let Some(Object::Terminal(t)) = self.vm.objects.get_mut(index) {
            if keyboard { t.keyboard_coding = coding; } else { t.terminal_coding = coding; }
        }
        Ok(())
    }
    /// decode_any_window: nil is the selected window; dead windows count.
    fn any_window(&mut self, window: Value) -> std::result::Result<Value, Fault> {
        self.vm.ensure_windows().map_err(internal)?;
        let window = if window == Value::NIL { self.vm.window_root(SELECTED) } else { window };
        if matches!(self.vm.object_if(window), Some(Object::Window(_))) { Ok(window) } else { Err(Fault::Wrong("windowp", window)) }
    }
    pub(crate) fn set_cursor_off(&mut self, window: Value, off: bool) -> std::result::Result<(), Fault> {
        let window = self.any_window(window)?;
        self.vm.window_mut(window).map_err(internal)?.cursor_off = off;
        Ok(())
    }
    pub(crate) fn cursor_off(&mut self, window: Value) -> std::result::Result<bool, Fault> {
        let window = self.any_window(window)?;
        Ok(self.get(window)?.cursor_off)
    }
    /// terminal.c Fframe_initial_p. VM2's one terminal is GNU's
    /// output_initial kind, whose frame writes messages and printing to the
    /// standard streams, until the session opens a display or an embedder
    /// renders its echo area (`Vm::set_echo_area_host`): either makes it the
    /// session's display, as a window system's frame is, and not initial.
    /// debug.el decides by this whether to show *Backtrace* or print it.
    pub(crate) fn frame_initial_p(&mut self, frame: Value) -> Out {
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        let host = self.vm.slot(crate::prim::ECHO_AREA_HOST);
        let initial = self.vm.frame_type() == Value::TRUE
            && matches!(self.vm.slots[host].value, Value::NIL | Value::UNBOUND);
        if let Some(Object::Frame(f)) = self.vm.object_if(frame) {
            return Ok(if f.live && initial { Value::TRUE } else { Value::NIL });
        }
        // decode_terminal yields NULL for anything else: nil, not an error.
        Ok(if self.terminal(frame).is_some() && initial { Value::TRUE } else { Value::NIL })
    }
    /// decode_live_terminal, then the terminal's name: GNU's
    /// "initial_terminal" until a display opens, then the display's name
    /// (a window-system terminal is named for its display). The second
    /// value is whether the terminal is still GNU's output_initial kind.
    pub(crate) fn terminal_name(&mut self, object: Value) -> std::result::Result<(String, bool), Fault> {
        self.vm.ensure_windows().map_err(internal)?;
        self.terminal(object).ok_or(Fault::Wrong("terminal-live-p", object))?;
        Ok(match self.vm.window_system_host() {
            Some(host) => (host.display_name(), false),
            None => ("initial_terminal".into(), true),
        })
    }
    /// terminal.c:443: a live terminal's type, as framep's.
    pub(crate) fn terminal_live_p(&mut self, object: Value) -> Value {
        if self.terminal(object).is_some() { self.vm.frame_type() } else { Value::NIL }
    }
    pub(crate) fn terminal_list(&mut self) -> Out {
        let terminal = self.frame_terminal(Value::NIL)?;
        Ok(self.vm.list_value(&[terminal]))
    }
    pub(crate) fn tty_top_frame(&mut self, terminal: Value) -> Out {
        self.terminal(terminal).ok_or(Fault::Wrong("terminal-live-p", terminal))?;
        // GNU returns nil for output_initial. There is no TTY backend here.
        Ok(Value::NIL)
    }
    /// Fset_window_buffer's checks: WINDOW live and BUFFER-OR-NAME a live
    /// buffer (Fget_buffer, then CHECK_BUFFER). Returns the window, the
    /// buffer, and whether the window shows another buffer now.
    pub(crate) fn set_window_buffer_check(&mut self, window: Value, buffer_or_name: Value) -> std::result::Result<(Value, Value, bool), Fault> {
        let v = self.live(window)?;
        let buffer = if matches!(self.vm.object_if(buffer_or_name), Some(Object::Buffer(_) | Object::ByteBuffer(_))) {
            buffer_or_name
        } else {
            let name = self.vm.buffer_name_text(buffer_or_name).ok_or(Fault::Wrong("stringp", buffer_or_name))?;
            self.vm.named_buffer(&name).unwrap_or(Value::NIL)
        };
        if buffer == Value::NIL {
            return Err(Fault::Wrong("bufferp", Value::NIL));
        }
        if !self.vm.buffer_state(buffer).is_ok_and(|s| s.live) {
            return Err(Fault::Error("Attempt to display deleted buffer".into()));
        }
        Ok((v, buffer, self.get(v)?.buffer != buffer))
    }
    /// The name of the buffer WINDOW is strongly dedicated to, if it is.
    pub(crate) fn strongly_dedicated(&mut self, window: Value) -> std::result::Result<Option<String>, Fault> {
        let w = self.get(window)?;
        if w.dedicated != Value::TRUE { return Ok(None); }
        let buffer = w.buffer;
        Ok(Some(self.vm.buffer_state(buffer).ok().and_then(|s| s.name.clone()).unwrap_or_default()))
    }
    /// set_window_buffer, after Fset_window_buffer's checks.
    pub(crate) fn set_window_buffer(&mut self, window: Value, buffer: Value, keep_margins: Value) -> Out {
        self.vm.set_window_buffer_value(window, buffer, keep_margins != Value::NIL).map_err(internal)?;
        Ok(Value::NIL)
    }
    /// window.c select_window: WINDOW, live, becomes the selected window and
    /// its buffer current; without NORECORD its use time and buffer are
    /// recorded.  The selected window and its buffer.
    pub(crate) fn select_window(&mut self, window: Value, norecord: bool) -> std::result::Result<(Value, Value), Fault> {
        let v = self.live(window)?;
        let v = self.vm.select_window_value(v, !norecord).map_err(internal)?;
        self.vm.display_state.version += 1;
        Ok((v, self.get(v)?.buffer))
    }
    /// GNU `window-prev-buffers` (NEXT false) / `window-next-buffers`.
    pub(crate) fn window_buffers(&mut self, window: Value, next: bool) -> Out {
        let v = self.live(window)?;
        let w = self.get(v)?;
        Ok(if next { w.next_buffers } else { w.prev_buffers })
    }
    /// window.c window_discard_buffer_from_window: BUFFER leaves WINDOW's
    /// previous buffers (an alist), its next buffers, and with ALL the
    /// `quit-restore' parameters that name it (nth 3, or the car of nth 1).
    pub(crate) fn discard_buffer(&mut self, buffer: Value, window: Value, all: bool) -> Out {
        if !self.vm.buffer_state(buffer).is_ok_and(|s| s.live) { return Err(Fault::Error("Not a live buffer".into())); }
        let v = match self.vm.object_if(window) {
            Some(Object::Window(x)) if x.live() => window,
            _ => return Err(Fault::Error("Not a live window".into())),
        };
        let keep = |vm: &mut Vm, list: Value, alist: bool| -> std::result::Result<Value, Fault> {
            let items = vm.list_values(list).map_err(internal)?;
            let kept: Vec<Value> = items.into_iter()
                .filter(|&item| (if alist { vm.cons_pair(item).map(|(car, _)| car).unwrap_or(item) } else { item }) != buffer)
                .collect();
            Ok(vm.list_value(&kept))
        };
        let (prev, next) = { let w = self.get(v)?; (w.prev_buffers, w.next_buffers) };
        let prev = keep(self.vm, prev, true)?;
        let next = keep(self.vm, next, false)?;
        { let w = self.get_mut(v)?; w.prev_buffers = prev; w.next_buffers = next; }
        if !all { return Ok(Value::NIL); }
        let names = [self.vm.intern("quit-restore"), self.vm.intern("quit-restore-prev")];
        let mentions = |vm: &Vm, value: Value| -> bool {
            let items = vm.list_values(value).unwrap_or_default();
            items.get(3) == Some(&buffer)
                || items.get(1).and_then(|&quint| vm.cons_pair(quint).ok()).is_some_and(|(car, _)| car == buffer)
        };
        let parameter = |w: &Window, key: Value| w.parameters.iter().find(|(k, _)| *k == key).map_or(Value::NIL, |(_, v)| *v);
        let (restore, restore_prev) = { let w = self.get(v)?; (parameter(w, names[0]), parameter(w, names[1])) };
        let set = |w: &mut Window, key: Value, value: Value| {
            if let Some((_, slot)) = w.parameters.iter_mut().find(|(k, _)| *k == key) { *slot = value; }
            else { w.parameters.insert(0, (key, value)); }
        };
        if mentions(self.vm, restore_prev) { set(self.get_mut(v)?, names[1], Value::NIL); }
        let restore_prev = { let w = self.get(v)?; parameter(w, names[1]) };
        if mentions(self.vm, restore) {
            let w = self.get_mut(v)?;
            set(w, names[0], restore_prev);
            set(w, names[1], Value::NIL);
        }
        Ok(Value::NIL)
    }
    /// GNU `set-window-prev-buffers` (NEXT false) / `set-window-next-buffers`.
    pub(crate) fn set_window_buffers(&mut self, window: Value, buffers: Value, next: bool) -> Out {
        let v = self.live(window)?;
        let w = self.get_mut(v)?;
        if next { w.next_buffers = buffers } else { w.prev_buffers = buffers }
        Ok(buffers)
    }
    /// window.c Fframe_first_window: the top-left live window of FRAME-OR-WINDOW's frame.
    pub(crate) fn frame_first_window(&mut self, frame_or_window: Value) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        let frame = if frame_or_window == Value::NIL {
            self.frame()
        } else if self.window_valid_p(frame_or_window) != Value::NIL {
            self.get(frame_or_window)?.frame
        } else {
            self.live_frame(frame_or_window)?;
            frame_or_window
        };
        let mut window = self.vm.frame(frame).map_err(internal)?.root;
        while let Some(&child) = self.get(window)?.children.first() {
            window = child;
        }
        Ok(window)
    }
    pub(crate) fn window_list(&mut self, frame: Value, minibuf: Value, window: Value) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        let window = if window == Value::NIL {
            if self.is_frame(frame) { self.vm.frame(frame).map_err(internal)?.selected }
            else { self.vm.window_root(SELECTED) }
        } else { window };
        if !matches!(self.vm.object_if(window), Some(Object::Window(_))) {
            return Err(Fault::Wrong("windowp", window));
        }
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        if self.get(window)?.frame != frame {
            return Err(Fault::Error("Window is on a different frame".into()));
        }
        self.window_list_1(window, minibuf, frame)
    }
    pub(crate) fn get_buffer_window(&mut self, buffer: Value, all_frames: Value) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        let buffer = if buffer == Value::NIL {
            let index = self.vm.buffer_index().map_err(internal)?;
            Value::reference(index + 1, 0)
        }
        else if matches!(self.vm.object_if(buffer), Some(Object::Buffer(_) | Object::ByteBuffer(_))) { buffer }
        else {
            let name = self.vm.buffer_name_text(buffer).ok_or(Fault::Wrong("stringp", buffer))?;
            let Some(buffer) = self.vm.named_buffer(&name) else { return Ok(Value::NIL); };
            buffer
        };
        // window.c window_loop GET_BUFFER_WINDOW: ALL-FRAMES nil or a frame
        // is that frame's windows, from its selected window.
        let visible = self.vm.intern("visible");
        let (selected, all_frames) = if all_frames == Value::NIL || self.is_frame(all_frames) {
            let frame = if all_frames == Value::NIL { self.frame() } else { all_frames };
            (self.vm.frame(frame).map_err(internal)?.selected, self.vm.intern("lambda"))
        } else if all_frames == visible || all_frames.as_integer() == Ok(0) {
            (self.vm.window_root(SELECTED), all_frames)
        } else { (self.vm.window_root(SELECTED), Value::TRUE) };
        let (_, mut windows) = self.candidates(selected, Value::TRUE, all_frames)?;
        if let Some(at) = windows.iter().position(|&w| w == selected) {
            windows.rotate_left(at);
        }
        Ok(windows.into_iter().find(|&w| {
            self.get(w).is_ok_and(|w| w.live() && w.buffer == buffer && (!w.mini || self.vm.names.get("romacs--minibuf-level").is_some_and(|&i| self.vm.slots[i].value.integer_value().is_some_and(|depth| depth > 0))))
        }).unwrap_or(Value::NIL))
    }
    /// GNU window.c:748.
    pub(crate) fn set_window_combination_limit(&mut self, window: Value, limit: Value) -> Out {
        if window == Value::NIL { return Err(Fault::Wrong("window-valid-p", window)); }
        let window = self.valid(window)?;
        if self.get(window)?.children.is_empty() {
            return Err(Fault::Error("Combination limit is meaningful for internal windows only".into()));
        }
        self.get_mut(window)?.combination_limit = limit;
        Ok(limit)
    }
    /// GNU window.c:1278.
    pub(crate) fn window_hscroll(&mut self, window: Value) -> Out {
        let window = self.live(window)?;
        Self::int(self.get(window)?.hscroll)
    }
    /// GNU window.c:1313.
    pub(crate) fn set_window_hscroll(&mut self, window: Value, ncol: Value) -> Out {
        let n = ncol.integer_value().ok_or(Fault::Wrong("fixnump", ncol))?;
        let window = self.live(window)?;
        self.scroll_to(window, n)
    }
    /// GNU set_window_hscroll: WINDOW scrolled N columns, clipped to
    /// 0..most-positive-fixnum, holding automatic hscrolling.
    fn scroll_to(&mut self, window: Value, n: i64) -> Out {
        let n = n.clamp(0, Value::MAX_FIXNUM);
        let w = self.get_mut(window)?;
        if w.hscroll != n { w.end_valid = false; }
        (w.hscroll, w.suspend_auto_hscroll) = (n, true);
        self.vm.display_state.version += 1;
        Self::int(n)
    }
    /// GNU `scroll-left' (LEFT) and `scroll-right' (window.c:7101): the
    /// selected window COLUMNS further, by default its body's width in
    /// columns less 2; with SET-MINIMUM the result bounds automatic
    /// hscrolling from below.
    pub(crate) fn scroll_horizontally(&mut self, columns: Option<i64>, set_minimum: Value, left: bool) -> Out {
        let window = self.vm.selected_window_value().map_err(Fault::Error)?;
        let columns = match columns {
            Some(n) => n,
            None => self.window_body_size(window, Value::NIL, true)?.integer_value().unwrap_or(0) - 2,
        };
        let hscroll = self.get(window)?.hscroll;
        let result = self.scroll_to(window, if left { hscroll.saturating_add(columns) } else { hscroll.saturating_sub(columns) })?;
        let w = self.get_mut(window)?;
        if set_minimum != Value::NIL { w.min_hscroll = w.hscroll; }
        Ok(result)
    }
    /// GNU window.c:2048 (`Vm::window_line_height').
    pub(crate) fn window_line_height(&mut self, line: Value, window: Value) -> Out {
        let window = self.live(window)?;
        let bands = ["tab-line", "header-line", "mode-line"].map(|name| self.vm.intern(name));
        if line != Value::NIL && line.integer_value().is_none() && !bands.contains(&line) { return Err(Fault::Wrong("fixnump", line)); }
        // VM2 draws no tab line (DIVERGENCES.md VM2-WIN3).
        if line == bands[0] { return Ok(Value::NIL); }
        self.vm.window_line_height(line, window).map_err(Fault::Error)
    }
    pub(crate) fn selected_window(&mut self) -> Out {
        self.vm.selected_window_value().map_err(Fault::Error)
    }
    /// Whether live WINDOW's point is at its old point.
    pub(crate) fn old_point_is_point(&mut self, window: Value) -> std::result::Result<bool, Fault> {
        let old = self.window_old_point(window)?;
        let point = self.vm.window_point_value(window).map_err(Fault::Error)?;
        Ok(old == point)
    }
    /// Move live WINDOW's old point to its point.
    pub(crate) fn old_point_to_point(&mut self, window: Value) -> std::result::Result<(), Fault> {
        let window = self.live(window)?;
        let (point, buffer) = (self.vm.window_point_value(window).map_err(Fault::Error)?, self.get(window)?.buffer);
        self.vm.set_window_old_point(window, point, buffer).map_err(Fault::Error)
    }
    /// GNU window.c:1803: WINDOW's point at its last redisplay.
    pub(crate) fn window_old_point(&mut self, window: Value) -> Out {
        let window = self.live(window)?;
        let marker = self.get(window)?.old_point;
        if marker == Value::NIL { return Ok(Value::NIL); }
        self.vm.marker_position_value(marker).map_err(internal)
    }
    pub(crate) fn window_combination_limit(&mut self, window: Value) -> Out {
        if window == Value::NIL { return Err(Fault::Wrong("window-valid-p", window)); }
        let window = self.valid(window)?;
        let w = self.get(window)?;
        if w.children.is_empty() {
            return Err(Fault::Error("Combination limit is meaningful for internal windows only".into()));
        }
        Ok(w.combination_limit)
    }
    /// VM2's minibuffer depth: the Lisp half of read-from-minibuffer
    /// (prims/minibuf.el) counts it in `romacs--minibuf-level`.
    fn minibuf_level(&mut self) -> i64 {
        let slot = self.vm.slot("romacs--minibuf-level");
        self.vm.variable_value(slot).ok().and_then(|v| v.integer_value()).unwrap_or(0)
    }
    /// read_minibuf_unwind's resize_mini_window (minibuf.c:1197): t when
    /// the mini-window's height changed.
    pub(crate) fn resize_mini_window_now(&mut self) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        let changed = self.vm.resize_mini_window_now().map_err(internal)?;
        if changed { self.vm.display_state.version += 1; }
        Ok(if changed { Value::TRUE } else { Value::NIL })
    }
    /// minibuf.c:215: the minibuffer window while a minibuffer is active.
    pub(crate) fn active_minibuffer_window(&mut self) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        Ok(if self.minibuf_level() > 0 { self.vm.window_root(MINI) } else { Value::NIL })
    }
    /// GNU `window_list`: every frame's windows, frames in `frame-list'
    /// order, each frame's leaves in tree order, then the minibuffer window
    /// on the frame that owns it (the root's `next`).
    fn all_windows(&self) -> std::result::Result<Vec<Value>, Fault> {
        let mut out = vec![];
        for frame in self.vm.frames() {
            let f = self.vm.frame(frame).map_err(internal)?;
            let (root, mini) = (f.root, f.minibuffer);
            self.leaves(root, &mut out)?;
            if self.get(mini)?.frame == frame { out.push(mini); }
        }
        Ok(out)
    }
    fn leaves(&self, v: Value, out: &mut Vec<Value>) -> std::result::Result<(), Fault> {
        let w = self.get(v)?;
        if w.children.is_empty() {
            out.push(v);
        } else {
            for &c in &w.children {
                self.leaves(c, out)?;
            }
        }
        Ok(())
    }
    /// GNU `decode_next_window_args` then `candidate_window_p` (window.c),
    /// specialized to one frame: every frame filter admits it, a window
    /// filter admits it when MINIBUF is t or that window's buffer is not the
    /// depth-0 minibuffer.
    fn candidates(&mut self, window: Value, minibuf: Value, all_frames: Value)
        -> std::result::Result<(Value, Vec<Value>), Fault> {
        let window = self.live(window)?;
        let mini = self.vm.window_root(MINI);
        let level = self.minibuf_level();
        let lambda = self.vm.intern("lambda");
        let minibuf = if minibuf == Value::NIL {
            if level > 0 { mini } else { lambda }
        } else if minibuf != Value::TRUE {
            lambda
        } else {
            minibuf
        };
        // decode_next_window_args (window.c:2827): nil becomes the
        // minibuffer window when MINIBUF counts it; `visible', 0, a frame and
        // t stand; anything else, including an explicit window, means nil.
        let visible = self.vm.intern("visible");
        let all_frames = if all_frames == Value::NIL {
            if minibuf != lambda { mini } else { Value::NIL }
        } else if all_frames == Value::TRUE
            || all_frames == visible
            || all_frames.as_integer() == Ok(0)
            || self.is_frame(all_frames)
        {
            all_frames
        } else {
            Value::NIL
        };
        let mini_buffer = self.get(mini)?.buffer;
        let filter_window = matches!(self.vm.object_if(all_frames), Some(Object::Window(_)));
        let own = self.get(window)?.frame;
        let mut out = vec![];
        for w in self.all_windows()? {
            let (live, frame) = { let x = self.get(w)?; (x.live(), x.frame) };
            if !live || !self.frame_admits(frame, own, all_frames, mini)? {
                continue;
            }
            let x = self.get(w)?;
            if x.mini && (minibuf == lambda || (minibuf != Value::TRUE && minibuf != w)) {
                continue;
            }
            if filter_window && minibuf != Value::TRUE {
                let shown = self.get(all_frames)?.buffer;
                if shown == mini_buffer && level == 0 {
                    continue;
                }
            }
            out.push(w);
        }
        Ok((window, out))
    }
    /// candidate_window_p's frame test: ALL-FRAMES t admits every frame,
    /// `visible' and 0 the visible ones, a frame itself, a window the
    /// frames using it as their minibuffer window, nil WINDOW's own frame.
    fn frame_admits(&mut self, frame: Value, own: Value, all_frames: Value, mini: Value) -> std::result::Result<bool, Fault> {
        let visible = self.vm.intern("visible");
        let is_frame = self.is_frame(all_frames);
        let f = self.vm.frame(frame).map_err(internal)?;
        Ok(if all_frames == Value::TRUE { true }
           else if all_frames == visible || all_frames.as_integer() == Ok(0) { f.visible }
           else if is_frame { frame == all_frames }
           // A frame sharing the active minibuffer window counts while its
           // focus is redirected to that window's frame (Bug#24500).
           else if all_frames == mini {
               let mini_frame = self.get(mini)?.frame;
               frame == mini_frame || (f.minibuffer == mini && f.focus == mini_frame)
           }
           else { frame == own })
    }
    /// window.c:2941/2980 `next_window`.
    pub(crate) fn next_window(&mut self, window: Value, minibuf: Value, all_frames: Value, next: bool) -> Out {
        let (window, candidates) = self.candidates(window, minibuf, all_frames)?;
        let all = self.all_windows()?;
        let is = |w: &Value| candidates.contains(w);
        let at = all.iter().position(|w| *w == window);
        if next {
            if let Some(at) = at {
                if let Some(&w) = all[at + 1..].iter().find(|w| is(w)) {
                    return Ok(w);
                }
            }
            let stop = at.unwrap_or(all.len());
            Ok(all[..stop].iter().copied().find(|w| is(w)).unwrap_or(window))
        } else {
            let mut candidate = None;
            for &w in &all {
                if w == window {
                    if candidate.is_some() {
                        break;
                    }
                } else if is(&w) {
                    candidate = Some(w);
                }
            }
            Ok(candidate.unwrap_or(window))
        }
    }
    /// window.c:3079 `window_list_1`: the candidates rotated to start at
    /// WINDOW.
    pub(crate) fn window_list_1(&mut self, window: Value, minibuf: Value, all_frames: Value) -> Out {
        let (window, mut list) = self.candidates(window, minibuf, all_frames)?;
        if let Some(at) = list.iter().position(|w| *w == window) {
            list.rotate_left(at);
        }
        Ok(self.vm.list_value(&list))
    }
    /// window.c:456.
    pub(crate) fn set_frame_selected_window(&mut self, frame: Value, window: Value, norecord: Value) -> Out {
        self.live_frame(frame)?;
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        let w = match self.vm.object_if(window) {
            Some(Object::Window(x)) if x.live() => window,
            _ => return Err(Fault::Wrong("window-live-p", window)),
        };
        if self.get(w)?.frame != frame {
            return Err(Fault::Error("In `set-frame-selected-window', WINDOW is not on FRAME".into()));
        }
        // Only the selected frame's selected window is selected.
        if frame != self.frame() {
            self.vm.frame_mut(frame).map_err(internal)?.selected = w;
            return Ok(w);
        }
        self.vm.select_window_value(w, norecord == Value::NIL).map_err(internal)?;
        self.vm.display_state.version += 1;
        Ok(w)
    }
    /// window.c:3467 `Fdelete_other_windows_internal`, for VM2's one frame.
    pub(crate) fn delete_other_windows_internal(&mut self, window: Value, root: Value) -> Out {
        let w = self.valid(window)?;
        let frame_root = self.vm.root_of(w).map_err(internal)?;
        let root = if root == Value::NIL {
            frame_root
        } else {
            let r = self.valid(root)?;
            let mut p = self.get(w)?.parent;
            while p != Value::NIL && p != r {
                p = self.get(p)?.parent;
            }
            if p != r {
                return Err(Fault::Error("Specified root is not an ancestor of specified window".into()));
            }
            r
        };
        if w == root {
            return Ok(Value::NIL);
        }
        if self.get(w)?.mini {
            return Err(Fault::Error("Can’t expand minibuffer to full frame".into()));
        }
        let selected = self.vm.window_root(SELECTED);
        if self.get(w)?.live() {
            if w != selected {
                self.vm.select_window_value(w, true).map_err(internal)?;
            }
        } else {
            // The selected window, or the nearest previous window inside
            // WINDOW's subtree, stays selected.
            let mut s = selected;
            loop {
                let mut p = s;
                while p != Value::NIL && p != w {
                    p = self.get(p)?.parent;
                }
                if p == w {
                    break;
                }
                let lambda = self.vm.intern("lambda");
                s = self.next_window(s, lambda, Value::NIL, false)?;
            }
            if s != selected {
                self.vm.select_window_value(s, true).map_err(internal)?;
            }
        }
        // WINDOW takes ROOT's place and geometry; everything else in ROOT's
        // subtree is deleted. An internal WINDOW's children are resized
        // proportionally (GNU asks `window--resize-root-window`; VM2 uses its
        // frame-resize plan, DIVERGENCES.md VM2-WIN2).
        let (x, y, width, height) = {
            let r = self.get(root)?;
            (r.x, r.y, r.width, r.height)
        };
        let parent = self.get(w)?.parent;
        self.get_mut(parent)?.children.retain(|c| *c != w);
        // replace_window (root, window, true) (window.c:3667, 2571-2574):
        // WINDOW inherits ROOT's normal sizes; ROOT is retired below.
        let (normal_lines, normal_cols) = {
            let r = self.get(root)?;
            (r.normal_lines, r.normal_cols)
        };
        {
            let x = self.get_mut(w)?;
            x.normal_lines = normal_lines;
            x.normal_cols = normal_cols;
        }
        self.vm.replace_tree_node(root, w).map_err(internal)?;
        self.vm.retire_window(root).map_err(internal)?;
        let (line, column) = (self.vm.window_state.line, self.vm.window_state.column);
        let mut geometry = vec![];
        self.vm
            .plan_window_geometry(w, x, y, width, height, (0, 0), line, column, &mut geometry)
            .map_err(internal)?;
        // Live-window start is intentionally unchanged: GNU's vmotion-based
        // scroll preservation is deferred (DIVERGENCES.md VM2-WIN4).
        for (v, x, y, wd, ht) in geometry {
            let window = self.get_mut(v)?;
            window.x = x;
            window.y = y;
            window.width = wd;
            window.height = ht;
            window.end_valid = false;
        }
        self.vm.display_state.version += 1;
        Ok(Value::NIL)
    }
    /// window.c:3852: the window's buffer, its start, and the hook value,
    /// for the T2 body that runs `window-scroll-functions`.
    pub(crate) fn scroll_functions_call(&mut self, window: Value) -> std::result::Result<(Value, Value, Value), Fault> {
        let w = self.live(window)?;
        let (buffer, start) = {
            let x = self.get(w)?;
            (x.buffer, x.start)
        };
        let start = self.vm.marker_position_value(start).map_err(internal)?;
        Ok((w, buffer, start))
    }
}
