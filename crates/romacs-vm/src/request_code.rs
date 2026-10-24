//! Request-owned code uses stable slots. Explicit compile handles remain pinned.
use super::*;

#[derive(Default)]
pub(super) struct RequestCode {
    pub(super) permanent: Vec<bool>,
    pub(super) image_caches: usize,
    pub(super) live: Vec<bool>,
    free: Vec<usize>,
    free_caches: Vec<usize>,
    compiling: bool,
    compiled: Vec<usize>,
    since_collection: usize,
}

impl Vm {
    pub(super) fn require_live_function(&self, function: usize) -> Result<()> {
        if self.request_code.live.get(function) == Some(&true) {
            Ok(())
        } else {
            Err("invalid function".into())
        }
    }
    pub(super) fn install_code(&mut self, function: Function) -> usize {
        let code = &mut self.request_code;
        let id = if code.compiling
            && let Some(id) = code.free.pop()
        {
            self.functions[id] = Arc::new(function);
            code.live[id] = true;
            id
        } else {
            let id = self.functions.len();
            self.functions.push(Arc::new(function));
            code.permanent.push(!code.compiling);
            code.live.push(true);
            id
        };
        if code.compiling {
            code.compiled.push(id);
            code.since_collection += 1;
        }
        id
    }

    pub(super) fn install_cache(&mut self) -> usize {
        let cache = Cache {
            version: 0,
            target: Value::UNBOUND,
        };
        if self.request_code.compiling
            && let Some(id) = self.request_code.free_caches.pop()
        {
            self.caches[id] = cache;
            id
        } else {
            let id = self.caches.len();
            self.caches.push(cache);
            id
        }
    }

    pub(super) fn compile_request(&mut self, form: &Form) -> Result<(usize, Vec<usize>)> {
        let previous = self.request_code.compiling;
        let start = self.request_code.compiled.len();
        self.request_code.compiling = true;
        let result = self.compile(form);
        if result.is_err() {
            self.request_code.since_collection += 1;
        }
        self.request_code.compiling = previous;
        let compiled = self.request_code.compiled[start..].to_vec();
        if !previous {
            self.request_code.compiled.clear();
        }
        result.map(|id| (id, compiled))
    }

    /// Compile a lazily held body (R215) as request-owned code, so its nested
    /// closures are retired by the collector like any other request's.
    pub(super) fn compile_lazy_request(
        &mut self,
        args: &[Form],
        body: &[Form],
        outer: &HashMap<Name, Binding>,
        local_specials: &HashSet<String>,
    ) -> Result<(usize, Vec<Name>)> {
        let previous = self.request_code.compiling;
        self.request_code.compiling = true;
        let result = self.function(args, body, outer, local_specials);
        if result.is_err() {
            self.request_code.since_collection += 1;
        }
        self.request_code.compiling = previous;
        if !previous {
            self.request_code.compiled.clear();
        }
        result
    }

    /// Move a freshly installed function out of its slot and free the slot.
    /// The moved code keeps its child function IDs and cache ordinals.
    pub(super) fn take_compiled_function(&mut self, id: usize) -> Function {
        let function = std::mem::replace(
            Arc::get_mut(&mut self.functions[id]).expect("fresh code is private"),
            Function {
                capture_mutations: Vec::new(),
                mapped_code: None,
                code: Vec::new(),
                regs: 0,
                captures: 0,
                arity: 0,
                required: 0,
                rest: false,
                sites: Vec::new(),
                lazy: None,
            },
        );
        if self.request_code.live[id] && !self.request_code.permanent[id] {
            self.request_code.live[id] = false;
            self.request_code.free.push(id);
        }
        function
    }

    pub(super) fn reserve_request_artifact(
        &mut self,
        functions: &[Function],
        caches: usize,
    ) -> (Vec<usize>, Vec<usize>) {
        let previous = self.request_code.compiling;
        self.request_code.compiling = true;
        let ids = functions
            .iter()
            .map(|f| self.install_function(f.clone()))
            .collect();
        let cache_ids = (0..caches).map(|_| self.install_cache()).collect();
        self.request_code.compiling = previous;
        if !previous {
            self.request_code.compiled.clear();
        }
        (ids, cache_ids)
    }

