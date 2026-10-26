use super::*;
include!("charset_define.rs");

impl Leaf<'_> {
    fn charset_record(&mut self, name: Value) -> std::result::Result<usize, Fault> {
        self.vm.ensure_charsets();
        if self.kind(name) != Kind::Symbol {
            return Err(Fault::Wrong("charsetp", name));
        }
        self.vm
            .charsets
            .lookup(name)
            .ok_or(Fault::Wrong("charsetp", name))
    }
    pub fn is_charset(&mut self, object: Value) -> bool {
        self.charset_record(object).is_ok()
    }
    pub fn charset_plist(&mut self, name: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        Ok(self.vm.charsets.records[i].attrs[2])
    }
    pub fn set_charset_plist(&mut self, name: Value, plist: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        self.vm.charsets.records[i].attrs[2] = plist;
        Ok(plist)
    }
    /// charset.c Funify_charset. De-unifying a unified charset needs its
    /// decoder (DECODE_CHAR) and is refused until decoders exist.
    pub fn unify_charset(&mut self, name: Value, unify_map: Value, deunify: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        let id = self.vm.charsets.records[i].attrs[0].integer_value().unwrap_or(0) as usize;
        let record = &self.vm.charsets.records[i];
        let unified = record.unified;
        if if deunify == Value::NIL { unified && record.attrs[9] != Value::NIL } else { !unified } {
            return Ok(Value::NIL);
        }
        if deunify != Value::NIL {
            return Err(Fault::Error("VM2 unify-charset DEUNIFY of a unified charset is unsupported".into()));
        }
        self.vm.charsets.records[i].unified = false;
        let record = &self.vm.charsets.records[i];
        if record.offset.is_none_or(|offset| offset < 0x110000) {
            let symbol = self.symbol_name(name)?;
            return Err(Fault::Error(format!("Can\u{2019}t unify charset: {symbol}")));
        }
        if unify_map != Value::NIL {
            if !matches!(self.kind(unify_map), Kind::String | Kind::Bytes | Kind::Vector) {
                let message = self.string("Bad unify-map");
                let tail = self.cons(unify_map, Value::NIL);
                let data = self.cons(message, tail);
                return Err(Fault::Signal(self.intern("error"), data));
            }
            self.vm.charsets.records[i].attrs[8] = unify_map;
        }
        let (min, max) = (self.vm.charsets.records[i].min_char, self.vm.charsets.records[i].max_char);
        let ranges = &mut self.vm.charsets.unify_ranges;
        ranges.retain(|&(_, _, owner)| owner != id);
        ranges.push((min, max, id));
        self.vm.charsets.records[i].unified = true;
        Ok(Value::NIL)
    }
    /// charset.c Fset_charset_priority: move CHARSETS, in order, to the
    /// front of Vcharset_ordered_list and refilter the ISO/emacs-mule lists.
    pub fn set_charset_priority(&mut self, charsets: &[Value]) -> super::super::Out {
        // Fset_charset_priority works on a copy: a bad argument changes nothing.
        self.vm.ensure_charsets();
        let mut rest = self.vm.charsets.ordered.clone();
        let mut head = Vec::new();
        for &charset in charsets {
            let i = self.charset_record(charset)?;
            let id = self.vm.charsets.records[i].attrs[0].integer_value().unwrap_or(0) as usize;
            if let Some(at) = rest.iter().position(|&o| o == id) {
                rest.remove(at);
                head.push(id);
            }
        }
        let registry = &mut self.vm.charsets;
        registry.ordered = head.into_iter().chain(rest).collect();
        registry.ordered_tick += 1;
        let ordered = registry.ordered.clone();
        registry.iso_list = ordered.iter().copied().filter(|id| registry.iso_list.contains(id)).collect();
        registry.mule_list = ordered.iter().copied().filter(|id| registry.mule_list.contains(id)).collect();
        Ok(Value::NIL)
    }
    /// charset.c Fcharset_priority_list.
    pub fn charset_priority_list(&mut self, highestp: Value) -> super::super::Out {
        self.vm.ensure_charsets();
        let names: Vec<Value> = self.vm.charsets.ordered.iter()
            .map(|&id| self.vm.charsets.records[self.vm.charsets.ids[id]].attrs[1]).collect();
        if highestp != Value::NIL { return Ok(names.first().copied().unwrap_or(Value::NIL)); }
        Ok(names.into_iter().rev().fold(Value::NIL, |tail, name| self.cons(name, tail)))
    }
    pub fn charset_alias(&mut self, alias: Value, name: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        if let Some(binding) = self.vm.charsets.bindings.iter_mut().find(|(key, _)| *key == alias) {
            binding.1 = i;
        } else {
            self.vm.charsets.bindings.push((alias, i));
        }
        let slot = self.vm.slot("charset-list");
        self.vm.slots[slot].value = self.cons(alias, self.vm.slots[slot].value);
        Ok(Value::NIL)
    }
}
