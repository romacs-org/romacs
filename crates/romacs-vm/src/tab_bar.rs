//! The host frame's tab bar as the frame's snapshot publishes it (xdisp.c
//! display_tab_bar): the row `romacs--tab-bar-string' (rlisp/vm/prelude.el)
//! makes of the tab bar keymap's captions, each character in its `face'
//! property's named face over the `tab-bar' face, `tab-bar-lines' lines
//! tall. The faces it publishes are the `tab-bar' face first, then those
//! its characters use.
use crate::{data::Object, *};
use romacs_layout_input::{FaceHandle, FaceRun, TabBarSnapshot, TextSlice};

impl Vm {
    /// The tab bar to paint, None while the frame shows none. A signal
    /// from the Lisp that makes its row is a display diagnostic, and the
    /// row is empty.
    pub(crate) fn tab_bar_snapshot(&mut self) -> Result<Option<TabBarSnapshot>> {
        let lines = self.tab_bar_lines();
        if lines == 0 { return Ok(None); }
        // What may allocate comes first: the row is read with nothing
        // collecting after the call that returns it.
        let table = self.display_faces()?;
        let face = self.intern("face");
        let row = match self.call_prepared_source_raw("(lambda () (romacs--tab-bar-string))", &[]) {
            Ok(row) => row,
            Err(error) => {
                self.display_diagnostic(format!("tab bar: {error}"));
                Value::NIL
            }
        };
        let text = match self.object_if(row) {
            Some(Object::String(_)) => String::from_utf8_lossy(&self.string_bytes(row)?).into_owned(),
            _ => String::new(),
        };
        // Each run of one named face, from its first character.
        let mut named: Vec<(usize, Option<String>)> = vec![];
        if !text.is_empty() {
            let bytes: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
            for start in self.property_run_starts(row).map_err(crate::forwarded::fault_text)? {
                let value = self.property_textget(row, start, face).map_err(crate::forwarded::fault_text)?;
                let byte = bytes.get(start as usize).copied().unwrap_or(text.len());
                named.push((byte, self.ml_face_name(value)));
            }
        }
        let base_name = if self.named(&table, "tab-bar").is_some() { "tab-bar" } else { "default" };
        let base = self.named(&table, base_name).ok_or("tab-bar face required")?;
        let mut faces = vec![table.face(base)];
        let mut handles: Vec<(u32, FaceHandle)> = vec![];
        let mut face_runs = vec![];
        for (i, (start, name)) in named.iter().enumerate() {
            let end = named.get(i + 1).map_or(text.len(), |next| next.0);
            let Some(handle) = name.as_deref().and_then(|name| self.band_face(&table, name, base_name)) else { continue };
            let local = match handles.iter().find(|(h, _)| *h == handle.0) {
                Some((_, local)) => *local,
                None => {
                    faces.push(table.face(handle));
                    let local = FaceHandle(faces.len() as u32 - 1);
                    handles.push((handle.0, local));
                    local
                }
            };
            if *start < end { face_runs.push(FaceRun { bytes: *start..end, face: local }); }
        }
        Ok(Some(TabBarSnapshot { height: lines * self.window_state.line, text: TextSlice { extends: vec![], utf8: text, face_runs, raw: vec![] },
                                 face: FaceHandle(0), faces }))
    }
}
