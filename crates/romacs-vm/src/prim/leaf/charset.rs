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
    pub fn charset_plist(&mut self, name: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        Ok(self.vm.charsets.records[i].attrs[2])
    }
    pub fn set_charset_plist(&mut self, name: Value, plist: Value) -> super::super::Out {
        let i = self.charset_record(name)?;
        self.vm.charsets.records[i].attrs[2] = plist;
        Ok(plist)
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
