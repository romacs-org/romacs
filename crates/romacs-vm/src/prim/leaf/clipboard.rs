use super::*;
use crate::prim::Out;

impl Leaf<'_> {
    pub(crate) fn clipboard_text(&mut self) -> Out {
        // No Lisp callback or GC occurs during the host's bounded wait. End the
        // host borrow before allocating the returned, owned Unicode text.
        let text = self.vm.host_input.as_mut()
            .ok_or_else(|| Fault::Error("native clipboard is unavailable".into()))?
            .clipboard_text()
            .map_err(Fault::Error)?;
        Ok(match text {
            Some(text) => self.string(&text),
            None => Value::NIL,
        })
    }

    pub(crate) fn clipboard_set(&mut self, text: Value) -> Out {
        // Copy before entering the host: only owned Rust bytes can cross the
        // display transport, never a Value or a borrow into the VM heap.
        let text = self.string_text(text)?;
        self.vm.host_input.as_mut()
            .ok_or_else(|| Fault::Error("native clipboard is unavailable".into()))?
            .set_clipboard_text(&text)
            .map_err(Fault::Error)?;
        Ok(Value::NIL)
    }
}
