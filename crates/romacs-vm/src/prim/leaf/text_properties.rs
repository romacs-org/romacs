use super::*;

impl Leaf<'_> {
    pub(crate) fn add_text_properties(
        &mut self,
        start: Value,
        end: Value,
        properties: Value,
        object: Value,
    ) -> super::super::Out {
        self.vm
            .add_text_properties_native(start, end, properties, object)
    }
    pub(crate) fn next_property_change(
        &mut self,
        position: Value,
        object: Value,
        limit: Value,
    ) -> super::super::Out {
        self.vm.next_property_change_values(position, object, limit)
    }
    pub(crate) fn next_single_char_property_change(
        &mut self,
        position: Value,
        property: Value,
        object: Value,
        limit: Value,
    ) -> super::super::Out {
        self.vm
            .next_single_char_property_change_values(position, property, object, limit)
    }
    pub(crate) fn text_property_not_all(&mut self, start: Value, end: Value, property: Value, value: Value, object: Value) -> super::super::Out {
        self.vm.text_property_not_all_values(start, end, property, value, object)
    }
    pub(crate) fn set_text_properties(&mut self, start: Value, end: Value, properties: Value, object: Value) -> super::super::Out {
        self.vm.set_text_properties_native(start, end, properties, object)
    }
    pub(crate) fn remove_text_properties(&mut self, start: Value, end: Value, names: &[Value], object: Value) -> super::super::Out {
        self.vm.remove_text_properties_native(start, end, names, object)
    }
}

impl Leaf<'_> {
    pub(crate) fn get_pos_property(&mut self, position: Value, property: Value, object: Value) -> super::super::Out {
        self.vm.get_pos_property_values(position, property, object)
    }
}
