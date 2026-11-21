//! Non-collecting bool-vector access; no borrowed heap references escape.
use super::*;
impl Leaf<'_> {
    /// `bool-vector-p`: where `bool_vector` answers, with nothing copied and
    /// no error built.
    pub(crate) fn is_bool_vector(&self, value: Value) -> bool {
        matches!(self.vm.object_if(value), Some(data::Object::BoolVector(_)))
    }
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
        *self.object_mut(value)? = data::Object::BoolVector(Box::new(bits));
        Ok(())
    }
}
