//! ccl.c:2245 register-ccl-program with resolve_symbol_ccl_program: the
//! index a program name gets, re-registration in place, symbols resolved
//! through their id properties, and GNU's refusals. Pinned GNU 32.0.50.
use romacs_vm::Vm;

#[test]
fn register_ccl_program_numbers_programs_as_gnu() {
    let mut vm = Vm::new();
    let source = "(list (register-ccl-program 'rc-a [1 3 0 0]) (register-ccl-program 'rc-a nil) (register-ccl-program 'rc-b [1 3 0 sym])
      (progn (put 'sym2 'translation-table-id 7) (register-ccl-program 'rc-d [1 3 sym2 (sym2 . translation-table-id)]))
      (condition-case e (register-ccl-program 'rc-c [1]) (error e)) (condition-case e (register-ccl-program 'rc-c '(1)) (error e))
      (condition-case e (register-ccl-program 5 nil) (error e)) (get 'rc-b 'ccl-program-idx))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(0 0 1 2 (error \"Error in CCL program\") (wrong-type-argument vectorp (1)) (wrong-type-argument symbolp 5) 1)");
}
