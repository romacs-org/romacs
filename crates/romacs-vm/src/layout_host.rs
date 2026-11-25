//! The host layout hook (ledger/divergences VM2-WTPS1). Within a measuring
//! primitive, the VM hands the host one window's display input, owned
//! text and faces, and the host answers with the geometry its own layout
//! gives that input, presenting nothing. No Lisp runs in the host.
use crate::Vm;
use romacs_layout_input::{FrameSnapshot, MeasureInput, Measured, WindowEnd};

/// The embedder's layout, asked synchronously.
pub trait LayoutHost {
    fn measure(&mut self, input: &MeasureInput) -> Result<Measured, String>;
    /// The journal painting FRAME would report, presenting nothing.
    fn lay_out(&mut self, frame: &FrameSnapshot) -> Result<Vec<WindowEnd>, String>;
}

impl Vm {
    /// Attach (Some) or detach (None) the host layout. While attached, a
    /// window the host has laid out is measured with it.
    pub fn set_layout_host(&mut self, host: Option<Box<dyn LayoutHost>>) {
        let host = host.map(|host| Box::new(crate::host_calls::Counted(host)) as Box<dyn LayoutHost>);
        self.ui_set(host, |ui| &mut ui.layout_host);
    }
    pub(crate) fn has_layout_host(&self) -> bool { self.ui_ref(|ui| ui.layout_host.is_some()).unwrap_or(false) }
    /// INPUT's host geometry; an error when no host is attached.
    pub(crate) fn host_layout(&mut self, input: &MeasureInput) -> crate::Result<Measured> {
        let measured = self.ui(|ui| ui.layout_host.as_mut().map(|host| host.measure(input))).flatten().ok_or("no layout host")??;
        if measured.glyphs.len() != input.text.utf8.chars().count() || measured.ascents.len() != measured.glyphs.len() || measured.fallback.len() != measured.glyphs.len() || measured.face_ascents.len() != measured.face_heights.len() || measured.face_heights.len() != input.faces.len() {
            return Err("layout host answered for other input".into());
        }
        Ok(measured)
    }
    /// What painting FRAME would report for its windows.
    pub(crate) fn host_lay_out(&mut self, frame: &FrameSnapshot) -> crate::Result<Vec<WindowEnd>> {
        let ends = self.ui(|ui| ui.layout_host.as_mut().map(|host| host.lay_out(frame))).flatten().ok_or("no layout host")??;
        if ends.len() != frame.windows.len() { return Err("layout host answered for other windows".into()); }
        Ok(ends)
    }
}
