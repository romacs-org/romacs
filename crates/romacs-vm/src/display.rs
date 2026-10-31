//! G1a's single VM-owned frame/window. Only owned data leaves this module.
use crate::{coordinates::ByteOffset, data::Object, *};
#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
#[path = "mode_line.rs"]
mod mode_line;
#[path = "mini_window.rs"]
mod mini_window;
pub(crate) use mini_window::buffer_of;
use romacs_face::{
    Attribute, AttrsBuilder, Background, Base, BoundedDatum, Deffaces, LispGraph, NodeView,
    Slot as FaceSlot, Theme, Value as FaceValue,
};
use romacs_layout_input::{Face, FaceHandle, FaceRun, TextSlice, WindowSnapshot};

/// The face table a snapshot publishes: `default' first, then the theme's
/// other faces, realized once per theme revision rather than per frame.
pub(super) struct FaceTable {
    pub faces: Vec<Face>,
    index: std::collections::HashMap<String, u32>,
}
impl FaceTable {
    fn handle(&self, name: &str) -> Option<FaceHandle> {
        self.index.get(name).map(|&i| FaceHandle(i))
    }
}
romacs_snapshot::snapshot_struct!(FaceTable { faces, index });


/// What laying a window out from its start established, in its buffer's
/// bytes: where its last displayed row ends, where its fully visible rows
/// end (MAX: through the end of its text) and its text area's height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Laid {
    pub end: usize,
    pub visible_end: usize,
    pub body_height: f32,
}
romacs_snapshot::snapshot_struct!(Laid { end, visible_end, body_height });

/// A window's text as a snapshot publishes it (`Vm::window_text`): byte
/// offsets are into TEXT, which starts at buffer byte BASE.
struct ShownText {
    text: TextSlice,
    start_byte: usize,
    point_byte: usize,
    faces: Vec<Face>,
    base: usize,
    /// TEXT reaches the end of the accessible text, not only the last row
    /// the window can show.
    at_zv: bool,
    table: std::rc::Rc<FaceTable>,
    /// The merged face rows FACES gained, by `face' value.
    merged: std::collections::HashMap<u64, FaceHandle>,
    /// Where TEXT is the display elements of the buffer (`display'
    /// strings and spaces, overlay strings, invisible text left out), not
    /// its characters: which buffer bytes its bytes show, and its stretches.
    /// BASE is then 0 and the segments hold buffer bytes.
    segments: Option<Vec<mini_window::Segment>>,
    stretches: Vec<romacs_layout_input::Stretch>,
    line_numbers: Option<romacs_layout_input::LineNumbers>,
}

/// The labels LABELS gives the lines starting at STARTS, (display byte,
/// buffer byte) in TEXT, whose accessible part starts at BEGV; none for
/// the empty line at the end of a text of LEN bytes reaching ZV.
fn line_numbers(table: &FaceTable, text: &str, begv: usize, labels: &crate::prim::LineLabels, starts: Vec<(usize, usize)>, len: usize, at_zv: bool)
    -> romacs_layout_input::LineNumbers {
    let face = table.handle("line-number").unwrap_or(FaceHandle(0));
    let current = table.handle("line-number-current-line").unwrap_or(face);
    let (mut from, mut line) = (begv, 0i64);
    let mut out = vec![];
    for (display, buffer) in starts {
        if display == len && at_zv { break; }
        line += text.as_bytes()[from.min(buffer)..buffer].iter().filter(|&&c| c == b'\n').count() as i64;
        from = from.max(buffer);
        out.push((display, labels.label(line), line == labels.point_line));
    }
    romacs_layout_input::LineNumbers { columns: (labels.width + 2) as u32, face, current, labels: out }
}

/// What a window's display elements read for their faces: its buffer,
/// the overlays there with a `face' that apply to it, in increasing
/// priority, and the buffer face's layers and realized face from byte
/// RUN.0 to RUN.1; with the faces realized for them.
struct ElementFaces {
    buffer: Value,
    face: Value,
    overlays: Vec<(std::ops::Range<usize>, Value)>,
    /// The text whose property runs bound RUN.
    end: usize,
    run: (usize, usize, FaceHandle, Vec<Value>),
    /// No `char-property-alias-alist' entry or `default-text-properties'
    /// can give text a face.
    plain: bool,
    /// The last element's position and byte.
    cursor: (i64, usize),
    merged: std::collections::HashMap<u64, FaceHandle>,
    layered: std::collections::HashMap<Vec<u64>, FaceHandle>,
}

impl ElementFaces {
    /// For WINDOW's BUFFER over the bytes SPAN.
    fn new(vm: &mut Vm, window: Value, buffer: Value, span: std::ops::Range<usize>) -> Result<Self> {
        let face = vm.display_symbol("face");
        let end = span.end;
        let mut covering = vec![];
        if span.start < span.end {
            vm.visit_overlays_in(buffer, ByteOffset(span.start)..ByteOffset(span.end), |overlay, range| covering.push((overlay, range)))?;
        }
        let mut overlays = vec![];
        for (overlay, range) in covering {
            let value = vm.overlay_get(overlay, face)?;
            if value != Value::NIL && range.start < range.end && vm.overlay_applies_to(overlay, window) {
                overlays.push((vm.overlay_order_key(overlay), range.start.0..range.end.0, value));
            }
        }
        overlays.sort_by_key(|o| o.0);
        let aliases = vm.list_values(vm.annotation_variable("char-property-alias-alist"))?;
        let plain = vm.annotation_variable("default-text-properties") == Value::NIL
            && !aliases.into_iter().any(|entry| vm.cons_pair(entry).is_ok_and(|(key, _)| key == face));
        Ok(ElementFaces { buffer, face, overlays: overlays.into_iter().map(|(_, range, value)| (range, value)).collect(), end,
            run: (0, 0, FaceHandle(0), vec![]), plain, cursor: (0, 0), merged: Default::default(), layered: Default::default() })
    }
}

impl ShownText {
    /// The buffer byte TEXT's byte DISPLAY shows.
    fn buffer_byte(&self, display: usize) -> usize {
        self.segments.as_deref().map_or(self.base + display, |s| shown_byte(s, display))
    }
}

pub(super) struct DisplayState {
    /// The theme, without the face edits still pending: read it through
    /// [`DisplayState::theme`].
    theme: std::cell::RefCell<Theme>,
    /// `internal-set-lisp-face-attribute` edits since the theme was last
    /// read, in order. Recomputing a frame's faces sets each face's every
    /// attribute; one build for them all replaces a build per attribute.
    pending_faces: std::cell::RefCell<Vec<(String, String, BoundedDatum)>>,
    /// THEME's realized face table, until THEME or a pending edit changes.
    face_table: Option<std::rc::Rc<FaceTable>>,
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
    /// Per window id, what its last full snapshot read (`ShownKey`), for
    /// GNU's redisplay shortcuts (`Vm::reuse_shown`).
    shown: std::collections::BTreeMap<u64, Shown>,
    /// Advanced by every overlay change: overlays are display state no
    /// buffer modification tick counts.
    pub(super) overlays: u64,
    /// Per window id, how its last publication's display text maps to
    /// buffer text, when overlay strings were spliced in.
    segments: std::collections::BTreeMap<u64, Vec<mini_window::Segment>>,
    /// Per window id, the buffer byte its last publication's text starts at.
    bases: std::collections::BTreeMap<u64, usize>,
    /// Advances when what fonts faces realize may change.
    face_revision: u64,
    /// The least height a painted row has, for a face revision.
    row_floor: Option<(u64, u32)>,
    pub version: u64,
    /// Whether a host has applied a layout: batch never does (VM2-POSVIS1).
    pub(super) hosted: bool,
    pub(super) width: u32,
    pub(super) height: u32,
    building_band: bool,
    diagnostics: std::cell::RefCell<Vec<String>>,
}
romacs_snapshot::snapshot_struct!(DisplayState { theme, pending_faces, face_table, font_selection_order, font_alternatives, previous, shown, overlays, segments, bases, face_revision, row_floor, version, hosted, width, height, building_band, diagnostics });
impl DisplayState {
    /// The theme with every pending face edit applied: built once for them
    /// all, or, when that fails, one edit at a time, so an edit the theme
    /// rejects is reported and the rest apply, as each would have alone.
    fn theme(&self) -> std::cell::Ref<'_, Theme> {
        let edits = std::mem::take(&mut *self.pending_faces.borrow_mut());
        if !edits.is_empty() {
            let mut theme = self.theme.borrow_mut();
            match theme.with_face_attributes(edits.iter().map(|(f, k, v)| (f.as_str(), k.as_str(), v))) {
                Ok(next) => *theme = next,
                Err(_) => {
                    for (face, keyword, value) in &edits {
                        match theme.with_face_attribute(face, keyword, value) {
                            Ok(next) => *theme = next,
                            Err(error) => self.diagnostics.borrow_mut().push(format!("face {face} {keyword}: {error}")),
                        }
                    }
                }
            }
        }
        self.theme.borrow()
    }
    fn set_theme(&mut self, theme: Theme) {
        *self.theme.get_mut() = theme;
        self.faces_changed();
    }
    /// The theme changed, or will at its next read: what faces realize
    /// may differ.
    fn faces_changed(&mut self) {
        self.face_table = None;
        self.face_revision += 1;
    }
    pub(super) fn set_font_alternative_families(&mut self, names: Vec<Vec<String>>) {
        self.font_alternatives.set_family_alist(names);
        self.face_revision += 1;
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
            self.face_revision += 1;
            self.previous.clear();
            self.version = self.version.wrapping_add(1);
        }
    }
    /// Republish window ID in full next time (force-window-update).
    pub(super) fn repaint_window(&mut self, id: u64) {
        self.previous.remove(&id);
        self.version = self.version.wrapping_add(1);
    }
    /// Forget a deleted window's last publication. Ids are monotonic and never
    /// reused, so without this the map grows for the life of the VM.
    pub(super) fn forget_window(&mut self, id: u64) {
        self.previous.remove(&id);
        self.shown.remove(&id);
        self.segments.remove(&id);
        self.bases.remove(&id);
    }
    /// How window ID's last publication maps display bytes to buffer bytes.
    pub(super) fn segments(&self, id: u64) -> Option<&[mini_window::Segment]> {
        self.segments.get(&id).map(Vec::as_slice)
    }
    /// The buffer byte window ID's last publication starts at.
    pub(super) fn base(&self, id: u64) -> usize {
        self.bases.get(&id).copied().unwrap_or(0)
    }
    /// Window END.id's last publication was painted as END reports.
    pub(super) fn painted_rows(&mut self, end: &romacs_layout_input::WindowEnd) {
        let base = self.base(end.id);
        let segments = self.segments.get(&end.id).cloned();
        if let Some(shown) = self.shown.get_mut(&end.id) && !shown.mini {
            shown.laid = Some((shown.key.clone(), laid(end, base, shown.len, shown.at_zv, segments.as_deref())));
        }
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
            face_table: None,
            previous: std::collections::BTreeMap::new(),
            shown: std::collections::BTreeMap::new(),
            overlays: 0,
            segments: std::collections::BTreeMap::new(),
            bases: std::collections::BTreeMap::new(),
            face_revision: 0,
            row_floor: None,
            theme: std::cell::RefCell::new(Theme::build(
                Arc::new(defs),
                None,
                Background::Dark,
                Base::new(attrs.build()).expect("absolute frame base"),
            )
            .expect("default theme")),
            pending_faces: Default::default(),
            version: 0,
            hosted: false,
            width: 0,
            height: 0,
            building_band: false,
            diagnostics: Default::default(),
        }
    }
}
/// The buffer-local variables a window's text, faces, start or mode line
/// read, besides its buffer's text and the window itself.
const DISPLAY_VARIABLES: [&str; 20] = [
    "mode-line-format", "header-line-format", "truncate-lines", "word-wrap", "tab-width",
    "truncate-partial-width-windows", "mark-active", "transient-mark-mode", "line-spacing",
    "buffer-invisibility-spec", "selective-display", "scroll-margin", "face-remapping-alist",
    "cursor-type", "show-trailing-whitespace", "buffer-display-table", "fontification-functions",
    "display-line-numbers", "display-line-numbers-width", "display-line-numbers-current-absolute",
];
const MARK_ACTIVE: usize = 6;
const TRANSIENT_MARK_MODE: usize = 7;
const SCROLL_MARGIN: usize = 11;

