//! Renderer effects (rlisp/romacs-effects.el): the properties the renderer
//! defines, published to Lisp and set from it. The evaluator keeps the
//! requested snapshot; the open display receives each change.
use super::*;
use crate::prim::Out;
use romacs_display_protocol::{EffectOperation, EffectValue, PropertyKind, VisualConfig, schema_for};

impl Leaf<'_> {
    /// `romacs--effect-set': EFFECT's PROPERTIES, keyword and value pairs,
    /// over its current ones, or with RESET over its defaults
    /// (`romacs--effect-replace'). The whole change validates before any of
    /// it applies.
    pub(crate) fn effect_set(&mut self, effect: Value, properties: &[Value], reset: bool) -> Out {
        let effect = self.symbol_name(effect)?;
        if properties.len() % 2 != 0 {
            return Err(Fault::Error(format!("romacs--effect-set: `{effect}' needs a value for each property")));
        }
        let properties = properties
            .chunks(2)
            .map(|pair| Ok((self.symbol_name(pair[0])?.trim_start_matches(':').to_owned(), self.effect_value(pair[1])?)))
            .collect::<std::result::Result<Vec<_>, Fault>>()?;
        let operations: Vec<EffectOperation> = reset.then(|| EffectOperation::reset(effect.as_str())).into_iter()
            .chain([EffectOperation::set(effect.as_str(), properties)])
            .collect();
        let next = self.vm.visual
            .apply_effects(&operations)
            .map_err(|error| Fault::Error(error.to_string()))?;
        self.vm.set_visual_config(next);
        Ok(Value::NIL)
    }

    /// `romacs-effect-get': EFFECT's requested properties as a plist.
    pub(crate) fn effect_get(&mut self, effect: Value) -> Out {
        let effect = self.symbol_name(effect)?;
        let values = self.vm.visual.effect_values(&effect).map_err(|error| Fault::Error(error.to_string()))?;
        let mut items = Vec::with_capacity(values.len() * 2);
        for (property, value) in values {
            items.push(self.intern(&format!(":{property}")));
            items.push(self.lisp_effect_value(&value)?);
        }
        Ok(self.list(&items))
    }

    /// `romacs-effect-names': every effect, or with SCOPE `shader' the
    /// decorative ones.
    pub(crate) fn effect_names(&mut self, scope: Value) -> Out {
        let names = match self.symbol_name(scope)?.as_str() {
            "nil" => self.vm.visual.effect_names(),
            "shader" => self.vm.visual.effects.effect_names(),
            other => return Err(Fault::Error(format!("romacs-effect-names: SCOPE is nil or shader, not `{other}'"))),
        };
        let names: Vec<Value> = names.iter().map(|name| self.intern(name)).collect();
        Ok(self.list(&names))
    }

    /// `romacs-effect-schema': EFFECT's properties, each (KEYWORD KIND
    /// . NAMES), NAMES the symbols a `symbol' property accepts.
    pub(crate) fn effect_schema(&mut self, effect: Value) -> Out {
        let name = self.symbol_name(effect)?;
        let schema = schema_for(&name).ok_or_else(|| Fault::Error(format!("unknown effect `{name}'")))?;
        let mut entries = Vec::with_capacity(schema.len());
        for property in schema {
            let property = property.refined();
            let (kind, names): (&str, &[&str]) = match property.kind {
                PropertyKind::Boolean => ("boolean", &[]),
                PropertyKind::Integer => ("integer", &[]),
                PropertyKind::Number => ("number", &[]),
                PropertyKind::UnitInterval => ("unit-interval", &[]),
                PropertyKind::Percentage => ("percentage", &[]),
                PropertyKind::Color => ("color", &[]),
                PropertyKind::ColorList => ("color-list", &[]),
                PropertyKind::Seconds => ("seconds", &[]),
                PropertyKind::Symbol(names) => ("symbol", names),
            };
            let mut entry = vec![self.intern(&format!(":{}", property.property.replace('_', "-"))), self.intern(kind)];
            entry.extend(names.iter().map(|name| self.intern(name)));
            entries.push(self.list(&entry));
        }
        Ok(self.list(&entries))
    }

    fn effect_value(&mut self, value: Value) -> std::result::Result<EffectValue, Fault> {
        if value == Value::TRUE || value == Value::NIL {
            return Ok(EffectValue::Bool(value == Value::TRUE));
        }
        if let Ok(n) = value.as_integer() { return Ok(EffectValue::Integer(n)); }
        if let Ok(n) = self.float_value(value) { return Ok(EffectValue::Number(n)); }
        if let Ok(text) = self.string_text(value) { return Ok(EffectValue::String(text)); }
        if let Some((_, _)) = self.pair(value) {
            let items = self.vm.list_values(value).map_err(Fault::Error)?;
            return items.into_iter().map(|item| self.effect_value(item)).collect::<std::result::Result<_, _>>().map(EffectValue::List);
        }
        Ok(EffectValue::Symbol(self.symbol_name(value)?))
    }

    fn lisp_effect_value(&mut self, value: &EffectValue) -> Out {
        Ok(match value {
            EffectValue::Bool(on) => if *on { Value::TRUE } else { Value::NIL },
            EffectValue::Integer(n) => Value::integer(*n).map_err(Fault::Error)?,
            EffectValue::Number(n) => self.float(*n),
            EffectValue::Symbol(name) => self.intern(name),
            EffectValue::String(text) => self.string(text),
            EffectValue::List(items) => {
                let items = items.iter().map(|item| self.lisp_effect_value(item)).collect::<std::result::Result<Vec<_>, _>>()?;
                self.list(&items)
            }
        })
    }
}

impl Vm {
    /// Adopt CONFIG as the requested visual configuration, and show it on
    /// the open display.
    pub(crate) fn set_visual_config(&mut self, config: VisualConfig) {
        if *self.visual == config { return; }
        *self.visual = config;
        if let Some(display) = self.display.as_mut().filter(|display| display.open) {
            display.host.set_visual_config(&self.visual);
        }
    }
}
