//! Ordinary primitives first exercised by the unmodified bundled loadup.
use super::*;
impl Vm {
    /// data.c:1021: a non-nil DOCSTRING becomes SYMBOL's
    /// `function-documentation'.
    fn defalias_documentation(&mut self, args: &[Value]) -> Result<()> {
        if let Some(&doc) = args.get(2) && doc != Value::NIL {
            let property = self.intern("function-documentation");
            self.put_symbol_property(args[0], property, doc)?;
        }
        Ok(())
    }
    pub(super) fn bootstrap_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        let arity = match p {
            List => None,
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
                // data.c:993 automatic advice activation: a symbol's
                // `defalias-fset-function' stores the definition instead of
                // Ffset (nadvice keeps an autoloaded function's advice so).
                let property = self.intern("defalias-fset-function");
                let hook = self.symbol_property(a[0], property)?;
                if hook != Value::NIL {
                    self.autoload_note_defalias(slot);
                    self.call_lisp_at_write(hook, &[a[0], a[1]])?;
                    self.defalias_documentation(&a)?;
                    return Ok(a[0]);
                }
                let mut definition = a[1];
                let mut is_macro = false;
                if definition.tag() == 3 && self.resolve_keymap(definition).is_none() {
                    let (head, tail) = self.cons_pair(definition)?;
                    // Ffset (data.c:928) stores any other list as given: a
                    // `(lambda ...)' or `(autoload ...)' descriptor.
                    if head.tag() == 7 && self.symbol_names.get(head.index(7)?.saturating_sub(1)).is_some_and(|name| name == "macro") {
                        definition = tail;
                        is_macro = true;
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
                // data.c defalias: LOADHIST_ATTACH ((defun . SYMBOL)), which
                // `symbol-file' reads.
                let defun = self.intern("defun");
                let entry = self.alloc_cons(defun, a[0]);
                let history = self.slot("current-load-list");
                let old = self.variable_value(history)?;
                let cons = self.alloc_cons(entry, old);
                self.assign_variable(history, cons)?;
                self.defalias_documentation(&a)?;
                Ok(a[0])
            }
            Boundp => {
                let value = a[0];
                if value == Value::NIL || value == Value::TRUE {
                    return Ok(Value::TRUE);
                }
                let slot = value.index(7)?.checked_sub(1).ok_or("symbol required")?;
                Ok(
                    if self.fenced_and_bound(slot) || self.variable_value(slot)? != Value::UNBOUND {
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
                    if self.equal_text(a[0], car)? {
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
            ExpandFileName => self.expand_file_name_legacy(a[0], a.get(1).copied().unwrap_or(Value::NIL)),
            FileDirectoryP => Ok(
                if crate::host::probe(crate::host::Probe::IsDir, self.source_string(a[0])?) {
                    Value::TRUE
                } else {
                    Value::NIL
                },
            ),
            Max => self.minmax(&a, std::cmp::Ordering::Greater),
            _ => unreachable!("bootstrap primitive dispatch"),
        }
    }
}
