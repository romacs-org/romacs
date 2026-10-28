//! Typed condition identities and ancestry, with a mutable Lisp property projection.
//! Symbol IDs never move. Projection graphs are strong, forwarding GC edges;
//! destructive cons edits invalidate derived ancestry instead of creating two truths.
use super::*;
#[path = "conditions_builtin.rs"]
mod builtins;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct SymbolId(u64);
impl SymbolId {
    pub(super) fn new(v: Value) -> Option<Self> {
        (v == Value::NIL || v == Value::TRUE || v.tag() == 7 && v != Value::UNBOUND)
            .then_some(Self(v.0))
    }
    pub(super) fn value(self) -> Value {
        Value(self.0)
    }
}
#[derive(Default)]
pub(super) struct Entry {
    pub(super) properties: Vec<(Value, Value)>,
    ancestry: Option<Vec<SymbolId>>,
    builtin: Option<(Vec<SymbolId>, &'static str)>,
}
#[derive(Default)]
pub(super) struct Registry {
    pub(super) entries: HashMap<SymbolId, Entry>,
}
impl Registry {
    pub(super) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for entry in self.entries.values_mut() {
            for (key, value) in &mut entry.properties {
                visit(key);
                visit(value);
            }
        }
    }
    pub(super) fn invalidate(&mut self) {
        for entry in self.entries.values_mut() {
            entry.ancestry = None;
        }
    }
    pub(super) fn put(&mut self, symbol: SymbolId, property: Value, value: Value) {
        let entry = self.entries.entry(symbol).or_default();
        entry.ancestry = None;
        if let Some((_, old)) = entry
            .properties
            .iter_mut()
            .find(|(key, _)| *key == property)
        {
            *old = value;
        } else {
            entry.properties.push((property, value));
        }
    }
}
pub(super) struct Fault {
    condition: &'static str,
    values: Vec<Value>,
}
impl Fault {
    fn wrong(predicate: Value, value: Value) -> Self {
        Self {
            condition: "wrong-type-argument",
            values: vec![predicate, value],
        }
    }
}
type Checked<T> = std::result::Result<T, Fault>;

impl Vm {
    fn intern_symbol(&mut self, name: &str) -> Value {
        match name {
            "nil" => Value::NIL,
            "t" => Value::TRUE,
            _ => Value::reference(self.slot(name) + 1, 7),
        }
    }
    fn symbol_id(&mut self, v: Value) -> Checked<SymbolId> {
        SymbolId::new(v).ok_or_else(|| Fault::wrong(self.intern_symbol("symbolp"), v))
    }
    pub(super) fn initialize_conditions(&mut self) {
        self.intern_symbol("error-conditions");
        self.intern_symbol("error-message");
        let symbol = self.intern_symbol("not-unicode-string");
        let error = self.intern_symbol("error");
        let ancestors = vec![SymbolId::new(symbol).unwrap(), SymbolId::new(error).unwrap()];
        self.conditions.entries.insert(SymbolId::new(symbol).unwrap(), Entry {
            ancestry: Some(ancestors.clone()), builtin: Some((ancestors, "String is not Unicode text")), ..Entry::default()
        });
        for &(name, ancestors, text) in builtins::BUILTINS {
            let symbol = self.intern_symbol(name);
            let id = SymbolId::new(symbol).unwrap();
            let ancestors = ancestors
                .iter()
                .map(|name| SymbolId::new(self.intern_symbol(name)).unwrap())
                .collect::<Vec<_>>();
            let entry = self.conditions.entries.entry(id).or_default();
            entry.ancestry = Some(ancestors.clone());
            entry.builtin = Some((ancestors, text));
        }
    }
    fn get_property(&mut self, id: SymbolId, property: Value) -> Value {
        if let Some(value) = self.conditions.entries.get(&id).and_then(|e| {
            e.properties
                .iter()
                .find(|(key, _)| *key == property)
                .map(|(_, value)| *value)
        }) {
            return value;
        }
        let Some((ancestors, text)) = self
            .conditions
            .entries
            .get(&id)
            .and_then(|e| e.builtin.clone())
        else {
            return Value::NIL;
        };
        let value = if property == self.intern_symbol("error-conditions") {
            self.list_value(
                &ancestors
                    .into_iter()
                    .map(SymbolId::value)
                    .collect::<Vec<_>>(),
            )
        } else if property == self.intern_symbol("error-message") {
            self.allocate_object(data::Object::String(Box::new(data::LispString::new(
                text,
            ))))
        } else {
            return Value::NIL;
        };
        self.conditions.put(id, property, value);
        value
    }
    fn unobserved_condition_ancestry(&self, id: SymbolId, property: Value) -> Option<&[SymbolId]> {
        let entry = self.conditions.entries.get(&id)?;
        if entry.properties.iter().any(|(key, _)| *key == property) { return None; }
        entry.builtin.as_ref().map(|(ancestry, _)| ancestry.as_slice())
    }
    // Search retains this projection when a returning handler replaces its property.
    pub(super) fn signal_conditions(&mut self, condition: Value) -> Value {
        let Some(id) = SymbolId::new(condition) else { return Value::NIL; };
        let property = self.intern_symbol("error-conditions");
        if let Some(ancestry) = self.unobserved_condition_ancestry(id, property) {
            let values: Vec<_> = ancestry.iter().map(|id| id.value()).collect();
            return self.list_value(&values);
        }
        let conditions = self.get_property(id, property);
        if conditions != Value::NIL { return conditions; }
        let error = self.intern_symbol("error");
        self.list_value(&[error])
    }

