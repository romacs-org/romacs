//! GNU `eval` (src/eval.c Feval): evaluate a Lisp datum as a form.
//!
//! The datum is converted to source, compiled and executed by a reentrant
//! engine. LEXICAL nil makes every
//! binding the form introduces dynamic, as GNU's interpreter does with
//! `internal-interpreter-environment' nil; t or an alist selects lexical
//! binding, an alist's `(SYMBOL . VALUE)' entries becoming the initial lexical
//! bindings. A lexical pair already backed by a capture cell shares that
//! cell with the evaluation. A signal or throw is handed back to the caller's
//! engine unchanged, so `condition-case' and `catch' around `eval' see GNU's
//! condition, data and tag (`threaded::reentrant_load').
//!
//! As in `eval_sub', a non-symbol atom, a quoted datum and an alist-bound
//! symbol evaluate to the identical object without compiling anything.
//!
//! Recorded limits (ledger/divergences VM2-EVAL2): other forms are converted to
//! compiler source. Objects with no source spelling (closures, buffers, ...)
//! are carried as themselves (`Form::Object`), so they evaluate and stay
//! `eq'; list, string and vector structure is copied, so nested quoted
//! structure is not `eq' to the original. A bare symbol in the alist is
//! locally special, as in GNU: the form's `let' bindings of it, and those of
//! the closures it makes, are dynamic, while an alist binding of the same
//! name stays lexical.
use super::*;
use std::ops::ControlFlow;