/// What a window's last full snapshot read, besides its point, its cursor
/// and its mode line's Lisp: while this is unchanged, so are the window's
/// text, faces and start (GNU redisplay's test that a window needs no
/// redrawing). A moving collection renames every Value in it, so a
/// collection counts as a change.
#[derive(Clone, PartialEq)]
struct ShownKey {
    collections: u64,
    overlays: u64,
    hosted: bool,
    buffer: Value,
    selected: Value,
    modiff: u64,
    /// The accessible region (GNU's clip_changed): narrowing edits nothing.
    accessible: (usize, usize),
    start: i64,
    mark: Value,
    /// A terminal's session and output revision: its colors restyle the
    /// buffer's faces with no modification of its own.
    terminal: Option<(u64, Option<u64>)>,
    geometry: (u32, u32, u32, i64, u32),
    root: (u32, u32),
    metrics: (u32, u32),
    /// What fonts faces realize (`DisplayState::face_revision`).
    faces: u64,
    variables: Vec<Value>,
    /// The header line format the window wants (window_wants_header_line):
    /// its parameter's or its buffer's, nil when it shows none.
    header: Value,
}

romacs_snapshot::snapshot_struct!(ShownKey { collections, overlays, hosted, buffer, selected, modiff, accessible, start, mark, terminal, geometry, root, metrics, faces, variables, header });

struct Shown {
    key: ShownKey,
    point: i64,
    mini: bool,
    /// The published text's length, and whether it reaches ZV.
    len: usize,
    at_zv: bool,
    /// The window laid out from its start under a key: by its last paint
    /// under KEY, or by redisplay since under the key then (`window_layout`).
    laid: Option<(ShownKey, Laid)>,
}
romacs_snapshot::snapshot_struct!(Shown { key, point, mini, len, at_zv, laid });

/// The bands redisplay built for a window, before their faces.
#[derive(Default)]
struct Bands {
    mode_line: Option<mode_line::Band>,
    header_line: Option<mode_line::Band>,
}

