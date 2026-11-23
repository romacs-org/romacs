//! GNU interpreted-closure slots and the lexical-environment/capture-cell bridge.
#[cfg(test)]
mod tests {
    use super::*;
    fn check(vm: &mut Vm, source: &str, expected: &str) {
        vm.lazy_regime = true;
        let value = vm.eval_source_raw(source).unwrap();
        assert_eq!(vm.display(value).unwrap(), expected, "{source}");
    }
    #[test]
    fn ordinary_document_is_traced_without_exposing_metadata() {
        for lazy in [false, true] {
            let mut vm = Vm::new();
            vm.lazy_regime = lazy;
            vm.eval_source_raw("(make-vector 20 nil) (defalias 'private-document (lambda () (:documentation (copy-sequence \"κείμενο\")) 7))").unwrap();
            vm.collect(&mut []);
            let value = vm.slots[vm.names["private-document"]].function;
            let closure = &vm.closures[value.index(4).unwrap()];
            assert!(closure.metadata.is_empty());
            assert_eq!(vm.str_view(closure.document).unwrap().bytes(), "κείμενο".as_bytes());
        }
    }
    #[test]
    fn ordinary_document_survives_a_code_image() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defalias 'image-document (lambda () (:documentation 'image-kind) 7)) (defalias 'image-string-document (lambda () \"κείμενο\" 8))").unwrap();
        let entry = vm.compile_source("(if (eq (romacs--oclosure-type (symbol-function 'image-document)) 'image-kind) (image-document) -1)").unwrap();
        let path = std::env::temp_dir().join(format!("romacs-document-image-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let key = vm.publish_code_image(&path, entry, [93; 32]).unwrap();
        let image = ImageRegion::open(&path, &key).unwrap();
        let (mut loaded, entry) = Vm::from_code_image(image, [93; 32]).unwrap();
        loaded.collect(&mut []);
        assert_eq!(loaded.execute_raw(entry).unwrap().as_integer().unwrap(), 7);
        let f = loaded.slots[loaded.names["image-document"]].function;
        assert!(loaded.closures[f.index(4).unwrap()].metadata.is_empty());
        let f = loaded.slots[loaded.names["image-string-document"]].function;
        let document = loaded.closures[f.index(4).unwrap()].document;
        assert!(!Vm::image_value(document));
        assert_eq!(loaded.str_view(document).unwrap().bytes(), "κείμενο".as_bytes());
        assert_eq!(loaded.closure_slot(f, 0).unwrap_err(), "array required");
        assert_eq!(loaded.closure_slot(f, 2).unwrap_err(), "array required");
        assert_eq!(loaded.closure_length(f).unwrap(), 5);
        assert_eq!(loaded.closure_slot(f, 4).unwrap(), document);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn interpreted_metadata_and_document_evaluation() {
        let mut vm = Vm::new();
        check(
            &mut vm,
            "(let ((f (make-interpreted-closure '(a) '((+ a x)) '((x . 7) t) 'kind nil))) (list (length f) (aref f 0) (aref f 1) (aref f 2) (aref f 3) (aref f 4) (funcall f 5)))",
            "(5 (a) ((+ a x)) ((x . 7) t) nil kind 12)",
        );
        check(
            &mut vm,
            "(let ((n 0)) (let ((f (make-interpreted-closure nil '(n) (list (cons 'n 1) t) (progn (setq n (1+ n)) 'kind) nil))) (list n (funcall f) (aref f 4))))",
            "(1 1 kind)",
        );
    }
    #[test]
    fn interpreted_env_mutations_and_reconstruction_share_cells() {
        let mut vm = Vm::new();
        check(
            &mut vm,
            "(progn (setq closure-test (let ((env (list (cons 'x 7) t))) (list (make-interpreted-closure nil '(x) env 'kind nil) (make-interpreted-closure '(v) '((setq x v)) env nil nil)))) t)",
            "t",
        );
        check(
            &mut vm,
            "(let* ((f (car closure-test)) (g (make-interpreted-closure (aref f 0) (aref f 1) (aref f 2) (aref f 4) nil))) (setcdr (car (aref f 2)) 9) (list (funcall f) (funcall g) (funcall (car (cdr closure-test)) 12) (funcall g)))",
            "(9 9 12 12)",
        );
        vm.collect(&mut []);
        check(
            &mut vm,
            "(let ((f (car closure-test))) (setcdr (car (aref f 2)) 31) (funcall f))",
            "31",
        );
        check(
            &mut vm,
            "(let* ((f (car closure-test)) (g (make-interpreted-closure (aref f 0) (aref f 1) (cons (cons 'x 99) (cdr (aref f 2))) (aref f 4) nil))) (list (funcall f) (funcall g) (aref g 4)))",
            "(31 99 kind)",
        );
        check(
            &mut vm,
            "(progn (setq gc-closure (make-interpreted-closure nil '(x) (list (cons 'x (cons 'old nil)) t) nil nil)) (setcdr (car (aref gc-closure 2)) (cons 'new nil)) t)",
            "t",
        );
        vm.collect(&mut []);
        check(&mut vm, "(funcall gc-closure)", "(new)");
    }
    /// A closure made from another's environment shares its cells, found by
    /// pair (romacs#367): through a compacting full that moves both after
    /// garbage, a minor, a sweep in place, the image a restore makes, and a
    /// dead pair's entry reused by a fresh one.
    #[test]
    fn a_closure_made_from_an_environment_shares_its_cells_through_collections() {
        let mut vm = Vm::new();
        vm.set_gc_verify(true);
        // COUNTER counts in X; (twin F) is a closure over F's environment, as
        // nadvice makes one: both from explicit interpreted-closure metadata.
        check(&mut vm, "(progn (defalias 'twin (lambda (f) (make-interpreted-closure '() '((setq x (1+ x))) (aref f 2) nil nil))) \
            (setq garbage (make-list 500 'g)) (setq counter (make-interpreted-closure '() '((setq x (1+ x))) (list (cons 'x 0) t) nil nil)) (setq garbage nil) t)", "t");
        let shared = "(let ((g (twin counter))) (list (funcall counter) (funcall g) (funcall counter) (eq (car (aref g 2)) (car (aref counter 2)))))";
        check(&mut vm, shared, "(1 2 3 t)");
        vm.allocation.compact = true;
        vm.collect(&mut []);
        check(&mut vm, shared, "(4 5 6 t)");
        assert!(vm.collect_minor(&[]));
        check(&mut vm, shared, "(7 8 9 t)");
        vm.collect(&mut []);
        check(&mut vm, shared, "(10 11 12 t)");
        // The image a restore makes: dense, then permanent.
        vm.allocation.compact = true;
        vm.collect(&mut []);
        vm.make_image_permanent();
        vm.check_cell_pairs();
        check(&mut vm, shared, "(13 14 15 t)");
        vm.collect(&mut []);
        check(&mut vm, shared, "(16 17 18 t)");
        // A closure dead with its cell: its pair goes, and the cons that takes
        // the pair's entry is a new pair, with a cell of its own.
        for _ in 0..3 {
            check(&mut vm, "(progn (setq dying (make-interpreted-closure '() '(y) (list (cons 'y 1) t) nil nil)) (setq dying-env (car (aref dying 2))) (setq dying nil dying-env nil) t)", "t");
            vm.collect(&mut []);
            check(&mut vm, "(let* ((f (make-interpreted-closure '() '(y) (list (cons 'y 40) t) nil nil)) (g (make-interpreted-closure '() '((setq y (1+ y))) (aref f 2) nil nil))) (list (funcall g) (funcall f)))", "(41 41)");
            vm.check_cell_pairs();
        }
    }
    #[test]
    fn interpreted_interactive_and_image_metadata() {
        let mut vm = Vm::new();
        check(
            &mut vm,
            "(let ((f (make-interpreted-closure nil '(7) '(t) 'kind '(interactive \"p\")))) (list (length f) (aref f 4) (aref f 5)))",
            "(6 kind \"p\")",
        );
        check(
            &mut vm,
            "(let ((f (make-interpreted-closure '(x) '(x) '(t) 'kind '(interactive \"p\")))) (list (interactive-form f) (funcall f 12) (aref f 5)))",
            "((interactive \"p\") 12 \"p\")",
        );
        let mut source = Vm::new();
        source
            .eval_source_raw(
                "(defalias 'frozen-closure (make-interpreted-closure '(x) '((+ x 1)) '(t) 'kind nil))",
            )
            .unwrap();
        let entry = source.compile_source("(frozen-closure 8)").unwrap();
        let path = std::env::temp_dir().join(format!("closure-image-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let key = source.publish_code_image(&path, entry, [37; 32]).unwrap();
        let image = ImageRegion::open(&path, &key).unwrap();
        let (mut loaded, entry) = Vm::from_code_image(image, [37; 32]).unwrap();
        loaded.collect(&mut []);
        assert_eq!(loaded.execute_raw(entry).unwrap().as_integer().unwrap(), 9);
        check(
            &mut loaded,
            "(let ((f (symbol-function 'frozen-closure))) (list (aref f 0) (aref f 1) (aref f 2) (aref f 4)))",
            "((x) ((+ x 1)) (t) kind)",
        );
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn eager_source_load_defers_constant_errors_in_uncalled_bodies() {
        let directory = std::env::temp_dir().join(format!("romacs-load-constant-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("constant.el");
        std::fs::write(&path, "(defun loaded-constant () (let ((t nil)) 7))\n(setq loaded-after-constant t)\n").unwrap();
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun internal-macroexpand-for-load (form &rest ignored) form)").unwrap();
        vm.load_file_raw(&path).unwrap();
        check(&mut vm, "loaded-after-constant", "t");
        let error = vm.eval_source_raw("(condition-case e (loaded-constant) (error e))").unwrap();
        assert!(vm.display(error).unwrap().contains("constant"));
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn explicit_interpreted_body_is_compiled_only_when_called() {
        let mut vm = Vm::new();
        vm.lazy_regime = true;
        check(&mut vm, "(progn (setq deferred-constant (make-interpreted-closure nil '((let ((t nil)) 7)) '(t) nil nil)) t)", "t");
        let error = vm.eval_source_raw("(condition-case e (funcall deferred-constant) (error e))").unwrap();
        assert!(vm.display(error).unwrap().contains("constant"));
    }
    #[test]
    fn interpreted_closures_are_not_mutable_vectors() {
        let mut vm = Vm::new();
        check(
            &mut vm,
            "(let ((f (make-interpreted-closure nil '(1) '(t) nil nil))) (list (length f) (condition-case nil (copy-sequence f) (wrong-type-argument 'rejected)) (condition-case nil (aset f 0 nil) (wrong-type-argument 'rejected))))",
            "(3 rejected rejected)",
        );
    }
    #[test]
    fn closure_error_data_prints_without_exposing_or_forcing_the_body() {
        for lazy in [false, true] {
            for (factory, ordinary) in [
                ("(lambda () 7)", true),
                ("(make-interpreted-closure nil '(7) '(t) nil nil)", false),
            ] {
                let mut vm = Vm::new();
                vm.lazy_regime = lazy;
                vm.eval_source_raw(&format!("(setq diagnostic-function {factory})"))
                    .unwrap();
                vm.collect(&mut []);
                let slot = vm.names["diagnostic-function"];
                let function = vm.slots[slot].value;
                let closure = &vm.closures[function.index(4).unwrap()];
                let code = closure.function;
                let metadata = closure.metadata.clone();
                let body = vm.functions[code].lazy.as_ref().map(|body| body.body);
                assert_eq!(body.is_some(), lazy);
                assert_eq!(metadata.is_empty(), ordinary);

                let data = vm.eval_source_raw(
                    "(condition-case e (car diagnostic-function) (error e))",
                ).unwrap();
                let expected = "(wrong-type-argument listp #<interpreted-function>)";
                assert_eq!(vm.display(data).unwrap(), expected);
                assert_eq!(vm.display(function).unwrap(), "#<interpreted-function>");
                for printer in ["(format \"%S\" e)", "(prin1-to-string e)"] {
                    let text = vm.eval_source_raw(&format!(
                        "(let ((print-circle t)) (condition-case e (car diagnostic-function) (error {printer})))",
                    )).unwrap();
                    assert_eq!(vm.str_view(text).unwrap().bytes(), expected.as_bytes());
                }
                assert_eq!(vm.closures[function.index(4).unwrap()].metadata, metadata);
                assert_eq!(vm.functions[code].lazy.as_ref().map(|body| body.body), body);
                let result = vm.eval_source_raw("(funcall diagnostic-function)").unwrap();
                assert_eq!(result.as_integer().unwrap(), 7);
            }
        }
    }

    #[test]
    fn ordinary_closure_slots_in_both_regimes() {
        for lazy in [false, true] {
            let mut vm = Vm::new();
            vm.lazy_regime = lazy;
            let value = vm.eval_source_raw(r#"
                (setq slot-test (let ((y 5))
                  (lambda (x) "doc" (interactive (list y)) (+ x y))))"#).unwrap();
            assert_eq!(vm.closure_length(value).unwrap(), 6);
            for (index, expected) in [
                (0, "(x)"), (2, "((y . 5))"), (3, "nil"), (4, "\"doc\""), (5, "(list y)"),
            ] {
                let slot = vm.closure_slot(value, index).unwrap();
                assert_eq!(vm.display(slot).unwrap(), expected);
            }
            if lazy {
                let body = vm.closure_slot(value, 1).unwrap();
                assert_eq!(vm.display(body).unwrap(), "((+ x y))");
                assert_eq!(vm.eval_source_raw("(funcall slot-test 2)").unwrap().as_integer().unwrap(), 7);
            }
            assert_eq!(vm.closure_slot(value, 1).unwrap_err(), "array required");
            assert_eq!(vm.closure_slot(value, 6).unwrap_err(), "index out of range");
        }
    }

    #[test]
    fn ordinary_closure_environment_pair_shares_its_cell_across_collection() {
        for lazy in [false, true] {
            let mut vm = Vm::new();
            vm.lazy_regime = lazy;
            vm.set_gc_verify(true);
            vm.allocation.compact = true;
            let created = vm.eval_source_raw("
                (let ((y (list 5)))
                  (setq slot-counter (lambda () (setq y (list (1+ (car y)))))
                        slot-env (aref slot-counter 2)))
                (assq 'y slot-env)").unwrap();
            assert_eq!(vm.display(created).unwrap(), "(y 5)");
            vm.collect(&mut []);
            let result = vm.eval_source_raw("
                (setcdr (assq 'y slot-env) (list 40))
                (list (funcall slot-counter)
                      (cdr (assq 'y slot-env))
                      (eq (assq 'y slot-env) (assq 'y (aref slot-counter 2))))").unwrap();
            assert_eq!(vm.display(result).unwrap(), "((41) (41) t)");
        }
    }

    #[test]
    fn a_forwarded_capture_follows_the_lexical_shadow() {
        let mut vm = Vm::new();
        let layout = CaptureLayout {
            slots: vec![
                CaptureSlot { name: None, binding: CaptureBinding::Late { shadow: Some(1) } },
                CaptureSlot { name: None, binding: CaptureBinding::Late { shadow: Some(2) } },
                CaptureSlot { name: None, binding: CaptureBinding::Lexical },
            ],
            specials: vec![],
            bare_context: false,
        };
        let forwarded = Value::reference(vm.alloc_cell(Value::UNBOUND), 5);
        let lexical = Value::reference(vm.alloc_cell(Value::TRUE), 5);
        assert_eq!(vm.lexical_capture(&layout, &[forwarded, forwarded, lexical], 0), Some(lexical));
        assert_eq!(vm.lexical_capture(&layout, &[lexical, forwarded, Value::NIL], 0), Some(lexical));
    }

    #[test]
    fn ordinary_closure_environment_keeps_an_uninterned_name_alive() {
        let mut vm = Vm::new();
        vm.eval_source_raw("
            (setq uninterned-capture
                  (let ((name (make-symbol \"local-name\")))
                    (eval `(let ((,name 12)) (lambda () ,name)) t)))").unwrap();
        vm.collect(&mut []);
        let result = vm.eval_source_raw("(symbol-name (car (car (aref uninterned-capture 2))))").unwrap();
        assert_eq!(vm.display(result).unwrap(), "\"local-name\"");
    }

    #[test]
    fn ordinary_closures_answer_their_lambda_list() {
        for lazy in [false, true] {
            let mut vm = Vm::new();
            vm.lazy_regime = lazy;
            check(
                &mut vm,
                "(progn (defalias 'listed (lambda (a &optional b &rest c) (list a b c))) (list (aref (symbol-function 'listed) 0) (aref (let ((y 1)) (lambda (x) (+ x y))) 0) (aref (lambda () 1) 0) (listed 1)))",
                "((a &optional b &rest c) (x) nil (1 nil nil))",
            );
            // A parameter only the lambda list names survives collection.
            vm.eval_source_raw("(setq uninterned-parameter (eval (list 'lambda (list (intern \"argument-name\" (obarray-make))) 1) t)) (funcall uninterned-parameter 0)").unwrap();
            vm.collect(&mut []);
            check(&mut vm, "(symbol-name (car (aref uninterned-parameter 0)))", "\"argument-name\"");
        }
    }
}

use super::*;

#[derive(Clone, Debug, romacs_snapshot::Snapshot)]
pub(super) struct CaptureSlot {
    pub(super) name: Option<Name>,
    pub(super) binding: CaptureBinding,
}

#[derive(Clone, Copy, Debug, romacs_snapshot::Snapshot)]
pub(super) enum CaptureBinding {
    Lexical,
    Late { shadow: Option<usize> },
}

#[derive(Clone, Debug, romacs_snapshot::Snapshot)]
pub(super) struct CaptureLayout {
    pub(super) slots: Vec<CaptureSlot>,
    pub(super) specials: Vec<String>,
    /// GNU keeps untrimmed declarations and the lexical sentinel when
    /// no lexical variable was in view.
    pub(super) bare_context: bool,
}

/// What slot 2 shows of one closure: its named lexical captures, each the
/// value or cell it reads, and its local special declarations.
type ClosureBindings = (Vec<(Name, Value)>, Vec<String>);

fn ordinary_slot_count(document: Value, interactive: bool) -> usize {
    if interactive { 6 } else if document != Value::NIL { 5 } else { 3 }
}

impl Vm {
    pub(super) fn read_capture_cell(&self, index: usize) -> Value {
        self.cell_pairs
            .get(index)
            .map_or(self.cells[index], |pair| {
                self.cons_pair(pair).expect("live lexical pair").1
            })
    }
    pub(super) fn write_capture_cell(&mut self, index: usize, value: Value) {
        self.cells.set(index, value);
        if let Some(pair) = self.cell_pairs.get(index) {
            self.conses.set_cdr(pair.index(3).expect("private lexical pair"), value);
        }
    }
    pub(super) fn interactive_entry(&mut self, closure: Value) -> Option<(Value, Value)> {
        let slot = *self.names.get(prim::INTERACTIVE_TABLE)?;
        let table = self.slots[slot].value;
        let entry = self.declaration_hash_get(table, closure).ok()??;
        self.cons_pair(entry).ok()
    }

    pub(super) fn register_interactive_spec(&mut self, closure: Value, spec: Value, thunk: Value) -> Result<()> {
        let slot = self.slot(prim::INTERACTIVE_TABLE);
        if self.slots[slot].value == Value::UNBOUND {
            // The key-weak table sheds capacity as well as dead closures.
            let table = Box::new(hash::HashTable::new_local_bindings());
            self.slots[slot].value = self.allocate_object(data::Object::HashTable(table));
        }
        let table = self.slots[slot].value;
        let entry = self.keymap_cons(spec, thunk);
        self.declaration_hash_put(table, closure, entry)
    }

    pub(super) fn capture_layout(
        &self, names: &[Name], outer: &NameMap<Name, Binding>, specials: &NameSet<String>,
        code: Option<&[Op]>,
    ) -> CaptureLayout {
        let slots = names.iter().map(|name| {
            let exposed = !matches!(name, Name::Generated(id) if !self.generated_symbols.contains_key(id));
            let binding = outer[name];
            let binding = if binding.late {
                let shadow = binding.shadow.and_then(|id| names.binary_search(&Name::Generated(id)).ok());
                CaptureBinding::Late { shadow }
            } else {
                CaptureBinding::Lexical
            };
            CaptureSlot { name: exposed.then(|| name.clone()), binding }
        }).collect();
        let bare_context = outer.is_empty();
        let specials = self.closure_specials(specials, if bare_context { None } else { code });
        CaptureLayout { slots, specials, bare_context }
    }

    fn closure_specials(&self, in_scope: &NameSet<String>, code: Option<&[Op]>) -> Vec<String> {
        if in_scope.is_empty() {
            return Vec::new();
        }
        let mut used: NameSet<&str> = NameSet::default();
        if let Some(code) = code {
            for op in code {
                match *op {
                    Op::Bind(slot, _) => { used.insert(&self.symbol_names[slot]); }
                    Op::Closure(_, id, _) => {
                        if let Some(layout) = &self.functions[id].environment {
                            used.extend(layout.specials.iter().map(String::as_str));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut specials: Vec<_> = in_scope.iter().filter(|name| {
            (code.is_none() || used.contains(name.as_str()))
                && self.names.get(name).is_none_or(|&slot| !self.slot_is_special(slot))
        }).cloned().collect();
        specials.sort_unstable();
        specials
    }

    pub(super) fn closure_length(&mut self, value: Value) -> Result<usize> {
        let closure = &self.closures[value.index(4)?];
        if !closure.metadata.is_empty() {
            return Ok(closure.metadata.len());
        }
        let document = closure.document;
        Ok(ordinary_slot_count(document, self.interactive_entry(value).is_some()))
    }

    pub(super) fn closure_slot(&mut self, value: Value, index: usize) -> Result<Value> {
        let closure = &self.closures[value.index(4)?];
        let out_of_range = || "index out of range".into();
        if !closure.metadata.is_empty() {
            return closure.metadata.get(index).copied().ok_or_else(out_of_range);
        }
        let (function, document) = (closure.function, closure.document);
        let interactive = (index >= 2).then(|| self.interactive_entry(value)).flatten();
        if index >= ordinary_slot_count(document, interactive.is_some()) {
            return Err(out_of_range());
        }
        let function = &self.functions[function];
        let missing = || "array required".into();
        match index {
            0 => self.literal(&Form::List(function.params.clone().ok_or_else(missing)?)),
            1 => function.lazy.as_ref().map(|body| body.body).ok_or_else(missing),
            2 => self.closure_environment(value, interactive.map(|(_, thunk)| thunk)).ok_or_else(missing),
            3 => Ok(Value::NIL),
            4 => Ok(document),
            5 => interactive.map(|(spec, _)| spec).ok_or_else(out_of_range),
            _ => Err(out_of_range()),
        }
    }

    fn closure_environment(&mut self, value: Value, thunk: Option<Value>) -> Option<Value> {
        let function = &self.functions[self.closures[value.index_if(4)?].function];
        if function.dynamic? {
            return Some(Value::NIL);
        }
        let mut bare_context = function.environment.as_ref().is_some_and(|layout| layout.bare_context);
        let (mut entries, mut specials) = self.closure_bindings(value)?;
        if let Some(thunk) = thunk.filter(|&thunk| thunk != Value::NIL) {
            let (other_entries, other_specials) = self.closure_bindings(thunk)?;
            let named: NameSet<_> = entries.iter().map(|(name, _)| name.clone()).collect();
            entries.extend(other_entries.into_iter().filter(|(name, _)| !named.contains(name)));
            specials.extend(other_specials);
            specials.sort_unstable();
            specials.dedup();
        }
        bare_context |= entries.is_empty() && specials.is_empty();
        let mut env = if bare_context { self.list_value(&[Value::TRUE]) } else { Value::NIL };
        for name in specials.into_iter().rev() {
            let symbol = self.intern(&name);
            env = self.keymap_cons(symbol, env);
        }
        for (name, capture) in entries.into_iter().rev() {
            let symbol = Value::reference(self.name_slot(&name) + 1, 7);
            let pair = self.lexical_pair(symbol, capture);
            env = self.keymap_cons(pair, env);
        }
        Some(env)
    }

    fn closure_bindings(&self, value: Value) -> Option<ClosureBindings> {
        let closure = self.closures.get(value.index_if(4)?)?;
        let function = &self.functions[closure.function];
        let Some(layout) = &function.environment else {
            return closure.captures.is_empty().then(|| (Vec::new(), Vec::new()));
        };
        let entries = layout.slots.iter().enumerate().filter_map(|(index, slot)| {
            Some((slot.name.clone()?, self.lexical_capture(layout, &closure.captures, index)?))
        }).collect();
        Some((entries, layout.specials.clone()))
    }

    fn lexical_capture(&self, layout: &CaptureLayout, captures: &[Value], mut index: usize) -> Option<Value> {
        for _ in 0..captures.len() {
            let capture = *captures.get(index)?;
            let CaptureBinding::Late { shadow } = layout.slots.get(index)?.binding else {
                return Some(capture);
            };
            // A forwarded late cell holds a sentinel, never a lexical value.
            if capture.index_if(5).map_or(capture, |cell| self.read_capture_cell(cell)) != Value::UNBOUND {
                return Some(capture);
            }
            index = shadow?;
        }
        None
    }

    fn lexical_pair(&mut self, name: Value, capture: Value) -> Value {
        if capture.tag() == 5 {
            let index = capture.payload() as usize;
            if let Some(pair) = self.cell_pairs.get(index) {
                return pair;
            }
            let pair = self.keymap_cons(name, self.cells[index]);
            self.cell_pairs.insert(index, pair);
            pair
        } else {
            self.keymap_cons(name, capture)
        }
    }
    pub(super) fn closure_native_call(
        &mut self,
        b: &mut Builder,
        primitive: Primitive,
        args: &[usize],
    ) -> usize {
        let fun = b.reg();
        b.emit(Op::Const(fun, Value::reference(primitive as usize, 6)));
        let start = b.next;
        for &arg in args {
            let dst = b.reg();
            b.emit(Op::Move(dst, arg));
        }
        let result = b.reg();
        b.emit(Op::Call(result, fun, start, args.len()));
        result
    }

    pub(super) fn interpreted_closure_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        let a = self.stack[start..start + n].to_vec();
        self.interpreted_closure_values(p, a)
    }
    /// The closure primitives on their arguments A, already out of the file.
    pub(super) fn interpreted_closure_values(&mut self, p: Primitive, mut a: Vec<Value>) -> Result<Value> {
        let n = a.len();
        match p {
            Primitive::ClosureMetadata => {
                if n != 6 {
                    return Err("wrong primitive arity".into());
                }
                let index = a[0].index(4)?;
                let captures = self.closures[index].captures.clone();
                let mut pairs = Vec::new();
                for entry in self.list_values(a[3])? {
                    let (name, at) = self.cons_pair(entry)?;
                    let capture = *captures
                        .get(at.as_integer()? as usize)
                        .ok_or("invalid capture descriptor")?;
                    pairs.push(self.lexical_pair(name, capture));
                }
                let mut env = if a[5] == Value::NIL {
                    Value::NIL
                } else {
                    self.list_value(&[Value::TRUE])
                };
                for pair in pairs.into_iter().rev() {
                    env = self.keymap_cons(pair, env);
                }
                let mut metadata = vec![a[1], a[2], env];
                if a[4] != Value::NIL {
                    metadata.extend([Value::NIL, a[4]]);
                }
                let closure = self.closures.entry_mut(index);
                closure.metadata = metadata;
                closure.document = a[4];
                Ok(a[0])
            }
            Primitive::ClosureDocument => {
                if !(2..=3).contains(&n) {
                    return Err("wrong primitive arity".into());
                }
                let closure = self.closures.entry_mut(a[0].index(4)?);
                closure.document = a[1];
                if closure.metadata.is_empty() {
                    return Ok(a[0]);
                }
                if a[1] != Value::NIL {
                    if closure.metadata.len() < 5 {
                        closure.metadata.resize(5, Value::NIL);
                    }
                    closure.metadata[4] = a[1];
                }
                if n == 3 {
                    closure.metadata.resize(6, Value::NIL);
                    closure.metadata[5] = a[2];
                }
                Ok(a[0])
            }
            Primitive::MakeInterpretedClosure => {
                // eval.c:525: ARGS BODY ENV &optional DOCSTRING IFORM.
                if !(3..=5).contains(&n) {
                    return Err("wrong primitive arity".into());
                }
                a.resize(5, Value::NIL);
                let args = self
                    .list_values(a[0])?
                    .into_iter()
                    .map(|v| self.expansion_form(v, 0))
                    .collect::<Result<Vec<_>>>()?;
                self.list_values(a[1])?;
                let mut outer = NameMap::default();
                let mut values = NameMap::default();
                let mut specials = NameSet::default();
                for pair in self.list_values(a[2])? {
                    if pair.tag() != 3 {
                        if pair != Value::TRUE {
                            if let Ok(name) = self.expansion_form(pair, 0).and_then(|f| f.name()) {
                                if let Some(s) = name.source() {
                                    specials.insert(s.to_owned());
                                }
                            }
                        }
                        continue;
                    }
                    let (symbol, value) = self.cons_pair(pair)?;
                    let name = self.expansion_form(symbol, 0)?.name()?;
                    if outer.contains_key(&name) {
                        continue;
                    }
                    let cell = if let Some(index) = self.cell_pairs.cell_of(pair) {
                        index
                    } else {
                        let index = self.alloc_cell(value);
                        self.cell_pairs.insert(index, pair);
                        index
                    };
                    outer.insert(
                        name.clone(),
                        Binding {
                            reg: outer.len(),
                            cell: true,
                            late: false,
                            shadow: None,
                        },
                    );
                    values.insert(name, Value::reference(cell, 5));
                }
                let key_args = args.clone();
                let (function, names) = if self.lazy_regime || self.source_load_active() {
                let (arity, required, rest) = lambda_arity(&args)?;
                let mut names: Vec<_> = outer.keys().cloned().collect();
                names.sort();
                let lazy = LazyBody {
                    body: a[1],
                    args,
                    names: names.clone(),
                    cells: vec![true; names.len()],
                    lates: vec![false; names.len()],
                    shadows: vec![None; names.len()],
                    local_specials: specials.clone(),
                    pending_specials: self.pending_specials.clone(),
                    dynamic_scope: a[2] == Value::NIL,
                    deferred: false,
                    file_level: false,
                    // GNU's interpreted closure evaluates the structure it holds.
                    evaluated: true,
                    interpreting: false,
                };
                let function = self.install_function(Function {
                    capture_mutations: vec![true; names.len()],
                    mapped_code: None,
                    code: vec![],
                    regs: names.len() + arity + usize::from(rest),
                    captures: names.len(),
                    arity,
                    required,
                    rest,
                    sites: vec![],
                    params: Some(lazy.args.clone()),
                    key: None,
                    lazy: Some(Arc::new(lazy)),
                    pending: None,
                    dynamic: Some(a[2] == Value::NIL),
                    environment: Some(self.capture_layout(&names, &outer, &specials, None)),
                });
                (function, names)
                } else {
                    let body = self.list_values(a[1])?.into_iter()
                        .map(|v| self.expansion_form(v, 0)).collect::<Result<Vec<_>>>()?;
                    let saved_scope = std::mem::replace(&mut self.dynamic_scope, a[2] == Value::NIL);
                    let saved_captures = std::mem::replace(&mut self.keep_captures, true);
                    let result = self.function(&args, &body, &outer, &specials);
                    self.dynamic_scope = saved_scope;
                    self.keep_captures = saved_captures;
                    result?
                };
                // fns.c internal_equal compares the closure's interactive
                // spec too, a slot of its own here: the key takes it beside
                // the body, whichever way it compiled (`equal' compares the
                // docstring, its other slot, apart).
                let key = crate::closure_key::of_lazy(self, &crate::closure_key::Context { dynamic: a[2] == Value::NIL, names: &names, specials: &specials }, &key_args, a[1], a[4]);
                Arc::make_mut(self.functions.change(function)).key = key;
                let captures = names.iter().map(|name| values[name]).collect();
                let mut metadata = vec![a[0], a[1], a[2]];
                if a[3] != Value::NIL || a[4] != Value::NIL {
                    metadata.extend([Value::NIL, a[3]]);
                }
                if a[4] != Value::NIL {
                    // eval.c:539-544: SPEC, or [SPEC MODES] when MODES is non-nil.
                    let (_, tail) = self.cons_pair(a[4])?;
                    let (spec, modes) = if tail == Value::NIL {
                        (Value::NIL, Value::NIL)
                    } else {
                        self.cons_pair(tail)?
                    };
                    metadata.push(if modes == Value::NIL { spec } else { self.allocate_object(data::Object::Vector(vec![spec, modes])) });
                }
                let result = self.alloc_closure(Closure {
                    function,
                    captures,
                    metadata,
                    document: a[3],
                });
                if a[4] != Value::NIL {
                    let slot_value = self.closures[result.index(4)?].metadata[5];
                    let spec = match self.object_if(slot_value) {
                        Some(data::Object::Vector(v)) => v.first().copied().unwrap_or(slot_value),
                        _ => slot_value,
                    };
                    let thunk = if spec == Value::NIL || self.str_view(spec).is_some() {
                        Value::NIL
                    } else {
                        let form = self.expansion_form(spec, 0)?;
                        self.keep_captures = true;
                        let (function, names) = self.function(&[], &[form], &outer, &specials)?;
                        let captures = names.iter().map(|name| values[name]).collect();
                        let body = self.list_value(&[spec]);
                        let thunk = self.alloc_closure(Closure {
                            function,
                            captures,
                            metadata: vec![Value::NIL, body, a[2]],
                            document: Value::NIL,
                        });
                        thunk
                    };
                    self.register_interactive_spec(result, slot_value, thunk)?;
                }
                Ok(result)
            }
            _ => unreachable!(),
        }
    }
}