#[derive(Default)]
struct EvalEnvironment {
    pairs: NameMap<Name, (Value, Value)>,
    specials: NameSet<String>,
}


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
                self.mutator.propagated_escape = Some(threaded::Escape::Signal(condition, data));
                return Err(format!("{name} in eval form"));
            }
        }
        match self.eval_environment(lexical, symbol.then_some(datum))? {
            ControlFlow::Break(value) => Ok(value),
            ControlFlow::Continue(environment) => {
                let form = self.eval_datum_form(datum)?;
                self.execute_eval(&form, lexical == Value::NIL, environment)
            }
        }
    }

    fn eval_environment(&mut self, lexical: Value, symbol: Option<Value>)
        -> Result<ControlFlow<Value, EvalEnvironment>>
    {
        let mut env = EvalEnvironment::default();
        if lexical.tag() != 3 {
            return Ok(ControlFlow::Continue(env));
        }
        let mut tail = lexical;
        let mut steps = 0;
        let limit = self.list_limit();
        while let Ok((entry, next)) = self.cons_pair(tail) {
            steps += 1;
            if steps > limit {
                return if symbol.is_some() {
                    Err(self.eval_environment_error("circular-list", lexical))
                } else {
                    env.pairs.clear();
                    Ok(ControlFlow::Continue(env))
                };
            }
            if let Ok((name, value)) = self.cons_pair(entry) {
                if symbol == Some(name) {
                    return Ok(ControlFlow::Break(value));
                }
                if name.tag() == 7 && name != Value::UNBOUND {
                    let name = self.expansion_form(name, 0)?.name()?;
                    env.pairs.entry(name).or_insert((entry, value));
                }
            } else if entry.tag() == 7 && entry != Value::TRUE && entry != Value::UNBOUND
                && let Some(name) = self.expansion_form(entry, 0)?.name()?.source()
            {
                env.specials.insert(name.to_owned());
            }
            tail = next;
        }
        if symbol.is_some() && tail != Value::NIL {
            return Err(self.eval_environment_error("wrong-type-argument", lexical));
        }
        Ok(ControlFlow::Continue(env))
    }

    fn eval_environment_error(&mut self, name: &str, lexical: Value) -> String {
        let condition = self.intern(name);
        let data = if name == "circular-list" {
            self.list_value(&[lexical])
        } else {
            let predicate = self.intern("listp");
            self.list_value(&[predicate, lexical])
        };
        self.mutator.propagated_escape = Some(threaded::Escape::Signal(condition, data));
        format!("{name} in eval lexical environment")
    }

    fn execute_eval(&mut self, form: &Form, dynamic: bool, environment: EvalEnvironment) -> Result<Value> {
        let (outer, cells) = self.eval_capture_cells(environment.pairs);
        // Request code: the entry is released on return and the closures the
        // form made are retired once unreachable, as for any request. Code
        // compiled per call and never retired grew every collection's walk.
        let saved = std::mem::replace(&mut self.mutator.dynamic_scope, dynamic);
        // GNU's `eval' interprets, expanding each macro call at its reach:
        // an expansion that escapes defers there, and a definition's body
        // compiles at its first call.
        let saved_evaluating = std::mem::replace(&mut self.mutator.evaluating, true);
        let saved_defer = std::mem::replace(&mut self.mutator.defer_expansion_errors, true);
        let compiled = self.compile_eval_request(form, &outer, environment.specials);
        self.mutator.defer_expansion_errors = saved_defer;
        self.mutator.evaluating = saved_evaluating;
        self.mutator.dynamic_scope = saved;
        let (function, names) = compiled?;
        let captures: Vec<_> = names.iter().map(|name| cells[name]).collect();
        self.propagate_next = true;
        let result = threaded::execute_args(self, function, &captures);
        self.propagate_next = false;
        self.release_request_entry(function);
        result
    }

    fn eval_capture_cells(&mut self, pairs: NameMap<Name, (Value, Value)>)
        -> (NameMap<Name, Binding>, NameMap<Name, Value>)
    {
        let mut outer = NameMap::default();
        let mut cells = NameMap::default();
        for (name, (pair, value)) in pairs {
            // Ordinary alists supply initial values. A published pair
            // already names a location, which nested evals must share.
            let cell = self.cell_pairs.cell_of(pair).unwrap_or_else(|| self.alloc_cell(value));
            outer.insert(name.clone(), Binding { reg: outer.len(), cell: true, late: false, shadow: None });
            cells.insert(name, Value::reference(cell, 5));
        }
        (outer, cells)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each `eval' compiles its form; that code must die with its last
    /// reference, or every collection walks all code ever evaluated.
    #[test]
    fn evaluated_code_is_retired_once_unreachable() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun eval-probe (n) (eval (list '+ n 2)))").unwrap();
        vm.eval_source_raw("(setq keeper (eval '(let ((x 40)) (lambda () (+ x 2))) t))").unwrap();
        // The source cache roots each request text's own code.
        vm.clear_source_cache();
        vm.collect(&mut []);
        let live = |vm: &Vm| vm.request_code.live.iter().filter(|&&live| live).count();
        let before = live(&vm);
        vm.eval_source_raw("(dotimes (i 5000) (eval-probe i) (eval `(lambda () ,i) t))").unwrap();
        vm.clear_source_cache();
        vm.collect(&mut []);
        assert!(live(&vm) <= before, "evaluated code retained: {} live functions, {before} before", live(&vm));
        assert_eq!(vm.eval_source_raw("(funcall keeper)").unwrap().as_integer().unwrap(), 42);
        assert_eq!(vm.eval_source_raw("(eval-probe 40)").unwrap().as_integer().unwrap(), 42);
    }

    // The escape an `eval` engine hands to its caller holds heap Values. It is
    // a collector root while present: garbage allocated before it is reclaimed,
    // its payload moves, and the stored handles must follow the move.
    // A throw's no-catch is marked by its error object while it escapes
    // (`Vm::thrown_no_catch'); caught, the mark retires. Marks are weak: one
    // whose error is gone leaves at the next collection.
    #[test]
    fn a_caught_thrown_no_catch_retires_its_mark() {
        let mut vm = Vm::new();
        vm.set_gc_moving(true);
        vm.install_lisp_prelude().unwrap();
        let value = vm.eval_source("(condition-case nil (eval '(throw 'missing (make-vector 3 nil)) t) (no-catch 'caught))").unwrap();
        assert_eq!(value.display().unwrap(), "caught");
        assert!(vm.mutator.thrown_no_catch.is_empty());
        // An error a Rust caller dropped: its mark goes with it.
        let object = vm.list_value(&[Value::NIL]);
        vm.mutator.thrown_no_catch.push(object);
        vm.collect_at_wait(&mut [], false);
        assert!(vm.mutator.thrown_no_catch.is_empty());
        // Garbage first, so the kept error moves when the cons space compacts.
        for i in 0..64 {
            vm.list_value(&[Value::integer(i).unwrap()]);
        }
        let mut kept = [vm.list_value(&[Value::NIL])];
        let before = kept[0];
        vm.mutator.thrown_no_catch.push(kept[0]);
        vm.collect_at_wait(&mut kept, false);
        assert_ne!(kept[0], before, "the kept error moved");
        assert_eq!(vm.mutator.thrown_no_catch, kept.to_vec(), "a live error keeps its mark, moved with it");
    }

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
        vm.mutator.propagated_escape = Some(threaded::Escape::Signal(condition, data));
        let before = vm.heap_objects();
        vm.collect(&mut []);
        assert!(vm.heap_objects() < before, "the collection must reclaim garbage and move survivors");
        let Some(threaded::Escape::Signal(condition, data)) = vm.mutator.propagated_escape.take() else {
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
