use super::*;

impl Vm {
    pub(super) fn surface_primitive(
        &mut self,
        primitive: Primitive,
        start: usize,
        argc: usize,
    ) -> Result<Value> {
        if argc != 1 {
            return Err("wrong primitive arity".into());
        }
        let value = self.stack[start];
        let yes = match primitive {
            Primitive::Integerp => value.integer_value().is_some()
                || matches!(self.object_if(value), Some(data::Object::Bignum(_))),
            Primitive::Fixnump => value.integer_value().is_some(),
            Primitive::Bignump => matches!(self.object_if(value), Some(data::Object::Bignum(_))),
            Primitive::Consp => value.tag() == 3,
            Primitive::Symbolp => {
                value == Value::NIL
                    || value == Value::TRUE
                    || value.tag() == 7 && value != Value::UNBOUND
            }
            Primitive::CarSafe => {
                return if value.tag() == 3 {
                    Ok(self.cons_pair(value)?.0)
                } else {
                    Ok(Value::NIL)
                };
            }
            _ => unreachable!("scalar surface dispatch"),
        };
        Ok(if yes { Value::TRUE } else { Value::NIL })
    }
}
