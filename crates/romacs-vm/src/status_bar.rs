//! The root frame's status bar (VM2-STATUS-BAR): a band between its root
//! window and its mini-window showing the mode line of the window GNU's
//! mode-line face rule treats as selected, expanded at the bar's columns
//! in `mode-line-active', like a window's own band. `romacs-status-bar-lines'
//! reserves it (window.rs `status_bar_reservation'); its height is the row
//! the host measures for its faces.
use super::mode_line;
use crate::{data::Object, *};
use romacs_layout_input::{FaceHandle, FaceRun, StatusBarSnapshot, TextSlice};

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
        if !self.status_bar_wanted() {
            self.display_state.status_bar_text = None;
            return Ok(None);
        }
        let (owner, focused) = self.status_bar_owner()?;
        let window = self.window(owner)?.id;
        let [width, height] = self.window_state.text_area;
        let columns = (width / self.window_state.column.max(1)) as usize;
        let band = self.status_bar_band(owner, columns)?;
        let table = self.display_faces()?;
        let base = table.handle("mode-line-active").or_else(|| table.handle("mode-line")).ok_or("mode-line face required")?;
        let (text, runs) = band.map_or((String::new(), vec![]), |band| (band.text, band.runs));
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
        self.display_state.status_bar_text = (reserved > 0).then(|| text.clone());
        let border = self.host_border();
        Ok(Some(StatusBarSnapshot {
            x: border, y: border + height - self.window_state.mini_reserved - reserved, width, height: reserved,
            text: TextSlice { utf8: text, face_runs, raw: vec![] }, face: FaceHandle(0), faces, window, focused,
        }))
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
    /// The text the status bar last showed, nil while it shows none.
    pub(crate) fn status_bar_text(&mut self) -> Value {
        match self.vm.display_state.status_bar_text.clone() {
            Some(text) => { let multibyte = !text.is_ascii(); self.vm.make_lisp_string(text.into_bytes(), multibyte) }
            None => Value::NIL,
        }
    }
}
