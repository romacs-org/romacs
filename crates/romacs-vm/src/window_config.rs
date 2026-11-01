//! window.c window configurations: `current-window-configuration` records
//! each window object of the frame with its buffer, markers, sizes and
//! persistent parameters; `set-window-configuration` rebuilds that tree from
//! the same objects, reviving windows deleted since, as GNU does.
use super::*;

#[derive(Clone)]
pub(crate) struct SavedWindow {
    window: Value,
    /// The leaf's buffer; nil for an internal window.
    buffer: Value,
    /// Markers copied from the window's start and point (the buffer's
    /// point for the selected window); nil for an internal window.
    start: Value,
    point: Value,
    parent: Value,
    children: Vec<Value>,
    horizontal: bool,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    normal_lines: Value,
    normal_cols: Value,
    left_margin: u32,
    right_margin: u32,
    /// GNU `p->left_fringe_width' and the rest (window.c:8296).
    fringes: [Option<u32>; 2],
    fringes_outside_margins: bool,
    fringes_persistent: bool,
    vscroll_px: u32,
    hscroll: i64,
    combination_limit: Value,
    /// GNU `p->dedicated`.
    dedicated: Value,
    /// (PARAMETER . VALUE) for each parameter `window-persistent-parameters'
    /// makes persistent, VALUE nil where the window had none.
    parameters: Vec<(Value, Value)>,
}
romacs_snapshot::snapshot_struct!(SavedWindow { window, buffer, start, point, parent, children, horizontal, x, y, width, height, normal_lines, normal_cols, left_margin, right_margin, fringes, fringes_outside_margins, fringes_persistent, vscroll_px, hscroll, combination_limit, dedicated, parameters });

/// GNU `struct save_window_data`: one frame's windows, and the frame
/// selected when they were saved.
#[derive(Clone)]
pub(crate) struct WindowConfiguration {
    frame: Value,
    selected_frame: Value,
    current_window: Value,
    current_buffer: Value,
    root: Value,
    /// The root's subtree in preorder, then the minibuffer window.
    windows: Vec<SavedWindow>,
}
romacs_snapshot::snapshot_struct!(WindowConfiguration { frame, selected_frame, current_window, current_buffer, root, windows });

/// What Fset_window_configuration restores after its record_buffer: the
/// buffer recorded, the frame to select again, and the buffer current at
/// saving with its point (`finish_window_configuration').
pub(crate) struct RestoredConfiguration {
    pub recorded: Option<Value>,
    pub reselect: Value,
    pub new_current: Value,
    pub old_point: Value,
}

impl WindowConfiguration {
    pub(crate) fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) {
        for v in [&mut self.frame, &mut self.selected_frame, &mut self.current_window, &mut self.current_buffer, &mut self.root] {
            f(v);
        }
        for w in &mut self.windows {
            for v in [&mut w.window, &mut w.buffer, &mut w.start, &mut w.point, &mut w.parent,
                      &mut w.normal_lines, &mut w.normal_cols, &mut w.combination_limit, &mut w.dedicated] {
                f(v);
            }
            for v in &mut w.children {
                f(v);
            }
            for (k, v) in &mut w.parameters {
                f(k);
                f(v);
            }
        }
    }
    pub(crate) fn storage_bytes(&self) -> usize {
        size_of::<Self>()
            + self.windows.iter().map(|w| {
                size_of::<SavedWindow>()
                    + w.children.capacity() * size_of::<Value>()
                    + w.parameters.capacity() * size_of::<(Value, Value)>()
            }).sum::<usize>()
    }
}

