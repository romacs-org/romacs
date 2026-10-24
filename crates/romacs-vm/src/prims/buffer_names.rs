use super::*;
use crate::prim::Fault;
defsubr! { name = "generate-new-buffer-name", gnu = "buffer.c:1252", effect = Reads,
fn generate_new_buffer_name(cx, name: Value; optional ignore: Value) {
    let view = cx.string_view(name)?;
    let ignored = if let Ok(text) = cx.symbol_name(ignore) {
        let chars = text.chars().count();
        (text.into_bytes(), chars)
    } else {
        let text = cx.string_view(ignore)?;
        (text.bytes, text.chars)
    };
    let allowed = |bytes: &[u8], chars: usize| bytes == ignored.0 && chars == ignored.1;
    let exists = |bytes: &[u8]| std::str::from_utf8(bytes).ok()
        .is_some_and(|text| cx.buffer_name_exists(text));
    if (ignore != Value::NIL && allowed(&view.bytes, view.chars)) || !exists(&view.bytes) {
        return Ok(name);
    }
    let multibyte = view.is_multibyte();
    let mut base = view.bytes;
    let mut chars = view.chars;
    if base.first() == Some(&b' ') {
        // VM2 has no shared Lisp PRNG yet: seeding/advancing GNU's random
        // state is not represented by this OS-backed entropy source.
        use std::io::Read;
        let mut random = [0; 8];
        std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| Fault::Error(format!("generate-new-buffer-name: {e}")))?;
        let suffix = format!("-{}", u64::from_ne_bytes(random) % 1_000_000);
        chars += suffix.len();
        base.extend_from_slice(suffix.as_bytes());
        if !exists(&base) {
            return Ok(cx.buffer_name_string(name, base, multibyte));
        }
    }
    for counter in 2u64.. {
        let suffix = format!("<{counter}>");
        let mut candidate = base.clone();
        candidate.extend_from_slice(suffix.as_bytes());
        if allowed(&candidate, chars + suffix.len()) || !exists(&candidate) {
            return Ok(cx.buffer_name_string(name, candidate, multibyte));
        }
    }
    Err(Fault::Error("generate-new-buffer-name: exhausted names".into()))
}}

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
