use super::*;

impl Leaf<'_> {
    pub(crate) fn value_order_record(&self, value: Value) -> Vec<Value> {
        let Some(data::Object::Record(values)) = self.vm.object_if(value) else {
            unreachable!("value ordering checks the record kind")
        };
        values.clone()
    }

    pub(crate) fn value_order_marker_buffer(&self, value: Value) -> Value {
        let Some(data::Object::Marker(marker)) = self.vm.object_if(value) else {
            unreachable!("value ordering checks the marker kind")
        };
        marker.buffer
    }

    pub(crate) fn value_order_symbol_name(
        &self,
        value: Value,
    ) -> std::result::Result<crate::lstring::OwnedStr, Fault> {
        if let Ok(slot) = self.symbol_slot(value) {
            if let Some(&name) = self.vm.obarray_symbol_names.get(&slot) {
                return self.string_view(name);
            }
        }
        if let Some(&name) = self.vm.permanent_symbol_names.get(&value.0) {
            return self.string_view(name);
        }
        // No name made yet: the one its key stands for, not the key.
        let key = self.symbol_key(value).ok_or(Fault::Wrong("symbolp", value))?;
        Ok(self.read_symbol_name(value, key, |name| name.to_owned()))
    }
}
