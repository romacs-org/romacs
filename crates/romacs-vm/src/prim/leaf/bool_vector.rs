//! Non-collecting bool-vector access; no borrowed heap references escape.
use super::*;
impl Leaf<'_> {
    pub(crate) fn bool_vector(&self, value: Value) -> std::result::Result<data::BoolVector, Fault> {
        match self.vm.object_if(value) {
            Some(data::Object::BoolVector(bits)) => Ok((**bits).clone()),
            _ => Err(Fault::Wrong("bool-vector-p", value)),
        }
    }
    pub(crate) fn bool_vector_from(&mut self, bits: data::BoolVector) -> Value {
        self.vm.allocate_object(data::Object::BoolVector(Box::new(bits)))
    }
    pub(crate) fn bool_vector_store(&mut self, value: Value, bits: data::BoolVector) -> std::result::Result<(), Fault> {
        if !matches!(self.vm.object_if(value), Some(data::Object::BoolVector(_))) {
            return Err(Fault::Wrong("bool-vector-p", value));
        }
        self.vm.objects[value.payload() as usize - 1] = data::Object::BoolVector(Box::new(bits));
        Ok(())
    }
}
