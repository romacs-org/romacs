//! GNU `eval` (src/eval.c Feval): evaluate a Lisp datum as a form.
//!
//! The datum is converted to source and compiled with an empty lexical
//! environment, then executed by a reentrant engine. LEXICAL nil makes every
//! binding the form introduces dynamic, as GNU's interpreter does with
//! `internal-interpreter-environment' nil; t or an alist selects lexical
//! binding, an alist's `(SYMBOL . VALUE)' entries becoming the initial lexical
//! bindings. A signal or throw leaving the form is handed back to the caller's
//! engine unchanged, so `condition-case' and `catch' around `eval' see GNU's
//! condition, data and tag (`threaded::reentrant_load').
//!
//! As in `eval_sub', a non-symbol atom, a quoted datum and an alist-bound
//! symbol evaluate to the identical object without compiling anything.
//!
//! Recorded limits (DIVERGENCES.md VM2-EVAL2): other forms are converted to
//! compiler source. Objects with no source spelling (closures, buffers, ...)
//! are carried as themselves (`Form::Object`), so they evaluate and stay
//! `eq'; list, string and vector structure is copied, so nested quoted
//! structure is not `eq' to the original; bare symbols in an alist, which
//! GNU treats as locally special, are ignored.
use super::*;

impl Vm {
    pub(super) fn eval_primitive(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() || args.len() > 2 {
            return Err("wrong primitive arity".into());
        }
        let datum = args[0];
        let lexical = args.get(1).copied().unwrap_or(Value::NIL);
        let symbol = datum == Value::NIL || datum == Value::TRUE || datum.tag() == 7;
        if !symbol && datum.tag() != 3 {
            return Ok(datum);
        }
        if let Ok((head, tail)) = self.cons_pair(datum) {
            if head == self.intern("quote")
                && let Ok((quoted, Value::NIL)) = self.cons_pair(tail)
            {
                return Ok(quoted);
            }
            // eval_sub checks the argument list, then a non-symbol head,
            // before looking at the rest of the form (which may be circular).
            let lambda = self.cons_pair(head).is_ok_and(|(car, _)| car == self.intern("lambda"));
            let failure = if tail != Value::NIL && tail.tag() != 3 {
                Some(("wrong-type-argument", vec![self.intern("listp"), tail]))
            } else if !(head == Value::NIL || head == Value::TRUE || matches!(head.tag(), 4 | 6 | 7) || lambda) {
                Some(("invalid-function", vec![head]))
            } else {
                None
            };
            if let Some((name, data)) = failure {
                let condition = self.intern(name);
                let data = self.list_value(&data);
                self.propagated_escape = Some(threaded::Escape::Signal(condition, data));
                return Err(format!("{name} in eval form"));
            }
        }
        let mut bindings = Vec::new();
        if lexical.tag() == 3 {
            let mut seen = HashSet::new();
            let mut tail = lexical;
            let limit = self.list_limit();
            let mut steps = 0usize;
            let mut circular = false;
            // GNU looks bindings up with assq: the first entry for a name wins.
            while let Ok((entry, next)) = self.cons_pair(tail) {
                steps += 1;
                if steps > limit {
                    circular = true;
                    break;
                }
                if let Ok((name, value)) = self.cons_pair(entry) {
                    if symbol && name == datum {
                        return Ok(value);
                    }
                    if name.tag() == 7 && name != Value::NIL && name != Value::TRUE && seen.insert(name.0) {
                        bindings.push((name, value));
                    }
                }
                tail = next;
            }
            // Looking a symbol up walks the whole alist: assq signals on a
            // circular or dotted one (src/fns.c Fassq). Other forms look
            // nothing up unless they reference a variable; VM2-EVAL2 records
            // that such forms then evaluate without the environment.
            if symbol && (circular || tail != Value::NIL) {
                let (name, data) = if circular {
                    ("circular-list", vec![lexical])
                } else {
                    ("wrong-type-argument", vec![self.intern("listp"), lexical])
                };
                let condition = self.intern(name);
                let data = self.list_value(&data);
                self.propagated_escape = Some(threaded::Escape::Signal(condition, data));
                return Err(format!("{name} in eval lexical environment"));
            }
            if circular {
                bindings.clear();
            }
        }
        let mut form = self.expansion_form(datum, 0)?;
        if !bindings.is_empty() {
            let quote = |f| Form::List(vec![Form::Sym("quote".into()), f]);
            let bindings = bindings
                .into_iter()
                .map(|(name, value)| {
                    Ok(Form::List(vec![self.source_form(name, 0)?, quote(self.expansion_form(value, 0)?)]))
                })
                .collect::<Result<Vec<_>>>()?;
            form = Form::List(vec![Form::Sym("let".into()), Form::List(bindings), form]);
        }
        let saved = std::mem::replace(&mut self.dynamic_scope, lexical == Value::NIL);
        let compiled = self.compile(&form);
        self.dynamic_scope = saved;
        let function = compiled?;
        self.propagate_next = true;
        let result = self.execute_raw(function);
        self.propagate_next = false;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The escape an `eval` engine hands to its caller holds heap Values. It is
    // a collector root while present: garbage allocated before it is reclaimed,
    // its payload moves, and the stored handles must follow the move.
    #[test]
    fn propagated_escape_is_traced_across_a_moving_collection() {
        let mut vm = Vm::new();
        // Unreachable strings AND conses precede the payload, so both the
        // object and the cons spaces compact and every escape handle moves.
        for i in 0..64 {
            vm.make_lisp_string(format!("garbage {i}").into_bytes(), true);
            vm.list_value(&[Value::integer(i).unwrap(), Value::NIL]);
        }
        let payload = vm.make_lisp_string(b"payload bytes".to_vec(), true);
        let pair = vm.list_value(&[Value::integer(1).unwrap(), payload]);
        let data = vm.list_value(&[payload, pair]);
        let condition = vm.intern("arith-error");
        vm.propagated_escape = Some(threaded::Escape::Signal(condition, data));
        let before = vm.heap_objects();
        vm.collect(&mut []);
        assert!(vm.heap_objects() < before, "the collection must reclaim garbage and move survivors");
        let Some(threaded::Escape::Signal(condition, data)) = vm.propagated_escape.take() else {
            panic!("escape lost")
        };
        assert_eq!(vm.display(condition).unwrap(), "arith-error");
        let forwarded = vm.cons_pair(data);
        assert!(forwarded.is_ok(), "the escape's data handle must follow the move: {forwarded:?}");
        let (first, rest) = forwarded.unwrap();
        assert_eq!(vm.source_string(first).unwrap(), "payload bytes");
        let (pair, _) = vm.cons_pair(rest).unwrap();
        let (_, tail) = vm.cons_pair(pair).unwrap();
        let (second, _) = vm.cons_pair(tail).unwrap();
        assert_eq!(second, first, "both handles to the payload must forward to one object");
        assert_eq!(vm.display(data).unwrap(), "(\"payload bytes\" (1 \"payload bytes\"))");
    }
}
