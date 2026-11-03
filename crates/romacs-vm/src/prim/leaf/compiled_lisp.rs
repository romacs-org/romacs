//! Background compiles of compiled Lisp (`crate::compiled_lisp`): what
//! rlisp/romacs-precompile.el's switch asks.
use super::*;

impl Leaf<'_> {
    pub(crate) fn compiled_lisp_background(&mut self, on: bool, delay: Value) -> crate::prim::Out {
        if !crate::compiled_lisp::compiles() {
            return Ok(Value::NIL);
        }
        if !on {
            crate::compiled_lisp::stop_background();
            return Ok(Value::TRUE);
        }
        let seconds = match delay.integer_value() {
            Some(n) => n as f64,
            None => self.float_value(delay)?,
        };
        let load_path = self.vm.load_path_strings().map_err(Fault::Error)?;
        crate::compiled_lisp::start_background(load_path, std::time::Duration::from_secs_f64(seconds.max(0.0)));
        Ok(Value::TRUE)
    }
}
