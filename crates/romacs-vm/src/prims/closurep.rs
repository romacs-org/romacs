//! GNU's closure type predicate, using the VM's closure representation.
use super::*;
use crate::prim::leaf::Kind;

defsubr! { name = "closurep", gnu = "data.c:521", effect = Reads,
fn closurep(cx, object: Value) {
    Ok(if cx.kind(object) == Kind::Closure { Value::TRUE } else { Value::NIL })
}}

#[cfg(test)]
mod tests {
    #[test]
    fn native_closure_predicate() {
        let mut vm = crate::Vm::new();
        let value = vm.eval_source_raw("(list (closurep (lambda () nil)) (closurep 'car) (closurep (symbol-function 'car)) (closurep '(closure nil nil)) (closurep nil))").unwrap();
        assert_eq!(vm.display(value).unwrap(), "(t nil nil nil nil)");
    }
}
