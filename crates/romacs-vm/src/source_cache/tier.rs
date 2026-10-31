//! Bounded direct-apply tier: at most eight syntax nodes, atomic arguments,
//! and a currently installed primitive. Everything else uses guarded cached IR.
use super::*;
pub const DIRECT_NODE_LIMIT: usize = 8;
impl Vm {
    pub(crate) fn direct_form(&mut self, form: &Form) -> Option<Result<Value>> {
        let Form::List(a) = form else { return None };
        if a.is_empty() || a.len() + 1 > DIRECT_NODE_LIMIT {
            return None;
        }
        let Ok(name) = a[0].sym() else { return None };
        let &slot = self.names.get(name)?;
        // A macro (or alias of one) expands first, as in the compiler. Then a
        // byte-op call is the primitive itself (sealed.rs); any other call is
        // what the cell holds.
        if self.call_is_macro(slot) {
            return None;
        }
        let target = sealed::target(name, a.len() - 1).unwrap_or(self.slots[slot].function);
        if target.tag() != 6 {
            return None;
        }
        let p = prim::legacy(target)?;
        if !matches!(
            p,
            Primitive::Add
                | Primitive::Sub
                | Primitive::Mul
                | Primitive::Div
                | Primitive::Abs
                | Primitive::Inc
                | Primitive::Dec
                | Primitive::Less
                | Primitive::Equal
                | Primitive::Cons
        ) {
            return None;
        }
        if a[1..]
            .iter()
            .any(|f| !matches!(f, Form::Int(_) | Form::Float(_) | Form::Sym(_)))
        {
            return None;
        }
        let result = (|| {
            let mut args = vec![];
            for f in &a[1..] {
                args.push(match f {
                    Form::Sym(s) if !matches!(s.as_str(), "nil" | "t") => {
                        let slot = self.slot(s);
                        let value = self.variable_value(slot)?;
                        if value == Value::UNBOUND {
                            return Err("unbound variable".into());
                        }
                        value
                    }
                    _ => self.literal(f)?,
                });
            }
            self.stack.extend_from_slice(&args);
            let result = self.primitive(p, 0, args.len());
            self.stack.clear();
            result
        })();
        // This tier has no escape channel. Fall back before returning a fault
        // whose symbol data must be carried by the ordinary evaluator.
        if matches!(&result, Err(error) if error == "unbound variable") {
            return None;
        }
        self.source_cache.stats.direct += 1;
        Some(result)
    }
}
