//! Romacs is built without xwidgets, as GNU is without `--with-xwidgets`:
//! the `xwidget-internal` feature is absent and none of xwidget.c's
//! primitives is defined, so xwidget.el's commands report the missing
//! support themselves. There is no embedded-browser backend to reach.
use romacs_vm::Vm;

#[test]
fn xwidget_primitives_answer_as_an_unsupported_build() {
    let mut vm = Vm::new();
    let form = "(prin1-to-string (list (featurep 'xwidget-internal) (fboundp 'make-xwidget) \
                (fboundp 'xwidgetp) (fboundp 'xwidget-live-p) (fboundp 'xwidget-webkit-goto-uri)))";
    let shown = vm.eval_source(form).expect("eval").display().expect("display");
    let shown = serde_json::from_str::<String>(&shown).unwrap_or(shown);
    assert_eq!(shown, "(nil nil nil nil nil)");
}
