//! GNU eval.c Fmacroexpand: shallow, repeated expansion with an identity stop.
use super::*;

impl Vm {
    fn macroexpand_fault<T>(&mut self, name: &str, data: &[Value]) -> Result<T> {
        let condition = self.intern(name);
        let data = self.list_value(data);
        self.propagated_escape = Some(threaded::Escape::Signal(condition, data));
        Err(format!("{name} in macroexpand"))
    }

    // Fassq ignores non-cons entries, stops at the first match, and reports
    // the original improper list rather than its final tail.
    fn macro_environment_entry(
        &mut self,
        symbol: Value,
        environment: Value,
    ) -> Result<Option<Value>> {
        let mut tail = environment;
        let mut seen = HashSet::default();
        while tail != Value::NIL {
            if !seen.insert(tail.0) {
                return self.macroexpand_fault("circular-list", &[environment]);
            }
            let Ok((entry, next)) = self.cons_pair(tail) else {
                let listp = self.intern("listp");
                return self.macroexpand_fault("wrong-type-argument", &[listp, environment]);
            };
            if let Ok((key, expander)) = self.cons_pair(entry) {
                if key == symbol {
                    return Ok(Some(expander));
                }
            }
            tail = next;
        }
        Ok(None)
    }

    /// Shared with source_cache's compiler expansion. Call the resolved
    /// function object, not the head symbol (and never `macroexpand' itself).
    /// Keep quoted argument objects intact and make the wrapper request-owned.
    pub(super) fn call_macro_expander(&mut self, function: Value, args: &[Value]) -> Result<Value> {
        let lambda = self.intern("lambda");
        let function = if self
            .cons_pair(function)
            .is_ok_and(|(head, _)| head == lambda)
        {
            let form = self.expansion_form(function, 0)?;
            let form = Form::List(vec![Form::Sym("function".into()), form]);
            let entry = self.compile_request(&form)?.0;
            // ARGS are rooted across making the closure and read back.
            let frame = self.root_frame(args);
            self.propagate_next = true;
            let result = self.execute_raw(entry);
            self.release_request_entry(entry);
            self.propagate_next = false;
            let args: Vec<Value> = (0..args.len()).map(|i| self.rooted(&frame, i)).collect();
            self.drop_frame(frame);
            return self.reader_call(result?, &args);
        } else {
            function
        };
        self.reader_call(function, args)
    }

    pub(super) fn macroexpand_primitive(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() || args.len() > 2 {
            let name = Value::reference(Primitive::Macroexpand as usize, 6);
            let count = Value::integer(args.len() as i64)?;
            return self.macroexpand_fault("wrong-number-of-arguments", &[name, count]);
        }
        self.macroexpand_value(args[0], args.get(1).copied().unwrap_or(Value::NIL))
    }

    fn macro_symbol_slot(&self, symbol: Value) -> Option<usize> {
        if symbol == Value::NIL {
            self.names.get("nil").copied()
        } else if symbol == Value::TRUE {
            self.names.get("t").copied()
        } else {
            symbol.index_if(7).and_then(|i| i.checked_sub(1))
        }
    }

    /// FORM and ENVIRONMENT are rooted across each step, which can load an
    /// autoload and call an expander; every use reads them back.
    fn macroexpand_value(&mut self, form: Value, environment: Value) -> Result<Value> {
        let frame = self.root_frame(&[form, environment]);
        let result = self.macroexpand_steps(&frame);
        self.drop_frame(frame);
        result
    }

