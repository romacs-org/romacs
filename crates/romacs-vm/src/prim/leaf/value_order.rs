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
        let name = self.symbol_name(value)?;
        Ok(crate::lstring::OwnedStr {
            chars: name.chars().count(),
            bytes: name.into_bytes(),
            multibyte: true,
        })
    }
}
