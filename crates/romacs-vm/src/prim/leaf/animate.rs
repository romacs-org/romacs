use super::*;
use crate::prim::Out;

impl Leaf<'_> {
    /// `romacs--display-animates': t, nil, or `unknown' while the host has
    /// not chosen its adapter or none is attached (a read's wait lends it
    /// out, and redisplay there runs Lisp).
    pub(crate) fn display_animates(&mut self) -> Out {
        let animates = self.vm.host_input.as_mut().and_then(|host| host.display_animates());
        Ok(match animates {
            Some(true) => Value::TRUE,
            Some(false) => Value::NIL,
            None => self.intern("unknown"),
        })
    }
}

impl Leaf<'_> {
    /// `romacs--native-window': ACTION `query', `show' or `hide' asked of
    /// the host; t when its window shows after the request, nil when it
    /// does not or the host has no native windows.
    /// (X . Y) of the pointer over the host frame, else nil.
    pub(crate) fn pointer(&mut self) -> Out {
        let Some((x, y)) = self.vm.host_input.as_mut().and_then(|host| host.pointer()) else { return Ok(Value::NIL) };
        let [x, y] = [x, y].map(|n| Value::integer(i64::from(n)).expect("a pixel is a fixnum"));
        Ok(self.cons(x, y))
    }
    pub(crate) fn native_window(&mut self, action: Value) -> Out {
        let action = match self.symbol_name(action).ok().as_deref() {
            Some("query") => crate::NativeWindow::Query,
            Some("show") => crate::NativeWindow::Show,
            Some("hide") => crate::NativeWindow::Hide,
            _ => return Err(crate::prim::Fault::Error("romacs--native-window: ACTION is query, show or hide".into())),
        };
        let shown = self.vm.host_input.as_mut().and_then(|host| host.native_window(action));
        Ok(if shown == Some(true) { Value::TRUE } else { Value::NIL })
    }
}