    /// Evaluate a single-use form. Unlike `compile`, this does not create a
    /// permanent executable handle.
    ///
    /// This call may mark/compact even after a short request, including on error.
    /// Only the returned `Value` is rewritten. Inside the crate, any other raw
    /// heap `Value` must be passed to `execute_rooted`/`collect` as a root or not
    /// reused; embedders hold `Local`/`Rooted` (handle.rs) and cannot get one.
    pub(crate) fn eval_form_raw(&mut self, form: &Form) -> Result<Value> {
        let outcome = self.compile_request(form).and_then(|(id, _)| {
            let outcome = self.execute_raw(id);
            // A top-level request entry is never wrapped in a Closure. Its
            // children may escape, but the entry itself has a single owner.
            self.release_request_entry(id);
            outcome
        });
        self.finish_request(outcome)
    }

    pub(super) fn release_request_entry(&mut self, id: usize) {
        for op in &self.functions[id].code {
            if let Op::Direct(_, _, cache, _, _) | Op::Prepared(_, _, _, cache, _, _) = *op {
                self.caches[cache] = Cache {
                    version: 0,
                    target: Value::UNBOUND,
                };
                self.request_code.free_caches.push(cache);
            }
        }
        let f = Arc::get_mut(&mut self.functions[id]).expect("request entry is private");
        f.code = Vec::new();
        f.sites = Vec::new();
        f.capture_mutations = Vec::new();
        self.request_code.live[id] = false;
        self.request_code.free.push(id);
        self.execution.release_entry(id);
    }

    pub(super) fn finish_request(&mut self, mut outcome: Result<Value>) -> Result<Value> {
        if self.eval_depth == 0 {
            self.propagated_escape = None;
            self.propagated_escape_hooked = false;
            self.propagated_eval_depth = None;
        }
        // Nested loads and macro expansion temporarily disable GC: builders and
        // suspended outer register stacks are not a request boundary.
        if self.gc_threshold != usize::MAX
            && self.stack.is_empty()
            && (self.heap_objects() >= self.gc_threshold
                || self.request_code.since_collection >= 1024)
        {
            match &mut outcome {
                Ok(value) => self.collect(std::slice::from_mut(value)),
                Err(_) => self.collect(&mut []),
            }
        }
        outcome
    }

