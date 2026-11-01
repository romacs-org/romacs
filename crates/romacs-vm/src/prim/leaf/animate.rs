use super::*;
use crate::prim::Out;

impl Leaf<'_> {
    /// `romacs--display-animates': t, nil, or `unknown' while the host has
    /// not chosen its adapter; nil with no host.
    pub(crate) fn display_animates(&mut self) -> Out {
        let animates = self.vm.host_input.as_mut().map_or(Some(false), |host| host.display_animates());
        Ok(match animates {
            Some(true) => Value::TRUE,
            Some(false) => Value::NIL,
            None => self.intern("unknown"),
        })
    }

    /// The id WINDOW's glyphs carry on the display.
    pub(crate) fn display_window_id(&self, window: Value) -> std::result::Result<u64, Fault> {
        self.vm.window(window).map(|w| w.id).map_err(|_| Fault::Wrong("window-live-p", window))
    }

    /// Hand text animation ID to the host display (None: remove it).
    pub(crate) fn text_animation(&mut self, id: u64, animation: Option<romacs_display_protocol::text_animation::TextAnimation>) -> Out {
        self.vm.host_input.as_mut()
            .ok_or_else(|| Fault::Error("this display draws no text animation".into()))?
            .text_animation(id, animation)
            .map_err(Fault::Error)?;
        Ok(Value::NIL)
    }
}
