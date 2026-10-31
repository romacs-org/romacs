use super::*;
use crate::forwarded::Fwd;
use crate::prim::Out;
use crate::data::Object;

impl Leaf<'_> {
    fn local_symbol(&mut self, symbol: Value) -> std::result::Result<Option<usize>, Fault> {
        if symbol == Value::NIL || symbol == Value::TRUE { return Ok(None); }
        Ok(Some(self.vm.indirect_variable(self.symbol_slot(symbol)?)))
    }
    pub(crate) fn has_buffer_local(&mut self, symbol: Value, buffer: Value) -> std::result::Result<bool, Fault> {
        let Some(slot) = self.local_symbol(symbol)? else { return Ok(false) };
        // enable-multibyte-characters is always local (buffer.c:5300).
        if slot == self.vm.multibyte_slot { return Ok(true); }
        if let Some(fwd) = self.vm.forwarding(slot) {
            return Ok(matches!(fwd, Fwd::Buffer(i) if self.vm.per_buffer_local_p(buffer.index(0).unwrap() - 1, i)));
        }
        let Some(table) = self.vm.local_table(slot).map_err(Fault::Error)? else { return Ok(false) };
        Ok(self.vm.declaration_hash_get(table, buffer).map_err(Fault::Error)?.is_some())
    }
    pub(crate) fn kill_local(&mut self, symbol: Value) -> Out {
        let Some(slot) = self.local_symbol(symbol)? else { return Ok(symbol) };
        let buffer = self.decode_buffer(Value::NIL)?;
        // Fkill_local_variable reports makunbound with the current buffer, for
        // a localized variable only (a plain or forwarded one returns first).
        let localized = self.vm.forwarding(slot).is_none() && self.vm.local_table(slot).map_err(Fault::Error)?.is_some();
        if localized { self.notify(slot, Value::NIL, "makunbound", Some(buffer))?; }
        self.clear_local_slot(slot, buffer)?;
        Ok(symbol)
    }
    fn clear_local_slot(&mut self, slot: usize, buffer: Value) -> std::result::Result<(), Fault> {
        if let Some(fwd) = self.vm.forwarding(slot) {
            if let Fwd::Buffer(i) = fwd { self.vm.clear_forwarded_local(buffer.index(0).unwrap() - 1, i); }
        } else if let Some(table) = self.vm.local_table(slot).map_err(Fault::Error)? {
            self.vm.declaration_hash_remove(table, buffer).map_err(Fault::Error)?;
        }
        Ok(())
    }
    pub(crate) fn buffer_local_read(&mut self, symbol: Value, buffer: Value) -> Out {
        let slot = self.local_symbol(symbol)?;
        if buffer == Value::NIL { return Err(Fault::Wrong("bufferp", buffer)); }
        let buffer = self.decode_buffer(buffer)?;
        let Some(slot) = slot else { return Ok(symbol) };
        let value = self.vm.value_in_buffer(slot, buffer).map_err(Fault::Error)?;
        if value == Value::UNBOUND {
            Err(Fault::VoidVariable(symbol))
        } else { Ok(value) }
    }
    pub(crate) fn buffer_local_inventory(&mut self, buffer: Value) -> Out {
        // Inherent locals appear even before their first Lisp lookup.
        for name in ["major-mode", "mode-name", "buffer-file-name", "buffer-read-only", "buffer-file-format"] {
            self.intern(name);
        }
        // Snapshot slot IDs, not borrowed values, before allocating list cells.
        let slots = self.vm.local_candidates(buffer.index(0).unwrap() - 1);
        let mut entries = Vec::new();
        for slot in slots {
            if self.vm.local_binding_slots.contains(&slot) { continue; }
            if self.vm.indirect_variable(slot) != slot { continue; }
            let symbol = Value::reference(slot + 1, 7);
            if self.has_buffer_local(symbol, buffer)? {
                let value = match self.buffer_local_read(symbol, buffer) {
                    Ok(value) => Some(value),
                    Err(Fault::VoidVariable(_)) => None,
                    Err(fault) => return Err(fault),
                };
                entries.push(match value { Some(v) => self.cons(symbol, v), None => symbol });
            }
        }
        Ok(self.vm.list_value(&entries))
    }
    pub(crate) fn reset_buffer_locals(&mut self, kill_permanent: Value) -> Out {
        let buffer = self.decode_buffer(Value::NIL)?;
        let permanent = self.intern("permanent-local");
        let permanent_hook = self.intern("permanent-local-hook");
        let reset = self.intern("reset");
        for slot in self.vm.local_candidates(buffer.index(0).unwrap() - 1) {
            if self.vm.local_binding_slots.contains(&slot) || self.vm.indirect_variable(slot) != slot { continue; }
            let symbol = Value::reference(slot + 1, 7);
            if !self.has_buffer_local(symbol, buffer)? { continue; }
            let file_coding = self.vm.names.get("buffer-file-coding-system") == Some(&slot);
            if let Some(Fwd::Buffer(_)) = self.vm.forwarding(slot) {
                // buffer_permanent_local_flags (buffer.c:4811).
                if !(file_coding && kill_permanent == Value::NIL) { self.clear_local_slot(slot, buffer)?; }
                continue;
            }
            let property = self.vm.symbol_property(symbol, permanent).map_err(Fault::Error)?;
            let builtin_permanent = slot == self.vm.truncate_lines_slot || file_coding;
            if kill_permanent == Value::NIL && (property != Value::NIL || builtin_permanent) {
                if property == permanent_hook {
                    let value = match self.buffer_local_read(symbol, buffer) {
                        Ok(value) => value,
                        Err(Fault::VoidVariable(_)) => return Err(Fault::Wrong("arrayp", self.vm.forwarded_unbound_symbol())),
                        Err(fault) => return Err(fault),
                    };
                    if self.pair(value).is_some() {
                        let mut values = Vec::new();
                        let mut rest = value;
                        while let Some((item, tail)) = self.pair(rest) {
                            if item == Value::TRUE || (self.kind(item) == Kind::Symbol && self.vm.symbol_property(item, permanent_hook).map_err(Fault::Error)? != Value::NIL) {
                                values.push(item);
                            }
                            rest = tail;
                        }
                        let filtered = self.vm.list_value(&values);
                        self.vm.set_variable_value(slot, filtered).map_err(Fault::Error)?;
                    }
                }
            } else { self.clear_local_slot(slot, buffer)?; }
        }
        let index = buffer.index(0).unwrap() - 1;
        if kill_permanent == reset { self.vm.reset_forwarded_visit(index); }
        let case_tables = self.standard_case_roots()?;
        match &mut self.vm.objects[index] {
            Object::Buffer(b) => { b.local_keymap = Value::NIL; b.syntax_table = Value::NIL; b.category_table = Value::NIL; b.case_tables = case_tables; }
            Object::ByteBuffer(b) => { b.local_keymap = Value::NIL; b.syntax_table = Value::NIL; b.category_table = Value::NIL; b.case_tables = case_tables; }
            _ => unreachable!(),
        }
        self.vm.reset_forwarded_modes(index);
        Ok(Value::NIL)
    }
}
