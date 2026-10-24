use super::*;

defsubr! { name = "bufferp", gnu = "data.c:481", effect = Reads,
fn bufferp(cx, object: Value) { Ok(if cx.lifecycle_buffer_p(object) { Value::TRUE } else { Value::NIL }) }}
defsubr! { name = "buffer-list", gnu = "buffer.c:424", effect = Reads,
fn buffer_list(cx; optional frame: Value) {
    if frame != Value::NIL { return Err(Fault::Error("VM2 buffer-list FRAME is unsupported".into())); }
    Ok(cx.lifecycle_list())
}}

defsubr! { name = "get-buffer-create", gnu = "buffer.c:568", effect = Global,
fn get_buffer_create(cx, name: Value; optional inhibit: Value) -> Step {
    let (buffer, created) = cx.lifecycle_buffer(name, true, inhibit != Value::NIL)?;
    if !created || inhibit != Value::NIL { return Ok(Step::Done(buffer)); }
    list_updated(cx, buffer)
}}

fn list_updated(cx: &mut Leaf<'_>, result: Value) -> Next {
    let f = cx.intern("run-hooks");
    let hook = cx.intern("buffer-list-update-hook");
    Ok(Step::Call { f, args: vec![hook], then: return_saved, saved: vec![result] })
}
fn return_saved(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(saved[0]))
}

fn return_result(_: &mut Leaf<'_>, result: Value, _: &[Value]) -> Next {
    Ok(Step::Done(result))
}

#[cfg(test)]
mod tests {
    use crate::{Value, Vm};

    #[test]
    fn lifecycle_native_smoke() {
        let mut vm = Vm::new();
        for source in [
            "(let* ((events nil) (buffer-list-update-hook (list (lambda () (setq events (cons 'list events))))) (kill-buffer-hook (list (lambda () (setq events (cons 'kill events))))) (kill-buffer-query-functions (list (lambda () (setq events (cons 'query events)) t)))) (let ((b (get-buffer-create \"life-normal\"))) (and (kill-buffer b) (not (buffer-live-p b)) (equal events '(list kill query list)))))",
            "(let* ((events nil) (buffer-list-update-hook (list (lambda () (setq events t)))) (kill-buffer-hook (list (lambda () (setq events t)))) (kill-buffer-query-functions (list (lambda () nil)))) (let ((b (get-buffer-create (generate-new-buffer-name \"life-inhibited\") t))) (and (kill-buffer b) (not events))))",
            "(let ((kill-buffer-query-functions (list (lambda () nil)))) (let ((b (get-buffer-create \"life-veto\"))) (and (not (kill-buffer b)) (buffer-live-p b))))",
            "(let ((old (current-buffer)) (b (get-buffer-create \"life-error\")) (kill-buffer-query-functions (list (lambda () (error \"query\"))))) (condition-case nil (kill-buffer b) (error nil)) (and (eq old (current-buffer)) (buffer-live-p b)))",
            "(let ((b (get-buffer-create \"life-dead\"))) (with-current-buffer b (kill-buffer b)) (and (not (buffer-live-p b)) (buffer-live-p (current-buffer)) (not (kill-buffer b))))",
        ] {
            assert_eq!(vm.eval_source_raw(source).unwrap(), Value::TRUE, "{source}");
        }
    }
}

defsubr! { name = "kill-buffer", gnu = "buffer.c:1902", effect = Global,
fn kill_buffer(cx; optional arg: Value) -> Step {
    let buffer = if arg == Value::NIL { cx.decode_buffer(arg)? }
        else { cx.lifecycle_buffer(arg, false, false)?.0 };
    if !cx.lifecycle_killable(buffer) { return Ok(Step::Done(Value::NIL)); }
    let source = cx.string(include_str!("buffer_lifecycle.el"));
    let parsed = cx.read_from_string(source, Value::NIL, Value::NIL)?;
    let form = cx.pair(parsed).expect("reader result").0;
    let f = cx.intern("eval");
    let inhibit = if cx.buffer_state(buffer).unwrap().inhibit_buffer_hooks { Value::TRUE } else { Value::NIL };
    Ok(Step::Call { f, args: vec![form, Value::TRUE], then: kill_helper, saved: vec![buffer, inhibit] })
}}
fn kill_helper(_: &mut Leaf<'_>, helper: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: helper, args: saved.to_vec(), then: return_result, saved: vec![] })
}

defsubr! { name = "romacs--kill-buffer-finish", gnu = "buffer.c:2003", effect = Global,
fn kill_buffer_finish(cx, buffer: Value) -> Step {
    let buffer = cx.decode_buffer(buffer)?;
    let state = cx.buffer_state(buffer).unwrap();
    if !state.live { return Ok(Step::Done(Value::TRUE)); }
    let inhibited = state.inhibit_buffer_hooks;
    cx.kill_buffer(buffer)?;
    if inhibited { Ok(Step::Done(Value::TRUE)) }
    else { list_updated(cx, Value::TRUE) }
}}
