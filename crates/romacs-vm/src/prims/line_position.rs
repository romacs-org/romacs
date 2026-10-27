//! Point-preserving logical line boundary queries.
use super::*;

defsubr! { name = "line-beginning-position", gnu = "editfns.c:774", effect = Reads,
fn line_beginning_position(cx; optional n: Value) {
    cx.line_boundary_position(n, false)
}}

defsubr! { name = "line-end-position", gnu = "editfns.c:823", effect = Reads,
fn line_end_position(cx; optional n: Value) {
    cx.line_boundary_position(n, true)
}}

defsubr! { name = "line-number-at-pos", gnu = "fns.c:6688", effect = Reads,
fn line_number_at_pos(cx; optional position: Value, absolute: Value) {
    cx.line_number_at_pos(position, absolute)
}}

#[cfg(test)]
mod tests {
    #[test]
    fn boundaries_preserve_point_and_restriction() {
        let mut vm = crate::Vm::new();
        let value = vm.eval_source_raw("(with-temp-buffer (insert \"aα\nbb\ncc\") (goto-char 5) (list (line-beginning-position) (line-end-position) (line-beginning-position 0) (line-end-position 2) (line-end-position 999) (point) (progn (narrow-to-region 2 8) (list (line-beginning-position -999) (line-end-position 999) (point)))))").unwrap();
        assert_eq!(vm.display(value).unwrap(), "(4 6 1 9 9 5 (2 8 5))");
    }
}
