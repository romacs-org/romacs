//! Live GNU 45b0f7699df preserves the original alias, not its void terminal.
use romacs_vm::Vm;

#[test]
fn void_terminal_reports_original_alias_for_each_call_path() {
    for call in ["(alias-a)", "(funcall 'alias-a)", "(apply 'alias-a nil)"] {
        let mut vm = Vm::new();
        vm.eval_source("(progn (fset 'alias-a 'alias-b) (fset 'alias-b 'alias-missing))").unwrap();
        let source = format!("(condition-case e {call} (error e))");
        let observed = vm.eval_source(&source).and_then(|v| v.display());
        assert_eq!(observed.as_deref(), Ok("(void-function alias-a)"),
            "GNU preserves original alias: {call}");
    }
}
