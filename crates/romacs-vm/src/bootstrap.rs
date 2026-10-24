//! Ordinary primitives first exercised by the unmodified bundled loadup.
use super::*;
impl Vm {
    fn equal_bootstrap(&self, a: Value, b: Value, depth: usize) -> Result<bool> {
        if a == b {
            return Ok(true);
        }
        if depth > 100 {
            return Err("equal nesting limit exceeded".into());
        }
        if let (Some(x), Some(y)) = (self.str_view(a), self.str_view(b)) {
            return Ok(x.len() == y.len() && x.bytes() == y.bytes());
        }
        if a.tag() == 3 && b.tag() == 3 {
            let (ac, ad) = self.cons_pair(a)?;
            let (bc, bd) = self.cons_pair(b)?;
            return Ok(self.equal_bootstrap(ac, bc, depth + 1)?
                && self.equal_bootstrap(ad, bd, depth + 1)?);
        }
        Ok(match (self.object_if(a), self.object_if(b)) {
            (Some(data::Object::Float(a)), Some(data::Object::Float(b))) => a == b,
            (Some(data::Object::Bignum(a)), Some(data::Object::Bignum(b))) => a == b,
            (Some(data::Object::Bytes(a)), Some(data::Object::Bytes(b))) => a == b,
            (Some(data::Object::BoolVector(a)), Some(data::Object::BoolVector(b))) => a == b,
            (Some(data::Object::Vector(a)), Some(data::Object::Vector(b)))
            | (Some(data::Object::Record(a)), Some(data::Object::Record(b))) => {
                if a.len() != b.len() {
                    false
                } else {
                    let mut equal = true;
                    for (&a, &b) in a.iter().zip(b) {
                        equal &= self.equal_bootstrap(a, b, depth + 1)?;
                    }
                    equal
                }
            }
            _ => false,
        })
    }
    pub(super) fn bootstrap_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        let arity = match p {
            List | Nconc => None,
            Get | Fset => Some(2),
            Put => Some(3),
            Defalias if (2..=3).contains(&n) => None,
            Max if n > 0 => None,
            Member | Nth => Some(2),
            ExpandFileName if n == 2 => None,
            _ => Some(1),
        };
        if arity.is_some_and(|a| a != n) {
            return Err("wrong primitive arity".into());
        }
        let a = self.stack[start..start + n].to_vec();
        match p {
            List => Ok(self.list_value(&a)),
            BareSymbol => {
                if a[0].tag() == 7 || a[0] == Value::NIL || a[0] == Value::TRUE {
                    Ok(a[0])
                } else {
                    Err("symbol required".into())
                }
            }
            Fset => self.set_function_leaf(a[0], a[1])
                .map_err(|fault| match fault {
                    prim::Fault::CyclicFunctionIndirection(_) => "fset cyclic function indirection".into(),
                    prim::Fault::SettingConstant(_) => "fset setting constant".into(),
                    _ => "fset symbol required".into(),
                }),
            Defalias => {
                if !(2..=3).contains(&n) {
                    return Err("wrong primitive arity".into());
                }
                let slot = a[0].index(7)?.checked_sub(1).ok_or("symbol required")?;
                if slot >= self.slots.len() {
                    return Err("invalid symbol".into());
                }
                let mut definition = a[1];
                let mut is_macro = false;
                if definition.tag() == 3 && self.resolve_keymap(definition).is_none() {
                    let (head, tail) = self.cons_pair(definition)?;
                    if head.tag() == 7 && self.symbol_names.get(head.index(7)?.saturating_sub(1)).is_some_and(|name| name == "macro") {
                        definition = tail;
                        is_macro = true;
                    } else {
                        return Err("list function descriptors are not yet supported".into());
                    }
                }
                self.autoload_note_defalias(slot);
                // defalias stores through Ffset (data.c:997), which refuses a
                // symbol chain reaching SYMBOL before writing (data.c:910-913).
                // Only a symbol DEFINITION is walked; `(macro . S)` is a cons.
                let mut target = a[1];
                let mut seen = std::collections::HashSet::new();
                while target != Value::NIL && (target == Value::TRUE || target.tag() == 7) {
                    if target == a[0] || !seen.insert(target.0) {
                        return Err("defalias cyclic function indirection".into());
                    }
                    let next = if target == Value::TRUE { self.slot("t") } else { target.index(7)?.checked_sub(1).ok_or("symbol required")? };
                    target = self.slots.get(next).map_or(Value::NIL, |s| s.function);
                    if target == Value::UNBOUND {
                        break;
                    }
                }
                self.define_function(slot, definition, is_macro)?;
                Ok(a[0])
            }
            Boundp => {
                let value = a[0];
                if value == Value::NIL || value == Value::TRUE {
                    return Ok(Value::TRUE);
                }
                let slot = value.index(7)?.checked_sub(1).ok_or("symbol required")?;
                Ok(
                    if self.variable_value(slot)? != Value::UNBOUND {
                        Value::TRUE
                    } else {
                        Value::NIL
                    },
                )
            }
            Stringp => Ok(
                if self.str_view(a[0]).is_some()
                {
                    Value::TRUE
                } else {
                    Value::NIL
                },
            ),
            Member => {
                let mut tail = a[1];
                let mut steps = 0;
                while tail != Value::NIL {
                    let (car, cdr) = self.cons_pair(tail)?;
                    if self.equal_bootstrap(a[0], car, 0)? {
                        return Ok(tail);
                    }
                    tail = cdr;
                    steps += 1;
                    if steps > self.list_limit() {
                        return Err("circular list".into());
                    }
                }
                Ok(Value::NIL)
            }
            Nconc => {
                let mut out = Value::NIL;
                let mut last: Option<usize> = None;
                for (i, &list) in a.iter().enumerate() {
                    if list == Value::NIL {
                        continue;
                    }
                    if let Some(index) = last {
                        self.conses[index].1 = list;
                    } else {
                        out = list;
                    }
                    if i + 1 == a.len() {
                        break;
                    }
                    let mut tail = list;
                    let mut steps = 0;
                    loop {
                        if Self::image_value(tail) {
                            return Err("cannot mutate immutable image".into());
                        }
                        let index = tail.index(3)?;
                        let cdr = self.cons_pair(tail)?.1;
                        if cdr == Value::NIL {
                            last = Some(index);
                            break;
                        }
                        tail = cdr;
                        steps += 1;
                        if steps > self.list_limit() {
                            return Err("circular list".into());
                        }
                    }
                }
                Ok(out)
            }
            ExpandFileName => {
                let name = self.source_string(a[0])?;
                let name = if name == "~" || name.starts_with("~/") {
                    std::path::PathBuf::from(std::env::var_os("HOME").ok_or("HOME missing")?)
                        .join(name.strip_prefix("~/").unwrap_or(""))
                } else if name.starts_with('~') {
                    return Err("~user path expansion is not supported".into());
                } else {
                    std::path::PathBuf::from(name)
                };
                let dir = if n == 2 && a[1] != Value::NIL {
                    std::path::PathBuf::from(self.source_string(a[1])?)
                } else {
                    std::env::current_dir().map_err(|e| e.to_string())?
                };
                let base = if dir.is_absolute() {
                    dir
                } else {
                    std::env::current_dir()
                        .map_err(|e| e.to_string())?
                        .join(dir)
                };
                let path = if name.is_absolute() {
                    name
                } else {
                    base.join(name)
                };
                let mut out = std::path::PathBuf::new();
                for c in path.components() {
                    match c {
                        std::path::Component::ParentDir => {
                            out.pop();
                        }
                        std::path::Component::CurDir => {}
                        _ => out.push(c.as_os_str()),
                    }
                }
                self.literal(&Form::String(out.to_string_lossy().into_owned()))
            }
            FileDirectoryP => Ok(
                if std::path::Path::new(self.source_string(a[0])?).is_dir() {
                    Value::TRUE
                } else {
                    Value::NIL
                },
            ),
            Max => {
                for &v in &a {
                    self.number(v)?;
                }
                let mut out = a[0];
                for &v in &a[1..] {
                    if self.number(v)?.is_nan()
                        || self.numeric_cmp(out, v)? == Some(std::cmp::Ordering::Less)
                    {
                        out = v;
                    }
                }
                Ok(out)
            }
            _ => unreachable!("bootstrap primitive dispatch"),
        }
    }
}
