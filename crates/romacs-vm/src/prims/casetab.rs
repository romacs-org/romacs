use super::*;

defsubr! { name = "case-table-p", gnu = "casetab.c:35", effect = Pure,
fn case_table_p(cx, value: Value) { Ok(if cx.case_table_p(value) { Value::TRUE } else { Value::NIL }) }}

defsubr! { name = "current-case-table", gnu = "casetab.c:65", effect = Reads,
fn current_case_table(cx) { cx.current_case_mapping(true) }}

defsubr! { name = "set-case-table", gnu = "casetab.c:82", effect = Global,
fn set_case_table(cx, table: Value) { cx.install_case_table(table, false) }}

defsubr! { name = "set-standard-case-table", gnu = "casetab.c:104", effect = Global,
fn set_standard_case_table(cx, table: Value) { cx.install_case_table(table, true) }}

#[cfg(test)]
mod tests {
    use crate::Vm;
    #[test]
    fn casetab_gnu_installation_and_independent_roots() {
        let mut vm = Vm::new();
        let source = "(let ((d (make-char-table 'case-table))) (aset d 65 97) (aset d 97 97) (set-case-table d) (list (case-table-p d) (eq d (current-case-table)) (aref (char-table-extra-slot d 0) 97) (aref (char-table-extra-slot d 1) 65) (aref (char-table-extra-slot d 2) 65)))";
        let result = vm.eval_source_raw(source).unwrap();
        assert_eq!(vm.display(result).unwrap(), "(t t 65 97 97)");
    }
    #[test]
    fn casetab_standard_replacement_only_affects_new_buffers() {
        let mut vm = Vm::new();
        let result = vm.eval_source_raw("(let ((old (current-case-table)) (b (get-buffer-create \"before\")) (d (make-char-table 'case-table))) (aset d 65 97) (aset d 97 97) (set-standard-case-table d) (list (eq old (current-case-table)) (with-current-buffer b (eq old (current-case-table))) (with-temp-buffer (eq d (current-case-table))) (eq d (standard-case-table))))").unwrap();
        assert_eq!(vm.display(result).unwrap(), "(t t t t)");
    }
    #[test]
    fn casetab_many_to_one_ranges_follow_gnu_shuffle_order() {
        let mut vm = Vm::new();
        let result = vm.eval_source_raw("(let ((d (make-char-table 'case-table))) (set-char-table-range d '(65 . 66) 97) (aset d 97 97) (set-case-table d) (list (list (aref (char-table-extra-slot d 0) 65) (aref (char-table-extra-slot d 0) 66) (aref (char-table-extra-slot d 0) 97)) (list (aref (char-table-extra-slot d 1) 65) (aref (char-table-extra-slot d 1) 66) (aref (char-table-extra-slot d 1) 97)) (list (aref (char-table-extra-slot d 2) 65) (aref (char-table-extra-slot d 2) 66) (aref (char-table-extra-slot d 2) 97))))").unwrap();
        assert_eq!(vm.display(result).unwrap(), "((97 65 66) (97 97 97) (97 65 66))");
    }
    #[test]
    fn casetab_predicate_rejects_malformed_extras() {
        let mut vm = Vm::new();
        let result = vm.eval_source_raw("(let ((d (make-char-table 'case-table))) (set-char-table-extra-slot d 2 (make-char-table nil)) (list (case-table-p d) (condition-case e (set-case-table d) (wrong-type-argument (car e)))))").unwrap();
        assert_eq!(vm.display(result).unwrap(), "(nil wrong-type-argument)");
    }
}

defsubr! { name = "standard-case-table", gnu = "casetab.c:72", effect = Reads,
fn standard_case_table(cx) {
    cx.standard_case_table()
}}
