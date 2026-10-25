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
}
