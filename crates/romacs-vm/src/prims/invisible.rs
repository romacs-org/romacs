//! GNU xdisp.c invisible_prop and Finvisible_p.
use super::*;

defsubr! { name = "invisible-p", gnu = "xdisp.c:30344", effect = Reads,
fn invisible_p(cx, pos: Value) {
    let property = cx.invisible_property(pos)?;
    Ok(match cx.invisible_kind(property)? {
        0 => Value::NIL,
        1 => Value::TRUE,
        _ => Value::integer(2).unwrap(),
    })
}}

#[cfg(test)]
mod tests {
    #[test]
    fn invisible_marker_uses_its_validated_numeric_position() {
        let mut vm = crate::Vm::new();
        let value = vm.eval_source_raw("(with-temp-buffer (insert \"a\") (put-text-property 1 2 'invisible 'hide) (let ((buffer-invisibility-spec t)) (list (invisible-p 1) (condition-case e (invisible-p (copy-marker 1)) (error e)))))").unwrap();
        assert_eq!(vm.display(value).unwrap(), "(t t)");
    }

    #[test]
    fn invisible_gnu_oracle() {
        let cases = [
            "(list buffer-invisibility-spec (local-variable-p 'buffer-invisibility-spec) (invisible-p nil) (invisible-p 'hide) (invisible-p -1) (invisible-p 1.0))",
            "(let ((buffer-invisibility-spec '(hide (dots . t) (plain)))) (mapcar #'invisible-p '(hide dots plain other (other dots hide) (hide dots) (other . dots) nil)))",
            "(let ((buffer-invisibility-spec '(nil))) (list (invisible-p nil) (invisible-p '(nil))))",
            "(let* ((p (list 'a 'b)) (buffer-invisibility-spec (list (cons p t) 'a))) (invisible-p p))",
            "(with-temp-buffer (insert \"aαbc\") (put-text-property 2 4 'invisible 'hide) (let ((buffer-invisibility-spec '((hide . t)))) (list (invisible-p 1) (invisible-p 2) (invisible-p 4) (invisible-p 5) (invisible-p (copy-marker 3)))))",
            "(with-temp-buffer (insert \"abcdef\") (put-text-property 1 7 'invisible 'hide) (let ((o (make-overlay 2 5)) (buffer-invisibility-spec '(hide (dots . t)))) (overlay-put o 'invisible 'dots) (list (invisible-p 1) (invisible-p 2) (invisible-p 5) (progn (overlay-put o 'invisible 'other) (invisible-p 2)) (progn (overlay-put o 'invisible nil) (invisible-p 2)))))",
            "(with-temp-buffer (insert \"abcdef\") (let ((a (make-overlay 1 6)) (b (make-overlay 2 4)) (buffer-invisibility-spec '(hide (dots . t)))) (overlay-put a 'invisible 'hide) (overlay-put b 'invisible 'dots) (list (invisible-p 2) (progn (overlay-put a 'priority 10) (invisible-p 2)))))",
            "(with-temp-buffer (insert \"abc\") (put-text-property 1 4 'invisible t) (narrow-to-region 2 3) (list (condition-case e (invisible-p 1) (error e)) (invisible-p 3) (condition-case e (invisible-p 4) (error e))))",
            "(with-temp-buffer (list (condition-case e (invisible-p 0) (error e)) (condition-case e (invisible-p 9) (error e)) (condition-case e (invisible-p (make-marker)) (error e))))",
            "(with-temp-buffer (setq buffer-invisibility-spec nil) (list (invisible-p 'hide) (with-temp-buffer (list buffer-invisibility-spec (invisible-p 'hide))) buffer-invisibility-spec))",
        ];
        let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("source romacs-keymap-env.sh");
        for source in cases {
            let output = std::process::Command::new(&oracle)
                .args(["--batch", "-Q", "--eval", &format!("(prin1 {source})")])
                .output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let expected = String::from_utf8(output.stdout).unwrap();
            let mut vm = crate::Vm::new();
            vm.install_lisp_prelude_raw().unwrap();
            let actual = vm.eval_source_raw(source).unwrap_or_else(|e| panic!("{source}: {e}"));
            assert_eq!(vm.display(actual).unwrap(), expected, "{source}");
        }
    }
}