impl Vm {
    fn config_buffer_live(&self, buffer: Value) -> bool {
        buffer != Value::NIL && self.buffer_state(buffer).is_ok_and(|s| s.live)
    }
    /// The PARAMETERs of `window-persistent-parameters' entries (PARAMETER
    /// . PERSISTENCE) whose PERSISTENCE is non-nil.
    fn persistent_window_parameters(&mut self) -> Result<Vec<Value>> {
        let Some(&slot) = self.names.get("window-persistent-parameters") else { return Ok(vec![]) };
        let mut tail = self.variable_value(slot)?;
        let mut out = vec![];
        // A circular value saves nothing, as GNU resets it.
        for _ in 0..4096 {
            let Ok((entry, rest)) = self.cons_pair(tail) else { return Ok(out) };
            if let Ok((parameter, persistence)) = self.cons_pair(entry) {
                if persistence != Value::NIL {
                    out.push(parameter);
                }
            }
            tail = rest;
        }
        Ok(vec![])
    }
    /// GNU save_window_save: WINDOW and its subtree, preorder.
    fn save_window(&mut self, window: Value, persistent: &[Value], out: &mut Vec<SavedWindow>) -> Result<()> {
        let w = self.window(window)?.clone();
        let (start, point) = if w.children.is_empty() && w.buffer != Value::NIL {
            let point = self.window_point_value(window)?;
            let start = self.marker_position_value(w.start)?;
            (self.window_marker(w.buffer, start)?, self.window_marker(w.buffer, point)?)
        } else {
            (Value::NIL, Value::NIL)
        };
        let parameters = persistent.iter().map(|&p| {
            (p, w.parameters.iter().find(|(k, _)| *k == p).map_or(Value::NIL, |(_, v)| *v))
        }).collect();
        out.push(SavedWindow {
            window,
            buffer: if w.children.is_empty() { w.buffer } else { Value::NIL },
            start,
            point,
            parent: w.parent,
            children: w.children.clone(),
            horizontal: w.horizontal,
            x: w.x,
            y: w.y,
            width: w.width,
            height: w.height,
            normal_lines: w.normal_lines,
            normal_cols: w.normal_cols,
            left_margin: w.left_margin,
            right_margin: w.right_margin,
            fringes: w.fringes,
            fringes_outside_margins: w.fringes_outside_margins,
            fringes_persistent: w.fringes_persistent,
            vscroll_px: w.vscroll_px,
            hscroll: w.hscroll,
            combination_limit: w.combination_limit,
            dedicated: w.dedicated,
            parameters,
        });
        for child in w.children {
            self.save_window(child, persistent, out)?;
        }
        Ok(())
    }
    /// Fcurrent_window_configuration for FRAME (nil, the selected frame).
    pub(super) fn current_window_configuration(&mut self, frame: Value) -> Result<Value> {
        self.ensure_windows()?;
        let frame = if frame == Value::NIL { self.frame_value_peek() } else { frame };
        let persistent = self.persistent_window_parameters()?;
        let mut windows = vec![];
        let (root, mini, current_window) = { let f = self.frame(frame)?; (f.root, f.minibuffer, f.selected) };
        self.save_window(root, &persistent, &mut windows)?;
        // A child frame uses its parent's minibuffer window.
        if self.window(mini)?.frame == frame {
            self.save_window(mini, &persistent, &mut windows)?;
        }
        let config = WindowConfiguration {
            frame,
            selected_frame: self.frame_value_peek(),
            current_window,
            current_buffer: self.slots[self.buffer_slot].value,
            root,
            windows,
        };
        Ok(self.allocate_object(Object::WindowConfiguration(Box::new(config))))
    }
    /// POSITION's character position, clipped to BUFFER's accessible region
    /// (set_marker_restricted).
    fn config_restricted(&mut self, buffer: Value, position: Value) -> Result<Value> {
        let n = match self.marker_position_value(position) {
            Ok(v) if v != Value::NIL => v.as_integer()?,
            _ => 1,
        };
        let Object::Buffer(b) = self.object(buffer)? else { return Value::integer(n) };
        let r = b.accessible();
        let (low, high) = (b.char_of_byte(r.start)?.0 as i64 + 1, b.char_of_byte(r.end)?.0 as i64 + 1);
        Value::integer(n.clamp(low, high))
    }
    /// Point of the current buffer to POSITION (goto-char, already clipped).
    fn config_goto(&mut self, position: Value) -> Result<()> {
        let n = position.as_integer()?;
        let buffer = self.slots[self.buffer_slot].value;
        if let Object::Buffer(b) = self.object_mut(buffer)? {
            b.point = b.byte_of_char(coordinates::CharOffset((n - 1) as usize))?;
        }
        Ok(())
    }
    /// Show BUFFER in leaf WINDOW with start START and point POINT. A window
    /// given another buffer than it showed drops it from its buffer history.
    fn config_show(&mut self, window: Value, buffer: Value, start: Value, point: Value, previous: Value) -> Result<()> {
        if buffer != previous {
            self.discard_buffer_from_window(buffer, window)?;
        }
        let (start, point) = (self.config_restricted(buffer, start)?, self.config_restricted(buffer, point)?);
        let (start_marker, point_marker) = (self.window(window)?.start, self.window(window)?.point);
        let start_marker = if start_marker == Value::NIL { self.window_marker(buffer, start)? } else {
            self.set_marker_object(start_marker, start, buffer)?;
            start_marker
        };
        let point_marker = if point_marker == Value::NIL { self.window_marker(buffer, point)? } else {
            self.set_marker_object(point_marker, point, buffer)?;
            point_marker
        };
        let w = self.window_mut(window)?;
        w.buffer = buffer;
        w.start = start_marker;
        w.point = point_marker;
        Ok(())
    }
    /// Fset_window_configuration up to its record_buffer: None when the
    /// configuration's frame is dead (the value is nil); else the buffer it
    /// recorded, whose `buffer-list-update-hook' the caller runs there, and
    /// what `finish_window_configuration' restores after it.
    pub(super) fn set_window_configuration(&mut self, config: Value, dont_set_frame: Value, dont_set_miniwindow: Value) -> Result<Option<RestoredConfiguration>> {
        self.ensure_windows()?;
        let entry_frame = self.frame_value_peek();
        let Some(Object::WindowConfiguration(data)) = self.object_if(config) else {
            return Err("window configuration required".into());
        };
        let data = (**data).clone();
        let current = self.slots[self.buffer_slot].value;
        let new_current = if self.config_buffer_live(data.current_buffer) { data.current_buffer } else { Value::NIL };
        let selected = self.window_root(SELECTED);
        let current_window_buffer = self.window(data.current_window)?.buffer;
        // Point of the buffer to make current, chosen as GNU does so that one
        // window's point is not copied into another.
        let old_point = if new_current == Value::NIL {
            Value::NIL
        } else if current_window_buffer == new_current && selected != data.current_window
            && (new_current != current || self.window(selected)?.buffer == new_current) {
            self.marker_position_value(self.window(data.current_window)?.point)?
        } else {
            let Object::Buffer(b) = self.object(new_current)? else { return Err("buffer required".into()) };
            Value::integer(b.char_of_byte(b.point)?.0 as i64 + 1)?
        };
        // Swap out the selected window's point into its marker.
        let selected_buffer = self.window(selected)?.buffer;
        if selected_buffer != Value::NIL {
            let point = self.window_point_value(selected)?;
            self.set_marker_object(self.window(selected)?.point, point, selected_buffer)?;
        }
        // delete_all_child_windows: every window of the current tree is
        // deleted; a leaf remembers its buffer in case it is restored.
        let frame = data.frame;
        if !self.frame(frame)?.live { return Ok(None); }
        let mut tree = vec![self.frame(frame)?.root];
        let mut kept = HashMap::default();
        let mut at = 0;
        while at < tree.len() {
            let v = tree[at];
            at += 1;
            let w = self.window(v)?.clone();
            tree.extend(w.children.iter().copied());
            if w.children.is_empty() && w.buffer != Value::NIL {
                kept.insert(v.0, w.buffer);
            }
            let w = self.window_mut(v)?;
            w.dead = true;
            w.children.clear();
        }
        for p in &data.windows {
            let mini = self.window(p.window)?.mini;
            {
                let w = self.window_mut(p.window)?;
                w.dead = false;
                if !mini {
                    w.parent = p.parent;
                    w.children = p.children.clone();
                    w.horizontal = p.horizontal;
                    (w.x, w.y, w.width, w.height) = (p.x, p.y, p.width, p.height);
                    w.normal_lines = p.normal_lines;
                    w.normal_cols = p.normal_cols;
                }
                w.left_margin = p.left_margin;
                w.right_margin = p.right_margin;
                (w.fringes, w.fringes_outside_margins, w.fringes_persistent) = (p.fringes, p.fringes_outside_margins, p.fringes_persistent);
                w.vscroll_px = p.vscroll_px;
                w.hscroll = p.hscroll;
                w.combination_limit = p.combination_limit;
                // Saved parameters are restored; a nil one only resets an
                // existing non-nil value. Others are left alone.
                for &(parameter, value) in &p.parameters {
                    match w.parameters.iter_mut().find(|(k, _)| *k == parameter) {
                        Some((_, old)) => *old = value,
                        None if value != Value::NIL => w.parameters.push((parameter, value)),
                        None => {}
                    }
                }
            }
            if p.buffer == Value::NIL && !p.children.is_empty() {
                self.window_mut(p.window)?.buffer = Value::NIL;
                continue;
            }
            // window.c:7889: every leaf gets its saved dedication back.
            self.window_mut(p.window)?.dedicated = p.dedicated;
            if (dont_set_miniwindow == Value::NIL || !mini) && self.config_buffer_live(p.buffer) {
                let previous = kept.get(&p.window.0).copied().unwrap_or(Value::NIL);
                self.config_show(p.window, p.buffer, p.start, p.point, previous)?;
                // Point is not restored in the buffer current when the
                // configuration was recorded.
                if p.buffer != new_current && p.buffer == current {
                    let point = self.marker_position_value(self.window(p.window)?.point)?;
                    self.config_goto(point)?;
                }
            } else if let Some(&buffer) = kept.get(&p.window.0).filter(|&&b| self.config_buffer_live(b)) {
                // The window keeps the buffer it showed.
                let point = self.window(p.window)?.point;
                let start = self.window(p.window)?.start;
                self.config_show(p.window, buffer, start, point, buffer)?;
            } else if !mini {
                let other = self.create_named_buffer("*scratch*", false)?;
                let one = Value::integer(1)?;
                self.config_show(p.window, other, one, one, Value::NIL)?;
                // Its buffer is gone: the window is no more dedicated.
                self.window_mut(p.window)?.dedicated = Value::NIL;
            }
        }
        // Windows of the old tree that the configuration does not name stay
        // deleted.
        let restored: HashSet<u64> = data.windows.iter().map(|p| p.window.0).collect();
        for v in tree {
            if !restored.contains(&v.0) {
                self.retire_window(v)?;
            }
        }
        self.frame_mut(frame)?.root = data.root;
        if self.window(data.current_window)?.buffer == new_current {
            let (marker, buffer) = (self.window(data.current_window)?.point, new_current);
            self.set_marker_object(marker, old_point, buffer)?;
        }
        // select_window, recording the use and the buffer (window.c:8068,
        // Bug#16207), without swapping out the point of the old selected
        // window, which was saved above.
        let window = data.current_window;
        let mut recorded = None;
        if self.window(window)?.live() {
            self.frame_mut(frame)?.selected = window;
            let slot = self.slot(crate::frame::SELECTED_FRAME);
            self.slots[slot].value = frame;
            let point = self.marker_position_value(self.window(window)?.point)?;
            self.set_buffer_point_for_window(window, point)?;
            self.window_state.select_count += 1;
            let count = self.window_state.select_count;
            self.window_mut(window)?.use_time = count;
            let buffer = self.window(window)?.buffer;
            self.slots[self.buffer_slot].value = buffer;
            self.record_buffer_model(buffer);
            recorded = Some(buffer);
        }
        self.display_state.version += 1;
        let reselect = if dont_set_frame == Value::NIL { data.selected_frame } else { entry_frame };
        Ok(Some(RestoredConfiguration { recorded, reselect, new_current, old_point }))
    }
    /// Fset_window_configuration after its record_buffer: select_window made
    /// the configuration's frame the selected one; RESELECT, the frame
    /// selected at saving or with DONT-SET-FRAME at entry, is selected again
    /// (window.c:8075), and NEW-CURRENT, the buffer current at saving, is
    /// current again, at OLD-POINT unless the selected window shows it.
    pub(super) fn finish_window_configuration(&mut self, reselect: Value, new_current: Value, old_point: Value) -> Result<Value> {
        if reselect != self.frame_value_peek() && self.frame(reselect)?.live {
            self.select_frame_value(reselect, false)?;
        }
        if new_current != Value::NIL && self.config_buffer_live(new_current) {
            self.slots[self.buffer_slot].value = new_current;
            let selected = self.window_root(SELECTED);
            if self.window(selected)?.buffer != new_current {
                self.config_goto(old_point)?;
            }
        }
        self.display_state.version += 1;
        Ok(Value::TRUE)
    }
    /// The configuration's windows that will show another live buffer than
    /// now, not a minibuffer: Fset_window_configuration records each one's
    /// current buffer (record-window-buffer) before restoring.
    pub(super) fn window_configuration_records(&mut self, config: Value) -> Result<Vec<Value>> {
        let Some(Object::WindowConfiguration(data)) = self.object_if(config) else {
            return Err("window configuration required".into());
        };
        let saved: Vec<(Value, Value)> = data.windows.iter().map(|p| (p.window, p.buffer)).collect();
        let mut out = vec![];
        for (window, buffer) in saved {
            let current = self.window(window)?.buffer;
            let minibuffer = self.minibuffer_window_buffer() == Some(buffer);
            if current != Value::NIL && current != buffer && self.config_buffer_live(buffer) && !minibuffer {
                out.push(window);
            }
        }
        Ok(out)
    }
    pub(super) fn window_configuration_frame(&mut self, config: Value) -> Result<Value> {
        match self.object_if(config) {
            Some(Object::WindowConfiguration(c)) => Ok(c.frame),
            _ => Err("window configuration required".into()),
        }
    }
}

