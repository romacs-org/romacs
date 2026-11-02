//! The root frame's status bar (VM2-STATUS-BAR): a band between its root
//! window and its mini-window showing the mode line of the window GNU's
//! mode-line face rule treats as selected, expanded at the bar's columns
//! in `mode-line-active', like a window's own band. `romacs-status-bar-lines'
//! reserves it (window.rs `status_bar_reservation'); its height is the row
//! the host measures for its faces.
use super::mode_line;
use crate::{data::Object, *};
use romacs_layout_input::{BandMarks, FaceHandle, FaceRun, Pill, Segment, StatusBarSnapshot, TextSlice};

/// The corner radius of a segment's pill, in pixels.
const PILL_RADIUS: u32 = 6;

impl Vm {
    /// The window GNU's mode-line face rule treats as selected
    /// (CURRENT_MODE_LINE_ACTIVE_FACE_ID_3, dispextern.h:1553, with the
    /// real selected window, xdisp.c:28153): the selected window, or while
    /// it is the mini-window of an active minibuffer, the window
    /// `minibuffer-selected-window' names.
    pub(crate) fn real_selected_window(&mut self) -> Result<Value> {
        let selected = self.selected_window_value()?;
        if !self.window(selected)?.mini { return Ok(selected); }
        let level = self.slot("romacs--minibuf-level");
        if !self.variable_value(level)?.integer_value().is_some_and(|n| n > 0) { return Ok(selected); }
        let origin = self.slot("romacs--minibuf-selected-window");
        let origin = self.variable_value(origin)?;
        Ok(match self.object_if(origin) {
            Some(Object::Window(w)) if w.live() => origin,
            _ => selected,
        })
    }
    /// The window the status bar shows (`romacs-status-bar-window') and
    /// whether it holds the focus: the real selected window while it is one
    /// of the host frame's ordinary windows; while a child frame's window
    /// is selected, the host frame's own selected window, unfocused.
    pub(crate) fn status_bar_owner(&mut self) -> Result<(Value, bool)> {
        let real = self.real_selected_window()?;
        let host = self.host_frame_peek();
        let w = self.window(real)?;
        if w.frame == host && !w.mini { return Ok((real, true)); }
        let selected = self.frame(host)?.selected;
        if self.window(selected).is_ok_and(|w| w.live() && !w.mini) { return Ok((selected, false)); }
        // The host frame's mini-window is selected without a minibuffer:
        // the ordinary window selected last.
        let last = self.leaf_windows()?.into_iter().max_by_key(|&v| self.window(v).map_or(0, |w| w.use_time));
        Ok((last.unwrap_or(selected), false))
    }
    /// The status bar the frame wants, whether or not its last layout
    /// found room for it (`status_bar_shown' says), so a bar dropped for
    /// its height is measured again. Runs the owning window's mode-line
    /// Lisp.
    pub(crate) fn status_bar_snapshot(&mut self) -> Result<Option<StatusBarSnapshot>> {
        let reserved = self.window_state.status_reserved;
        if !self.status_bar_wanted() { return Ok(None); }
        let (owner, focused) = self.status_bar_owner()?;
        let window = self.window(owner)?.id;
        let [width, height] = self.window_state.text_area;
        let columns = (width / self.window_state.column.max(1)) as usize;
        let band = self.status_bar_band(owner, columns)?;
        let table = self.display_faces()?;
        let base = table.handle("mode-line-active").or_else(|| table.handle("mode-line")).ok_or("mode-line face required")?;
        let (text, runs, props, segments) = band.map_or_else(Default::default, |band| (band.text, band.runs, band.props, band.segments));
        let marks = self.band_marks(&text, &props, &segments, &table)?;
        // The bar's faces: its base face first, then those its runs use.
        let mut faces = vec![table.faces[base.0 as usize].clone()];
        let mut local: Vec<(u32, FaceHandle)> = vec![];
        let mut face_runs = vec![];
        for (bytes, name) in runs {
            let Some(handle) = table.handle(&name) else { continue };
            let face = match local.iter().find(|(h, _)| *h == handle.0) {
                Some(&(_, face)) => face,
                None => {
                    faces.push(table.faces[handle.0 as usize].clone());
                    local.push((handle.0, FaceHandle(faces.len() as u32 - 1)));
                    FaceHandle(faces.len() as u32 - 1)
                }
            };
            face_runs.push(FaceRun { bytes, face });
        }
        let border = self.host_border();
        Ok(Some(StatusBarSnapshot {
            x: border, y: border + height - self.window_state.mini_reserved - reserved, width, height: reserved,
            text: TextSlice { utf8: text, face_runs, raw: vec![] }, face: FaceHandle(0), faces, window, focused, marks,
        }))
    }
    /// The marks of a bar's TEXT: its segments, from the `:propertize'
    /// constructs that name one (SEGMENTS, which own all they expanded to)
    /// and from strings that carry `romacs-segment' (PROPS), each a pill on
    /// the background of its `romacs-pill' face, its text in the foreground
    /// of its `romacs-pill-text' face; and each character a `romacs-icon'
    /// alist names, drawn as that icon (`icon_characters').
    fn band_marks(&mut self, text: &str, props: &[(std::ops::Range<usize>, mode_line::Projected)],
                  segments: &[(std::ops::Range<usize>, mode_line::Projected)], table: &super::FaceTable) -> Result<BandMarks> {
        let mut marks = BandMarks::default();
        let structural = segments.iter().filter_map(|(bytes, projected)| self.segment_of(bytes, projected, table)).collect::<Vec<_>>();
        // Each structural segment's ranges by name, ordered, to find the one
        // containing a property run by search.
        let mut by_name: std::collections::HashMap<&str, Vec<std::ops::Range<usize>>> = std::collections::HashMap::new();
        for s in &structural { by_name.entry(s.name.as_str()).or_default().push(s.bytes.clone()); }
        for ranges in by_name.values_mut() { ranges.sort_by_key(|r| r.start); }
        let contained = |name: &str, bytes: &std::ops::Range<usize>| by_name.get(name).is_some_and(|ranges| {
            let at = ranges.partition_point(|r| r.start <= bytes.start);
            at > 0 && ranges[at - 1].end >= bytes.end
        });
        let mut icons: Option<Vec<(String, char)>> = None;
        for (bytes, projected) in props {
            if let Some(segment) = self.segment_of(bytes, projected, table)
                && !contained(&segment.name, bytes) {
                match marks.segments.last_mut() {
                    // One segment's pieces are adjacent runs of its name.
                    Some(last) if last.name == segment.name && last.bytes.end == segment.bytes.start && last.pill == segment.pill => {
                        last.bytes.end = segment.bytes.end;
                    }
                    _ => marks.segments.push(segment),
                }
            }
            let Some((_, alist)) = projected.iter().find(|(name, _)| *name == "romacs-icon") else { continue };
            let alist = self.ml_icon_alist(alist);
            let icons = match &mut icons { Some(icons) => icons, None => icons.insert(self.icon_characters()?) };
            for (at, c) in text[bytes.clone()].char_indices() {
                let named = alist.iter().find(|(key, _)| *key == c as i64).map(|(_, name)| name);
                if let Some(&(_, icon)) = named.and_then(|n| icons.iter().find(|(name, _)| name == n)) {
                    marks.icons.push((bytes.start + at, icon));
                }
            }
        }
        marks.segments.extend(structural);
        marks.segments.sort_by_key(|s| (s.bytes.start, std::cmp::Reverse(s.bytes.end)));
        Ok(marks)
    }
    /// The segment PROJECTED names over BYTES, if it names one.
    fn segment_of(&self, bytes: &std::ops::Range<usize>, projected: &mode_line::Projected, table: &super::FaceTable) -> Option<Segment> {
        let value = |name: &str| projected.iter().find(|(n, _)| *n == name).and_then(|(_, form)| self.ml_form_name(form));
        let face = |name: &str| value(name).and_then(|face| table.handle(&face)).map(|h| &table.faces[h.0 as usize]);
        let name = value("romacs-segment")?;
        let pill = face("romacs-pill").map(|f| Pill { background: f.background, radius: PILL_RADIUS,
                                                      foreground: face("romacs-pill-text").map(|t| t.foreground) });
        Some(Segment { bytes: bytes.clone(), name, pill })
    }
    /// What the bar shows now, as `romacs-status-bar-string' gives it.
    fn status_bar_shown_text(&mut self) -> Result<Option<String>> {
        if self.window_state.status_reserved == 0 { return Ok(None); }
        let Some(bar) = self.status_bar_snapshot()? else { return Ok(None) };
        let column = self.window_state.column.max(1) as f32;
        let advances: Vec<f32> = if self.display_state.hosted && self.has_layout_host() {
            // The characters as painted: an icon in place of the one it marks.
            let mut shown = String::new();
            let mut map = vec![];
            let mut icons = bar.marks.icons.iter().peekable();
            for (byte, ch) in bar.text.utf8.char_indices() {
                while icons.peek().is_some_and(|(at, _)| *at < byte) { icons.next(); }
                map.push((byte, shown.len()));
                shown.push(icons.peek().filter(|(at, _)| *at == byte).map_or(ch, |&&(_, icon)| icon));
            }
            map.push((bar.text.utf8.len(), shown.len()));
            let at = |byte: usize| map[map.partition_point(|(b, _)| *b < byte)].1;
            let face_runs = bar.text.face_runs.iter().map(|run| FaceRun { bytes: at(run.bytes.start)..at(run.bytes.end), face: run.face }).collect();
            let background = bar.faces[bar.face.0 as usize].background;
            let input = romacs_layout_input::MeasureInput {
                font_selection: self.display_state.font_selection_order,
                font_alternatives: self.font_alternative_state()?,
                faces: bar.faces.iter().map(|f| romacs_layout_input::Face { background, ..f.clone() }).collect(),
                text: TextSlice { utf8: shown, face_runs, raw: vec![] },
            };
            self.host_layout(&input)?.glyphs.iter().map(|(advance, _)| *advance as f32).collect()
        } else {
            bar.text.utf8.chars().map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) as f32 * column).collect()
        };
        let places = bar.places(&advances);
        Ok(Some(bar.shown_text(&places, column)))
    }
    /// A symbol's name from a projected value's FORM.
    fn ml_form_name(&self, form: &Form) -> Option<String> {
        match form {
            Form::Sym(name) if name != "nil" => Some(name.clone()),
            Form::Object(object) => object.value(self).ok().and_then(|v| self.ml_face_name(v)),
            _ => None,
        }
    }
    /// A `romacs-icon' alist FORM's entries: a character and an icon name.
    fn ml_icon_alist(&self, form: &Form) -> Vec<(i64, String)> {
        let form = match form { Form::Object(object) => object.value(self).map(|v| self.mode_line_form(v)).unwrap_or(Form::Sym("nil".into())), form => form.clone() };
        let Form::List(entries) = form else { return vec![] };
        entries.into_iter().filter_map(|entry| match entry {
            Form::Dotted(key, name) => match (key.as_slice(), *name) {
                ([Form::Int(k)], Form::Sym(name)) => Some((*k, name)),
                _ => None,
            },
            _ => None,
        }).collect()
    }
    /// Each icon's name and character from `romacs-icons-alist', while
    /// Romacs Icons is there to draw them (`romacs-icons--font' t) and
    /// wanted (`romacs-icons-source' `romacs', `romacs-icons-ascii' nil),
    /// as `romacs-icon' decides; else none.
    fn icon_characters(&mut self) -> Result<Vec<(String, char)>> {
        let [font, source, ascii] = ["romacs-icons--font", "romacs-icons-source", "romacs-icons-ascii"].map(|name| self.slot(name));
        let romacs = self.intern("romacs");
        if self.variable_value(font)? != Value::TRUE || self.variable_value(source)? != romacs
            || !matches!(self.variable_value(ascii)?, Value::NIL | Value::UNBOUND) { return Ok(vec![]); }
        let alist = self.slot("romacs-icons-alist");
        let alist = self.variable_value(alist)?;
        let Form::List(entries) = self.mode_line_form(alist) else { return Ok(vec![]) };
        Ok(entries.into_iter().filter_map(|entry| match entry {
            Form::List(fields) => match fields.as_slice() {
                [Form::Sym(name), Form::Int(point), ..] => char::from_u32(u32::try_from(*point).ok()?).map(|c| (name.clone(), c)),
                _ => None,
            },
            _ => None,
        }).collect())
    }
    /// OWNER's effective mode-line format expanded as its band is, at
    /// COLUMNS: `%-' fills them and the text is clipped there. None when it
    /// has no format.
    fn status_bar_band(&mut self, owner: Value, columns: usize) -> Result<Option<mode_line::Band>> {
        let Some(format) = self.window_mode_line_format(owner)? else { return Ok(None) };
        let format = self.mode_line_form(format);
        let [root, buffer_slot, saved] = ["#:vm2-status-bar-window", "#:vm2-status-bar-buffer", "#:vm2-status-bar-current"].map(|name| self.slot(name));
        self.slots[root].value = owner;
        self.slots[buffer_slot].value = self.window(owner)?.buffer;
        self.slots[saved].value = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = self.slots[buffer_slot].value;
        self.display_state.building_band = true;
        let outcome = (|| {
            let window = self.slots[root].value;
            let [_, below] = self.window_dividers(window)?;
            let text_height = self.window(window)?.height.saturating_sub(self.window_state.line + below);
            let mut state = self.mode_line_state(root, buffer_slot, text_height)?;
            state.clip = Some(columns);
            state.marks = true;
            let [_, band] = self.expand_bands(root, buffer_slot, &state, [None, Some(&format)])?;
            Ok(band)
        })();
        self.display_state.building_band = false;
        if self.buffer_state(self.slots[saved].value).is_ok_and(|b| b.live) {
            self.slots[self.buffer_slot].value = self.slots[saved].value;
        }
        for slot in [root, buffer_slot, saved] { self.slots[slot].value = Value::UNBOUND; }
        outcome
    }
    /// BAR, the bar the frame wants, when its last layout made room for it.
    pub(crate) fn status_bar_shown(&self, bar: Option<StatusBarSnapshot>) -> Option<StatusBarSnapshot> {
        bar.filter(|_| self.window_state.status_reserved > 0)
    }
    /// Whether BAR's row, as the host measures its faces, is a new height
    /// for the bar: then it is recorded, and the frame must be laid out
    /// again around it before it is painted.
    pub(crate) fn refit_status_bar(&mut self, bar: Option<&StatusBarSnapshot>) -> Result<bool> {
        let Some(bar) = bar else { return Ok(false) };
        if !self.display_state.hosted || !self.has_layout_host() { return Ok(false); }
        let runs = std::iter::once(0).chain(bar.text.face_runs.iter().map(|run| run.face.0 as usize)).collect();
        let measured = self.face_row_height(bar.faces.clone(), runs)?;
        if measured == 0 || measured == self.window_state.status_height { return Ok(false); }
        self.window_state.status_height = measured;
        Ok(true)
    }
}