    pub(super) fn publish_signal_conditions(&mut self, condition: Value, conditions: Value) {
        let Some(id) = SymbolId::new(condition) else { return; };
        let property = self.intern_symbol("error-conditions");
        if self.unobserved_condition_ancestry(id, property).is_some() {
            // A callback can observe and mutate the list with `get'. Publish
            // the search's own list so subsequent matching sees those edits.
            self.conditions.put(id, property, conditions);
        }
    }

    pub(super) fn condition_matches(&mut self, condition: Value, choice: Value) -> bool {
        if choice == Value::TRUE {
            return true;
        }
        let Some(id) = SymbolId::new(condition) else {
            return false;
        };
        let Some(wanted) = SymbolId::new(choice) else {
            return false;
        };
        if let Some(ancestors) = self
            .conditions
            .entries
            .get(&id)
            .and_then(|e| e.ancestry.as_ref())
        {
            return ancestors.contains(&wanted);
        }
        let property = self.intern_symbol("error-conditions");
        let projection = self.get_property(id, property);
        let ancestors = if projection == Value::NIL {
            // GNU signals undefined conditions through error, not their own name.
            vec![SymbolId::new(self.intern_symbol("error")).unwrap()]
        } else {
            self.list_values(projection)
                .unwrap_or_default()
                .into_iter()
                .filter_map(SymbolId::new)
                .collect()
        };
        // Only unobserved builtin ancestry is cached. Once a projection escapes
        // to Lisp, any cons mutator can edit it, including specialized kernels.
        ancestors.contains(&wanted)
    }
    pub(super) fn condition_call(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
        error: &mut Option<String>,
        signal: &mut Option<(Value, Value)>,
    ) -> Option<Value> {
        match self.condition_primitive(p, start, n) {
            Ok(v) => Some(v),
            Err(fault) => {
                let condition = self.intern_symbol(fault.condition);
                let data = self.list_value(&fault.values);
                *signal = Some((condition, data));
                *error = Some(fault.condition.into());
                None
            }
        }
    }
    fn condition_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Checked<Value> {
        use Primitive::*;
        let valid = match p {
            DefineError => (2..=3).contains(&n),
            Get => n == 2,
            Put => n == 3,
            ErrorMessage => n == 1,
            _ => unreachable!(),
        };
        if !valid {
            return Err(Fault {
                condition: "wrong-number-of-arguments",
                values: vec![
                    Value::reference(p as usize, 6),
                    Value::integer(n as i64).unwrap(),
                ],
            });
        }
        let x = self.stack[start];
        if p == ErrorMessage {
            return self.error_message(x);
        }
        let id = self.symbol_id(x)?;
        let y = self.stack[start + 1];
        // Every property except the two condition projections lives on the
        // symbol OBJECT's own plist (GNU `u.s.plist`), under any `eq` key.
        // error-conditions and error-message stay in the registry, which
        // caches ancestry; `symbol-plist' does not show them (VM2-PLIST1).
        if matches!(p, Get | Put)
            && y != self.intern_symbol("error-conditions")
            && y != self.intern_symbol("error-message")
        {
            if p == Get {
                if self.fence.is_some() {
                    self.fence_property(x, Some(y), world::Access::Read);
                }
                return Ok(self.symbol_property(x, y).expect("checked symbol"));
            }
            let value = self.stack[start + 2];
            return self.put_symbol_property_checked(x, y, value).map_err(|fault| match fault {
                annotations::PlistFault::Symbol(v) => Fault::wrong(self.intern_symbol("symbolp"), v),
                annotations::PlistFault::Plistp(v) => Fault::wrong(self.intern_symbol("plistp"), v),
                annotations::PlistFault::Circular(v) => Fault { condition: "circular-list", values: vec![v] },
            });
        }
        if p == Get {
            return Ok(self.get_property(id, y));
        }
        if p == Put {
            let value = self.stack[start + 2];
            self.conditions.put(id, y, value);
            return Ok(value);
        }
        let parents = if n == 2 || self.stack[start + 2] == Value::NIL {
            self.intern_symbol("error")
        } else {
            self.stack[start + 2]
        };
        let property = self.intern_symbol("error-conditions");
        let mut ancestors = vec![x];
        let parent_list = parents.tag() == 3;
        let parents = if parent_list {
            self.list_values(parents)
                .map_err(|_| Fault::wrong(self.intern_symbol("listp"), parents))?
        } else {
            vec![parents]
        };
        for parent in parents {
            let parent_id = self.symbol_id(parent)?;
            let projection = self.get_property(parent_id, property);
            let inherited = if projection == Value::NIL {
                if parent_list {
                    let name = self.display(parent).unwrap_or_default();
                    let message = self.allocate_object(data::Object::String(Box::new(
                        data::LispString::new(&format!("Unknown signal ‘{name}’")),
                    )));
                    return Err(Fault {
                        condition: "error",
                        values: vec![message],
                    });
                }
                vec![parent]
            } else {
                self.list_values(projection)
                    .map_err(|_| Fault::wrong(self.intern_symbol("listp"), projection))?
            };
            for ancestor in inherited {
                if !ancestors.contains(&ancestor) {
                    ancestors.push(ancestor);
                }
            }
        }
        let projection = self.list_value(&ancestors);
        self.conditions.put(id, property, projection);
        let property = self.intern_symbol("error-message");
        self.conditions.put(id, property, y);
        Ok(y)
    }
    fn message_string(&self, value: Value) -> Result<&str> {
        if Self::image_value(value) {
            return self.string_value(value);
        }
        match self.object(value)? {
            data::Object::String(s) => Ok(s.as_str()),
            data::Object::Bytes(b) if b.is_ascii() => Ok(unsafe { std::str::from_utf8_unchecked(b) }),
            data::Object::RawString(_) | data::Object::Bytes(_) => Err("not-unicode-string".into()),
            _ => Err("string required".into()),
        }
    }
    fn error_message(&mut self, error: Value) -> Checked<Value> {
        let (condition, data) = self
            .cons_pair(error)
            .map_err(|_| Fault::wrong(self.intern_symbol("consp"), error))?;
        let id = self.symbol_id(condition)?;
        let property = self.intern_symbol("error-message");
        let message = self.get_property(id, property);
        let mut message = self
            .message_string(message)
            .unwrap_or("peculiar error")
            .to_owned();
        let mut values = self
            .list_values(data)
            .map_err(|_| Fault::wrong(self.intern_symbol("listp"), data))?;
        let is_error = condition == self.intern_symbol("error");
        let file_error = self.intern_symbol("file-error");
        let is_file_error = self.condition_matches(condition, file_error);
        // Only the condition's message template is quote-substituted; an
        // explicit error/file-error message is user data and stays literal.
        message = message.replace('\'', "’");
        if (is_error || is_file_error) && !values.is_empty() {
            message = self
                .message_string(values.remove(0))
                .unwrap_or("peculiar error")
                .to_owned();
        }
        if !values.is_empty() {
            message.push_str(": ");
            let rendered: Vec<_> = values
                .into_iter()
                .map(|v| {
                    if is_file_error {
                        self.message_string(v)
                            .map(str::to_owned)
                            .unwrap_or_else(|_| self.display(v).unwrap_or_default())
                    } else {
                        self.display(v).unwrap_or_default()
                    }
                })
                .collect();
            message.push_str(&rendered.join(", "));
        }
        Ok(
            self.allocate_object(data::Object::String(Box::new(data::LispString::new(
                &message,
            )))),
        )
    }
}
