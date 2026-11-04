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
        let image = unsafe { ImageRegion::open(&path, &key) }.unwrap();
        let (mut loaded, entry) = Vm::from_code_image(image, [93; 32]).unwrap();
        loaded.collect(&mut []);
        assert_eq!(loaded.execute_raw(entry).unwrap().as_integer().unwrap(), 7);
        let f = loaded.slots[loaded.names["image-document"]].function;
        assert!(loaded.closures[f.index(4).unwrap()].metadata.is_empty());
        let f = loaded.slots[loaded.names["image-string-document"]].function;
        let document = loaded.closures[f.index(4).unwrap()].document;
        assert!(!Vm::image_value(document));
        assert_eq!(loaded.str_view(document).unwrap().bytes(), "κείμενο".as_bytes());
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
        let image = unsafe { ImageRegion::open(&path, &key) }.unwrap();
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
    fn ordinary_closures_remain_opaque_in_both_regimes() {
        for lazy in [false, true] {
            let mut vm = Vm::new();
            vm.lazy_regime = lazy;
            for source in [
                "(lambda () 1)",
                "(lambda () (:documentation 'kind) 1)",
                "(lambda () (interactive \"p\") 1)",
            ] {
                let value = vm.eval_source_raw(&format!("(let ((f {source})) (list (condition-case nil (length f) (wrong-type-argument 'opaque)) (condition-case nil (aref f 1) (wrong-type-argument 'opaque)) (funcall f)))")).unwrap();
                assert_eq!(vm.display(value).unwrap(), "(opaque opaque 1)");
            }
        }
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

impl Vm {
    pub(super) fn read_capture_cell(&self, index: usize) -> Value {
        self.cell_pairs
            .get(&index)
            .map_or(self.cells[index], |&pair| {
                self.cons_pair(pair).expect("live lexical pair").1
            })
    }
    pub(super) fn write_capture_cell(&mut self, index: usize, value: Value) {
        self.cells[index] = value;
        if let Some(&pair) = self.cell_pairs.get(&index) {
            self.conses[pair.index(3).expect("private lexical pair")].1 = value;
        }
    }
    fn lexical_pair(&mut self, name: Value, capture: Value) -> Value {
        if capture.tag() == 5 {
            let index = capture.payload() as usize;
            if let Some(&pair) = self.cell_pairs.get(&index) {
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
        let mut a = self.stack[start..start + n].to_vec();
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
                self.closures[index].metadata = metadata;
                self.closures[index].document = a[4];
                Ok(a[0])
            }
            Primitive::ClosureDocument => {
                if !(2..=3).contains(&n) {
                    return Err("wrong primitive arity".into());
                }
                let closure = &mut self.closures[a[0].index(4)?];
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
                    let cell = if let Some((&index, _)) = self
                        .cell_pairs
                        .iter()
                        .find(|(_, existing)| **existing == pair)
                    {
                        index
                    } else {
                        let index = self.cells.len();
                        self.cells.push(value);
                        self.cell_pairs.insert(index, pair);
                        index
                    };
                    outer.insert(
                        name.clone(),
                        Binding {
                            reg: outer.len(),
                            cell: true,
                        },
                    );
                    values.insert(name, Value::reference(cell, 5));
                }
                let (function, names) = if self.lazy_regime || self.source_load_active() {
                let (arity, required, rest) = lambda_arity(&args)?;
                let mut names: Vec<_> = outer.keys().cloned().collect();
                names.sort();
                let lazy = LazyBody {
                    body: a[1],
                    args,
                    names: names.clone(),
                    cells: vec![true; names.len()],
                    local_specials: specials.clone(),
                    pending_specials: self.pending_specials.clone(),
                    dynamic_scope: a[2] == Value::NIL,
                    deferred: false,
                    file_level: false,
                    // GNU's interpreted closure evaluates the structure it holds.
                    evaluated: true,
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
                    lazy: Some(Arc::new(lazy)),
                    pending: None,
                    dynamic: names.is_empty().then_some(a[2] == Value::NIL),
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
                let result = Value::reference(self.closures.len(), 4);
                self.closures.push(Closure {
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
                        let thunk = Value::reference(self.closures.len(), 4);
                        self.closures.push(Closure {
                            function,
                            captures,
                            metadata: vec![Value::NIL, body, a[2]],
                            document: Value::NIL,
                        });
                        thunk
                    };
                    let slot = self.slot(prim::INTERACTIVE_TABLE);
                    if self.slots[slot].value == Value::UNBOUND {
                        self.slots[slot].value = self.allocate_object(data::Object::HashTable(
                            Box::new(hash::HashTable::new_local_bindings()),
                        ));
                    }
                    let table = self.slots[slot].value;
                    let entry = self.keymap_cons(slot_value, thunk);
                    self.declaration_hash_put(table, result, entry)?;
                }
                Ok(result)
            }
            _ => unreachable!(),
        }
    }
}
