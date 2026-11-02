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
