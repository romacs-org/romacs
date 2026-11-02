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
        let name = self.buffer_name_text(value)?;
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
    /// Fmake_indirect_buffer (buffer.c:807) up to its Lisp: NAME's
    /// checks, BASE-BUFFER's, the buffer and, with CLONE, BASE's state.
    /// Returns the buffer and the base whose text it shares.
    pub(crate) fn indirect_buffer(&mut self, base: Value, name: Value, clone: bool, inhibit: bool) -> std::result::Result<(Value, Value), Fault> {
        let text = self.string_text(name).map_err(|_| Fault::Wrong("stringp", name))?;
        if self.vm.named_buffer(&text).is_some() || (text == "*scratch*" && self.vm.scratch_unmade()) {
            return Err(Fault::Error(format!("Buffer name ‘{text}’ is in use")));
        }
        let found = self.lifecycle_get(base)?;
        if found == Value::NIL {
            return Err(Fault::Error(format!("No such buffer: ‘{}’", self.string_text(base)?)));
        }
        if !self.lifecycle_live_p(found) {
            return Err(Fault::Error("Base buffer has been killed".into()));
        }
        if text.is_empty() {
            return Err(Fault::Error("Empty string for buffer name is not allowed".into()));
        }
        let buffer = self.vm.make_indirect_buffer(found, &text, inhibit).map_err(Fault::Error)?;
        let base = self.buffer_base(buffer);
        if clone {
            self.vm.clone_indirect_state(buffer, base).map_err(Fault::Error)?;
        }
        Ok((buffer, base))
    }
    /// Fbuffer_base_buffer: BUFFER's base buffer, or nil.
    pub(crate) fn buffer_base(&self, buffer: Value) -> Value {
        match self.vm.object_if(buffer) {
            Some(Object::Buffer(b)) => b.base,
            _ => Value::NIL,
        }
    }
    /// BUFFER's live indirect buffers in `buffer-list' order: Fkill_buffer's
    /// FOR_EACH_LIVE_BUFFER walk for those whose base is BUFFER, from the
    /// base's own record, so a buffer with none (most) costs no list walk.
    pub(crate) fn indirect_buffers(&mut self, buffer: Value) -> Value {
        if !matches!(self.vm.object_if(buffer), Some(Object::Buffer(b)) if !b.indirect.is_empty()) {
            return Value::NIL;
        }
        let order: Vec<Value> = self.vm.buffer_order().into_iter()
            .filter(|&b| self.buffer_base(b) == buffer && self.vm.buffer_state(b).is_ok_and(|s| s.live)).collect();
        order.into_iter().rev().fold(Value::NIL, |tail, b| self.cons(b, tail))
    }
    pub fn lifecycle_killable(&self, buffer: Value) -> bool {
        self.lifecycle_live_p(buffer) && self.vm.minibuffer_window_buffer() != Some(buffer)
    }
    pub fn lifecycle_finish(&mut self, buffer: Value) -> Out {
        if !self.lifecycle_killable(buffer) {
            return Ok(Value::NIL);
        }
        self.vm.kill_buffer_model(buffer).map_err(Fault::Error)?;
        Ok(Value::TRUE)
    }
}