    pub(super) fn retire_code(&mut self, marks: &[bool]) {
        let mut used_caches = vec![false; self.caches.len()];
        // Undecoded image bodies still own their original cache ordinals.
        used_caches[..self.request_code.image_caches].fill(true);
        for (id, &marked) in marks.iter().enumerate() {
            if !self.request_code.live[id] {
                continue;
            }
            if marked {
                for op in &self.functions[id].code {
                    if let Op::Direct(_, _, cache, _, _) | Op::Prepared(_, _, _, cache, _, _) = *op {
                        used_caches[cache] = true;
                    }
                }
            } else {
                let f = Arc::get_mut(&mut self.functions[id])
                    .expect("request code is private at a wait point");
                f.code = Vec::new();
                f.sites = Vec::new();
                f.capture_mutations = Vec::new();
                // An unreachable lazy body is garbage with its function.
                f.lazy = None;
                self.request_code.live[id] = false;
                self.request_code.free.push(id);
            }
        }
        self.request_code.free_caches.clear();
        for (id, used) in used_caches.into_iter().enumerate() {
            if !used {
                self.caches[id] = Cache {
                    version: 0,
                    target: Value::UNBOUND,
                };
                self.request_code.free_caches.push(id);
            }
        }
        self.execution.retire_code(marks);
        self.request_code.since_collection = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn form(text: &str) -> Form {
        Form::from_json(serde_json::from_str(text).unwrap()).unwrap()
    }
    #[test]
    fn retired_function_ids_are_rejected() {
        let name = "request_code::tests::retired_function_ids_are_rejected";
        if std::env::var("VM2_GC_PROBE_CHILD").as_deref() != Ok(name) {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--nocapture"])
                .env("VM2_GC_PROBE_CHILD", name)
                .status()
                .unwrap();
            assert!(status.success(), "retired-ID probe failed: {status}");
            return;
        }
        let mut vm = Vm::new();
        vm.eval_form_raw(&form(r#"["+",1,2]"#)).unwrap();
        let id = vm.request_code.live.iter().position(|live| !live).unwrap();
        assert_eq!(vm.execute_raw(id).unwrap_err().to_string(), "invalid function");
        assert_eq!(
            vm.execute_rooted(id, &mut []).unwrap_err().to_string(),
            "invalid function"
        );
        assert_eq!(
            threaded::execute_args(&mut vm, id, &[])
                .unwrap_err()
                .to_string(),
            "invalid function"
        );
        assert_eq!(vm.dump_ir(id).unwrap_err().to_string(), "invalid function");
        assert_eq!(
            vm.publish_code_image(std::path::Path::new("unused-store"), id, [0; 32])
                .unwrap_err()
                .to_string(),
            "invalid function"
        );
        assert_eq!(
            vm.execute_raw(usize::MAX).unwrap_err().to_string(),
            "invalid function"
        );
        let live = vm.compile(&form(r#"["+",20,22]"#)).unwrap();
        assert_eq!(vm.execute_raw(live).unwrap().as_integer().unwrap(), 42);
    }
    #[test]
    fn short_distinct_requests_collect_and_reuse_code_slots() {
        let mut vm = Vm::new();
        for n in 0..100_000 {
            let form = Form::List(vec![Form::Sym("+".into()), Form::Int(n), Form::Int(2)]);
            assert_eq!(vm.eval_form_raw(&form).unwrap().as_integer().unwrap(), n + 2);
        }
        assert!(vm.heap_stats().collections >= 97);
        assert!(vm.functions.len() <= 1024, "function high-water slots grow");
        assert!(vm.caches.len() <= 1024, "IR caches grow");
        vm.collect(&mut []);
        assert!(vm.request_code.live.iter().all(|live| !live));
        assert!(vm.execution.code_is_empty());
    }
    #[test]
    fn request_roots_preserve_definitions_closures_literals_and_warm_calls() {
        let mut vm = Vm::new();
        vm.set_collection_floor(64);
        vm.eval_source_raw("(defun target () '(41)) (defun caller () (car (target))) (setq keeper (let ((x '(42))) (lambda () (car x))))").unwrap();
        let pinned = vm.compile(&form(r#"["caller"]"#)).unwrap();
        assert_eq!(vm.execute_raw(pinned).unwrap().as_integer().unwrap(), 41);
        let garbage = form(r#"["progn",["cons",0,"nil"],0]"#);
        for _ in 0..100_000 {
            assert_eq!(vm.eval_form_raw(&garbage).unwrap().as_integer().unwrap(), 0);
        }
        vm.clear_source_cache();
        vm.collect(&mut []);
        assert_eq!(vm.execute_raw(pinned).unwrap().as_integer().unwrap(), 41);
        assert_eq!(
            vm.eval_source_raw("(funcall keeper)")
                .unwrap()
                .as_integer()
                .unwrap(),
            42
        );
        vm.eval_source_raw("(defun target () '(99))").unwrap();
        vm.collect(&mut []);
        assert_eq!(vm.execute_raw(pinned).unwrap().as_integer().unwrap(), 99);
        assert_eq!(
            vm.eval_source_raw("(catch 'tag (throw 'tag 17))")
                .unwrap()
                .as_integer()
                .unwrap(),
            17
        );
        assert_eq!(
            vm.eval_source_raw("(condition-case e (car 1) (error 23))")
                .unwrap()
                .as_integer()
                .unwrap(),
            23
        );
        vm.eval_source_raw("(setq timer-list (cons keeper nil))")
            .unwrap();
        vm.clear_source_cache();
        vm.collect(&mut []);
        assert_eq!(
            vm.eval_source_raw("(funcall (car timer-list))")
                .unwrap()
                .as_integer()
                .unwrap(),
            42
        );
        vm.eval_source_raw("(setq timer-list (cdr timer-list))")
            .unwrap();
        vm.clear_source_cache();
        vm.collect(&mut []);
        assert_eq!(vm.eval_source_raw("timer-list").unwrap(), Value::NIL);
        assert!(vm.heap_stats().collections > 1000);
        assert!(vm.functions.len() < 100, "dead request code retained");
    }
    #[test]
    fn distinct_source_requests_bound_existing_cache_roots() {
        let mut vm = Vm::new();
        for n in 0..10_000 {
            assert_eq!(
                vm.eval_source_raw(&format!("(let ((x {n})) (+ x 1))"))
                    .unwrap()
                    .as_integer()
                    .unwrap(),
                n + 1
            );
        }
        let mut roots = Vec::new();
        vm.source_code_roots(&mut roots);
        assert!(roots.len() <= 256);
        assert!(vm.functions.len() <= 1280);
        assert!(vm.heap_stats().collections > 0);
    }
}