    fn macroexpand_steps(&mut self, frame: &crate::gc::RootFrame) -> Result<Value> {
        for _ in 0..4096 {
            let (form, environment) = (self.rooted(frame, 0), self.rooted(frame, 1));
            let Ok((mut definition, _)) = self.cons_pair(form) else {
                return Ok(form);
            };
            let mut symbol = definition;
            let mut entry = None;
            let mut aliases = HashSet::default();
            while definition.tag() == 7 || definition == Value::NIL || definition == Value::TRUE {
                symbol = definition;
                if !aliases.insert(symbol.0) {
                    return self.macroexpand_fault("cyclic-function-indirection", &[symbol]);
                }
                entry = self.macro_environment_entry(symbol, environment)?;
                if entry.is_some() {
                    break;
                }
                let Some(slot) = self.macro_symbol_slot(symbol) else {
                    return Ok(form);
                };
                // Native macro cells store their expander directly; GNU's
                // (macro . EXPANDER) discriminator lives in this side table.
                if self.macros.contains(&slot) {
                    break;
                }
                definition = self.slots[slot].function;
                if definition == Value::NIL || definition == Value::UNBOUND {
                    return Ok(form);
                }
            }
            let expander = if let Some(expander) = entry {
                if expander == Value::NIL {
                    return Ok(form);
                }
                expander
            } else if let Some(slot) = self
                .macro_symbol_slot(symbol)
                .filter(|slot| self.macros.contains(slot))
            {
                self.slots[slot].function
            } else {
                let macro_symbol = self.intern("macro");
                let loaded = self.autoload_do_load(definition, symbol, macro_symbol)?;
                let form = self.rooted(frame, 0);
                // autoload_do_load can install an alias to a native macro.
                if let Some(slot) = self.macro_symbol_slot(symbol) {
                    let end = self.macro_alias_end(slot);
                    if self.macros.contains(&end) {
                        self.slots[end].function
                    } else if let Ok((kind, function)) = self.cons_pair(loaded) {
                        if kind != macro_symbol {
                            return Ok(form);
                        }
                        function
                    } else {
                        return Ok(form);
                    }
                } else if let Ok((kind, function)) = self.cons_pair(loaded) {
                    if kind != macro_symbol {
                        return Ok(form);
                    }
                    function
                } else {
                    return Ok(form);
                }
            };
            // apply1 validates the argument spine only after a macro is found.
            let form = self.rooted(frame, 0);
            let arguments = self.cons_pair(form)?.1;
            let args = match self.list_values(arguments) {
                Ok(args) => args,
                Err(_) => {
                    let listp = self.intern("listp");
                    return self.macroexpand_fault("wrong-type-argument", &[listp, arguments]);
                }
            };
            let newform = self.call_macro_expander(expander, &args)?;
            let form = self.rooted(frame, 0);
            if newform == form {
                return Ok(form);
            }
            self.set_rooted(frame, 0, newform);
        }
        Err("macro expansion limit exceeded".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(setup: &str, expression: &str, expected: &str) {
        let mut vm = Vm::new();
        if !setup.is_empty() {
            vm.eval_source_raw(setup).unwrap();
        }
        let result = vm.eval_source_raw(expression).unwrap();
        assert_eq!(vm.display(result).unwrap(), expected, "{expression}");
    }

    // GNU 32.0.50, commit 45b0f7699df36b3849edc3bdef37d4cb9e75b9ea,
    // eval.c:1283-1349. All expressions are also replayed by the optional
    // live-pin test below; no dumped Lisp helper implementation is substituted.
    const CASES: &[(&str, &str, &str)] = &[
        (
            "",
            "(list (fboundp 'macroexpand) (functionp 'macroexpand))",
            "(t t)",
        ),
        (
            "(defmacro mx-a (x) (list 'quote x))",
            "(macroexpand '(list (mx-a payload)))",
            "(list (mx-a payload))",
        ),
        (
            "(defun mx-helper (x) (list 'quote x)) (defalias 'mx-helper-alias 'mx-helper)",
            "(macroexpand '(mx-env 31) '((mx-env . mx-helper-alias)))",
            "(quote 31)",
        ),
        (
            "",
            "(macroexpand '(nil 31) '((nil . (lambda (x) (list 'quote x)))))",
            "(quote 31)",
        ),
        (
            "",
            "(macroexpand '((macro lambda (x) (list 'quote x)) 31))",
            "(quote 31)",
        ),
        (
            "",
            "(list (macroexpand 17 'bad) (macroexpand '(unknown 1)) (funcall 'macroexpand '(unknown 2)))",
            "(17 (unknown 1) (unknown 2))",
        ),
        (
            "(defmacro mx-a (x) (list 'mx-b x)) (defmacro mx-b (x) (list 'quote x))",
            "(macroexpand '(mx-a payload))",
            "(quote payload)",
        ),
        (
            "(defmacro mx-a (x) (list 'quote x))",
            "(macroexpand '(mx-a payload) '((mx-a)))",
            "(mx-a payload)",
        ),
        (
            "(defmacro mx-a (x) (list 'quote x)) (defalias 'mx-alias 'mx-a)",
            "(macroexpand '(mx-alias payload) '((mx-a . (lambda (x) (list 'quote (list 'override x))))))",
            "(quote (override payload))",
        ),
        (
            "(defmacro mx-a (x) (list 'quote x)) (defalias 'mx-alias 'mx-a)",
            "(macroexpand '(mx-alias payload) '((mx-alias) (mx-a . (lambda (x) 42))))",
            "(mx-alias payload)",
        ),
        (
            "",
            "(macroexpand '(mx-local 8) '((ignored . nil) mx-local (mx-local . (lambda (x) (list 'quote x))) (mx-local)))",
            "(quote 8)",
        ),
        (
            "",
            "(let ((f '(mx-self 1))) (eq f (macroexpand f (list (cons 'mx-self (lambda (&rest xs) f))))))",
            "t",
        ),
        (
            "",
            "(let ((x (list 'unknown (list 1)))) (eq x (macroexpand x)))",
            "t",
        ),
        (
            "",
            "(let ((x (list 1))) (eq x (macroexpand (list 'mx-id x) '((mx-id . (lambda (x) x))))))",
            "t",
        ),
        (
            "",
            "(condition-case e (macroexpand '(mx-bad) '((mx-bad . 42))) (error e))",
            "(invalid-function 42)",
        ),
        (
            "",
            "(condition-case e (macroexpand '(unknown) 'bad) (error e))",
            "(wrong-type-argument listp bad)",
        ),
        (
            "",
            "(condition-case e (macroexpand '(unknown) '((other) . bad)) (error e))",
            "(wrong-type-argument listp ((other) . bad))",
        ),
        (
            "",
            "(condition-case e (macroexpand '(mx-error) '((mx-error . (lambda () (signal 'arith-error '(payload)))))) (error e))",
            "(arith-error payload)",
        ),
        (
            "",
            "(catch 'done (macroexpand '(mx-throw) '((mx-throw . (lambda () (throw 'done 93))))))",
            "93",
        ),
        (
            "",
            "(condition-case e (funcall 'macroexpand) (error (list (car e) (functionp (car (cdr e))) (car (cdr (cdr e))))))",
            "(wrong-number-of-arguments t 0)",
        ),
        (
            "",
            "(condition-case e (macroexpand '(mx-dot . 7) '((mx-dot . (lambda (&rest xs) xs)))) (error e))",
            "(wrong-type-argument listp 7)",
        ),
        (
            "",
            "(macroexpand '((lambda (x) x) 1) 'bad)",
            "((lambda (x) x) 1)",
        ),
    ];

    #[test]
    fn macroexpand_gnu_contract() {
        for &(setup, expression, expected) in CASES {
            check(setup, expression, expected);
        }
    }

    #[test]
    fn macroexpand_autoload_and_shadow() {
        let dir = std::path::PathBuf::from(std::env::var("TMPDIR").unwrap())
            .join(format!("macroexpand-autoload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mx-load.el"), "(defmacro mx-loaded (x) (list 'quote (list 'loaded x)))\n(defalias 'mx-auto 'mx-loaded)\n").unwrap();
        let mut vm = Vm::new();
        vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
        let setup =
            "(autoload 'mx-auto \"mx-load\" nil nil 'macro) (defalias 'mx-auto-alias 'mx-auto)";
        let expression = "(list (macroexpand '(mx-auto-alias 9) '((mx-auto))) (car (symbol-function 'mx-auto)) (macroexpand '(mx-auto-alias 9)) (macroexpand '(mx-auto-alias 10)))";
        vm.eval_source_raw(setup).unwrap();
        let value = vm.eval_source_raw(expression).unwrap();
        let expected = "((mx-auto-alias 9) autoload (quote (loaded 9)) (quote (loaded 10)))";
        assert_eq!(vm.display(value).unwrap(), expected);
        if let Ok(oracle) = std::env::var("ROMACS_GNU_ORACLE") {
            let source = format!(
                "(let ((print-quoted nil) (load-path (cons {:?} load-path))) {setup} (prin1 {expression}))",
                dir.to_str().unwrap()
            );
            let output = std::process::Command::new(oracle)
                .args(["--batch", "-Q", "--eval", &source])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn macroexpand_live_pin() {
        let Ok(oracle) = std::env::var("ROMACS_GNU_ORACLE") else {
            return;
        };
        assert!(
            oracle.contains("45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"),
            "wrong GNU pin: {oracle}"
        );
        for &(setup, expression, expected) in CASES {
            // Compare structure without GNU's quote-printing shorthand.
            let output = std::process::Command::new(&oracle)
                .args([
                    "--batch",
                    "-Q",
                    "--eval",
                    &format!("(let ((print-quoted nil)) {setup} (prin1 {expression}))"),
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{expression}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                expected,
                "{expression}"
            );
        }
    }
}
