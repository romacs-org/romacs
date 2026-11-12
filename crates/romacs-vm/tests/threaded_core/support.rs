// The threaded engine's audited core and hot handlers, compiled as they are
// in the crate (`src/threaded_audited.rs`, `src/threaded_hot.rs`) over a
// stand-in engine with the same shape: what the core's types allow a body
// does not depend on the engine behind them. Each `fail/` program includes
// this and adds one body, `misuse::body`, under `forbid(unsafe_code)` as the
// hot handlers are.
#[path = "../../src/threaded_audited.rs"]
mod audited;
#[path = "../../src/threaded_hot.rs"]
mod hot;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Value(u64);
impl Value {
    const NIL: Value = Value(2);
    const TRUE: Value = Value(10);
}
type Handler = unsafe fn(Eng<'_, '_>, *mut Value, *const Inst) -> Value;
#[derive(Clone, Copy)]
struct Inst {
    kernel: Handler,
    a: usize,
    b: usize,
    a8: u8,
    b8: u8,
    d8: u8,
    branch: i32,
    value: Value,
}
struct Vm {
    stack: Vec<Value>,
    conses: Vec<(Value, Value)>,
}
struct Engine<'a> {
    vm: &'a mut Vm,
    budget: i64,
}
struct Eng<'x, 'a>(&'x mut Engine<'a>);
impl<'a> std::ops::Deref for Eng<'_, 'a> {
    type Target = Engine<'a>;
    fn deref(&self) -> &Engine<'a> {
        self.0
    }
}
impl std::ops::DerefMut for Eng<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}
// The engine's two raw entries the core calls, as `Handler`s; never run here.
fn stub(_: Eng<'_, '_>, _: *mut Value, _: *const Inst) -> Value {
    Value::NIL
}
#[allow(non_upper_case_globals, reason = "named as the engine's functions are")]
const next: Handler = stub;
#[allow(non_upper_case_globals, reason = "named as the engine's functions are")]
const poll_resume: Handler = stub;
fn fail(_: &mut Engine<'_>, _: &str) -> Value {
    Value::NIL
}

fn main() {
    use audited::*;
    let _: [Handler; 20] = [
        t_mov, t_mov_wide, t_constant, t_constant_wide, t_unless, t_unless_wide, t_jump, t_car, t_cdr,
        t_add, t_sub, t_inc, t_dec, t_less, t_greater, t_equal, t_null, t_consp, t_eq, terminator,
    ];
    let _: fn(Eng<'_, '_>, Regs, Pc) -> Value = misuse::body;
}
