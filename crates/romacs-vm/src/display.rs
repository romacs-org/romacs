//! G1a's single VM-owned frame/window. Only owned data leaves this module.
use crate::{coordinates::ByteOffset, data::Object, *};
#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
#[path = "mode_line.rs"]
mod mode_line;
use romacs_face::{
    Attribute, AttrsBuilder, Background, Base, BoundedDatum, Deffaces, LispGraph, NodeView,
    Slot as FaceSlot, Theme, Value as FaceValue,
};
use romacs_layout_input::{Face, FaceHandle, FaceRun, TextSlice, WindowSnapshot};

pub(super) struct DisplayState {
    theme: Theme,
    pub(super) font_selection_order: romacs_layout_input::FontSelectionState,
    font_alternatives: romacs_layout_input::FontAlternativeState,
    /// Last publication **per window**, keyed by the window's stable id. One
    /// shared snapshot made every window compare against whichever window
    /// published last, so alternating between two windows bumped the version
    /// every time and repainted the whole frame for nothing.
    ///
    /// Pruned in `delete_window_value`: ids are monotonic and never reused, so
    /// an unpruned map would grow for the life of the VM once windows start
    /// churning.
    previous: std::collections::BTreeMap<u64, WindowSnapshot>,
    pub version: u64,
    /// Whether a host has applied a layout: batch never does (VM2-POSVIS1).
    pub(super) hosted: bool,
    pub(super) width: u32,
    pub(super) height: u32,
    building_band: bool,
    diagnostics: Vec<String>,
}
impl DisplayState {
    pub(super) fn set_font_alternative_families(&mut self, names: Vec<Vec<String>>) {
        self.font_alternatives.set_family_alist(names);
        self.previous.clear();
        self.version = self.version.wrapping_add(1);
    }
    /// free_all_realized_faces: republish every window with fresh faces.
    pub(super) fn free_realized_faces(&mut self) {
        self.previous.clear();
        self.version = self.version.wrapping_add(1);
    }
    pub(super) fn set_font_selection_order(&mut self, order: [romacs_layout_input::FontSelectionAttribute; 4]) {
        if self.font_selection_order.set_order(order) {
            self.previous.clear();
            self.version = self.version.wrapping_add(1);
        }
    }
    /// Forget a deleted window's last publication. Ids are monotonic and never
    /// reused, so without this the map grows for the life of the VM.
    pub(super) fn forget_window(&mut self, id: u64) {
        self.previous.remove(&id);
    }
    pub fn new() -> Self {
        let mut attrs = AttrsBuilder::default();
        for a in Attribute::ALL {
            let v = match a {
                Attribute::Family => FaceValue::Str("Liberation Mono".into()),
                Attribute::Foundry => FaceValue::Str("unspecified".into()),
                Attribute::Foreground | Attribute::DistantForeground => {
                    FaceValue::Str("#ffffff".into())
                }
                Attribute::Background => FaceValue::Str("#000000".into()),
                Attribute::Height => FaceValue::Height(romacs_face::Height::Absolute(150)),
                Attribute::Width | Attribute::Weight | Attribute::Slant => {
                    FaceValue::Symbol("normal")
                }
                _ => FaceValue::Bool(false),
            };
            attrs.set(a, FaceSlot::Set(v));
        }
        let mut defs = Deffaces::default();
        defs.defface(
            "default",
            &BoundedDatum::parse("((t ()))").expect("default spec"),
        )
        .expect("default face");
        defs.defface(
            "mode-line",
            &BoundedDatum::parse("((t (:background \"#303030\")))").expect("mode-line spec"),
        )
        .expect("mode-line face");
        Self {
            font_selection_order: romacs_layout_input::FontSelectionState::default(),
            font_alternatives: romacs_layout_input::FontAlternativeState::default(),
            previous: std::collections::BTreeMap::new(),
            theme: Theme::build(
                Arc::new(defs),
                None,
                Background::Dark,
                Base::new(attrs.build()).expect("absolute frame base"),
            )
            .expect("default theme"),
            version: 0,
            hosted: false,
            width: 0,
            height: 0,
            building_band: false,
            diagnostics: vec![],
        }
    }
}
struct Graph<'a>(&'a Vm);
impl LispGraph for Graph<'_> {
    type Node = u64;
    fn view(&self, n: u64) -> NodeView<'_, u64> {
        let v = Value(n);
        if v == Value::NIL {
            return NodeView::Nil;
        }
        if v == Value::TRUE {
            return NodeView::T;
        }
        if let Some(i) = v.integer_value() {
            return NodeView::Int(i);
        }
        if let Some(s) = self.0.source_string_if(v) {
            return NodeView::Str(s);
        }
        if self.0.str_view(v).is_some() {
            return NodeView::Other("not-unicode-string");
        }
        if v.tag() == 7 {
            let i = v.payload() as usize;
            if i > 0
                && !self.0.generated_slots.contains_key(&(i - 1))
                && let Some(s) = self.0.symbol_names.get(i - 1)
            {
                return NodeView::Symbol(s);
            }
        }
        if let Ok((a, b)) = self.0.cons_pair(v) {
            return NodeView::Cons(a.0, b.0);
        }
        if let Some(Object::Float(f)) = self.0.object_if(v) {
            return NodeView::Float(*f);
        }
        NodeView::Other("unsupported face datum")
    }
}
impl Vm {
    /// Freeze every live leaf and the inactive minibuffer; no heap handles escape.
    pub fn frame_snapshot(&mut self, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot> {
        let font_alternatives = self.font_alternative_state()?;
        let windows = self.frame_window_snapshots(width, height)?
            .into_iter()
            .map(|(x, y, snapshot)| romacs_layout_input::PlacedWindowSnapshot { x, y, snapshot })
            .collect();
        Ok(romacs_layout_input::FrameSnapshot { font_selection: self.display_state.font_selection_order, font_alternatives, width, height, windows })
    }
    /// The exact list returned by the family setter, held in a lazy traced slot.
    pub fn font_alternative_family_root(&self) -> crate::handle::Local<'_> {
        crate::handle::Local::new(self, self.font_alternative_family_value())
    }
    fn font_alternative_family_value(&self) -> Value {
        self.names.get(prim::FONT_ALTERNATIVE_FAMILIES)
            .map(|&slot| self.slots[slot].value)
            .filter(|&value| value != Value::UNBOUND)
            .unwrap_or(Value::NIL)
    }
    /// Freeze live Lisp state, not a stale copy from the setter. Malformed later
    /// user mutation is an error rather than silently missing fallback rows.
    pub fn font_alternative_state(&mut self) -> Result<romacs_layout_input::FontAlternativeState> {
        fn spine(vm: &Vm, original: Value) -> Result<Vec<Value>> {
            let mut seen = std::collections::HashSet::new();
            let mut tail = original;
            let mut values = Vec::new();
            while tail != Value::NIL {
                if !seen.insert(tail.0) {
                    return Err("circular-list in alternative font family state".into());
                }
                let (car, cdr) = vm.cons_pair(tail)
                    .map_err(|_| "listp required in alternative font family state")?;
                values.push(car);
                tail = cdr;
            }
            Ok(values)
        }
        let mut rows = Vec::new();
        for row in spine(self, self.font_alternative_family_value())? {
            let mut names = Vec::new();
            for symbol in spine(self, row)? {
                let name = if symbol == Value::NIL { "nil" }
                    else if symbol == Value::TRUE { "t" }
                    else {
                        symbol.index_if(7).and_then(|index| index.checked_sub(1))
                            .and_then(|slot| self.symbol_names.get(slot))
                            .map(String::as_str)
                            .ok_or("symbolp required in alternative font family state")?
                    };
                names.push(name.to_owned());
            }
            rows.push(names);
        }
        if rows != self.display_state.font_alternatives.family_alist() {
            // Direct setcar/setcdr bypasses the setter. Invalidate when its
            // changed effective state is first observed, before publication.
            self.display_state.set_font_alternative_families(rows);
        }
        Ok(self.display_state.font_alternatives.clone())
    }
    pub fn font_selection_order(&self) -> [romacs_layout_input::FontSelectionAttribute; 4] {
        self.display_state.font_selection_order.order()
    }
    pub fn font_selection_state(&self) -> romacs_layout_input::FontSelectionState {
        self.display_state.font_selection_order
    }
    pub fn display_default_face(&self) -> Result<Face> {
        self.snapshot_face("default")
    }