impl Windows<'_> {
    fn configuration(&mut self, config: Value) -> std::result::Result<(), Fault> {
        match self.vm.object_if(config) {
            Some(Object::WindowConfiguration(_)) => Ok(()),
            _ => Err(Fault::Wrong("window-configuration-p", config)),
        }
    }
    pub(crate) fn current_window_configuration(&mut self, frame: Value) -> Out {
        self.live_frame(frame)?;
        self.vm.current_window_configuration(frame).map_err(internal)
    }
    pub(crate) fn set_window_configuration(&mut self, config: Value, dont_set_frame: Value, dont_set_miniwindow: Value) -> std::result::Result<Option<RestoredConfiguration>, Fault> {
        self.configuration(config)?;
        self.vm.set_window_configuration(config, dont_set_frame, dont_set_miniwindow).map_err(internal)
    }
    pub(crate) fn finish_window_configuration(&mut self, reselect: Value, new_current: Value, old_point: Value) -> Out {
        self.vm.finish_window_configuration(reselect, new_current, old_point).map_err(internal)
    }
    pub(crate) fn window_configuration_records(&mut self, config: Value) -> std::result::Result<Vec<Value>, Fault> {
        self.configuration(config)?;
        self.vm.window_configuration_records(config).map_err(internal)
    }
    /// The saved point markers of CONFIG, with the buffer each points into.
    pub(crate) fn configuration_points(&mut self, config: Value) -> std::result::Result<Vec<(Value, Value)>, Fault> {
        self.configuration(config)?;
        let Some(Object::WindowConfiguration(data)) = self.vm.object_if(config) else { unreachable!("checked") };
        Ok(data.windows.iter().filter(|p| p.point != Value::NIL).map(|p| (p.point, p.buffer)).collect())
    }
    pub(crate) fn set_marker_advances(&mut self, marker: Value) -> std::result::Result<(), Fault> {
        self.vm.set_marker_bias(marker, crate::anchors::Bias::Right).map_err(internal)
    }
    pub(crate) fn window_configuration_p(&mut self, object: Value) -> Value {
        if matches!(self.vm.object_if(object), Some(Object::WindowConfiguration(_))) { Value::TRUE } else { Value::NIL }
    }
    pub(crate) fn window_configuration_frame(&mut self, config: Value) -> Out {
        self.configuration(config)?;
        self.vm.window_configuration_frame(config).map_err(internal)
    }
}