/// A mode-line band as a text slice whose faces index TABLE.
fn band_slice(table: &FaceTable, band: mode_line::Band) -> TextSlice {
    TextSlice {
        face_runs: band.runs.into_iter()
            .filter_map(|(bytes, face)| Some(FaceRun { bytes, face: table.handle(&face)? }))
            .collect(),
        utf8: band.text,
        raw: vec![],
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
    /// The face NAME as painting realizes it: one the theme does not hold
    /// paints in the default face (`painted_face').
    pub(crate) fn display_face(&self, name: &str) -> Result<Face> {
        let known = self.display_state.theme().realize(name).is_some();
        self.snapshot_face(if known { name } else { "default" })
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
        match BoundedDatum::from_graph(&Graph(self), value.0) {
            Ok(datum) => {
                self.display_state.pending_faces.get_mut().push((face.to_owned(), keyword.to_owned(), datum));
                self.display_state.faces_changed();
                self.display_state.free_realized_faces();
            }
            Err(error) => self.display_state.diagnostics.get_mut().push(format!("face {face} {keyword}: {error}")),
        }
    }
    pub fn take_display_diagnostics(&mut self) -> Vec<String> {
        drop(self.display_state.theme());
        std::mem::take(self.display_state.diagnostics.get_mut())
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
            SetWindowVscroll => (2..=4).contains(&n),
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
                .theme()
                .with_defface(&face, &spec)
                .map_err(|e| e.to_string())?;
            let definition = next.definition(&face).ok_or("missing declared face")?.clone();
            self.display_state.set_theme(next);
            self.display_state.version += 1;
            // A new face can name text whose face no snapshot resolved yet.
            self.display_state.shown.clear();
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
            // window.c Fwindow_vscroll: pixels, else canonical lines,
            // a float when not whole.
            WindowVscroll => {
                let window = self.decode_window(if n > 0 { self.stack[start] } else { Value::NIL }, true, false)?;
                let pixels = n > 1 && self.stack[start + 1] != Value::NIL;
                self.window_vscroll(window, pixels)
            }
            // Fset_window_vscroll: VSCROLL lines of the frame's height, or
            // pixels, truncated, never negative; the value is the vscroll.
            SetWindowVscroll => {
                let value = self.stack[start + 1];
                let amount = match (value.integer_value(), self.object_if(value)) {
                    (Some(i), _) => i as f64,
                    (None, Some(Object::Float(f))) => *f,
                    _ => {
                        let predicate = self.intern("numberp");
                        return Err(self.signal_error("wrong-type-argument", &[predicate, value], "Wrong type argument: numberp".into()));
                    }
                };
                let pixels = n > 2 && self.stack[start + 2] != Value::NIL;
                let line = f64::from(self.window_state.line.max(1));
                let next = (if pixels { amount } else { line * amount }).trunc().clamp(0.0, f64::from(u32::MAX)) as u32;
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
                    self.display_state.diagnostics.get_mut().push(
                        ":eval: deferred set-window-vscroll until after paint (D-ML1)".into(),
                    );
                }
                if changed {
                    self.display_state.version += 1;
                }
                self.window_vscroll_of(next, pixels)
            }
            FramePixelWidth => Value::integer(self.display_state.width as i64),
            FramePixelHeight => Value::integer(self.display_state.height as i64),
            _ => Err("unknown display primitive".into()),
        }
    }
    /// WINDOW's vertical scroll, as `window-vscroll' answers it.
    fn window_vscroll(&mut self, window: Value, pixels: bool) -> Result<Value> {
        // The committed scroll: one set while a band is built applies after.
        let px = self.window(window)?.vscroll_px;
        self.window_vscroll_of(px, pixels)
    }
    /// PX pixels of vertical scroll, or with PIXELS nil in canonical lines
    /// (FRAME_CANON_Y_FROM_PIXEL_Y: a float unless whole).
    fn window_vscroll_of(&mut self, px: u32, pixels: bool) -> Result<Value> {
        let line = self.window_state.line.max(1);
        if pixels || px % line == 0 { return Value::integer(i64::from(if pixels { px } else { px / line })); }
        Ok(self.allocate_object(crate::data::Object::Float(f64::from(px) / f64::from(line))))
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
    /// Read-only: the buffer byte window ID's last published text starts at;
    /// the snapshot's byte offsets count from it.
    pub fn published_base(&self, id: u64) -> usize {
        self.display_state.base(id)
    }
    /// Read-only: the buffer byte window ID's last published text shows at
    /// its byte DISPLAY.
    pub fn published_buffer_byte(&self, id: u64, display: usize) -> usize {
        self.display_state.segments(id).map_or(self.display_state.base(id) + display, |s| shown_byte(s, display))
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
    /// init_iterator's line wrap in the current buffer, for a window
    /// COLUMNS wide that is or is not as wide as its frame.
    pub(crate) fn line_wrap(&mut self, full_width: bool, columns: i64) -> Result<romacs_layout_input::LineWrap> {
        use romacs_layout_input::LineWrap;
        let mut var = |name: &str| { let slot = self.slot(name); self.variable_value(slot) };
        let partial = var("truncate-partial-width-windows")?;
        let continued = (full_width || partial == Value::NIL || partial.integer_value().is_some_and(|n| n <= columns))
            && var("truncate-lines")? == Value::NIL;
        Ok(if !continued { LineWrap::Truncate } else if var("word-wrap")? == Value::NIL { LineWrap::Window } else { LineWrap::Word })
    }
    /// WINDOW's line wrap for BUFFER.
    fn window_line_wrap(&mut self, window: Value, buffer: Value) -> Result<romacs_layout_input::LineWrap> {
        let w = self.window(window)?;
        let (x, width) = (w.x, w.width);
        let root = self.window_root_peek(crate::window::ROOT);
        let full_width = self.window(root).is_ok_and(|r| r.x == x && r.width == width);
        let columns = i64::from(width / self.window_state.column.max(1));
        let saved = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = buffer;
        let wrap = self.line_wrap(full_width, columns);
        self.slots[self.buffer_slot].value = saved;
        wrap
    }
    /// BUFFER's `tab-width' as GNU's SANE_TAB_WIDTH: 1 to 1000, else 8.
    fn buffer_tab_width(&mut self, buffer: Value) -> Result<u32> {
        let saved = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = buffer;
        let slot = self.slot("tab-width");
        let width = self.variable_value(slot);
        self.slots[self.buffer_slot].value = saved;
        Ok(width?.integer_value().filter(|w| (1..=1000).contains(w)).unwrap_or(8) as u32)
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
        if let Some(snapshot) = self.reuse_shown(root, slot, width, height)? {
            return Ok(self.publish_window(snapshot));
        }
        let bands = self.redisplay_and_build_mode_line(root, slot, height, true)?;
        let window = self.slots[root].value;
        let band_face = self.mode_line_face_name(window)?;
        let header_face = self.header_line_face_name(window)?;
        let w = self.window(window)?;
        if !w.live() {
            return Err("window deleted during snapshot".into());
        }
        let (id, buffer, mini) = (w.id, w.buffer, w.mini);
        self.slots[slot].value = buffer;
        let ShownText { text: TextSlice { utf8, face_runs, raw }, start_byte, point_byte, mut faces, base, at_zv, table, mut merged, segments, stretches, line_numbers }
            = self.window_text(window, height, None)?;
        self.display_state.bases.insert(id, base);
        match segments {
            Some(segments) => { self.display_state.segments.insert(id, segments); }
            None if !mini => { self.display_state.segments.remove(&id); }
            None => {}
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
        // The mini-window shows its overlay strings (icomplete's candidates).
        let (utf8, face_runs, raw, start_byte, point_byte) = if mini {
            let spliced = self.splice_overlay_strings(window, buffer, &utf8)?;
            if spliced.has_strings() {
                let mut strings = vec![];
                for (bytes, value) in &spliced.string_faces {
                    strings.push(FaceRun { bytes: bytes.clone(), face: self.face_handle(&table, &mut faces, &mut merged, *value) });
                }
                let runs = mini_window::display_face_runs(&face_runs, &spliced.segments, strings);
                // Each buffer segment's raw bytes, where the splice put it.
                let raw = spliced.segments.iter().filter(|s| !s.string).flat_map(|s| {
                    let (display, from, len) = (s.display, s.buffer, s.len);
                    raw.iter().filter(move |&&p| (from..from + len).contains(&(p as usize))).map(move |&p| p - from as u32 + display as u32)
                }).collect();
                let (start, point) = (spliced.before_strings(start_byte), spliced.cursor(point_byte));
                self.display_state.segments.insert(id, spliced.segments);
                (spliced.text, runs, raw, start, point)
            } else {
                self.display_state.segments.remove(&id);
                (utf8, face_runs, raw, start_byte, point_byte)
            }
        } else {
            (utf8, face_runs, raw, start_byte, point_byte)
        };
        let snapshot = WindowSnapshot {
            id,
            width,
            height,
            start_byte,
            point_byte,
            buffer_version: self.display_state.version,
            text: TextSlice { utf8, face_runs, raw },
            mode_line: bands.mode_line.map(|band| band_slice(&table, band)),
            mode_line_face: table.handle(&band_face).ok_or("mode-line face required")?,
            header_line: bands.header_line.map(|band| band_slice(&table, band)),
            header_line_face: table.handle(&header_face).ok_or("header-line face required")?,
            vscroll_px: self.window(window)?.vscroll_px,
            faces,
            tab_width: self.buffer_tab_width(buffer)?,
            wrap: self.window_line_wrap(window, buffer)?,
            cursor_off: self.window(window)?.cursor_off,
            stretches,
            line_numbers,
        };
        let len = snapshot.text.utf8.len();
        let snapshot = self.publish_window(snapshot);
        let point = self.window_point_value(window)?.as_integer()?;
        let key = self.shown_key(window, width, height)?;
        self.display_state.shown.insert(id, Shown { key, point, mini, len, at_zv, laid: None });
        Ok(snapshot)
    }
    /// WINDOW, showing BUFFER, paints a mode line: it is no mini-window and
    /// BUFFER's `mode-line-format' is non-nil (redisplay_window_inner).
    fn window_has_mode_line(&mut self, window: Value, buffer: Value) -> Result<bool> {
        if self.window(window)?.mini { return Ok(false); }
        let saved = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = buffer;
        let slot = self.slot("mode-line-format");
        let format = self.variable_value(slot);
        self.slots[self.buffer_slot].value = saved;
        Ok(!matches!(format?, Value::NIL | Value::UNBOUND))
    }
    /// The mode line, or with HEADER the header line, WINDOW paints now
    /// and its face: the last painted band while it still has one, else one
    /// row of the band's face, else none (CURRENT_MODE_LINE_HEIGHT,
    /// CURRENT_HEADER_LINE_HEIGHT).
    fn painted_band(&mut self, window: Value, header: bool) -> Result<(Option<TextSlice>, FaceHandle)> {
        let w = self.window(window)?;
        let (id, buffer) = (w.id, w.buffer);
        let wants = if header {
            crate::window::Windows::new(self).header_line_format(window).map_err(crate::forwarded::fault_text)? != Value::NIL
        } else {
            self.window_has_mode_line(window, buffer)?
        };
        let shown = self.display_state.previous.get(&id)
            .map(|p| if header { (p.header_line.clone(), p.header_line_face) } else { (p.mode_line.clone(), p.mode_line_face) });
        Ok(match (wants, shown) {
            (false, shown) => (None, shown.map_or(FaceHandle(0), |(_, face)| face)),
            (true, Some((Some(band), face))) => (Some(band), face),
            (true, _) => {
                let name = if header { self.header_line_face_name(window)? } else { self.mode_line_face_name(window)? };
                let face = self.display_faces()?.handle(&name).ok_or("band face required")?;
                (Some(TextSlice { utf8: " ".into(), face_runs: vec![], raw: vec![] }), face)
            }
        })
    }
    /// WINDOW laid out from its start as painting will, with the mode line
    /// it last showed (GNU's CURRENT_MODE_LINE_HEIGHT): from its record
    /// (`Shown`) while what it read (`ShownKey`) is unchanged, else laid out
    /// now and recorded under the key now. None
    /// before its first paint in these faces, or with no layout host. Runs
    /// no Lisp.
    pub(crate) fn window_layout(&mut self, window: Value) -> Result<Option<Laid>> {
        let (width, height) = self.window_body_size(window)?;
        let key = self.shown_key(window, width, height)?;
        let w = self.window(window)?;
        let (id, buffer, vscroll) = (w.id, w.buffer, w.vscroll_px);
        let Some(shown) = self.display_state.shown.get(&id) else { return Ok(None) };
        if let Some((known, laid)) = &shown.laid && *known == key { return Ok(Some(*laid)); }
        if shown.key.faces != key.faces || !self.has_layout_host() { return Ok(None); }
        if !self.display_state.previous.contains_key(&id) { return Ok(None) }
        let (mode_line, mode_line_face) = self.painted_band(window, false)?;
        let (header_line, header_line_face) = self.painted_band(window, true)?;
        let (tab_width, wrap) = (self.buffer_tab_width(buffer)?, self.window_line_wrap(window, buffer)?);
        let ShownText { text, start_byte, point_byte, faces, base, at_zv, segments, stretches, line_numbers, .. } = self.window_text(window, height, None)?;
        let len = text.utf8.len();
        let snapshot = WindowSnapshot { id, width, height, start_byte, point_byte, buffer_version: 0, text, mode_line,
            mode_line_face, header_line, header_line_face, vscroll_px: vscroll, faces, tab_width, wrap, cursor_off: false, stretches, line_numbers };
        let end = self.lay_out_alone(snapshot)?;
        let laid = laid(&end, base, len, at_zv, segments.as_deref());
        if let Some(shown) = self.display_state.shown.get_mut(&id) { shown.laid = Some((key, laid)); }
        Ok(Some(laid))
    }
    /// The rows WINDOW's buffer shows from the line starting at byte FROM
    /// through the end of byte TO's line, laid out as painting lays them
    /// out in a window as wide as WINDOW and tall enough for them all: each
    /// row's buffer byte and height. None as for `window_layout'.
    pub(crate) fn rows_between(&mut self, window: Value, from: usize, to: usize) -> Result<Option<Vec<(usize, f32)>>> {
        let id = self.window(window)?.id;
        let faces = self.display_state.face_revision;
        if !self.has_layout_host() || !self.display_state.shown.get(&id).is_some_and(|shown| shown.key.faces == faces) { return Ok(None); }
        let (width, _) = self.window_body_size(window)?;
        let buffer = self.window(window)?.buffer;
        let (tab_width, wrap) = (self.buffer_tab_width(buffer)?, self.window_line_wrap(window, buffer)?);
        let Object::Buffer(b) = self.object(self.window(window)?.buffer)? else { return Ok(None) };
        let zv = b.accessible().end.0;
        let to = b.as_str().as_bytes()[to.min(zv)..zv].iter().position(|&c| c == b'\n').map_or(zv, |i| to + i + 1);
        let shown = self.window_text(window, 0, Some(from..to))?;
        // Taller than all of them: rows of the default face's line or more.
        let height = 1 << 24;
        let snapshot = WindowSnapshot { id, width, height, start_byte: shown.start_byte, point_byte: shown.point_byte, buffer_version: 0,
            text: shown.text.clone(), mode_line: None, mode_line_face: FaceHandle(0), header_line: None, header_line_face: FaceHandle(0),
            vscroll_px: 0, faces: shown.faces.clone(), tab_width, wrap, cursor_off: false, stretches: shown.stretches.clone(),
            line_numbers: shown.line_numbers.clone() };
        let end = self.lay_out_alone(snapshot)?;
        Ok(Some(end.rows.iter().map(|row| (shown.buffer_byte(row.start), row.height)).collect()))
    }
    /// What painting SNAPSHOT alone in a frame of its size would report.
    fn lay_out_alone(&mut self, snapshot: WindowSnapshot) -> Result<romacs_layout_input::WindowEnd> {
        let (width, height) = (snapshot.width, snapshot.height);
        let frame = romacs_layout_input::FrameSnapshot { font_selection: self.display_state.font_selection_order,
            font_alternatives: self.font_alternative_state()?, width, height,
            windows: vec![romacs_layout_input::PlacedWindowSnapshot { x: 0, y: 0, snapshot }] };
        self.host_lay_out(&frame)?.into_iter().next().ok_or_else(|| "layout host laid out no window".into())
    }
    /// The least height a painted row can have: the default face's row
    /// height as the host realizes it, which every row reaches (the frame's
    /// line, set only at a resize, can be taller after the default face
    /// shrinks), else the frame's line. Kept per face revision.
    pub(crate) fn min_row_height(&mut self) -> u32 {
        let line = self.window_state.line.max(1);
        if !self.has_layout_host() { return line; }
        let revision = self.display_state.face_revision;
        if let Some((known, height)) = self.display_state.row_floor && known == revision { return height; }
        let height = self.host_default_advance(' ').map_or(line, |(_, height)| height.clamp(1, line));
        self.display_state.row_floor = Some((revision, height));
        height
    }
    /// WINDOW's text width less its margins, and its height, as a frame's
    /// snapshot gives them.
    fn window_body_size(&self, window: Value) -> Result<(u32, u32)> {
        let w = self.window(window)?;
        Ok((w.width.saturating_sub((w.left_margin + w.right_margin) * self.window_state.column), w.height))
    }
    /// The text WINDOW shows from its start, as a snapshot publishes it:
    /// for any window but the mini-window, only the bytes the rows of a
    /// window HEIGHT pixels tall can show, so that a frame costs what its
    /// windows show rather than what their buffers hold. Runs no Lisp.
    /// SPAN, buffer bytes, overrides the start and the rows: the text from
    /// SPAN's start through its end.
    fn window_text(&mut self, window: Value, height: u32, span: Option<std::ops::Range<usize>>) -> Result<ShownText> {
        let w = self.window(window)?;
        let (buffer, start, mini, vscroll) = (w.buffer, w.start, w.mini, w.vscroll_px);
        let start = self
            .marker_position_value(start)?
            .integer_value()
            .unwrap_or(1);
        let point = self.window_point_value(window)?.as_integer()?;
        let region = self.snapshot_region(window, buffer, point)?;
        let table = self.display_faces()?;
        let floor = self.min_row_height();
        let (shown, start_byte, start, point_byte, at_zv, rows) = {
            let Object::Buffer(b) = self.object(buffer)? else {
                return Err("Unicode display buffer required".into());
            };
            let point_byte = b
                .byte_of_char(crate::coordinates::CharOffset((point - 1) as usize))?
                .0;
            let start_byte = match &span { Some(span) => span.start, None => b
                .byte_of_char(crate::coordinates::CharOffset((start - 1) as usize))?
                .0 };
            // The mini-window splices overlay strings into its whole buffer.
            // Any other window publishes only the bytes its rows can show, so a
            // frame costs what the window shows, not what its buffer holds.
            let accessible = b.accessible();
            let rows = (span.is_none() && !mini).then(|| height.saturating_add(vscroll).div_ceil(floor) as usize);
            let shown = if let Some(span) = span { span.start.max(accessible.start.0)..span.end.min(accessible.end.0) } else if mini { 0..b.as_str().len() } else {
                crate::window::shown_bytes(b.as_str(), accessible.start.0..accessible.end.0, start_byte, rows.unwrap_or(0))
            };
            let at_zv = shown.end >= accessible.end.0;
            // The display starts at START_BYTE, SPAN's start when given.
            let start = b.char_of_byte(ByteOffset(start_byte.clamp(accessible.start.0, accessible.end.0)))?.0 as i64 + 1;
            (shown, start_byte, start, point_byte, at_zv, rows)
        };
        // Where the window shows more than its buffer's characters, it
        // publishes the elements GNU's iterator delivers; where the iterator
        // refuses them, the characters.
        let (mut shown, mut at_zv) = (shown, at_zv);
        let elements = if mini || !self.shows_elements(window, buffer, shown.clone())? { None } else {
            let mut lines = rows.unwrap_or(0);
            loop {
                let Object::Buffer(b) = self.object(buffer)? else { return Err("Unicode display buffer required".into()) };
                let pos = |byte: usize| b.char_of_byte(ByteOffset(byte)).map(|c| c.0 as i64 + 1);
                let (from, to) = (pos(shown.start)?, pos(shown.end)?);
                let Ok(elements) = crate::prim::window_shown_elements(self, window, from, to) else { break None };
                // Hidden and replaced newlines end no row: publish lines
                // until as many show as the window has rows.
                let Some(rows) = rows.filter(|&rows| !at_zv && elements.iter().filter(|e| e.c == '\n').count() < rows) else { break Some(elements) };
                lines *= 2;
                let Object::Buffer(b) = self.object(buffer)? else { return Err("Unicode display buffer required".into()) };
                let accessible = b.accessible();
                shown = crate::window::shown_bytes(b.as_str(), accessible.start.0..accessible.end.0, start_byte, rows + lines);
                at_zv = shown.end >= accessible.end.0;
            }
        };
        let labels = if mini { None } else { crate::prim::window_line_numbers(self, window).ok().flatten() };
        let mut sources = ElementFaces::new(self, window, buffer, shown.clone())?;
        let mut faces = table.faces.clone();
        let terminal = self.terminal_bindings.iter().find(|binding| binding.buffer == buffer);
        if let Some(binding) = terminal {
            if let Some(color) = binding.default_colors[0] { faces[0].foreground = color; }
            if let Some(color) = binding.default_colors[1] { faces[0].background = color; }
        }
        let default = faces[0].clone();
        let styles = terminal.map_or(&[][..], |binding| binding.styles.as_ref());
        let mut terminal_faces = std::collections::HashMap::new();
        let mut region_faces = std::collections::HashMap::new();
        let region_background = table.handle("region").map(|i| faces[i.0 as usize].background);
        // A control character other than TAB and newline shows as an escape.
        let escape = table.handle("escape-glyph");
        let Object::Buffer(b) = self.object(buffer)? else { return Err("Unicode display buffer required".into()) };
        let mut style_index = styles.partition_point(|run| run.bytes.end <= shown.start);
        // The face the buffer's character CH at byte AT shows in.
        let mut buffer_face = |vm: &Self, faces: &mut Vec<Face>, sources: &mut ElementFaces, at: usize, ch: char| -> Result<FaceHandle> {
            // Within the last run of one face, that face.
            let mut face = if (sources.run.0..sources.run.1).contains(&at) { sources.run.2 }
                else { vm.element_face(&table, faces, sources, crate::prim::FaceKey::Buffer(0), Some(at))? };
            if ((ch.is_control() && ch != '\n' && ch != '\t') || b.is_raw_at(at)) && let Some(escape) = escape { face = escape; }
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
            if region.as_ref().is_some_and(|range| range.contains(&at)) && let Some(background) = region_background {
                face = *region_faces.entry(face.0).or_insert_with(|| {
                    let mut selected = faces[face.0 as usize].clone();
                    selected.background = background;
                    let handle = FaceHandle(faces.len() as u32);
                    faces.push(selected);
                    handle
                });
            }
            Ok(face)
        };
        let mut utf8 = String::new();
        let mut face_runs: Vec<FaceRun> = vec![];
        let push = |utf8: &mut String, face_runs: &mut Vec<FaceRun>, ch: char, face: FaceHandle| {
            let offset = utf8.len();
            utf8.push(ch);
            match face_runs.last_mut() {
                Some(last) if last.face == face => last.bytes.end = utf8.len(),
                _ => face_runs.push(FaceRun { bytes: offset..utf8.len(), face }),
            }
        };
        let Some(elements) = elements else {
            let base = shown.start;
            utf8.push_str(&b.as_str()[shown.clone()]);
            // The raw bytes shown, as offsets into the published text; an
            // element path publishes a raw byte as its escape's characters.
            let raw: Vec<u32> = b.raw_in(shown.clone()).iter().map(|&p| p - base as u32).collect();
            // Without a region or terminal styles, a face holds through its
            // property run but on an escaped control character.
            let plain = region.is_none() && styles.is_empty();
            let mut run = (base, FaceHandle(0));
            for (offset, ch) in utf8.char_indices() {
                let at = base + offset;
                let escaped = (ch.is_control() && ch != '\n' && ch != '\t') || b.is_raw_at(at);
                let face = if plain && at < run.0 && !escaped { run.1 } else {
                    let face = buffer_face(self, &mut faces, &mut sources, at, ch)?;
                    if plain && !escaped { run = (sources.run.1, face); }
                    face
                };
                let end = offset + ch.len_utf8();
                match face_runs.last_mut() {
                    Some(last) if last.face == face => last.bytes.end = end,
                    _ => face_runs.push(FaceRun { bytes: offset..end, face }),
                }
            }
            let start_byte = start_byte.clamp(shown.start, shown.end) - base;
            // Point outside the published bytes is not shown: no glyph is at it.
            let point_byte = point_byte.checked_sub(base).filter(|&p| p <= utf8.len()).unwrap_or(utf8.len());
            let merged = sources.merged;
            let line_numbers = labels.map(|labels| {
                let starts = std::iter::once(0).chain(utf8.match_indices('\n').map(|(at, _)| at + 1)).map(|at| (at, base + at)).collect();
                line_numbers(&table, b.as_str(), b.accessible().start.0, &labels, starts, utf8.len(), at_zv)
            });
            return Ok(ShownText { text: TextSlice { utf8, face_runs, raw }, start_byte, point_byte, faces, base, at_zv, table: table.clone(), merged,
                segments: None, stretches: vec![], line_numbers });
        };
        // Each element's character, its face, and the buffer byte it shows:
        // its own for the buffer's character, its position's for the rest.
        let mut segments: Vec<mini_window::Segment> = vec![];
        let mut stretches = vec![];
        let mut relative = vec![];
        let (mut start_at, mut point_at, mut cursor_at) = (None, None, None);
        // The display bytes rows of a new buffer line start at, and the
        // buffer bytes their first elements show, past any text hidden.
        let (mut starts, mut pending) = (vec![], Some(0));
        for e in &elements {
            let at = b.byte_of_char(crate::coordinates::CharOffset((e.charpos - 1).max(0) as usize))?.0;
            if let Some(display) = pending.take() { starts.push((display, at)); }
            let display = utf8.len();
            let own = e.kind == crate::prim::ShownKind::Buffer;
            let face = if own { buffer_face(self, &mut faces, &mut sources, at, e.c)? } else {
                let face = self.element_face(&table, &mut faces, &mut sources, e.face, None)?;
                if e.c.is_control() && e.c != '\n' && e.c != '\t' && let Some(escape) = escape { escape } else { face }
            };
            if let crate::prim::ShownKind::Stretch(width) = e.kind { stretches.push(romacs_layout_input::Stretch { byte: display, width }); }
            // A relative space's width waits for the host's glyph of C.
            if let crate::prim::ShownKind::Relative { factor, c } = e.kind {
                relative.push((display, face, c, factor));
                push(&mut utf8, &mut face_runs, ' ', face);
            } else {
                push(&mut utf8, &mut face_runs, e.c, face);
            }
            let len = utf8.len() - display;
            match segments.last_mut() {
                Some(s) if !s.string && own && s.buffer + s.len == at && s.display + s.len == display => s.len += len,
                Some(s) if s.string && !own && s.buffer == at && s.display + s.len == display => s.len += len,
                _ => segments.push(mini_window::Segment { display, buffer: at, len, string: !own }),
            }
            // The display starts with the first element at its start; point
            // shows on a string asking for it there, else on the first buffer
            // character or replacing element at or after it.
            if start_at.is_none() && e.charpos >= start { start_at = Some(display); }
            if point_at.is_none() && e.charpos >= point && e.replaces { point_at = Some(display); }
            if cursor_at.is_none() && e.charpos == point && matches!(e.kind, crate::prim::ShownKind::Other { cursor: true, .. }) { cursor_at = Some(display); }
            if own && e.c == '\n' { pending = Some(utf8.len()); }
        }
        if let Some(display) = pending { starts.push((display, shown.end)); }
        // Past the last element: the end of the text published.
        segments.push(mini_window::Segment { display: utf8.len(), buffer: shown.end, len: 1, string: false });
        let len = utf8.len();
        let merged = sources.merged;
        let line_numbers = labels.map(|labels| line_numbers(&table, b.as_str(), b.accessible().start.0, &labels, starts, len, at_zv));
        // produce_stretch_glyph: FACTOR times the glyph's pixel width.
        for (byte, face, c, factor) in relative {
            let input = romacs_layout_input::MeasureInput {
                font_selection: self.display_state.font_selection_order,
                font_alternatives: self.font_alternative_state()?,
                faces: faces.clone(),
                text: TextSlice { utf8: c.to_string(), face_runs: vec![FaceRun { bytes: 0..c.len_utf8(), face }], raw: vec![] },
            };
            let advance = if self.has_layout_host() { self.host_layout(&input)?.glyphs[0].0 } else { self.window_state.column };
            let width = romacs_layout_input::StretchWidth::Width { width: (factor * f64::from(advance)) as i64, zero_ok: false };
            let at = stretches.partition_point(|s: &romacs_layout_input::Stretch| s.byte < byte);
            stretches.insert(at, romacs_layout_input::Stretch { byte, width });
        }
        Ok(ShownText { text: TextSlice { utf8, face_runs, raw: vec![] }, start_byte: start_at.unwrap_or(len), point_byte: cursor_at.or(point_at).unwrap_or(len),
            faces, base: 0, at_zv, table: table.clone(), merged, segments: Some(segments), stretches, line_numbers })
    }
    /// Whether WINDOW shows BUFFER's bytes SHOWN as more than its
    /// characters: text there has a `display' or `invisible' property, maybe
    /// through a category or an alias, or an overlay there has one, or a
    /// string to show.
    fn shows_elements(&mut self, window: Value, buffer: Value, shown: std::ops::Range<usize>) -> Result<bool> {
        let display = [self.display_symbol("display"), self.display_symbol("invisible")];
        let keys = ["display", "invisible", "before-string", "after-string"].map(|n| self.display_symbol(n));
        let Object::Buffer(b) = self.object(buffer)? else { return Ok(false) };
        let (at, end) = (ByteOffset(shown.start), ByteOffset(shown.end));
        // An overlay ending at the start shows its after-string there, one
        // starting at the end its before-string: the characters around.
        let text = b.as_str();
        let mut low = shown.start.saturating_sub(1);
        while !text.is_char_boundary(low) { low -= 1; }
        let high = text[shown.end..].chars().next().map_or(shown.end, |c| shown.end + c.len_utf8());
        let around = ByteOffset(low)..ByteOffset(high);
        for name in ["display", "invisible", "category"] {
            if b.properties.is_empty() { break; }
            if b.properties.get_property(at, name).is_some() || b.properties.next_single_change(at, name, end) < end { return Ok(true); }
        }
        let aliases = self.list_values(self.annotation_variable("char-property-alias-alist"))?;
        for entry in aliases {
            if self.cons_pair(entry).is_ok_and(|(key, _)| display.contains(&key)) { return Ok(true); }
        }
        if self.annotation_variable("default-text-properties") != Value::NIL { return Ok(true); }
        let mut found = false;
        let mut overlays = vec![];
        self.visit_overlays_in(buffer, around, |overlay, _| overlays.push(overlay))?;
        for overlay in overlays {
            if !self.overlay_applies_to(overlay, window) { continue; }
            for key in keys { found |= self.overlay_get(overlay, key)? != Value::NIL; }
        }
        Ok(found)
    }
    /// Record SNAPSHOT as its window's last publication.
    fn publish_window(&mut self, mut snapshot: WindowSnapshot) -> WindowSnapshot {
        // The owned last publication also covers mutation paths with specialised
        // fast dispatch (point moves, regexp, yank), without retaining GC handles.
        // Keyed per window: one shared snapshot made each window compare
        // against whichever window published last, so alternating between
        // two unchanged windows repainted the frame every time.
        let id = snapshot.id;
        if let Some(previous) = self.display_state.previous.get(&id) {
            // Compared with the version aside, not by copying the previous.
            let version = std::mem::replace(&mut snapshot.buffer_version, previous.buffer_version);
            let changed = *previous != snapshot;
            snapshot.buffer_version = version;
            if changed {
                self.display_state.version += 1;
            }
        } else {
            // A window's first publication is itself a change.
            self.display_state.version += 1;
        }
        snapshot.buffer_version = self.display_state.version;
        self.display_state.previous.insert(id, snapshot.clone());
        snapshot
    }
    /// What WINDOW's snapshot reads besides point, cursor and mode line.
    fn shown_key(&mut self, window: Value, width: u32, height: u32) -> Result<ShownKey> {
        let w = self.window(window)?;
        let (buffer, start, x, hscroll, vscroll) = (w.buffer, w.start, w.x, w.hscroll, w.vscroll_px);
        let root = self.window_root_peek(crate::window::ROOT);
        let root = self.window(root).map_or((0, 0), |r| (r.x, r.width));
        let start = self.marker_position_value(start)?.integer_value().unwrap_or(1);
        let modiff = self.buffer_state(buffer)?.modiff;
        let accessible = match self.object(buffer)? {
            Object::Buffer(b) => { let region = b.accessible(); (region.start.0, region.end.0) }
            _ => (0, 0),
        };
        let mark = match self.buffer_mark_value(buffer) {
            Some(marker) => self.marker_position_value(marker)?,
            None => Value::NIL,
        };
        let terminal = self.terminal_bindings.iter().find(|binding| binding.buffer == buffer)
            .map(|binding| (binding.session, binding.revision));
        let saved = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = buffer;
        let variables = DISPLAY_VARIABLES.iter().map(|name| {
            let slot = self.slot(name);
            self.variable_value(slot).unwrap_or(Value::UNBOUND)
        }).collect();
        self.slots[self.buffer_slot].value = saved;
        let header = crate::window::Windows::new(self).header_line_format(window).map_err(crate::forwarded::fault_text)?;
        Ok(ShownKey {
            collections: self.gc_stats.collections,
            overlays: self.display_state.overlays,
            hosted: self.display_state.hosted,
            buffer,
            selected: self.selected_window_value()?,
            modiff,
            accessible,
            start,
            mark,
            terminal,
            geometry: (x, width, height, hscroll, vscroll),
            root,
            metrics: (self.window_state.line, self.window_state.column),
            faces: self.display_state.face_revision,
            variables,
            header,
        })
    }
    /// GNU redisplay's shortcut for the window in ROOT when nothing it shows
    /// changed since its last full snapshot (`ShownKey`) and point is where
    /// it was or inside rows the last layout showed whole: its text, faces
    /// and start stand, and only its mode line is evaluated again, so a
    /// blink or a cursor motion draws only the cursor and the mode line
    /// (try_cursor_movement). None when a full redisplay is due.
    fn reuse_shown(&mut self, root: usize, slot: usize, width: u32, height: u32) -> Result<Option<WindowSnapshot>> {
        let window = self.slots[root].value;
        let w = self.window(window)?;
        let (id, mini, forced, cursor_off) = (w.id, w.mini, w.force_start, w.cursor_off);
        if forced || !self.display_state.previous.contains_key(&id) {
            return Ok(None);
        }
        let Some(shown_point) = self.display_state.shown.get(&id).map(|s| s.point) else {
            return Ok(None);
        };
        let key = self.shown_key(window, width, height)?;
        if self.display_state.shown.get(&id).is_none_or(|s| s.key != key) {
            return Ok(None);
        }
        let point = self.window_point_value(window)?.as_integer()?;
        let mut snapshot = self.display_state.previous[&id].clone();
        snapshot.cursor_off = cursor_off;
        if mini {
            // No mode line: a mini-window with point unmoved is as it was.
            return Ok((point == shown_point).then_some(snapshot));
        }
        if point != shown_point {
            let region = key.variables[MARK_ACTIVE] != Value::NIL && key.variables[TRANSIENT_MARK_MODE] != Value::NIL;
            let margin = key.variables[SCROLL_MARGIN].integer_value().is_some_and(|m| m > 0);
            // Display elements place the cursor, and line numbers count, by
            // what they show there.
            if region || margin || self.display_state.segments(id).is_some() || snapshot.line_numbers.is_some() {
                return Ok(None);
            }
            // Point's row must be one the last paint showed whole.
            let shown = self.display_state.shown.get(&id);
            let Some(&(_, laid)) = shown.and_then(|s| s.laid.as_ref()).filter(|(laid_key, _)| *laid_key == key) else { return Ok(None) };
            let Object::Buffer(b) = self.object(key.buffer)? else { return Ok(None) };
            let point_byte = b.byte_of_char(crate::coordinates::CharOffset((point - 1).max(0) as usize))?.0;
            let base = self.display_state.base(id);
            let offset = point_byte.wrapping_sub(base);
            if point_byte < base + snapshot.start_byte || point_byte >= laid.visible_end || offset > snapshot.text.utf8.len() {
                return Ok(None);
            }
            snapshot.point_byte = offset;
        }
        // Face and font changes, `force-window-update' and new faces advance
        // the display version and drop the records this reuse rests on.
        let version = self.display_state.version;
        let bands = self.redisplay_and_build_mode_line(root, slot, height, false)?;
        // The band's Lisp may have changed what the window shows, or
        // collected and moved the window: only ROOT's traced copy is current.
        if self.gc_stats.collections != key.collections || self.display_state.version != version
            || !self.display_state.previous.contains_key(&id) {
            return Ok(None);
        }
        let window = self.slots[root].value;
        if self.shown_key(window, width, height)? != key {
            return Ok(None);
        }
        let (face, header_face) = (self.mode_line_face_name(window)?, self.header_line_face_name(window)?);
        let table = self.display_faces()?;
        snapshot.mode_line = bands.mode_line.map(|band| band_slice(&table, band));
        snapshot.mode_line_face = table.handle(&face).ok_or("mode-line face required")?;
        snapshot.header_line = bands.header_line.map(|band| band_slice(&table, band));
        snapshot.header_line_face = table.handle(&header_face).ok_or("header-line face required")?;
        let Some(shown) = self.display_state.shown.get_mut(&id) else { return Ok(None) };
        shown.point = point;
        Ok(Some(snapshot))
    }
    /// The row of FACES for a `face' property VALUE: a face symbol's own
    /// row, and for a list of faces or an attribute list a row merged as
    /// GNU's C merges them (xfaces.c merge_face_ref), added once per value
    /// to FACES and MERGED. What the display cannot realize shows as default.
    fn face_handle(&self, table: &FaceTable, faces: &mut Vec<Face>,
                   merged: &mut std::collections::HashMap<u64, FaceHandle>, value: Value) -> FaceHandle {
        let graph = Graph(self);
        match graph.view(value.0) {
            NodeView::Symbol(name) => return table.handle(name).unwrap_or(FaceHandle(0)),
            NodeView::Cons(..) => {}
            _ => return FaceHandle(0),
        }
        if let Some(&handle) = merged.get(&value.0) { return handle; }
        let face = BoundedDatum::from_graph(&graph, value.0).ok()
            .and_then(|datum| romacs_face::remap::parse_face_ref(&datum).ok())
            .and_then(|face_ref| {
                let view = romacs_face::View::bind(&self.display_state.theme(), &romacs_face::Remap::empty()).ok()?;
                face_of_attrs(&view.realize_ref(&face_ref), "merged face").ok()
            });
        let handle = match face {
            Some(face) => { faces.push(face); FaceHandle(faces.len() as u32 - 1) }
            None => FaceHandle(0),
        };
        merged.insert(value.0, handle);
        handle
    }
    /// GNU `redisplay_window` order for one window: make its buffer current,
    /// keep point visible, then `display_mode_line` (`src/xdisp.c:28200`)
    /// with that window's own -- possibly buffer-local -- `mode-line-format`.
    /// The window in ROOT's mode line, after its point visibility and
    /// fontification when LAYOUT (its start and text may have changed).
    fn redisplay_and_build_mode_line(&mut self, root: usize, buffer_slot: usize, height: u32, layout: bool) -> Result<Bands> {
        let saved = self.slot("#:vm2-band-saved-current-buffer");
        self.slots[saved].value = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = self.slots[buffer_slot].value;
        self.display_state.building_band = true;
        let outcome = self.redisplay_window_inner(root, buffer_slot, height, layout);
        self.display_state.building_band = false;
        if self.buffer_state(self.slots[saved].value).is_ok_and(|b| b.live) {
            self.slots[self.buffer_slot].value = self.slots[saved].value;
        }
        self.slots[saved].value = Value::UNBOUND;
        outcome
    }
    fn redisplay_window_inner(&mut self, root: usize, buffer_slot: usize, height: u32, layout: bool) -> Result<Bands> {
        if self.window(self.slots[root].value)?.mini {
            return if layout { self.redisplay_window_fontify(root, buffer_slot, height).map(|_| Bands::default()) } else { Ok(Bands::default()) };
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
        // The header line window_wants_header_line gives the window, its
        // parameter's or its buffer's format.
        let window = self.slots[root].value;
        let header = crate::window::Windows::new(self).header_line_format(window).map_err(crate::forwarded::fault_text)?;
        let header = (header != Value::NIL).then(|| self.mode_line_form(header));
        let band_height = self.window_state.line * (u32::from(format.is_some()) + u32::from(header.is_some()));
        let text_height = height.saturating_sub(band_height);
        if layout {
            self.redisplay_point_visibility(self.slots[root].value, text_height)?;
            if !self.redisplay_window_fontify(root, buffer_slot, text_height)? {
                return Ok(Bands::default());
            }
        }
        if format.is_none() && header.is_none() { return Ok(Bands::default()); }
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
        // display_mode_lines builds the header line before the mode line.
        let outcome = (|| Ok(Bands {
            header_line: header.as_ref().map(|form| self.mode_line_band(form, &state)).transpose()?,
            mode_line: format.as_ref().map(|form| self.mode_line_band(form, &state)).transpose()?,
        }))();
        self.frame_mut(self.frame_value_peek())?.selected = self.slots[saved].value;
        self.slots[saved].value = Value::UNBOUND;
        // Resolve the traced buffer again after collecting :eval, restoring
        // by character position if Lisp changed its text. As in GNU's restore,
        // a saved point beyond the new ZV falls back to the buffer's true end.
        let buffer = self.slots[buffer_slot].value;
        let Object::Buffer(b) = self.object_mut(buffer)? else {
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
    /// GNU `handle_fontified_prop` (`xdisp.c:4667`) from START to END of
    /// the current buffer, which a window shows. The walk over unfontified
    /// stretches and the calls are Lisp (`romacs--redisplay-fontify`,
    /// rlisp/vm/prelude.el), so nothing is held across them; a signal is
    /// logged there and never ends redisplay, and any other failure becomes
    /// a display diagnostic. False when `fontification-functions` is nil,
    /// so no Lisp ran.
    fn fontify_range(&mut self, start: usize, end: usize) -> Result<bool> {
        let functions = self.slot("fontification-functions");
        if self.variable_value(functions)? == Value::NIL {
            return Ok(false);
        }
        let args = [Value::integer(start as i64)?, Value::integer(end as i64)?];
        let call = "(lambda (start end) (romacs--redisplay-fontify start end))";
        if let Err(error) = self.call_prepared_source_raw(call, &args) {
            self.display_state.diagnostics.get_mut().push(format!("fontification: {error}"));
        }
        Ok(true)
    }
    /// Fontify the rows the window shows, then re-derive what that Lisp may
    /// have changed: the window's buffer, which becomes current again as GNU
    /// restores it after `fontification-functions`. Text or a window start
    /// the Lisp changed can bring other rows into view, so the whole range
    /// is fontified again, a bounded number of times as GNU's redisplay
    /// retries. Faces alone it changed can show text past the end walked (a
    /// smaller face): that text is fontified as GNU's iterator fontifies
    /// what it reaches, while the end keeps growing, which point-max bounds.
    /// False if Lisp deleted the window.
    fn redisplay_window_fontify(&mut self, root: usize, buffer_slot: usize, text_height: u32) -> Result<bool> {
        let functions = self.slot("fontification-functions");
        if self.variable_value(functions)? == Value::NIL {
            return Ok(true);
        }
        let mut range = self.displayed_range(self.slots[root].value, text_height)?;
        let mut tries = 1;
        while let Some((start, end)) = range {
            let shown = self.window_text_state(root)?;
            let ran = self.fontify_range(start, end)?;
            let w = self.window(self.slots[root].value)?;
            if !w.live() {
                return Ok(false);
            }
            let buffer = w.buffer;
            self.slots[buffer_slot].value = buffer;
            self.slots[self.buffer_slot].value = buffer;
            let now = self.window_text_state(root)?;
            if !ran || now == shown {
                break;
            }
            let next = self.displayed_range(self.slots[root].value, text_height)?;
            range = if (now.1, now.2) == (shown.1, shown.2) {
                // Only properties changed: the text now in view past END.
                next.filter(|&(_, later)| later > end).map(|(_, later)| (end, later))
            } else if tries < 3 {
                tries += 1;
                next
            } else {
                None
            };
        }
        Ok(true)
    }
    /// What decides the rows a live window shows: its buffer's edits
    /// (MODIFF, which property changes advance), its character edits
    /// (CHARS_MODIFF) and its start.
    fn window_text_state(&self, root: usize) -> Result<(u64, u64, Value)> {
        let w = self.window(self.slots[root].value)?;
        let start = self.marker_position_value(w.start)?;
        let state = self.buffer_state(w.buffer)?;
        Ok((state.modiff, state.chars_modiff, start))
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
            // Fselect_window (WINDOW, t): its point becomes its buffer's.
            self.select_window_value(window, false)?;
            self.slots[self.buffer_slot].value = buffer;
            let state = self.mode_line_state(root, buffer_slot, height)?;
            self.mode_line_expand(&format, &state, target)
        })();
        // unwind_format_mode_line: the old selection, if it lives, then
        // the old current buffer.
        let old = self.slots[saved_window].value;
        if self.window(old).is_ok_and(|w| w.live()) { self.select_window_value(old, false)?; }
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
        // `buffer-name', not a search of every symbol for its registry slot.
        let buffer_name = self.buffer_state(buffer)?.name.clone().unwrap_or_default();
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
        let defined = self.display_faces()?.handle(wanted).is_some();
        Ok(if defined { wanted } else { "mode-line" }.to_owned())
    }
    /// CURRENT_HEADER_LINE_ACTIVE_FACE_ID (dispextern.h): `header-line-active'
    /// or `-inactive' as the mode line chooses, else the `header-line' they
    /// inherit, else the mode line's face.
    fn header_line_face_name(&mut self, window: Value) -> Result<String> {
        let selected = window == self.selected_window_value()?;
        let slot = self.slot("mode-line-in-non-selected-windows");
        let distinguish = self.variable_value(slot)? != Value::NIL;
        let wanted = if selected || !distinguish { "header-line-active" } else { "header-line-inactive" };
        let table = self.display_faces()?;
        if table.handle(wanted).is_some() { return Ok(wanted.to_owned()); }
        if table.handle("header-line").is_some() { return Ok("header-line".to_owned()); }
        self.mode_line_face_name(window)
    }
    /// The face table a snapshot publishes, realized once per theme.
    fn display_faces(&mut self) -> Result<std::rc::Rc<FaceTable>> {
        if let Some(table) = &self.display_state.face_table { return Ok(table.clone()); }
        let names: Vec<String> = std::iter::once("default")
            .chain(self.display_state.theme().face_names().filter(|s| *s != "default"))
            .map(str::to_owned)
            .collect();
        let faces = names.iter().map(|n| self.snapshot_face(n)).collect::<Result<Vec<_>>>()?;
        let index = names.into_iter().enumerate().map(|(i, n)| (n, i as u32)).collect();
        let table = std::rc::Rc::new(FaceTable { faces, index });
        self.display_state.face_table = Some(table.clone());
        Ok(table)
    }
    /// The host geometry of display elements of WINDOW's buffer, current:
    /// each element's character in the face a snapshot paints it with
    /// (VM2-WTPS1).
    pub(crate) fn host_measure_elements(&mut self, window: Value, shown: &[(char, crate::prim::FaceKey)]) -> Result<romacs_layout_input::Measured> {
        let table = self.display_faces()?;
        let buffer = self.slots[self.buffer_slot].value;
        let span = match self.object(buffer)? {
            Object::Buffer(b) => {
                let byte = |pos: i64| b.byte_of_char(crate::coordinates::CharOffset((pos - 1).max(0) as usize)).map(|b| b.0);
                let charpos = |key: &crate::prim::FaceKey| match *key { crate::prim::FaceKey::Buffer(p) | crate::prim::FaceKey::String { charpos: p, .. } | crate::prim::FaceKey::Glyph { charpos: p, .. } => p };
                let (low, high) = shown.iter().map(|(_, key)| charpos(key)).fold((i64::MAX, 0), |(l, h), p| (l.min(p), h.max(p)));
                byte(low.min(high))?..byte(high + 1).unwrap_or(b.accessible().end.0).min(b.accessible().end.0)
            }
            _ => return Err("Unicode display buffer required".into()),
        };
        let mut sources = ElementFaces::new(self, window, buffer, span)?;
        let mut faces = table.faces.clone();
        let escape = table.handle("escape-glyph");
        let mut text = String::with_capacity(shown.len());
        let mut face_runs: Vec<FaceRun> = vec![];
        for &(c, key) in shown {
            let mut face = self.element_face(&table, &mut faces, &mut sources, key, None)?;
            // As painting does, a control character shows as its escape.
            if c.is_control() && c != '\n' && c != '\t' && let Some(escape) = escape { face = escape; }
            let start = text.len();
            text.push(c);
            match face_runs.last_mut() {
                Some(last) if last.face == face => last.bytes.end = text.len(),
                _ => face_runs.push(FaceRun { bytes: start..text.len(), face }),
            }
        }
        let input = romacs_layout_input::MeasureInput {
            font_selection: self.display_state.font_selection_order,
            font_alternatives: self.font_alternative_state()?,
            faces,
            text: TextSlice { utf8: text, face_runs, raw: vec![] },
        };
        self.host_layout(&input)
    }
    /// The face a display element shows in: KEY's, or at buffer byte AT
    /// when given (GNU face_at_buffer_position: the text's `face' under
    /// the faces of the overlays there, the higher priority first; and
    /// face_at_string_position: a string's `face' over that, or for an
    /// overlay's string over the text's alone, face_for_overlay_string).
    fn element_face(&self, table: &FaceTable, faces: &mut Vec<Face>, sources: &mut ElementFaces,
                    key: crate::prim::FaceKey, at: Option<usize>) -> Result<FaceHandle> {
        let (string, charpos, glyph) = match key {
            crate::prim::FaceKey::Buffer(charpos) => (None, charpos, Value::NIL),
            crate::prim::FaceKey::String { string, strpos, charpos, overlay } => (Some((string, strpos, overlay)), charpos, Value::NIL),
            crate::prim::FaceKey::Glyph { face, charpos } => (None, charpos, face),
        };
        let at = match at {
            Some(at) => at,
            None => match self.object(sources.buffer)? {
                Object::Buffer(b) => {
                    // Elements come in order: step from the last one's byte.
                    let (pos, byte) = sources.cursor;
                    let at = if pos > 0 && (pos..pos + 64).contains(&charpos) {
                        b.as_str()[byte..].char_indices().nth((charpos - pos) as usize).map_or(b.as_str().len(), |(i, _)| byte + i)
                    } else {
                        b.byte_of_char(crate::coordinates::CharOffset((charpos - 1).max(0) as usize))?.0
                    };
                    sources.cursor = (charpos, at);
                    at
                }
                _ => return Err("Unicode display buffer required".into()),
            },
        };
        if !(sources.run.0..sources.run.1).contains(&at) {
            let mut layers: Vec<Value> = sources.overlays.iter().rev().filter(|(range, _)| range.contains(&at)).map(|&(_, face)| face).collect();
            // The text's own face last, nil past the text; the mini-window
            // shows text outside a narrowing too.
            let text_face = match self.object(sources.buffer)? {
                Object::Buffer(_) if at >= sources.end => Value::NIL,
                // The property itself, where nothing could stand in for it.
                Object::Buffer(b) if sources.plain && b.properties.get_property(ByteOffset(at), "category").is_none() =>
                    b.properties.get_property(ByteOffset(at), "face").unwrap_or(Value::NIL),
                Object::Buffer(b) => {
                    let raw: Vec<(String, Value)> = b.properties.properties_at(ByteOffset(at)).into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
                    self.annotation_resolve(&raw, sources.face)?
                }
                _ => return Err("Unicode display buffer required".into()),
            };
            let end = sources.end.max(at + 1);
            layers.push(text_face);
            let next = self.next_property_boundary(sources.buffer, ByteOffset(at), ByteOffset(end))?.0.max(at + 1);
            let handle = self.layered_face(table, faces, sources, &layers);
            sources.run = (at, next, handle, layers);
        }
        if glyph != Value::NIL {
            let layers: Vec<Value> = std::iter::once(glyph).chain(sources.run.3.iter().copied()).collect();
            return Ok(self.layered_face(table, faces, sources, &layers));
        }
        let Some((string, strpos, overlay)) = string else { return Ok(sources.run.2) };
        let own = self.property_textget(string, strpos as i64, sources.face).map_err(crate::forwarded::fault_text)?;
        let base = if overlay { &sources.run.3[sources.run.3.len() - 1..] } else { &sources.run.3[..] };
        if own == Value::NIL && !overlay { return Ok(sources.run.2); }
        let layers: Vec<Value> = std::iter::once(own).chain(base.iter().copied()).collect();
        Ok(self.layered_face(table, faces, sources, &layers))
    }
    /// The face of the face references LAYERS, the first taking precedence.
    fn layered_face(&self, table: &FaceTable, faces: &mut Vec<Face>, sources: &mut ElementFaces, layers: &[Value]) -> FaceHandle {
        let layers: Vec<Value> = layers.iter().copied().filter(|&v| v != Value::NIL).collect();
        match layers.as_slice() {
            [] => return FaceHandle(0),
            [one] => return self.face_handle(table, faces, &mut sources.merged, *one),
            _ => {}
        }
        let key: Vec<u64> = layers.iter().map(|v| v.0).collect();
        if let Some(&handle) = sources.layered.get(&key) { return handle; }
        let graph = Graph(self);
        let face = layers.iter().map(|v| BoundedDatum::from_graph(&graph, v.0).ok().map(|d| (*d).clone())).collect::<Option<Vec<_>>>()
            .and_then(|items| BoundedDatum::new(&romacs_face::Datum::List(items)).ok())
            .and_then(|datum| romacs_face::remap::parse_face_ref(&datum).ok())
            .and_then(|face_ref| {
                let view = romacs_face::View::bind(&self.display_state.theme(), &romacs_face::Remap::empty()).ok()?;
                face_of_attrs(&view.realize_ref(&face_ref), "merged face").ok()
            });
        let handle = match face {
            Some(face) => { faces.push(face); FaceHandle(faces.len() as u32 - 1) }
            None => FaceHandle(0),
        };
        sources.layered.insert(key, handle);
        handle
    }
    /// The host advance of C in the default face, and that face's row height.
    pub(crate) fn host_default_advance(&mut self, c: char) -> Result<(u32, u32)> {
        self.host_face_advance("default", c)
    }
    /// The host advance of C in the face NAME, and that face's row height.
    pub(crate) fn host_face_advance(&mut self, name: &str, c: char) -> Result<(u32, u32)> {
        let face = self.snapshot_face(name)?;
        let input = romacs_layout_input::MeasureInput {
            font_selection: self.display_state.font_selection_order,
            font_alternatives: self.font_alternative_state()?,
            faces: vec![face],
            text: TextSlice { utf8: c.to_string(), face_runs: vec![], raw: vec![] },
        };
        let measured = self.host_layout(&input)?;
        Ok((measured.glyphs[0].0, measured.face_heights[0]))
    }
    /// The height the host gives WINDOW's mode line, or with HEADER its
    /// header line: GNU's CURRENT_MODE_LINE_HEIGHT and
    /// CURRENT_HEADER_LINE_HEIGHT (dispextern.h), the last displayed band's,
    /// else an estimate from the band's face.
    pub(crate) fn host_band_height(&mut self, window: Value, header: bool) -> Result<u32> {
        let id = self.window(window)?.id;
        let shown = self.display_state.previous.get(&id).and_then(|snapshot| {
            let (band, face) = if header { (&snapshot.header_line, snapshot.header_line_face) } else { (&snapshot.mode_line, snapshot.mode_line_face) };
            band.as_ref().map(|band| (snapshot, band, face))
        });
        let (faces, runs) = match shown {
            Some((snapshot, band, face)) => (snapshot.faces.clone(), std::iter::once(face.0 as usize)
                .chain(band.face_runs.iter().map(|run| run.face.0 as usize)).collect::<Vec<_>>()),
            None => {
                let table = self.display_faces()?;
                let band = if header { self.header_line_face_name(window)? } else { self.mode_line_face_name(window)? };
                let index = table.handle(&band).ok_or("band face required")?.0 as usize;
                (table.faces.clone(), vec![index])
            }
        };
        // A space in each face asks the host for that face's row height.
        let mut runs = runs;
        runs.sort_unstable();
        runs.dedup();
        let face_runs = runs.iter().enumerate().map(|(at, &face)| FaceRun { bytes: at..at + 1, face: FaceHandle(face as u32) }).collect();
        let input = romacs_layout_input::MeasureInput {
            font_selection: self.display_state.font_selection_order,
            font_alternatives: self.font_alternative_state()?,
            faces,
            text: TextSlice { utf8: " ".repeat(runs.len()), face_runs, raw: vec![] },
        };
        let heights = self.host_layout(&input)?.face_heights;
        Ok(runs.into_iter().map(|i| heights[i]).max().unwrap_or(0))
    }
    fn snapshot_face(&self, name: &str) -> Result<Face> {
        let attrs = self
            .display_state
            .theme()
            .realize(name)
            .ok_or("unknown face")?;
        face_of_attrs(&attrs, name)
    }
}

/// The display's face for realized ATTRS of the face NAME.
fn face_of_attrs(attrs: &romacs_face::Attrs, name: &str) -> Result<Face> {
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

/// The `Laid' painting reported in END for text published from buffer
/// byte BASE, LEN bytes long, through ZV when AT_ZV. Text that ran out
/// short of ZV leaves its end position unknown: its last fully visible
/// row ends there, never past.
/// The buffer byte a display's byte DISPLAY shows (SEGMENTS'), past them
/// the end of their text.
fn shown_byte(segments: &[mini_window::Segment], display: usize) -> usize {
    let end = segments.last().map_or(0, |s| if s.string { s.buffer } else { s.buffer + s.len });
    buffer_of(segments, display, end)
}

fn laid(end: &romacs_layout_input::WindowEnd, base: usize, len: usize, at_zv: bool, segments: Option<&[mini_window::Segment]>) -> Laid {
    let byte = |display: usize| segments.map_or(base + display, |s| shown_byte(s, display));
    let visible_end = match end.visible_end > len { true if at_zv => usize::MAX, true => byte(len), false => byte(end.visible_end) };
    Laid { end: byte(end.end), visible_end, body_height: end.body_height }
}
