//! The root frame's status bar (VM2-STATUS-BAR): a band between its root
//! window and its mini-window showing the mode line of the window GNU's
//! mode-line face rule treats as selected, expanded at the bar's columns
//! in `mode-line-active', like a window's own band. `romacs-status-bar-lines'
//! reserves it (window.rs `status_bar_reservation'); its height is the row
//! the host measures for its faces.
use super::mode_line;
use crate::{data::Object, *};
use romacs_layout_input::{BandMarks, Face, FaceHandle, FaceRun, Pill, Segment, StatusBarSnapshot, TextSlice};

/// What the bar's expansion depends on (`Vm::bar_key').
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BarKey {
    epoch: u64, collections: u64, faces: u64, columns: usize, focused: bool,
    minibuffer: (u64, u64, u64),
    lines: Vec<LineKey>,
}
romacs_snapshot::snapshot_struct!(BarKey { epoch, collections, faces, columns, focused, minibuffer, lines });

/// What one window's line in the bar depends on (`Vm::line_key').
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct LineKey {
    id: u64, buffer: u64, name: Option<String>, format: u64, modiff: u64, modified: bool,
    point: i64, start: i64, size: (u32, u32), accessible: (usize, usize),
}
romacs_snapshot::snapshot_struct!(LineKey { id, buffer, name, format, modiff, modified, point, start, size, accessible });

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
        let columns = StatusBarSnapshot::columns(width, self.window_state.column);
        let border = self.host_border();
        // At the top the bar ends where the root window starts; at the
        // bottom it starts above the mini-window.
        let root = self.window_root_peek(crate::window::ROOT);
        let y = if self.status_bar_top() { self.window(root)?.y.saturating_sub(reserved) }
                else { height - self.window_state.mini_reserved - reserved };
        let place = |bar: StatusBarSnapshot| StatusBarSnapshot { x: border, y: border + y, width, height: reserved, ..bar };
        // Expanded again only when GNU's redisplay would update the line
        // (`BarKey'); otherwise the last expansion is shown where the bar is now.
        let key = self.bar_key(owner, focused, columns)?;
        if let Some((known, bar)) = &self.display_state.bar_cache && *known == key {
            return Ok(Some(place(bar.clone())));
        }
        // Whether the owner's buffer is modified, read before its line's Lisp
        // runs: that may collect, which moves windows (`owner' is not rooted).
        let buffer = self.window(owner)?.buffer;
        let modified = matches!(self.object(buffer)?, Object::Buffer(b) if b.state.modified());
        let band = self.status_bar_band(owner, columns)?;
        // Kept for a click, which reads it as a string (`status_bar_target').
        self.status_bar_line = band.clone().map(|band| Box::new(band) as Box<dyn std::any::Any>);
        let table = self.display_faces()?;
        let base = table.handle("mode-line-active").or_else(|| table.handle("mode-line")).ok_or("mode-line face required")?;
        let (text, runs, props, segments) = band.map_or_else(Default::default, |band| (band.text, band.runs, band.props, band.segments));
        // The owner's line leads the bar, where it has one; its pills read
        // whether its buffer is modified (`romacs-pill-modified').
        let owner_chars = self.display_state.bar_parts.first().filter(|(_, id)| *id == window).map_or(0, |(chars, _)| chars.end);
        let owner_chars = self.display_state.bar_kept.as_ref().map_or(owner_chars, |kept| kept.partition_point(|&i| i < owner_chars));
        let owner_end = text.char_indices().nth(owner_chars).map_or(text.len(), |(at, _)| at);
        let marks = self.band_marks(&text, &props, &segments, &table, if modified { owner_end } else { 0 })?;
        // The bar's faces: its base face first, then those its runs use.
        let mut faces = vec![table.faces[base.0 as usize].clone()];
        let mut local: std::collections::HashMap<u32, FaceHandle> = std::collections::HashMap::new();
        let mut face_runs = vec![];
        for (bytes, name) in runs {
            let Some(handle) = table.handle(&name) else { continue };
            let face = *local.entry(handle.0).or_insert_with(|| {
                faces.push(table.faces[handle.0 as usize].clone());
                FaceHandle(faces.len() as u32 - 1)
            });
            face_runs.push(FaceRun { bytes, face });
        }
        let bar = StatusBarSnapshot { x: 0, y: 0, width: 0, height: 0,
            text: TextSlice { extends: vec![], utf8: text, face_runs, raw: vec![] }, face: FaceHandle(0), faces, window, focused, marks };
        self.display_state.bar_cache = Some((key, bar.clone()));
        Ok(Some(place(bar)))
    }
    /// What the bar's line shows depends on, as GNU's redisplay decides when
    /// to update a mode line (`update_mode_lines', a window's buffer, its
    /// point, start and size, the buffer's tick and modified state), for
    /// OWNER and each context window, with the faces, the bar's COLUMNS,
    /// FOCUSED, the minibuffer's state and the collections (cached values
    /// move with them).
    fn bar_key(&mut self, owner: Value, focused: bool, columns: usize) -> Result<BarKey> {
        let mut lines = vec![];
        for window in std::iter::once(owner).chain(self.context_windows(owner)?) {
            lines.push(self.line_key(window)?);
        }
        let [level, origin] = ["romacs--minibuf-level", "romacs--minibuf-selected-window"].map(|name| self.slot(name));
        Ok(BarKey {
            epoch: self.display_state.mode_line_epoch, collections: self.gc_stats.collections,
            faces: self.display_state.face_revision, columns, focused,
            minibuffer: (self.variable_value(level)?.0, self.variable_value(origin)?.0, self.selected_window_value()?.0),
            lines,
        })
    }
    /// What WINDOW's own line depends on (`bar_key').
    fn line_key(&mut self, window: Value) -> Result<LineKey> {
        let format = self.window_mode_line_format(window)?.map_or(0, |v| v.0);
        let w = self.window(window)?;
        let (id, buffer, size, start) = (w.id, w.buffer, (w.width, w.height), w.start);
        let start = self.marker_position_value(start)?.integer_value().unwrap_or(1);
        let point = self.window_point_value(window)?.integer_value().unwrap_or(1);
        let Object::Buffer(b) = self.object(buffer)? else { return Err("buffer required".into()) };
        let accessible = b.accessible();
        // Its name too: `rename-buffer' changes `%b' without a modification.
        Ok(LineKey { id, buffer: buffer.0, name: b.state.name.clone(), format, modiff: b.state.modiff, modified: b.state.modified(),
                     point, start, size, accessible: (accessible.start.0, accessible.end.0) })
    }
    /// The marks of a bar's TEXT: its segments, from the `:propertize'
    /// constructs that name one (SEGMENTS, which own all they expanded to)
    /// and from strings that carry `romacs-segment' (PROPS), each a pill on
    /// the background of its `romacs-pill' face, its text in the foreground
    /// of its `romacs-pill-text' face; and each character a `romacs-icon'
    /// alist names, drawn as that icon (`icon_index'). A pill that starts
    /// before MODIFIED, a byte, has its text in the foreground of its
    /// `romacs-pill-modified' face where it names one: the bar's own
    /// line, whose buffer is modified.
    pub(super) fn band_marks(&mut self, text: &str, props: &[(std::ops::Range<usize>, mode_line::Projected)],
                  segments: &[(std::ops::Range<usize>, mode_line::Projected)], table: &super::FaceTable, modified: usize) -> Result<BandMarks> {
        let mut marks = BandMarks::default();
        let structural: Vec<Segment> = segments.iter().filter_map(|(bytes, projected)| self.segment_of(bytes, projected, table, bytes.start < modified)).collect();
        // A string's segment inside a construct of the same name is that
        // construct's: each name's ranges, ordered, searched per run.
        let mut by_name: std::collections::HashMap<&str, Vec<std::ops::Range<usize>>> = std::collections::HashMap::new();
        for s in &structural { by_name.entry(s.name.as_str()).or_default().push(s.bytes.clone()); }
        for ranges in by_name.values_mut() { ranges.sort_by_key(|r| r.start); }
        let contained = |name: &str, bytes: &std::ops::Range<usize>| by_name.get(name).is_some_and(|ranges| {
            let at = ranges.partition_point(|r| r.start <= bytes.start);
            at > 0 && ranges[at - 1].end >= bytes.end
        });
        let mut icons: Option<std::collections::HashMap<u64, char>> = None;
        for (bytes, projected) in props {
            if let Some(segment) = self.segment_of(bytes, projected, table, bytes.start < modified) && !contained(&segment.name, bytes) {
                match marks.segments.last_mut() {
                    // One segment's pieces are adjacent runs of its name.
                    Some(last) if last.name == segment.name && last.bytes.end == segment.bytes.start && last.pill == segment.pill => {
                        last.bytes.end = segment.bytes.end;
                    }
                    _ => marks.segments.push(segment),
                }
            }
            let Some((_, alist)) = projected.iter().find(|(name, _)| *name == "romacs-icon") else { continue };
            let icons = match &icons { Some(icons) => icons, None => icons.insert(self.icon_index()?) };
            if icons.is_empty() { continue; }
            // The piece's own alist, its first entry per character (assq),
            // resolved to icons once.
            let mut own: std::collections::HashMap<i64, Option<char>> = std::collections::HashMap::new();
            for (key, name) in self.ml_icon_alist(alist) {
                if !own.contains_key(&key) { own.insert(key, icons.get(&self.intern(&name).0).copied()); }
            }
            for (at, c) in text[bytes.clone()].char_indices() {
                if let Some(&Some(icon)) = own.get(&(c as i64)) { marks.icons.push((bytes.start + at, icon)); }
            }
        }
        marks.segments.extend(structural);
        marks.segments.sort_by_key(|s| (s.bytes.start, std::cmp::Reverse(s.bytes.end)));
        Ok(marks)
    }
    /// Fit BAND, a marked band, into COLUMNS (VM2-MODE-LINE): drop each
    /// character its `romacs-icon' alist maps to `none', then whole
    /// segments that have a `romacs-priority', the lowest first (of equal
    /// ones the later), until the rest is no wider; a segment without one
    /// stays. Each shown character's index in the expanded text, when
    /// anything was dropped.
    pub(super) fn fit_band(&mut self, band: &mut mode_line::Band, columns: usize) -> Result<Option<Vec<usize>>> {
        if band.segments.is_empty() && band.props.is_empty() { return Ok(None); }
        // Each character's byte and columns, once; a dropped flag each and
        // the running width, so every drop costs its own characters only.
        let cells = |c: char| if ('\u{FE000}'..='\u{FE0FF}').contains(&c) { 1 } else { unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) };
        let chars: Vec<(usize, usize)> = band.text.char_indices().map(|(at, c)| (at, cells(c))).collect();
        let index = |byte: usize| chars.partition_point(|(at, _)| *at < byte);
        let mut dropped = vec![false; chars.len()];
        let mut width: usize = chars.iter().map(|(_, w)| w).sum();
        let mut drop = |range: std::ops::Range<usize>, dropped: &mut Vec<bool>, width: &mut usize| {
            for i in index(range.start)..index(range.end) {
                if !dropped[i] { dropped[i] = true; *width -= chars[i].1; }
            }
        };
        for (bytes, projected) in &band.props {
            let Some((_, alist)) = projected.iter().find(|(name, _)| *name == "romacs-icon") else { continue };
            // Each key's first entry decides, as `assq' (and the painter) does.
            let mut first: std::collections::HashMap<i64, bool> = std::collections::HashMap::new();
            for (key, name) in self.ml_icon_alist(alist) { first.entry(key).or_insert(name == "none"); }
            let none: std::collections::HashSet<i64> = first.into_iter().filter(|(_, none)| *none).map(|(key, _)| key).collect();
            if none.is_empty() { continue; }
            for (at, c) in band.text[bytes.clone()].char_indices() {
                if none.contains(&(c as i64)) { drop(bytes.start + at..bytes.start + at + c.len_utf8(), &mut dropped, &mut width); }
            }
        }
        if width > columns {
            let mut ranked: Vec<(i64, std::ops::Range<usize>)> = band.segments.iter().filter_map(|(range, projected)| {
                projected.iter().find(|(name, _)| *name == "romacs-priority")
                    .and_then(|(_, form)| match &**form { Form::Int(n) => Some((*n, range.clone())), _ => None })
            }).collect();
            ranked.sort_by_key(|(priority, range)| (*priority, std::cmp::Reverse(range.start)));
            for (_, range) in ranked {
                if width <= columns { break; }
                drop(range, &mut dropped, &mut width);
            }
        }
        // The dropped characters as ordered byte ranges, in one pass.
        let mut merged: Vec<std::ops::Range<usize>> = vec![];
        for (i, &(at, _)) in chars.iter().enumerate() {
            if !dropped[i] { continue; }
            let end = chars.get(i + 1).map_or(band.text.len(), |(next, _)| *next);
            match merged.last_mut() {
                Some(last) if last.end == at => last.end = end,
                _ => merged.push(at..end),
            }
        }
        if merged.is_empty() { return Ok(None); }
        Ok(Some(band.drop_ranges(&merged)))
    }
    /// The segment PROJECTED names over BYTES, if it names one; its pill's
    /// text in its `romacs-pill-modified' face when MODIFIED.
    fn segment_of(&self, bytes: &std::ops::Range<usize>, projected: &mode_line::Projected, table: &super::FaceTable, modified: bool) -> Option<Segment> {
        let value = |name: &str| projected.iter().find(|(n, _)| *n == name).and_then(|(_, form)| self.ml_form_name(form));
        let face = |name: &str| value(name).and_then(|face| table.handle(&face)).map(|h| &table.faces[h.0 as usize]);
        let name = value("romacs-segment")?;
        let pill = face("romacs-pill").map(|f| Pill { background: f.background, radius: PILL_RADIUS,
                                                      foreground: modified.then(|| face("romacs-pill-modified")).flatten()
                                                          .or_else(|| face("romacs-pill-text")).map(|t| t.foreground) });
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
                text: TextSlice { extends: vec![], utf8: shown, face_runs, raw: vec![] },
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
    /// Each icon's character by its name's symbol, read in place from
    /// `romacs-icons-alist' once for a bar, while Romacs Icons is there to
    /// draw them (`romacs-icons--font' t) and wanted (`romacs-icons-source'
    /// `romacs', `romacs-icons-ascii' nil), as `romacs-icon' decides; else
    /// none.
    fn icon_index(&mut self) -> Result<std::collections::HashMap<u64, char>> {
        let [font, source, ascii, alist] = ["romacs-icons--font", "romacs-icons-source", "romacs-icons-ascii", "romacs-icons-alist"].map(|name| self.slot(name));
        let romacs = self.intern("romacs");
        let mut index = std::collections::HashMap::new();
        if self.variable_value(font)? != Value::TRUE || self.variable_value(source)? != romacs
            || !matches!(self.variable_value(ascii)?, Value::NIL | Value::UNBOUND) { return Ok(index); }
        let mut tail = self.variable_value(alist)?;
        while let Ok((entry, rest)) = self.cons_pair(tail) {
            tail = rest;
            let Ok((key, fields)) = self.cons_pair(entry) else { continue };
            let point = self.cons_pair(fields).ok().and_then(|(point, _)| point.integer_value());
            // The first entry for a name is the one `assq' finds.
            if let Some(c) = point.and_then(|p| u32::try_from(p).ok()).and_then(char::from_u32) { index.entry(key.0).or_insert(c); }
        }
        Ok(index)
    }
    /// The bar's band (VM2-STATUS-BAR): OWNER's line, then the line of
    /// each other window of the host frame that sets its own format (a
    /// popup's, a side window's) as a context segment, all at COLUMNS and
    /// fitted there; the click map of what it shows recorded. None when no
    /// window has a line.
    fn status_bar_band(&mut self, owner: Value, columns: usize) -> Result<Option<mode_line::Band>> {
        let mut parts: Vec<(std::ops::Range<usize>, u64)> = vec![];
        let mut bar: Option<mode_line::Band> = None;
        let mut bar_chars = 0;
        let separator = " │ ";
        // By id: a line's `:eval' may collect, which moves windows.
        let mut ids = vec![self.window(owner)?.id];
        for window in self.context_windows(owner)? { ids.push(self.window(window)?.id); }
        let mut by_id: std::collections::HashMap<u64, Value> = std::collections::HashMap::new();
        let mut resolved_at = None;
        for (i, id) in ids.into_iter().enumerate() {
            if resolved_at != Some(self.gc_stats.collections) {
                by_id = self.leaf_windows()?.into_iter().filter_map(|w| self.window(w).ok().map(|x| (x.id, w))).collect();
                resolved_at = Some(self.gc_stats.collections);
            }
            let Some(&window) = by_id.get(&id) else { continue };
            let Some(line) = self.window_line(window, columns)? else { continue };
            let line = if i == 0 { line } else { self.plain_markers(line).into_context("mode-line-inactive") };
            let length = line.text.chars().count();
            match bar.as_mut() {
                None => {
                    parts.push((0..length, id));
                    bar = Some(line);
                    bar_chars = length;
                }
                Some(bar) => {
                    let start = bar_chars + separator.chars().count();
                    let separator_start = bar.text.len();
                    parts.push((start..start + length, id));
                    bar.append(line, separator, "mode-line-inactive");
                    // A context goes with its separator when the bar drops it.
                    if let Some((range, _)) = bar.segments.last_mut() { range.start = separator_start; }
                    bar_chars = start + length;
                }
            }
        }
        self.display_state.bar_parts = parts;
        self.display_state.bar_kept = None;
        if let Some(band) = bar.as_mut() {
            self.display_state.bar_kept = self.fit_band(band, columns)?;
        }
        Ok(bar)
    }
    /// LINE, a context's, with its `centre-align' and `right-align' markers
    /// plain text: the bar's zones are the selected line's. A marker is a
    /// segment and a projected property, a `:propertize''s symbol or a
    /// string's own value: none stays.
    fn plain_markers(&self, mut line: mode_line::Band) -> mode_line::Band {
        let marker = |(name, value): &(&'static str, std::rc::Rc<Form>)| *name == "romacs-segment"
            && matches!(self.ml_form_name(value).as_deref(), Some("right-align" | "centre-align"));
        line.segments.retain(|(_, projected)| !projected.iter().any(marker));
        for (_, projected) in line.props.iter_mut() {
            if projected.iter().any(marker) {
                *projected = std::rc::Rc::new(projected.iter().filter(|p| !marker(p)).cloned().collect());
            }
        }
        line
    }
    /// The host frame's ordinary windows but OWNER whose line differs from
    /// the default one (a buffer-local `mode-line-format' or a window
    /// parameter), in the frame's order; a nil or `none' format has no line.
    fn context_windows(&mut self, owner: Value) -> Result<Vec<Value>> {
        let mut found = vec![];
        for window in self.leaf_windows()? {
            if window == owner || !self.bar_owns_mode_line(window) { continue; }
            if !self.window_shows_default_mode_line(window)? && self.window_mode_line_format(window)?.is_some() {
                found.push(window);
            }
        }
        Ok(found)
    }
    /// WINDOW's line as the bar shows it: its effective format expanded at
    /// COLUMNS with its buffer current and WINDOW selected, as GNU's
    /// display_mode_lines selects it (frame-local, without select-window's
    /// point effects, restored), reading its own point and start. Unlike
    /// GNU's, no window's point is installed in its buffer: nothing moves.
    fn window_line(&mut self, window: Value, columns: usize) -> Result<Option<mode_line::Band>> {
        let Some(format) = self.window_mode_line_format(window)? else { return Ok(None) };
        let format = self.band_format_form(format);
        let [root, buffer_slot, saved, selection] = ["#:vm2-status-bar-window", "#:vm2-status-bar-buffer", "#:vm2-status-bar-current", "#:vm2-status-bar-selected"].map(|name| self.slot(name));
        self.slots[root].value = window;
        self.slots[buffer_slot].value = self.window(window)?.buffer;
        self.slots[saved].value = self.slots[self.buffer_slot].value;
        self.slots[self.buffer_slot].value = self.slots[buffer_slot].value;
        self.display_state.building_band = true;
        let outcome = (|| {
            let window = self.slots[root].value;
            let [_, below] = self.window_dividers(window)?;
            let text_height = self.window(window)?.height.saturating_sub(below);
            let mut state = self.mode_line_state(root, buffer_slot, text_height)?;
            state.clip = Some(columns);
            state.marks = true;
            let frame = self.frame_value_peek();
            let selected = self.frame(frame)?.selected;
            self.slots[selection].value = selected;
            self.frame_mut(frame)?.selected = window;
            let band = self.mode_line_band(&format, &state);
            let frame = self.frame_value_peek();
            self.frame_mut(frame)?.selected = self.slots[selection].value;
            band
        })();
        self.display_state.building_band = false;
        if self.buffer_state(self.slots[saved].value).is_ok_and(|b| b.live) {
            self.slots[self.buffer_slot].value = self.slots[saved].value;
        }
        for slot in [root, buffer_slot, saved, selection] { self.slots[slot].value = Value::UNBOUND; }
        outcome.map(Some)
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
        let mut runs: Vec<usize> = std::iter::once(0).chain(bar.text.face_runs.iter().map(|run| run.face.0 as usize)).collect();
        runs.sort_unstable();
        runs.dedup();
        // Measured once per face revision and faces used: a key repaints
        // the bar with the same faces.
        let revision = self.display_state.face_revision;
        let used: Vec<Face> = runs.iter().filter_map(|&i| bar.faces.get(i).cloned()).collect();
        let measured = match &self.display_state.bar_fit {
            Some((known, faces, height)) if *known == revision && *faces == used => *height,
            _ => {
                let height = self.face_row_height(bar.faces.clone(), runs)?;
                self.display_state.bar_fit = Some((revision, used, height));
                height
            }
        };
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
    /// `romacs--window-of-id': the live window whose display id is ID, the
    /// one a host's pointer event names, else nil.
    pub(crate) fn window_of_id(&mut self, id: Value) -> std::result::Result<Value, crate::prim::Fault> {
        let Some(id) = id.integer_value().and_then(|id| u64::try_from(id).ok()) else { return Ok(Value::NIL) };
        let windows = self.vm.leaf_windows().map_err(crate::prim::Fault::Error)?;
        Ok(windows.into_iter().find(|&w| self.vm.window(w).is_ok_and(|w| w.id == id)).unwrap_or(Value::NIL))
    }
    /// `romacs--status-bar-target': for the status bar's shown character
    /// COLUMN, (WINDOW STRING COLUMN): the window whose line it is, and the
    /// bar's text as it was last painted, a string with the properties a
    /// click reads; nil over a separator or with no bar.
    pub(crate) fn status_bar_target(&mut self, column: Value) -> std::result::Result<Value, crate::prim::Fault> {
        let at = column.integer_value().and_then(|c| usize::try_from(c).ok()).unwrap_or(0);
        let Some((id, _)) = self.vm.display_state.bar_target(at) else { return Ok(Value::NIL) };
        // The kept line as a string, built now: no Lisp runs, it allocates.
        let Some(line) = self.vm.status_bar_line.take() else { return Ok(Value::NIL) };
        let string = match line.downcast_ref::<mode_line::Band>() {
            Some(band) => self.vm.mode_line_string(band, Value::NIL),
            None => Ok(Value::NIL),
        };
        self.vm.status_bar_line = Some(line);
        let string = string.map_err(crate::prim::Fault::Error)?;
        let slot = self.vm.slot("#:vm2-status-bar-string");
        self.vm.slots[slot].value = string;
        let window = self.window_of_id(Value::integer(id as i64).map_err(crate::prim::Fault::Error)?)?;
        let string = std::mem::replace(&mut self.vm.slots[slot].value, Value::UNBOUND);
        if window == Value::NIL || string == Value::NIL { return Ok(Value::NIL); }
        Ok(self.vm.list_value(&[window, string, column]))
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
