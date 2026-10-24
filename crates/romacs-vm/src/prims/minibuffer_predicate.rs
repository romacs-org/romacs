//! Predicate over native minibuffer windows.
use super::*;

defsubr! { name = "minibufferp", gnu = "minibuf.c:403", effect = Reads,
fn minibufferp(cx; optional buffer: Value, live: Value) {
    cx.minibuffer_predicate(buffer, live)
}}

#[cfg(test)]
mod tests {
    #[test]
    fn minibuffer_predicate_uses_native_role_not_buffer_name() {
        let mut vm = crate::Vm::new();
        let got = vm.eval_source_raw("(list (minibufferp) (minibufferp nil t) (minibufferp \"missing\") (condition-case e (minibufferp 1) (error e)) (minibufferp (window-buffer (minibuffer-window))) (minibufferp (window-buffer (minibuffer-window)) t) (minibufferp (get-buffer-create \" *Minibuf-99*\")))").unwrap();
        assert_eq!(vm.display(got).unwrap(), "(nil nil nil (wrong-type-argument bufferp 1) t nil nil)");
    }
}