/// The status bar's surface for Lisp (prims/frame.rs).
impl crate::window::Windows<'_> {
    /// `romacs-status-bar-window': the window the status bar shows.
    pub(crate) fn status_bar_window(&mut self) -> std::result::Result<Value, crate::prim::Fault> {
        self.vm.ensure_windows().map_err(crate::prim::Fault::Error)?;
        Ok(self.vm.status_bar_owner().map_err(crate::prim::Fault::Error)?.0)
    }
    /// `romacs-status-bar-height': the pixels the root frame's last layout
    /// left for the status bar, 0 while it shows none.
    pub(crate) fn status_bar_height(&mut self) -> u32 {
        self.vm.window_state.status_reserved
    }
    /// The text the status bar shows (`romacs-status-bar-string'), placed
    /// as the painter places it (`StatusBarSnapshot::places') with the
    /// advances the host measures, or a column each without a host; nil
    /// while it shows none. Expands the bar's mode line now.
    pub(crate) fn status_bar_text(&mut self) -> std::result::Result<Value, crate::prim::Fault> {
        let text = self.vm.status_bar_shown_text().map_err(crate::prim::Fault::Error)?;
        Ok(text.map_or(Value::NIL, |text| self.vm.make_lisp_string(text.into_bytes(), true)))
    }
}
