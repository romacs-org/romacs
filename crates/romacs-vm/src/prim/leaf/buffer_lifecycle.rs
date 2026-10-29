use super::*;
use crate::prim::Out;
use crate::data::Object;

impl Vm {
    pub(crate) fn live_buffer_value(&self, value: Value) -> bool {
        matches!(self.object_if(value), Some(Object::Buffer(b)) if b.state.live)
            || matches!(self.object_if(value), Some(Object::ByteBuffer(b)) if b.state.live)
    }
}

impl Leaf<'_> {
    pub fn minibuffer_predicate(&mut self, buffer: Value, live: Value) -> Out {
        let buffer = if buffer == Value::NIL { self.current_buffer()? }
            else if self.lifecycle_buffer_p(buffer) { buffer }
            else if self.string_view(buffer).is_ok() { self.lifecycle_get(buffer)? }
            else { return Err(Fault::Wrong("bufferp", buffer)); };
        self.vm.ensure_windows().map_err(Fault::Error)?;
        let mini = self.vm.minibuffer_window_buffer() == Some(buffer);
        let active = self.vm.names.get("romacs--minibuf-level")
            .is_some_and(|&slot| self.vm.slots[slot].value.integer_value().is_some_and(|n| n > 0));
        Ok(if mini && (live == Value::NIL || active) { Value::TRUE } else { Value::NIL })
    }

    pub fn lifecycle_buffer_p(&self, value: Value) -> bool {
        matches!(
            self.vm.object_if(value),
            Some(Object::Buffer(_) | Object::ByteBuffer(_))
        )
    }
    pub fn lifecycle_live_p(&self, value: Value) -> bool {
        self.vm.live_buffer_value(value)
    }
    pub fn lifecycle_get(&mut self, value: Value) -> Out {
        if self.lifecycle_buffer_p(value) {
            return Ok(value);
        }
        let name = self
            .string_text(value)
            .map_err(|_| Fault::Wrong("stringp", value))?;
        Ok(self
            .vm
            .names
            .get(&format!("#:vm2-buffer:{name}"))
            .map(|slot| self.vm.slots[*slot].value)
            .filter(|v| *v != Value::UNBOUND)
            .unwrap_or(Value::NIL))
    }
    /// buffer-list: the live buffers in the buffer list's order.
    pub fn lifecycle_list(&mut self) -> Value {
        let order: Vec<Value> = self.vm.buffer_order().into_iter()
            .filter(|&b| self.vm.buffer_state(b).is_ok_and(|s| s.live)).collect();
        order.into_iter().rev().fold(Value::NIL, |tail, b| self.cons(b, tail))
    }
    pub(crate) fn rename_buffer_model(&mut self, buffer: Value, name: &str) -> std::result::Result<(), Fault> {
        self.vm.rename_buffer_model(buffer, name).map_err(Fault::Error)
    }
    pub(crate) fn bury_buffer_model(&mut self, buffer: Value) { self.vm.bury_buffer_model(buffer); }
    /// get_random: 62 random bits from the VM's generator.
    pub(crate) fn random_bits(&mut self) -> u64 {
        self.vm.fence_random();
        self.vm.random.bits()
    }
    pub fn lifecycle_killable(&self, buffer: Value) -> bool {
        self.lifecycle_live_p(buffer)
            && !self
                .vm
                .objects
                .iter()
                .any(|object| matches!(object, Object::Window(w) if w.mini && w.buffer == buffer))
    }
    pub fn lifecycle_finish(&mut self, buffer: Value) -> Out {
        if !self.lifecycle_killable(buffer) {
            return Ok(Value::NIL);
        }
        self.vm.kill_buffer_model(buffer).map_err(Fault::Error)?;
        Ok(Value::TRUE)
    }
}
