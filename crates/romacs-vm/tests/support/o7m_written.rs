//! Shared assertions for the O7m C4 written-bit evidence.

use romacs_vm::Vm;

pub fn observed(vm: &mut Vm, form: &str) -> Result<String, String> {
    vm.eval_source(form).and_then(|value| value.display())
}

pub fn written(vm: &mut Vm, symbol: &str) -> Result<String, String> {
    observed(vm, &format!("(romacs--written-p '{symbol})"))
}

#[track_caller]
pub fn assert_bit(answer: &Result<String, String>, expected: &str, stage: &str) {
    assert_eq!(answer.as_deref(), Ok(expected), "written bit {stage}");
}

#[track_caller]
pub fn check_write(vm: &mut Vm, symbol: &str, form: &str, probe: &str, expected: &str) {
    let before = written(vm, symbol);
    vm.eval_source(form)
        .unwrap_or_else(|err| panic!("{form}: {err}"));
    assert_eq!(observed(vm, probe).as_deref(), Ok(expected), "{probe}");
    let after = written(vm, symbol);
    assert_eq!(
        (before.as_deref(), after.as_deref()),
        (Ok("nil"), Ok("t")),
        "{symbol} must become written after {form}"
    );
}