    /// Commit scalar display mutations only after the frozen frame has painted.
    pub fn display_apply_journal(&mut self) -> bool {
        let mut changed = false;
        for object in &mut self.objects {
            if let Object::Window(w) = object
                && let Some(next) = w.pending_vscroll.take()
                && next != w.vscroll_px
            {
                w.vscroll_px = next;
                changed = true;
            }
        }
        if changed {
            self.display_state.version += 1;
        }
        changed
    }
    /// The Lisp face registry is authoritative; the display theme follows a
    /// frame's `internal-set-lisp-face-attribute`. A value the theme cannot
    /// represent stays in Lisp and is reported, never signalled.
    pub(crate) fn theme_face_attribute(&mut self, face: &str, keyword: &str, value: Value) {
        // faces.el gives a frame without a window system the terminal's own
        // colours, "unspecified-fg"/"unspecified-bg" (tty-colors); the theme
        // realizes those as its base colours.
        let sentinel = self.str_view(value).is_some_and(|v| matches!(v.bytes(), b"unspecified-fg" | b"unspecified-bg"));
        let value = if sentinel { self.intern("unspecified") } else { value };
        let next = BoundedDatum::from_graph(&Graph(self), value.0).map_err(|e| e.to_string())
            .and_then(|datum| self.display_state.theme.with_face_attribute(face, keyword, &datum).map_err(|e| e.to_string()));
        match next {
            Ok(next) => {
                self.display_state.theme = next;
                self.display_state.free_realized_faces();
            }
            Err(error) => self.display_state.diagnostics.push(format!("face {face} {keyword}: {error}")),
        }
    }
    pub fn take_display_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.display_state.diagnostics)
    }
    fn display_symbol(&mut self, name: &str) -> Value {
        Value::reference(self.slot(name) + 1, 7)
    }
    pub(super) fn display_buffer(&mut self, v: Value) -> Result<Value> {
        let v = if let Some(view) = self.str_view(v) {
            let name = view.as_str().ok_or("not-unicode-string")?.to_owned();
            let slot = self.slot(&format!("#:vm2-buffer:{name}"));
            self.slots[slot].value
        } else {
            v
        };
        if matches!(self.object_if(v), Some(Object::Buffer(b)) if b.state.live) {
            Ok(v)
        } else {
            Err("live Unicode buffer required".into())
        }
    }
    pub(super) fn display_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        if matches!(
            p,
            SelectedWindow | WindowBuffer | WindowPoint | WindowStart
        ) || (p as usize >= DeleteWindowInternal as usize
            && p as usize <= WindowTotalWidth as usize)
        {
            return self.window_primitive(p, start, n);
        }
        let valid = match p {
            DisplayDefineFace => n == 2,
            SelectedFrame => n == 0,
            WindowVscroll => n <= 2,
            SetWindowVscroll => n == 3,
            DisplayError => n >= 1,
            _ => n <= 1,
        };
        if !valid {
            return Err("wrong primitive arity".into());
        }
        if p == DisplayError {
            let message = self.format_string(start, n)?;
            return Err(self.source_string(message)?.to_owned());
        }
        if p == DisplayDefineFace {
            let name = self.stack[start];
            let face = match Graph(self).view(name.0) {
                NodeView::Symbol(s) => s.to_owned(),
                _ => return Err("face symbol required".into()),
            };
            let spec = BoundedDatum::from_graph(&Graph(self), self.stack[start + 1].0)
                .map_err(|e| e.to_string())?;
            let next = self
                .display_state
                .theme
                .with_defface(&face, &spec)
                .map_err(|e| e.to_string())?;
            let definition = next.definition(&face).ok_or("missing declared face")?.clone();
            self.display_state.theme = next;
            self.display_state.version += 1;
            self.register_declared_face(name, definition)?;
            return Ok(name);
        }
        let frame = matches!(p, SelectedFrame | FramePixelWidth | FramePixelHeight);
        let identity = if frame {
            self.selected_frame_value()?
        } else {
            self.selected_window_value()?
        };
        if frame && n > 0 && self.stack[start] != Value::NIL && self.stack[start] != identity {
            return Err("unknown frame/window".into());
        }
        match p {
            SelectedFrame => Ok(identity),
            WindowVscroll => {
                if n != 2 || self.stack[start + 1] == Value::NIL {
                    return Err("VM2 window-vscroll currently requires pixelwise t".into());
                }
                let window = self.decode_window(self.stack[start], true, false)?;
                Value::integer(self.window(window)?.vscroll_px as i64)
            }
            SetWindowVscroll => {
                if self.stack[start + 2] == Value::NIL {
                    return Err("VM2 set-window-vscroll currently requires pixelwise t".into());
                }
                let next = u32::try_from(self.stack[start + 1].as_integer()?)
                    .map_err(|_| "nonnegative pixel vscroll required")?;
                let window = self.decode_window(self.stack[start], true, false)?;
                let building_band = self.display_state.building_band;
                let (deferred, changed) = {
                    let w = self.window_mut(window)?;
                    if building_band {
                        let mut deferred = false;
                        if next != w.vscroll_px || w.pending_vscroll.is_some() {
                            deferred = w.pending_vscroll.is_none();
                            w.pending_vscroll = Some(next);
                        }
                        (deferred, false)
                    } else {
                        let changed = w.vscroll_px != next;
                        w.vscroll_px = next;
                        (false, changed)
                    }
                };
                if deferred {
                    self.display_state.diagnostics.push(
                        ":eval: deferred set-window-vscroll until after paint (D-ML1)".into(),
                    );
                }
                if changed {
                    self.display_state.version += 1;
                }
                Value::integer(next as i64)
            }
            FramePixelWidth => Value::integer(self.display_state.width as i64),
            FramePixelHeight => Value::integer(self.display_state.height as i64),
            _ => Err("unknown display primitive".into()),
        }
    }
    /// Copy the selected window into Seam L. No heap handle or borrowed storage escapes.
    /// `buffer_version` is a conservative VM display revision (also changes on face,
    /// point and window mutations); hosts may compare it to request a redraw.
    /// Read-only: how many windows hold a published snapshot. The publication
    /// map is Rust-side state, not a Lisp heap object, so no heap-object or
    /// live-byte count can observe a leak in it; a test has to ask directly.
    pub fn published_window_count(&self) -> usize {
        self.display_state.previous.len()
    }
    /// Read-only: the display version a renderer uses to decide whether to repaint.
    pub fn display_version(&self) -> u64 {
        self.display_state.version
    }
    pub fn display_snapshot(&mut self, width: u32, height: u32) -> Result<WindowSnapshot> {
        let window = self.selected_window_value()?;
        self.snapshot_window(window, width, height)
    }
    pub(super) fn snapshot_window(
        &mut self,
        window: Value,
        width: u32,
        height: u32,
    ) -> Result<WindowSnapshot> {
        let root = self.slot("#:vm2-snapshot-window");
        let slot = self.slot("#:vm2-window-buffer");
        self.slots[root].value = window;
        self.slots[slot].value = self.window(window)?.buffer;
        let outcome = self.snapshot_window_inner(root, slot, width, height);
        self.slots[root].value = Value::UNBOUND;
        self.slots[slot].value = Value::UNBOUND;
        outcome
    }
    fn snapshot_region(&mut self, window: Value, buffer: Value, point: i64) -> Result<Option<std::ops::Range<usize>>> {
        if window != self.selected_window_value()? { return Ok(None); }
        let saved = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = buffer;
        let active = (|| -> Result<bool> {
            let mark = self.slot("mark-active");
            let transient = self.slot("transient-mark-mode");
            Ok(self.variable_value(mark)? != Value::NIL && self.variable_value(transient)? != Value::NIL)
        })();
        self.slots[self.buffer_slot].value = saved;
        if !active? { return Ok(None); }
        let Some(marker) = self.buffer_mark_value(buffer) else { return Ok(None); };
        let Some(Object::Marker(mark)) = self.object_if(marker) else { return Ok(None); };
        if mark.buffer != buffer { return Ok(None); }
        let Some(mark) = self.marker_position_value(marker)?.integer_value() else { return Ok(None); };
        let Object::Buffer(b) = self.object(buffer)? else { return Ok(None); };
        let accessible = b.accessible();
        let mark = b.byte_of_char(crate::coordinates::CharOffset((mark - 1).max(0) as usize))?.0
            .clamp(accessible.start.0, accessible.end.0);
        let point = b.byte_of_char(crate::coordinates::CharOffset((point - 1).max(0) as usize))?.0
            .clamp(accessible.start.0, accessible.end.0);
        Ok(Some(mark.min(point)..mark.max(point)))
    }
    fn snapshot_window_inner(
        &mut self,
        root: usize,
        slot: usize,
        width: u32,
        height: u32,
    ) -> Result<WindowSnapshot> {
        let mode_line = self.redisplay_and_build_mode_line(root, slot, height)?;
        let window = self.slots[root].value;
        let band_face = self.mode_line_face_name(window)?;
        let w = self.window(window)?;
        if !w.live() {
            return Err("window deleted during snapshot".into());
        }
        let (id, buffer, start) = (w.id, w.buffer, w.start);
        self.slots[slot].value = buffer;
        let start = self
            .marker_position_value(start)?
            .integer_value()
            .unwrap_or(1);
        let point = self.window_point_value(window)?.as_integer()?;
        let region = self.snapshot_region(window, buffer, point)?;
        let Object::Buffer(b) = self.object(buffer)? else {
            return Err("Unicode display buffer required".into());
        };
        let utf8 = b.as_str().to_owned();
        let point_byte = b
            .byte_of_char(crate::coordinates::CharOffset((point - 1) as usize))?
            .0;
        let start_byte = b
            .byte_of_char(crate::coordinates::CharOffset((start - 1) as usize))?
            .0;
        let (names, mut faces) = self.display_faces()?;
        let terminal = self.terminal_bindings.iter().find(|binding| binding.buffer == buffer);
        if let Some(binding) = terminal {
            if let Some(color) = binding.default_colors[0] { faces[0].foreground = color; }
            if let Some(color) = binding.default_colors[1] { faces[0].background = color; }
        }
        let default = faces[0].clone();
        let styles = terminal.map_or(&[][..], |binding| binding.styles.as_ref());
        let mut style_index = 0;
        let mut terminal_faces = std::collections::HashMap::new();
        let mut region_faces = std::collections::HashMap::new();
        let region_background = names.iter().position(|name| name == "region").map(|i| faces[i].background);
        let mut face_runs: Vec<FaceRun> = vec![];
        for (at, ch) in utf8.char_indices() {
            let value = b
                .properties
                .get_property(ByteOffset(at), "face")
                .unwrap_or(Value::NIL);
            let mut face = self.painted_face(&names, value)?.0;
            while styles.get(style_index).is_some_and(|run| run.bytes.end <= at) { style_index += 1; }
            if let Some(run) = styles.get(style_index).filter(|run| run.bytes.contains(&at)) {
                face = *terminal_faces.entry(run.style).or_insert_with(|| {
                    let resolved = run.style.face(&default);
                    if resolved == default { return FaceHandle(0); }
                    let handle = FaceHandle(faces.len() as u32);
                    faces.push(resolved);
                    handle
                });
            }
            if region.as_ref().is_some_and(|range| range.contains(&at)) {
                if let Some(background) = region_background {
                    face = *region_faces.entry(face.0).or_insert_with(|| {
                        let mut selected = faces[face.0 as usize].clone();
                        selected.background = background;
                        let handle = FaceHandle(faces.len() as u32);
                        faces.push(selected);
                        handle
                    });
                }
            }
            if let Some(last) = face_runs.last_mut()
                && last.face == face
            {
                last.bytes.end = at + ch.len_utf8()
            } else {
                face_runs.push(FaceRun {
                    bytes: at..at + ch.len_utf8(),
                    face,
                })
            }
        }
        // No version bump here: this runs once per WINDOW, and
        // `display_state.width/height` is frame-wide, so each window's own
        // dimensions looked like a frame resize and every `frame_snapshot`
        // republished all windows as changed -- the same shared-state
        // mistake as the formerly shared `previous` snapshot. A genuine
        // resize is still caught, because width and height are fields of
        // the snapshot and so are compared per window below.
        self.display_state.width = width;
        self.display_state.height = height;
        let mut snapshot = WindowSnapshot {
            id,
            width,
            height,
            start_byte,
            point_byte,
            buffer_version: self.display_state.version,
            text: TextSlice { utf8, face_runs },
            mode_line: mode_line.map(|band| {
                let handle = |face: &str| names.iter().position(|n| n == face);
                TextSlice {
                    face_runs: band.runs.into_iter()
                        .filter_map(|(bytes, face)| Some(FaceRun { bytes, face: FaceHandle(handle(&face)? as u32) }))
                        .collect(),
                    utf8: band.text,
                }
            }),
            mode_line_face: FaceHandle(
                names
                    .iter()
                    .position(|n| *n == band_face)
                    .ok_or("mode-line face required")? as u32,
            ),
            vscroll_px: self.window(window)?.vscroll_px,
            faces,
        };
        // The owned last publication also covers mutation paths with specialised
        // fast dispatch (point moves, regexp, yank), without retaining GC handles.
        // Keyed per window: one shared snapshot made each window compare
        // against whichever window published last, so alternating between
        // two unchanged windows repainted the frame every time.
        if let Some(previous) = self.display_state.previous.get(&id) {
            let mut comparable = previous.clone();
            comparable.buffer_version = snapshot.buffer_version;
            if comparable != snapshot {
                self.display_state.version += 1;
            }
        } else {
            // A window's first publication is itself a change.
            self.display_state.version += 1;
        }
        snapshot.buffer_version = self.display_state.version;
        self.display_state.previous.insert(id, snapshot.clone());
        Ok(snapshot)
    }
    /// GNU `redisplay_window` order for one window: make its buffer current,
    /// keep point visible, then `display_mode_line` (`src/xdisp.c:28200`)
    /// with that window's own -- possibly buffer-local -- `mode-line-format`.
    fn redisplay_and_build_mode_line(&mut self, root: usize, buffer_slot: usize, height: u32) -> Result<Option<mode_line::Band>> {
        let saved = self.slot("#:vm2-band-saved-current-buffer");
        self.slots[saved].value = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = self.slots[buffer_slot].value;
        self.display_state.building_band = true;
        let outcome = self.redisplay_window_inner(root, buffer_slot, height);
        self.display_state.building_band = false;
        if self.buffer_state(self.slots[saved].value).is_ok_and(|b| b.live) {
            self.slots[self.buffer_slot].value = self.slots[saved].value;
        }
        self.slots[saved].value = Value::UNBOUND;
        outcome
    }
    fn redisplay_window_inner(&mut self, root: usize, buffer_slot: usize, height: u32) -> Result<Option<mode_line::Band>> {
        if self.window(self.slots[root].value)?.mini {
            return Ok(None);
        }
        let format_slot = self.slot("mode-line-format");
        let value = self.variable_value(format_slot)?;
        // Convert before entering Lisp: neither traversal tails nor environments
        // are retained as untraced heap Values across a collecting :eval. An
        // object with no source spelling displays as GNU's *invalid* where it
        // is itself an element (`mode_line_form`).
        let format = if value == Value::UNBOUND || value == Value::NIL {
            None
        } else {
            Some(self.mode_line_form(value))
        };
        let band_height = if format.is_some() { self.window_state.line } else { 0 };
        let text_height = height.saturating_sub(band_height);
        self.redisplay_point_visibility(self.slots[root].value, text_height)?;
        let Some(format) = format else { return Ok(None) };
        let state = self.mode_line_state(root, buffer_slot, text_height)?;
        // GNU redisplay_window installs a non-selected window's point before
        // display_mode_lines (xdisp.c:20620-20641), then restores buffer point
        // (21626-21645). Do this after visibility has finalized window-point:
        // that pass can move the window marker for a forced window-start.
        let buffer = self.slots[buffer_slot].value;
        let Object::Buffer(b) = self.object(buffer)? else {
            return Err("Unicode display buffer required".into());
        };
        let saved_point = b.char_of_byte(b.point)?;
        let begv = b.char_of_byte(b.accessible().start)?.0 + 1;
        let zv = b.char_of_byte(b.accessible().end)?.0 + 1;
        let real_selected = self.selected_window_value()?;
        if self.slots[root].value != real_selected {
            let point = self.window_point_value(self.slots[root].value)?.as_integer()?;
            let point = Value::integer(point.clamp(begv as i64, zv as i64))?;
            self.set_buffer_point_for_window(self.slots[root].value, point)?;
        }
        // GNU display_mode_lines (xdisp.c:28114-28156) selects W only
        // while evaluating its band, without select-window's point effects.
        // The saved selection is a slot root; the temporary selection belongs
        // to the Frame, so both edges move with a collecting :eval.
        let saved = self.slot("#:vm2-band-saved-selected-window");
        self.slots[saved].value = real_selected;
        self.frame_mut(self.frame_value_peek())?.selected = self.slots[root].value;
        let outcome = self.mode_line_band(&format, &state).map(Some);
        self.frame_mut(self.frame_value_peek())?.selected = self.slots[saved].value;
        self.slots[saved].value = Value::UNBOUND;
        // Resolve the traced buffer again after collecting :eval, restoring
        // by character position if Lisp changed its text. As in GNU's restore,
        // a saved point beyond the new ZV falls back to the buffer's true end.
        let buffer = self.slots[buffer_slot].value;
        let Object::Buffer(b) = &mut self.objects[buffer.index(0)? - 1] else {
            return Err("Unicode display buffer required".into());
        };
        let accessible = b.accessible();
        let begv = b.char_of_byte(accessible.start)?;
        let zv = b.char_of_byte(accessible.end)?;
        b.point = if saved_point < begv {
            accessible.start
        } else if saved_point > zv {
            b.byte_of_char(crate::coordinates::CharOffset(b.len_chars()))?
        } else {
            b.byte_of_char(saved_point)?
        };
        // snapshot_window_inner chooses the band face after restoration,
        // using the real selection, not the temporary evaluation context.
        outcome
    }
    /// GNU `Fformat_mode_line` (`src/xdisp.c:29058`): FORMAT expanded as
    /// WINDOW's band (the selected window by default) for BUFFER (WINDOW's
    /// by default), with WINDOW selected and BUFFER current meanwhile, by
    /// the engine that builds the band. FACE nil gives the characters only
    /// the faces their `:propertize' gives; t, the window's mode-line face;
    /// another face, that face where they have none and appended where they
    /// have one; an integer, no properties. Batch formats nothing.
    pub(crate) fn format_mode_line(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() || args.len() > 4 {
            return Err("wrong primitive arity".into());
        }
        let arg = |i: usize| args.get(i).copied().unwrap_or(Value::NIL);
        let (format, face, window, buffer) = (arg(0), arg(1), arg(2), arg(3));
        self.ensure_windows()?;
        let window = if window == Value::NIL { self.selected_window_value()? } else { window };
        if !matches!(self.object_if(window), Some(Object::Window(_))) {
            let predicate = self.intern("windowp");
            return Err(self.signal_error("wrong-type-argument", &[predicate, window], "Wrong type argument: windowp".into()));
        }
        // GNU's `w->contents': an internal window's first child.
        let buffer = if buffer != Value::NIL { buffer } else {
            let w = self.window(window)?;
            w.children.first().copied().unwrap_or(w.buffer)
        };
        if !matches!(self.object_if(buffer), Some(Object::Buffer(_) | Object::ByteBuffer(_))) {
            let predicate = self.intern("bufferp");
            return Err(self.signal_error("wrong-type-argument", &[predicate, buffer], "Wrong type argument: bufferp".into()));
        }
        if !self.buffer_state(buffer)?.live {
            return Err("Attempt to format a mode line for a dead buffer".into());
        }
        let noninteractive = self.slot("noninteractive");
        let target = if face.integer_value().is_some() || matches!(self.object_if(face), Some(Object::Bignum(_))) {
            mode_line::Target::NoProp
        } else {
            mode_line::Target::String
        };
        if format == Value::NIL || self.variable_value(noninteractive)? != Value::NIL {
            return Ok(self.allocate_object(Object::String(Box::new(data::LispString::from_bytes(vec![], false)))));
        }
        let face = if target == mode_line::Target::NoProp || face == Value::NIL || face == self.intern("default") {
            Value::NIL
        } else if face == Value::TRUE {
            let selected = window == self.selected_window_value()?;
            self.intern(if selected { "mode-line-active" } else { "mode-line-inactive" })
        } else {
            face
        };
        // A format can call format-mode-line again from :eval: each call
        // roots its window, buffer and the selection it restores in slots
        // of its own depth.
        let depth = (0..).find(|d| { let slot = self.slot(&format!("#:vm2-format-mode-line-{d}")); self.slots[slot].value == Value::UNBOUND }).unwrap();
        let [root, buffer_slot, face_slot, saved_window, saved_buffer] = ["", "-buffer", "-face", "-selected", "-current"]
            .map(|suffix| self.slot(&format!("#:vm2-format-mode-line-{depth}{suffix}")));
        self.slots[root].value = window;
        self.slots[buffer_slot].value = buffer;
        self.slots[face_slot].value = face;
        self.slots[saved_window].value = self.selected_window_value()?;
        self.slots[saved_buffer].value = self.slots[self.buffer_slot].value;
        let format = self.mode_line_form(format);
        let outcome = (|| {
            let height = self.window(window)?.height.saturating_sub(self.window_state.line);
            self.frame_mut(self.frame_value_peek())?.selected = window;
            self.slots[self.buffer_slot].value = buffer;
            let state = self.mode_line_state(root, buffer_slot, height)?;
            self.mode_line_expand(&format, &state, target)
        })();
        self.frame_mut(self.frame_value_peek())?.selected = self.slots[saved_window].value;
        if self.buffer_state(self.slots[saved_buffer].value).is_ok_and(|b| b.live) {
            self.slots[self.buffer_slot].value = self.slots[saved_buffer].value;
        }
        let face = self.slots[face_slot].value;
        for slot in [root, buffer_slot, face_slot, saved_window, saved_buffer] { self.slots[slot].value = Value::UNBOUND; }
        let band = outcome?;
        let elt_slot = self.slot(mode_line::ELT_RESTARTED);
        self.slots[elt_slot].value = Value::integer(band.elt)?;
        self.mode_line_string(&band, face)
    }
    /// BAND's text with GNU's properties: each piece's `mode-line-elt-no'
    /// and face, its `:propertize' face with FACE after it, else FACE.
    /// GNU adds a piece's element number before a FACE it adds but after a
    /// face the piece already had.
    fn mode_line_string(&mut self, band: &mode_line::Band, face: Value) -> Result<Value> {
        let text = &band.text;
        // VM2 keys string properties by interned name.
        self.intern("mode-line-elt-no");
        let mut string = data::LispString::from_bytes(text.as_bytes().to_vec(), !text.is_ascii());
        let mut bounds: Vec<usize> = band.runs.iter().map(|(r, _)| r).chain(band.elts.iter().map(|(r, _)| r))
            .flat_map(|r| [r.start, r.end]).chain([0, text.len()]).collect();
        bounds.sort_unstable();
        bounds.dedup();
        for span in bounds.windows(2) {
            let range = ByteOffset(span[0])..ByteOffset(span[1]);
            let own = band.runs.iter().find(|(r, _)| r.contains(&span[0])).map(|(_, name)| self.intern(name));
            let elt = band.elts.iter().find(|(r, _)| r.contains(&span[0])).map(|(_, n)| *n);
            let added = match own {
                Some(own) if face != Value::NIL => Some(self.list_value(&[own, face])),
                Some(own) => Some(own),
                None => (face != Value::NIL).then_some(face),
            };
            // A later put lists its property first.
            if own.is_none() && let Some(face) = added { string.properties.put(range.clone(), "face", face); }
            if let Some(n) = elt { string.properties.put(range.clone(), "mode-line-elt-no", Value::integer(n)?); }
            if own.is_some() && let Some(face) = added { string.properties.put(range, "face", face); }
        }
        Ok(self.allocate_object(Object::String(Box::new(string))))
    }
    /// The window and buffer facts `decode_mode_spec` reads, after scrolling.
    fn mode_line_state(&mut self, root: usize, buffer_slot: usize, text_height: u32) -> Result<mode_line::SpecState> {
        let buffer = self.slots[buffer_slot].value;
        let buffer_name = self
            .symbol_names
            .iter()
            .enumerate()
            .find(|(slot, name)| {
                name.starts_with("#:vm2-buffer:") && self.slots[*slot].value == buffer
            })
            .map(|(_, name)| name.trim_start_matches("#:vm2-buffer:").to_owned())
            .unwrap_or_default();
        let window = self.slots[root].value;
        let start = self.marker_position_value(self.window(window)?.start)?.integer_value().unwrap_or(1) as usize;
        let point = self.window_point_value(window)?.as_integer()? as usize;
        let tab_width = {
            let slot = self.slot("tab-width");
            self.variable_value(slot)?.integer_value().filter(|w| (1..=1000).contains(w)).unwrap_or(8) as usize
        };
        let ctl_arrow = {
            let slot = self.slot("ctl-arrow");
            self.variable_value(slot)? != Value::NIL
        };
        let (_, displayed) = self.window_rows(text_height);
        let frame_cols = (self.window_state.width / self.window_state.column.max(1)).max(1) as usize;
        let Object::Buffer(b) = self.object(buffer)? else {
            return Err("Unicode display buffer required".into());
        };
        let text = b.as_str();
        let accessible = b.accessible();
        let (begv_byte, zv_byte) = (accessible.start.0, accessible.end.0);
        let char_at = |byte: usize| b.char_of_byte(ByteOffset(byte)).map(|c| c.0 + 1);
        // Another buffer than the window's (format-mode-line's BUFFER) may be shorter.
        let byte_at = |pos: usize| b.byte_of_char(crate::coordinates::CharOffset(pos.clamp(1, b.len_chars() + 1) - 1)).map(|o| o.0.clamp(begv_byte, zv_byte));
        let (start_byte, point_byte) = (byte_at(start)?, byte_at(point)?);
        // Window end: after the last displayed row, as GNU's window_end_pos.
        let mut end_byte = start_byte;
        for _ in 0..displayed {
            match text[end_byte..zv_byte].find('\n') {
                Some(newline) => end_byte += newline + 1,
                None => {
                    end_byte = zv_byte;
                    break;
                }
            }
        }
        let line_start = text[begv_byte..point_byte].rfind('\n').map_or(begv_byte, |n| begv_byte + n + 1);
        // GNU current_column: tab stops, ^X control characters, wide characters.
        let mut column = 0;
        for c in text[line_start..point_byte].chars() {
            column += match c {
                '\t' => tab_width - column % tab_width,
                '\u{0}'..='\u{1f}' | '\u{7f}' => if ctl_arrow { 2 } else { 4 },
                '\u{80}'..='\u{9f}' => 4,
                _ => unicode_width::UnicodeWidthChar::width(c).unwrap_or(0),
            };
        }
        Ok(mode_line::SpecState {
            buffer_name,
            start: char_at(start_byte)?,
            end: char_at(end_byte)?,
            begv: char_at(begv_byte)?,
            zv: char_at(zv_byte)?,
            z: b.len_chars() + 1,
            line: text.as_bytes()[begv_byte..point_byte].iter().filter(|&&c| c == b'\n').count() + 1,
            column,
            modified: b.state.modified(),
            frame_cols,
        })
    }
    /// GNU's `CURRENT_MODE_LINE_ACTIVE_FACE_ID`: the selected window draws
    /// `mode-line-active`, others `mode-line-inactive` while
    /// `mode-line-in-non-selected-windows` is non-nil (its C default). Without
    /// faces.el's definitions a missing face falls back to `mode-line`.
    fn mode_line_face_name(&mut self, window: Value) -> Result<String> {
        let selected = window == self.selected_window_value()?;
        let slot = self.slot("mode-line-in-non-selected-windows");
        let distinguish = self.variable_value(slot)? != Value::NIL;
        let wanted = if selected || !distinguish { "mode-line-active" } else { "mode-line-inactive" };
        let defined = self.display_state.theme.face_names().any(|n| n == wanted);
        Ok(if defined { wanted } else { "mode-line" }.to_owned())
    }
    /// The face table a snapshot publishes: `default' first, then the
    /// theme's other faces.
    fn display_faces(&self) -> Result<(Vec<String>, Vec<Face>)> {
        let names: Vec<String> = std::iter::once("default")
            .chain(self.display_state.theme.face_names().filter(|s| *s != "default"))
            .map(str::to_owned)
            .collect();
        let faces = names.iter().map(|n| self.snapshot_face(n)).collect::<Result<Vec<_>>>()?;
        Ok((names, faces))
    }
    /// The face a snapshot paints a character whose `face' property is
    /// VALUE with: a symbol, or a list's first element, by name in NAMES,
    /// else the default. The second value says whether that is the whole
    /// of VALUE: nil, a symbol or a one-element list of one.
    fn painted_face(&self, names: &[String], value: Value) -> Result<(FaceHandle, bool)> {
        let (resolved, whole) = if value.tag() == 3 {
            let (head, tail) = self.cons_pair(value)?;
            (head, tail == Value::NIL)
        } else {
            (value, true)
        };
        let graph = Graph(self);
        let (name, symbol) = match graph.view(resolved.0) {
            _ if resolved == Value::NIL => ("default", true),
            NodeView::Symbol(s) => (s, true),
            _ => ("default", false),
        };
        Ok((FaceHandle(names.iter().position(|n| n == name).unwrap_or(0) as u32), whole && symbol))
    }
    /// The host geometry of the characters at BYTES of the current
    /// buffer, each in the face a snapshot paints it with (VM2-WTPS1),
    /// with whether the host paints it so: not for a control character
    /// other than newline, nor when its `face' is other than nil, a symbol
    /// or a one-element list of one, it has `font-lock-face', or an
    /// overlay's `face' covers it.
    pub(crate) fn host_measure_buffer(&mut self, bytes: std::ops::Range<usize>) -> Result<(romacs_layout_input::Measured, Vec<bool>)> {
        let (names, faces) = self.display_faces()?;
        let buffer = self.slots[self.buffer_slot].value;
        let mut covered = vec![];
        self.visit_overlays_in(buffer, ByteOffset(bytes.start)..ByteOffset(bytes.end), |overlay, range| covered.push((overlay, range)))?;
        let face = self.intern("face");
        let mut faced = vec![];
        for (overlay, range) in covered {
            if self.overlay_get(overlay, face)? != Value::NIL { faced.push(range); }
        }
        let Object::Buffer(b) = self.object(buffer)? else { return Err("Unicode display buffer required".into()) };
        let original = b.as_str().get(bytes.clone()).ok_or("measured range is not text")?;
        let mut text = String::with_capacity(original.len());
        let mut face_runs: Vec<FaceRun> = vec![];
        let mut exact = vec![];
        for (at, ch) in original.char_indices() {
            let byte = ByteOffset(bytes.start + at);
            let value = b.properties.get_property(byte, "face").unwrap_or(Value::NIL);
            let (face, whole) = self.painted_face(&names, value)?;
            // The host paints no control character but newline; a space
            // stands in for one, inexact, and the iterator refuses it anyway.
            let control = ch.is_control() && ch != '\n';
            exact.push(!control && whole && b.properties.get_property(byte, "font-lock-face").is_none_or(|v| v == Value::NIL)
                && !faced.iter().any(|range| range.contains(&byte)));
            let start = text.len();
            text.push(if control { ' ' } else { ch });
            match face_runs.last_mut() {
                Some(last) if last.face == face => last.bytes.end = text.len(),
                _ => face_runs.push(FaceRun { bytes: start..text.len(), face }),
            }
        }
        let input = romacs_layout_input::MeasureInput {
            font_selection: self.display_state.font_selection_order,
            font_alternatives: self.font_alternative_state()?,
            faces,
            text: TextSlice { utf8: text, face_runs },
        };
        Ok((self.host_layout(&input)?, exact))
    }
    /// The host advance of C in the default face, and that face's row height.
    pub(crate) fn host_default_advance(&mut self, c: char) -> Result<(u32, u32)> {
        let face = self.snapshot_face("default")?;
        let input = romacs_layout_input::MeasureInput {
            font_selection: self.display_state.font_selection_order,
            font_alternatives: self.font_alternative_state()?,
            faces: vec![face],
            text: TextSlice { utf8: c.to_string(), face_runs: vec![] },
        };
        let measured = self.host_layout(&input)?;
        Ok((measured.glyphs[0].0, measured.face_heights[0]))
    }
    /// The height the host gives WINDOW's mode line: GNU's
    /// CURRENT_MODE_LINE_HEIGHT (dispextern.h), the last displayed band's,
    /// else an estimate from the window's mode-line face.
    pub(crate) fn host_mode_line_height(&mut self, window: Value) -> Result<u32> {
        let id = self.window(window)?.id;
        let shown = self.display_state.previous.get(&id).and_then(|snapshot| snapshot.mode_line.as_ref().map(|band| (snapshot, band)));
        let (faces, runs) = match shown {
            Some((snapshot, band)) => (snapshot.faces.clone(), std::iter::once(snapshot.mode_line_face.0 as usize)
                .chain(band.face_runs.iter().map(|run| run.face.0 as usize)).collect::<Vec<_>>()),
            None => {
                let (names, faces) = self.display_faces()?;
                let band = self.mode_line_face_name(window)?;
                let index = names.iter().position(|n| *n == band).ok_or("mode-line face required")?;
                (faces, vec![index])
            }
        };
        let input = romacs_layout_input::MeasureInput {
            font_selection: self.display_state.font_selection_order,
            font_alternatives: self.font_alternative_state()?,
            faces,
            text: TextSlice { utf8: String::new(), face_runs: vec![] },
        };
        let heights = self.host_layout(&input)?.face_heights;
        Ok(runs.into_iter().map(|i| heights[i]).max().unwrap_or(0))
    }
    fn snapshot_face(&self, name: &str) -> Result<Face> {
        let attrs = self
            .display_state
            .theme
            .realize(name)
            .ok_or("unknown face")?;
        let string = |a| match attrs.get(a) {
            FaceSlot::Set(FaceValue::Str(s)) => Ok(s.to_string()),
            _ => Err("face string required".to_owned()),
        };
        let color = |a| -> Result<[u8; 3]> {
            let s = string(a)?;
            let (red, green, blue) = romacs_display_protocol::x11_color_value(&s)
                .ok_or_else(|| format!("invalid face colour {s:?} for {name}"))?;
            Ok([red, green, blue])
        };
        let size = match attrs.get(Attribute::Height) {
            FaceSlot::Set(FaceValue::Height(romacs_face::Height::Absolute(h))) => {
                *h as f32 * 96.0 / 720.0
            }
            _ => return Err("absolute face height required".into()),
        };
        Ok(Face {
            foreground: color(Attribute::Foreground)?,
            background: color(Attribute::Background)?,
            family: string(Attribute::Family)?,
            pixel_size: size,
        })
    }
}
