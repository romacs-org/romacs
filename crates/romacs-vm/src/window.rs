//! VM-owned windows. GNU window.c's prepared-size transactions, not Lisp policy.
use crate::{data::Object, *};
#[path = "redisplay.rs"]
mod redisplay;
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
    /// Per-window vertical scroll in pixels. GNU keeps this on the window as
    /// `w->vscroll` and both `window-vscroll` and `set-window-vscroll` begin by
    /// decoding the WINDOW argument (`src/window.c:8947`, `:8978`). It lived on
    /// `DisplayState` and was published into every window, so one window's
    /// scroll was observable from another. It belongs here, beside the margins,
    /// which are per-window for the same reason and never had that defect.
    pub vscroll_px: u32,
    /// Deferred scroll, applied after the frozen frame paints (D-ML1).
    pub pending_vscroll: Option<u32>,
    /// GNU `w->hscroll`, in columns. The display does not apply it yet
    /// (VM2-HSCROLL1).
    pub hscroll: i64,
    pub parameters: Vec<(Value, Value)>,
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
    pub force_start: bool,
}
impl Window {
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
            vscroll_px: 0,
            pending_vscroll: None,
            hscroll: 0,
            right_margin: 0,
            parameters: vec![],
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
}
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
    pub(super) fn buffer_is_displayed(&self, buffer: Value) -> Result<bool> {
        let frame = self.frame(self.frame_value_peek())?;
        let mut pending = vec![frame.root, frame.minibuffer];
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

    /// The buffer the frame's minibuffer window shows (VM2 has one frame).
    pub(crate) fn minibuffer_window_buffer(&self) -> Option<Value> {
        self.window(self.window_root_peek(MINI)).ok().map(|w| w.buffer)
    }
    /// The root/selected/minibuffer window, read without interning: the
    /// slot exists once `ensure_windows` has run.
    pub(crate) fn window_root_peek(&self, name: &str) -> Value {
        let Ok(f) = self.frame(self.frame_value_peek()) else { return Value::UNBOUND; };
        match name { ROOT => f.root, SELECTED => f.selected, MINI => f.minibuffer, _ => unreachable!() }
    }
    fn put_window_root(&mut self, name: &str, v: Value) {
        let f = self.frame_mut(self.frame_value_peek()).expect("initialized frame");
        match name { ROOT => f.root = v, SELECTED => f.selected = v, MINI => f.minibuffer = v, _ => unreachable!() }
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
    fn allocate_window(&mut self, buffer: Value) -> Result<Value> {
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
    fn frame_arg(&mut self, v: Value) -> Result<()> {
        let frame = self.selected_frame_value()?;
        if v == Value::NIL || v == frame {
            Ok(())
        } else {
            Err("unknown frame".into())
        }
    }
    pub(super) fn window_point_value(&mut self, v: Value) -> Result<Value> {
        if v == self.window_root(SELECTED) {
            let Object::Buffer(b) = self.object(self.window(v)?.buffer)? else {
                return Err("buffer required".into());
            };
            Value::integer((b.char_of_byte(b.point)?.0 + 1) as i64)
        } else {
            self.marker_position_value(self.window(v)?.point)
        }
    }
    pub(super) fn set_buffer_point_for_window(&mut self, v: Value, pos: Value) -> Result<()> {
        let buffer = self.window(v)?.buffer;
        let marker = self.window(v)?.point;
        self.set_marker_object(marker, pos, buffer)?;
        let pos = self.marker_position_value(marker)?.as_integer()?;
        let Object::Buffer(b) = &mut self.objects[buffer.index(0)? - 1] else {
            return Err("buffer required".into());
        };
        b.point = b.byte_of_char(coordinates::CharOffset((pos - 1) as usize))?;
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
                        if Self::image_value(cell) {
                            return Err("cannot mutate immutable image".into());
                        }
                        self.conses[cell.index(3)?].1 = rest;
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
                let Object::Buffer(buf) = self.object(buffer)? else {
                    return Err("buffer required".into());
                };
                Value::integer((buf.char_of_byte(buf.point)?.0 + 1) as i64)?
            };
            self.set_marker_object(point, pos, buffer)?;
            // GNU restores `b->last_window_start` here; VM2 keeps no
            // per-buffer last window start, so it starts at 1. See
            // DIVERGENCES.md VM2-WB1.
            self.set_marker_object(st, Value::integer(1)?, buffer)?;
            self.window_mut(v)?.hscroll = 0;
        }
        self.discard_buffer_from_window(buffer, v)?;
        let w = self.window_mut(v)?;
        w.buffer = buffer;
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
            self.put_window_root(SELECTED, v);
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
            self.put_window_root(ROOT, new);
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
    fn retire_window(&mut self, v: Value) -> Result<()> {
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
        let (point, start) = (self.window(v)?.point, self.window(v)?.start);
        for marker in [point, start] {
            if marker != Value::NIL {
                self.set_marker_object(marker, Value::NIL, Value::NIL)?;
            }
        }
        let w = self.window_mut(v)?;
        w.dead = true;
        w.buffer = Value::NIL;
        w.point = Value::NIL;
        w.start = Value::NIL;
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
        let root = self.window_root(ROOT);
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
        if !self.window(selected)?.live() {
            let root = self.window_root(ROOT);
            let next = self.first_leaf(root)?;
            self.put_window_root(SELECTED, next);
            self.slots[self.buffer_slot].value = self.window(next)?.buffer;
            let point = self.marker_position_value(self.window(next)?.point)?;
            self.set_buffer_point_for_window(next, point)?;
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
        for c in children {
            self.window_mut(c)?.parent = parent;
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
        let (buffer, left_margin, right_margin) = {
            let reference = self.window(reference)?;
            (reference.buffer, reference.left_margin, reference.right_margin)
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
    /// Freeze leaf publications using stable scalar IDs across collecting mode-line eval.
    pub(super) fn frame_window_snapshots(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<Vec<(u32, u32, romacs_layout_input::WindowSnapshot)>> {
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
        ids.push(self.window(mini)?.id);
        // Redisplay sizes the mini-window to its display before laying out.
        if self.resize_mini_window(mini)? {
            self.set_frame_geometry(width, height, self.window_state.line, self.window_state.column)?;
        }
        let live = |vm: &Self, id| {
            vm.objects.iter().enumerate().find_map(|(i, o)| match o {
                Object::Window(w) if w.id == id && w.live() => Some(Value::reference(i + 1, 0)),
                _ => None,
            })
        };
        let outcome = (|| {
            let mut snapshots = vec![];
            for id in ids {
                // Lisp run by an earlier window's redisplay may delete this one.
                let Some(v) = live(self, id) else { continue };
                let w = self.window(v)?;
                let (x, y, width, height) = (
                    w.x + w.left_margin * self.window_state.column,
                    w.y,
                    w.width.saturating_sub(
                        (w.left_margin + w.right_margin) * self.window_state.column,
                    ),
                    w.height,
                );
                match self.snapshot_window(v, width, height) {
                    Ok(snapshot) => snapshots.push((x, y, snapshot)),
                    // ... or its own.
                    Err(_) if live(self, id).is_none() => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(snapshots)
        })();
        self.display_state.width = width;
        self.display_state.height = height;
        outcome
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
    /// Install canonical font metrics before startup Lisp observes window sizes.
    pub fn set_frame_geometry(
        &mut self,
        width: u32,
        height: u32,
        line_height: u32,
        column_width: u32,
    ) -> Result<()> {
        if line_height == 0
            || column_width == 0
            || u64::from(height) < 2 * u64::from(line_height)
            || u64::from(width) < 2 * u64::from(column_width)
        {
            return Err("frame geometry is below safe window minimum".into());
        }
        self.ensure_windows()?;
        let root = self.window_root(ROOT);
        // A mini-window the root windows cannot make room for keeps one
        // line (window.c grow_mini_window, bug#69140).
        let mini_height = self.window_state.mini_lines.saturating_mul(line_height).min(height - line_height);
        let mut geometry = vec![];
        let planned = self.plan_window_geometry(root, 0, 0, width, height - mini_height, line_height, column_width, &mut geometry);
        let mini_height = if planned.is_err() && mini_height > line_height {
            self.window_state.mini_lines = 1;
            geometry.clear();
            self.plan_window_geometry(root, 0, 0, width, height - line_height, line_height, column_width, &mut geometry)?;
            line_height
        } else {
            planned?;
            mini_height
        };
        let mini = self.window_root(MINI);
        geometry.push((mini, 0, height - mini_height, width, mini_height));
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
        self.window_state.width = width;
        self.window_state.height = height;
        self.window_state.line = line_height;
        self.window_state.column = column_width;
        self.display_state.width = width;
        self.display_state.height = height;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn plan_window_geometry(
        &self,
        v: Value,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        line: u32,
        column: u32,
        out: &mut Vec<(Value, u32, u32, u32, u32)>,
    ) -> Result<()> {
        let w = self.window(v)?;
        if width < 2 * column || height < line {
            return Err("frame resize would violate safe window minimum".into());
        }
        out.push((v, x, y, width, height));
        let total = if w.horizontal { width } else { height };
        let old_total = if w.horizontal { w.width } else { w.height };
        let mut old_edge = 0u64;
        let mut edge = 0;
        for (i, &child) in w.children.iter().enumerate() {
            let c = self.window(child)?;
            old_edge += u64::from(if w.horizontal { c.width } else { c.height });
            let next = if i + 1 == w.children.len() {
                total
            } else {
                (old_edge * u64::from(total) / u64::from(old_total.max(1))) as u32
            };
            let size = next.saturating_sub(edge);
            if w.horizontal {
                self.plan_window_geometry(child, x + edge, y, size, height, line, column, out)?;
            } else {
                self.plan_window_geometry(child, x, y + edge, width, size, line, column, out)?;
            }
            edge = next;
        }
        Ok(())
    }
    pub fn display_apply_layout(&mut self, ends: &[(u64, usize)]) -> Result<()> {
        // Validate the complete journal before publishing any per-window end.
        let mut changes = vec![];
        for &(id, end) in ends {
            let (index, w) = self
                .objects
                .iter()
                .enumerate()
                .find_map(|(i, o)| match o {
                    Object::Window(w) if w.id == id && w.live() => Some((i, w)),
                    _ => None,
                })
                .ok_or("layout journal names an unknown live window")?;
            let Object::Buffer(b) = self.object(w.buffer)? else {
                return Err("buffer required".into());
            };
            // A display with overlay strings ends at a display byte.
            let end = self.display_state.segments(id)
                .map_or(end, |segments| crate::display::buffer_of(segments, end, b.len_bytes()));
            let end_char = b.char_of_byte(coordinates::ByteOffset(end))?.0;
            changes.push((index, b.len_chars() - end_char));
        }
        self.display_state.hosted = true;
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
            let window = matches!(p, FrameSelectedWindow | FrameRootWindow)
                && self.window(a).is_ok_and(|w| !w.dead);
            if p != SelectedWindow && !window {
                self.frame_arg(a)?;
            }
            return Ok(self.window_root(if p == MinibufferWindow {
                MINI
            } else if p == FrameRootWindow {
                ROOT
            } else {
                SELECTED
            }));
        }
        if p == WindowResizeApply {
            self.frame_arg(a)?;
            let root = self.window_root(ROOT);
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
                        self.conses.push((k, v));
                        Value::reference(self.conses.len() - 1, 3)
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
                self.conses.push((l, r));
                Value::reference(self.conses.len() - 1, 3)
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
                let w = self.window_mut(v)?;
                if (u64::from(l) + u64::from(r) + 2) * u64::from(col) > u64::from(w.width)
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
pub(crate) struct TextArea {
    pub window: Value,
    pub buffer: Value,
    pub columns: i64,
    pub left_margin: i64,
    pub right_margin: i64,
    pub rightmost: bool,
    pub full_width: bool,
    pub hscroll: i64,
    pub mode_line: i64,
    pub header_line: i64,
    pub column: i64,
    pub line: i64,
    /// A host has laid the frame out and measures for it (VM2-WTPS1):
    /// then MODE_LINE is the host's band, and these are the text area's
    /// pixel width and height and its vertical scroll.
    pub hosted: bool,
    pub body_width: i64,
    pub body_height: i64,
    pub vscroll: i64,
}
/// The window surface `prims/window.rs` declares as primitives (GNU
/// window.c, frame.c and minibuf.c's `active-minibuffer-window`). Reached only
/// through `Leaf::windows`, so a body never holds `&mut Vm`; nothing here
/// evaluates Lisp or collects, and every Value it returns is already rooted
/// in a window, a slot or a fresh allocation.
pub(crate) struct Windows<'a> {
    vm: &'a mut Vm,
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
        let buffer = match self.vm.str_view(object).and_then(|v| v.as_str()).map(str::to_owned) {
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
    /// Decorations VM2 does not draw: tab lines, scroll bars and
    /// dividers are 0 pixels (window.c:1224-1269 on a live window).
    pub(crate) fn live_zero(&mut self, window: Value) -> Out {
        self.live(window)?;
        Ok(Value::integer(0).unwrap())
    }
    /// window.c:1215 `WINDOW_MODE_LINE_HEIGHT` over `window_wants_mode_line`
    /// (window.c:6037): a live non-minibuffer window whose `mode-line-format`
    /// parameter is not `none`, with that parameter or `mode-line-format`
    /// non-nil, taller than one canonical line. VM2 draws a mode line one
    /// canonical line tall; it reads `mode-line-format`'s current value cell,
    /// as display.rs `build_mode_line` does, not the window buffer's local
    /// value (DIVERGENCES.md VM2-WIN3).
    pub(crate) fn window_mode_line_height(&mut self, window: Value) -> Out {
        let w = self.live(window)?;
        let format = {
            let slot = self.vm.slot("mode-line-format");
            self.vm.variable_value(slot).map_err(internal)?
        };
        let line = self.vm.window_state.line;
        let key = self.vm.intern("mode-line-format");
        let none = self.vm.intern("none");
        let x = self.get(w)?;
        let parameter = x.parameters.iter().find(|(k, _)| *k == key).map_or(Value::NIL, |(_, v)| *v);
        let wants = !x.mini
            && parameter != none
            && (parameter != Value::NIL || (format != Value::UNBOUND && format != Value::NIL))
            && x.height > line;
        Self::int(if wants { i64::from(line) } else { 0 })
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
    /// remapping) or pixels. The width loses the margins and, on a frame
    /// that is not a window system's, the column of the vertical bar a
    /// window left of another has; the height loses the mode line. VM2 has
    /// no fringes, scroll bars, dividers or tab lines; the height also loses
    /// the header line.
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
        let Object::Buffer(b) = self.vm.object(buffer).map_err(internal)? else {
            return Err(internal("buffer required".into()));
        };
        let accessible = b.accessible();
        let char_at = |byte| b.char_of_byte(byte).map(|c| c.0 as i64 + 1).map_err(internal);
        let (begv, zv) = (char_at(accessible.start)?, char_at(accessible.end)?);
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
        let Object::Buffer(b) = self.vm.object(buffer).map_err(internal)? else {
            return Err(internal("buffer required".into()));
        };
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
        let mut mode_line = self.window_mode_line_height(w)?.integer_value().unwrap_or(0);
        if hosted && mode_line != 0 {
            mode_line = i64::from(self.vm.host_band_height(w, false).map_err(internal)?);
        }
        let header_line = self.window_header_line_height(w)?.integer_value().unwrap_or(0);
        let (column, line) = (i64::from(self.vm.window_state.column), i64::from(self.vm.window_state.line));
        let root = self.vm.window_root_peek(ROOT);
        let (root_x, root_width) = self.vm.window(root).map_or((0, 0), |r| (i64::from(r.x), i64::from(r.width)));
        let x = self.get(w)?;
        let (left, width) = (i64::from(x.x), i64::from(x.width));
        let body_width = (width - i64::from(x.left_margin + x.right_margin) * column).max(0);
        let body_height = (i64::from(x.height) - mode_line - header_line).max(0);
        Ok(TextArea {
            window: w,
            buffer: x.buffer,
            columns: width / column,
            left_margin: i64::from(x.left_margin),
            right_margin: i64::from(x.right_margin),
            rightmost: left + width >= root_x + root_width,
            full_width: left == root_x && width == root_width,
            hscroll: x.hscroll,
            mode_line,
            header_line,
            column,
            line,
            hosted,
            body_width,
            body_height,
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
        let chars = pixelwise == Value::NIL || remap;
        let size = if width {
            let (right, margins, pixels) = {
                let x = self.get(w)?;
                (i64::from(x.x) + i64::from(x.width), i64::from(x.left_margin + x.right_margin) * column, i64::from(x.width))
            };
            let root = self.vm.window_root_peek(ROOT);
            let rightmost = self.vm.window(root).map_or(true, |r| right >= i64::from(r.x) + i64::from(r.width));
            let window_system = {
                let slot = self.vm.slot("window-system");
                let value = self.vm.slots[slot].value;
                value != Value::NIL && value != Value::UNBOUND
            };
            let bar = if !window_system && !rightmost { column } else { 0 };
            let px = pixels - bar - margins;
            if chars { px / column } else { px }
        } else {
            let mode_line = self.window_mode_line_height(window)?.integer_value().unwrap_or(0);
            let header_line = self.window_header_line_height(window)?.integer_value().unwrap_or(0);
            let px = i64::from(self.get(w)?.height) - mode_line - header_line;
            if chars { px / line } else { px }
        };
        Self::int(size.max(0))
    }
    /// window.c:8639: (LEFT RIGHT OUTSIDE-MARGINS PERSISTENT). VM2 draws no
    /// fringes.
    pub(crate) fn window_fringes(&mut self, window: Value) -> Out {
        self.live(window)?;
        let zero = Value::integer(0).unwrap();
        Ok(self.vm.list_value(&[zero, zero, Value::NIL, Value::NIL]))
    }
    /// frame.c:4358/4376: VM2 reports backend LINE_HEIGHT / COLUMN_WIDTH,
    /// even though frame-live-p returns t (DIVERGENCES.md VM2-WIN5).
    pub(crate) fn frame_char_size(&mut self, frame: Value, horizontal: bool) -> Out {
        self.any_frame(frame)?;
        let unit = if horizontal { self.vm.window_state.column } else { self.vm.window_state.line };
        Self::int(i64::from(unit))
    }
    /// frame.c:4524/4531/4538: VM2 frames draw no window dividers and no
    /// internal border.
    pub(crate) fn frame_absent_width(&mut self, frame: Value) -> Out {
        self.any_frame(frame)?;
        Ok(Value::integer(0).unwrap())
    }
    /// frame.c:327: VM2 returns the initial/termcap type t, but retains
    /// graphical character metrics (documented hybrid, VM2-WIN5).
    /// decode_live_frame: the selected frame for nil, else FRAME if live.
    pub(crate) fn decode_live_frame(&mut self, frame: Value) -> std::result::Result<Value, Fault> {
        self.live_frame(frame)?;
        Ok(if frame == Value::NIL { self.frame() } else { frame })
    }
    /// GNU frame.c:2103 select-frame through do_switch_frame: selecting the
    /// selected frame returns it. VM2 has one live frame.
    pub(crate) fn select_frame(&mut self, frame: Value) -> Out {
        if self.frame_live_p(frame) == Value::NIL {
            return Err(Fault::Wrong("frame-live-p", frame));
        }
        if frame != self.frame() {
            return Err(internal("VM2 switching to another frame is unsupported".into()));
        }
        Ok(frame)
    }
    pub(crate) fn frame_live_p(&mut self, object: Value) -> Value {
        if matches!(self.vm.object_if(object), Some(Object::Frame(f)) if f.live) { Value::TRUE } else { Value::NIL }
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
        let state = &self.vm.window_state;
        Self::int(i64::from(if horizontal { state.width } else { state.height }))
    }
    /// FRAME_COLS and FRAME_LINES of the frame's text area.
    pub(crate) fn frame_text_size(&mut self) -> (i64, i64) {
        let state = &self.vm.window_state;
        (i64::from(state.width / state.column.max(1)), i64::from(state.height / state.line.max(1)))
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
    pub(crate) fn frame_p(&self, object: Value) -> Value {
        if matches!(self.vm.object_if(object), Some(Object::Frame(_))) { Value::TRUE } else { Value::NIL }
    }
    pub(crate) fn frame_list(&mut self) -> Out {
        let frame = self.frame();
        Ok(self.vm.list_value(&[frame]))
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
    /// output_initial kind, so every live frame is an initial frame.
    pub(crate) fn frame_initial_p(&mut self, frame: Value) -> Out {
        let frame = if frame == Value::NIL { self.frame() } else { frame };
        if let Some(Object::Frame(f)) = self.vm.object_if(frame) {
            return Ok(if f.live { Value::TRUE } else { Value::NIL });
        }
        // decode_terminal yields NULL for anything else: nil, not an error.
        Ok(if self.terminal(frame).is_some() { Value::TRUE } else { Value::NIL })
    }
    pub(crate) fn terminal_live_p(&mut self, object: Value) -> Value {
        if self.terminal(object).is_some() { Value::TRUE } else { Value::NIL }
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
            let view = self.vm.str_view(buffer_or_name).ok_or(Fault::Wrong("stringp", buffer_or_name))?;
            let name = view.as_str().ok_or(Fault::NotUnicode(buffer_or_name))?.to_owned();
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
    pub(crate) fn get_buffer_window(&mut self, buffer: Value, _all_frames: Value) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        let buffer = if buffer == Value::NIL {
            let index = self.vm.buffer_index().map_err(internal)?;
            Value::reference(index + 1, 0)
        }
        else if matches!(self.vm.object_if(buffer), Some(Object::Buffer(_) | Object::ByteBuffer(_))) { buffer }
        else {
            let name = self.vm.str_view(buffer).and_then(|v| v.as_str()).ok_or(Fault::Wrong("stringp", buffer))?.to_owned();
            let Some(buffer) = self.vm.named_buffer(&name) else { return Ok(Value::NIL); };
            buffer
        };
        let selected = self.vm.window_root(SELECTED);
        let (_, mut windows) = self.candidates(selected, Value::NIL, Value::TRUE)?;
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
    /// GNU window.c:1313 and set_window_hscroll: clipped to 0..most-positive-fixnum.
    pub(crate) fn set_window_hscroll(&mut self, window: Value, ncol: Value) -> Out {
        let n = ncol.integer_value().ok_or(Fault::Wrong("fixnump", ncol))?;
        let window = self.live(window)?;
        let n = n.clamp(0, Value::MAX_FIXNUM);
        let w = self.get_mut(window)?;
        if w.hscroll != n { w.end_valid = false; }
        w.hscroll = n;
        self.vm.display_state.version += 1;
        Self::int(n)
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
    /// minibuf.c:215: the minibuffer window while a minibuffer is active.
    pub(crate) fn active_minibuffer_window(&mut self) -> Out {
        self.vm.ensure_windows().map_err(internal)?;
        Ok(if self.minibuf_level() > 0 { self.vm.window_root(MINI) } else { Value::NIL })
    }
    /// GNU `window_list` for VM2's one frame: the leaves of the root window
    /// in tree order, then the minibuffer window (the root's `next`).
    fn all_windows(&self) -> std::result::Result<Vec<Value>, Fault> {
        let mut out = vec![];
        self.leaves(self.vm.window_root_peek(ROOT), &mut out)?;
        out.push(self.vm.window_root_peek(MINI));
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
        let mut out = vec![];
        for w in self.all_windows()? {
            let x = self.get(w)?;
            if !x.live() {
                continue;
            }
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
        let w = match self.vm.object_if(window) {
            Some(Object::Window(x)) if x.live() => window,
            _ => return Err(Fault::Wrong("window-live-p", window)),
        };
        self.vm.select_window_value(w, norecord == Value::NIL).map_err(internal)?;
        self.vm.display_state.version += 1;
        Ok(w)
    }
    /// window.c:3467 `Fdelete_other_windows_internal`, for VM2's one frame.
    pub(crate) fn delete_other_windows_internal(&mut self, window: Value, root: Value) -> Out {
        let w = self.valid(window)?;
        let frame_root = self.vm.window_root(ROOT);
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
            .plan_window_geometry(w, x, y, width, height, line, column, &mut geometry)
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
