use super::*;
use crate::prim::Fault;
defsubr! { name = "generate-new-buffer-name", gnu = "buffer.c:1252", effect = Reads,
fn generate_new_buffer_name(cx, name: Value; optional ignore: Value) {
    generate(cx, name, ignore)
}}

/// Fgenerate_new_buffer_name: NAME if no live buffer has it (or it is
/// IGNORE), else NAME<2>, NAME<3>...; a name starting with a space first
/// tries NAME-N with N from get_random () % 1000000.
fn generate(cx: &mut Leaf<'_>, name: Value, ignore: Value) -> Out {
    let view = cx.string_view(name)?;
    let ignored = if let Ok(text) = cx.symbol_name(ignore) {
        let chars = text.chars().count();
        (text.into_bytes(), chars)
    } else {
        let text = cx.string_view(ignore)?;
        (text.bytes, text.chars)
    };
    let allowed = |bytes: &[u8], chars: usize| bytes == ignored.0 && chars == ignored.1;
    let multibyte = view.is_multibyte();
    let exists = |cx: &Leaf<'_>, bytes: &[u8]| {
        let candidate = crate::lstring::StrView { bytes, chars: 0, multibyte };
        cx.buffer_name_exists(&crate::buffer::buffer_name_from_view(&candidate))
    };
    if (ignore != Value::NIL && allowed(&view.bytes, view.chars)) || !exists(cx, &view.bytes) {
        return Ok(name);
    }
    let mut base = view.bytes;
    let mut chars = view.chars;
    if base.first() == Some(&b' ') {
        let suffix = format!("-{}", cx.random_bits() % 1_000_000);
        chars += suffix.len();
        base.extend_from_slice(suffix.as_bytes());
        if !exists(cx, &base) {
            return Ok(cx.buffer_name_string(name, base, multibyte));
        }
    }
    for counter in 2u64.. {
        let suffix = format!("<{counter}>");
        let mut candidate = base.clone();
        candidate.extend_from_slice(suffix.as_bytes());
        if allowed(&candidate, chars + suffix.len()) || !exists(cx, &candidate) {
            return Ok(cx.buffer_name_string(name, candidate, multibyte));
        }
    }
    Err(Fault::Error("generate-new-buffer-name: exhausted names".into()))
}

fn name_of(cx: &mut Leaf<'_>, buffer: Value) -> Value {
    match cx.buffer_state(buffer).and_then(|s| s.name.clone()) {
        Some(name) => cx.buffer_name_value(&name),
        None => Value::NIL,
    }
}

// Saved registers: the name rename-buffer was asked for, then UNIQUE.
defsubr! { name = "rename-buffer", gnu = "buffer.c:1689", effect = Global,
fn rename_buffer(cx, newname: Value; optional unique: Value) -> Step {
    let buffer = cx.current_buffer()?;
    let old = name_of(cx, buffer);
    if !matches!(cx.kind(newname), Kind::String | Kind::Bytes) { return Err(Fault::Wrong("stringp", newname)); }
    let requested = newname;
    let text = cx.buffer_name_text(newname)?;
    if text.is_empty() { return Err(Fault::Error("Empty string is invalid as a buffer name".into())); }
    let existing = cx.lifecycle_get(newname)?;
    let name = if existing != Value::NIL {
        if unique == Value::NIL && existing == buffer { return Ok(Step::Done(old)); }
        if unique == Value::NIL {
            // error () formats through format-message.
            let template = cx.string("Buffer name `%s' is in use");
            let message = cx.format_message(template, &[newname])?;
            let data = cx.cons(message, Value::NIL);
            return Err(Fault::Signal(cx.intern("error"), data));
        }
        let generated = generate(cx, newname, old)?;
        cx.buffer_name_text(generated)?
    } else { text };
    cx.rename_buffer_model(buffer, &name)?;
    // An auto-saved buffer that visits no file renames its auto-save file.
    let visited = cx.intern("buffer-file-name");
    let visited = cx.variable_value(visited)?.unwrap_or(Value::NIL);
    let auto_save = cx.intern("buffer-auto-save-file-name");
    let auto_save = cx.variable_value(auto_save)?.unwrap_or(Value::NIL);
    if visited == Value::NIL && auto_save != Value::NIL {
        return Ok(Step::Call { f: cx.intern("rename-auto-save-file"), args: vec![], then: list_update, saved: vec![requested, unique] });
    }
    list_update(cx, Value::NIL, &[requested, unique])
}}

/// run_buffer_list_update_hook, then uniquify's advice (preloaded in GNU).
fn list_update(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let buffer = cx.current_buffer()?;
    if cx.buffer_state(buffer).is_some_and(|s| !s.inhibit_buffer_hooks) {
        let hook = cx.intern("buffer-list-update-hook");
        return Ok(Step::Call { f: cx.intern("run-hooks"), args: vec![hook], then: uniquify, saved: saved.to_vec() });
    }
    uniquify(cx, Value::NIL, saved)
}
fn uniquify(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let advice = cx.intern("uniquify--rename-buffer-advice");
    if cx.symbol_function(advice).is_ok_and(|f| f != Value::NIL) {
        return Ok(Step::Call { f: advice, args: saved.to_vec(), then: renamed, saved: vec![] });
    }
    renamed(cx, Value::NIL, &[])
}
fn renamed(cx: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next {
    let buffer = cx.current_buffer()?;
    Ok(Step::Done(name_of(cx, buffer)))
}

defsubr! { name = "bury-buffer-internal", gnu = "buffer.c:2251", effect = Global,
fn bury_buffer_internal(cx, buffer: Value) -> Step {
    if !matches!(cx.kind(buffer), Kind::Buffer | Kind::ByteBuffer) { return Err(Fault::Wrong("bufferp", buffer)); }
    cx.bury_buffer_model(buffer);
    if cx.buffer_state(buffer).is_some_and(|s| !s.inhibit_buffer_hooks) {
        let hook = cx.intern("buffer-list-update-hook");
        return Ok(Step::Call { f: cx.intern("run-hooks"), args: vec![hook], then: done_nil, saved: vec![] });
    }
    Ok(Step::Done(Value::NIL))
}}
fn done_nil(_: &mut Leaf<'_>, _: Value, _: &[Value]) -> Next { Ok(Step::Done(Value::NIL)) }

#[cfg(test)]
mod tests {
    use crate::{Value, Vm};

    #[test]
    fn generated_buffer_names_reuse_withdrawn_registry_slots() {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.eval_source("(setq gnbn-retained (get-buffer-create \"gnbn-retired\"))").unwrap();
        let slot = vm.slot("#:vm2-buffer:gnbn-retired");
        assert_eq!(vm.eval_source_raw("(equal (generate-new-buffer-name \"gnbn-retired\") \"gnbn-retired<2>\")").unwrap(), Value::TRUE);
        vm.slots[slot].value = Value::UNBOUND;
        assert_eq!(vm.eval_source_raw("(let ((name (copy-sequence \"gnbn-retired\"))) (eq name (generate-new-buffer-name name)))").unwrap(), Value::TRUE);
        vm.eval_source("(get-buffer-create \"gnbn-numbered\") (setq gnbn-retained (get-buffer-create \"gnbn-numbered<2>\")) (get-buffer-create \"gnbn-numbered<3>\")").unwrap();
        let slot = vm.slot("#:vm2-buffer:gnbn-numbered<2>");
        vm.slots[slot].value = Value::UNBOUND;
        assert_eq!(vm.eval_source_raw("(equal (generate-new-buffer-name \"gnbn-numbered\") \"gnbn-numbered<2>\")").unwrap(), Value::TRUE);
    }
}
