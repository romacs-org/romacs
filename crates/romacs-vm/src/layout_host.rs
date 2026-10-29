//! The host layout hook (DIVERGENCES.md VM2-WTPS1). Within a measuring
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
        self.layout_host = host;
    }
    pub(crate) fn has_layout_host(&self) -> bool { self.layout_host.is_some() }
    /// INPUT's host geometry; an error when no host is attached.
    pub(crate) fn host_layout(&mut self, input: &MeasureInput) -> crate::Result<Measured> {
        let host = self.layout_host.as_mut().ok_or("no layout host")?;
        let measured = host.measure(input)?;
        if measured.glyphs.len() != input.text.utf8.chars().count() || measured.face_heights.len() != input.faces.len() {
            return Err("layout host answered for other input".into());
        }
        Ok(measured)
    }
    /// What painting FRAME would report for its windows.
    pub(crate) fn host_lay_out(&mut self, frame: &FrameSnapshot) -> crate::Result<Vec<WindowEnd>> {
        let host = self.layout_host.as_mut().ok_or("no layout host")?;
        let ends = host.lay_out(frame)?;
        if ends.len() != frame.windows.len() { return Err("layout host answered for other windows".into()); }
        Ok(ends)
    }
}
