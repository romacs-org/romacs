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
    /// Request functions installed since the last collection that only a
    /// collection can retire: an entry released when its request returns
    /// (`release_request_entry') is not one.
    pub(super) since_collection: usize,
    /// Cache slots a builder took that no installed function's code owns
    /// yet: at a collection, where no builder is open, those of a failed or
    /// discarded build.
    unclaimed_caches: Vec<usize>,
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
        // The function's code now owns the cache slots its builder took.
        let caches: Vec<usize> = function.code.iter().filter_map(|op| match *op {
            Op::Direct(_, _, cache, _, _) | Op::Prepared(_, _, _, cache, _, _) => Some(cache),
            _ => None,
        }).collect();
        self.claim_caches(&caches);
        self.install_slot(function, self.request_code.compiling)
    }
    /// The installed code that uses CACHES now owns them.
    pub(super) fn claim_caches(&mut self, caches: &[usize]) {
        let unclaimed = &mut self.request_code.unclaimed_caches;
        for cache in caches {
            if let Some(i) = unclaimed.iter().rposition(|c| c == cache) {
                unclaimed.swap_remove(i);
            }
        }
    }
    /// Install FUNCTION, request-owned when REQUEST, claiming no cache slot.
    fn install_slot(&mut self, function: Function, request: bool) -> usize {
        self.register_width = self.register_width.max(function.regs);
        let code = &mut self.request_code;
        let id = if request
            && let Some(id) = code.free.pop()
        {
            self.functions[id] = Arc::new(function);
            code.live[id] = true;
            id
        } else {
            let id = self.functions.len();
            self.functions.push(Arc::new(function));
            code.permanent.push(!request);
            code.live.push(true);
            id
        };
        if request {
            // Only an enclosing compile_request collects what it compiled.
            if code.compiling {
                code.compiled.push(id);
            }
            code.since_collection += 1;
        }
        id
    }

    pub(super) fn install_cache(&mut self) -> usize {
        self.install_cache_slot(self.request_code.compiling)
    }
    /// A cache slot, unclaimed until installed code that uses it is
    /// (`claim_caches'); a request reuses a free one.
    fn install_cache_slot(&mut self, request: bool) -> usize {
        let cache = Cache {
            version: 0,
            target: Value::UNBOUND,
        };
        let id = if request
            && let Some(id) = self.request_code.free_caches.pop()
        {
            self.caches[id] = cache;
            id
        } else {
            let id = self.caches.len();
            self.caches.push(cache);
            id
        };
        self.request_code.unclaimed_caches.push(id);
        id
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
        outer: &NameMap<Name, Binding>,
        local_specials: &NameSet<String>,
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
        // Request-owned placeholders: they still hold the artifact's own cache
        // ordinals, not slots, so installing them claims nothing. The slots
        // stay unclaimed until the caller has rewritten and stored the
        // functions that use them (`claim_caches'). No state is changed and
        // restored here, so an unwinding failure leaves only unreachable
        // placeholders and unclaimed slots, which a collection reclaims.
        let ids = functions
            .iter()
            .map(|f| {
                #[cfg(test)]
                artifact_fault(ArtifactFault::Installing);
                self.install_slot(f.clone(), true)
            })
            .collect();
        let cache_ids = (0..caches).map(|_| self.install_cache_slot(true)).collect();
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

    /// Evaluate FORM, a datum evaluated again and again (a mode-line
    /// `:eval'), through its cached code (`cached_datum'). One with no
    /// structural key is compiled for this use alone, as `eval_form_raw'.
    pub(crate) fn eval_datum_raw(&mut self, form: &Form) -> Result<Value> {
        match self.cached_datum(form) {
            Ok(Some(function)) => {
                let outcome = self.execute_raw(function);
                self.finish_request(outcome)
            }
            Ok(None) => self.eval_form_raw(form),
            Err(error) => self.finish_request(Err(error)),
        }
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
        self.request_code.since_collection = self.request_code.since_collection.saturating_sub(1);
        for program in self.programs_mut() {
            program.release_entry(id);
        }
    }

    pub(super) fn finish_request(&mut self, mut outcome: Result<Value>) -> Result<Value> {
        if self.eval_depth == 0 {
            self.propagated_escape = None;
            self.propagated_escape_hooked = false;
            self.propagated_eval_depth = None;
        }
        // A request boundary: no engine's registers are live here, and any
        // suspended frame or caller below holds its Values in traced storage
        // unless it holds collection (`Vm::may_collect`). Only an outermost
        // request retires request code, so only one collects for its count.
        let outermost = self.suspended_stacks.is_empty();
        if self.may_collect(0)
            && self.stack.is_empty()
            && (self.collection_due()
                || (outermost && self.request_code.since_collection >= 1024))
        {
            match &mut outcome {
                Ok(value) => self.collect(std::slice::from_mut(value)),
                Err(_) => self.collect(&mut []),
            }
        }
        outcome
    }

    pub(super) fn retire_code(&mut self, marks: &[bool]) {
        for program in self.programs_mut() {
            program.retire_code(marks);
        }
        // No builder is open at a collection: a slot no installed function
        // owns belongs to a build that failed or discarded that code.
        for id in std::mem::take(&mut self.request_code.unclaimed_caches) {
            self.caches[id] = Cache { version: 0, target: Value::UNBOUND };
            self.request_code.free_caches.push(id);
        }
        // With no request code dead, no cache becomes free: skip rescanning
        // every live body.
        if !(0..marks.len()).any(|id| !marks[id] && self.request_code.live[id]) {
            return;
        }
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
    }
}

/// Test-only failure points in persistent artifact installation.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ArtifactFault {
    Installing,
    Remapped,
}

#[cfg(test)]
thread_local! {
    pub(super) static ARTIFACT_FAULT: std::cell::Cell<Option<ArtifactFault>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(super) fn artifact_fault(at: ArtifactFault) {
    if ARTIFACT_FAULT.get() == Some(at) {
        panic!("injected artifact installation failure");
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
    /// A mode-line `:eval' form compiles once, not at every redisplay, and
    /// again only when a macro it expanded is redefined.
    #[test]
    fn repeated_mode_line_eval_forms_compile_once() {
        let mut vm = Vm::new();
        // Counts collections, so no forced ones (`ROMACS_GC_STRESS`).
        vm.set_gc_stress(0);
        vm.set_frame_geometry(80, 25, 1, 1).unwrap();
        vm.eval_source_raw("(defmacro ml-probe () \"one\") (setq ml-format '(:eval (concat (ml-probe) \"!\")))").unwrap();
        let shown = |vm: &mut Vm| {
            let value = vm.eval_source_raw("(format-mode-line ml-format)").unwrap();
            vm.display(value).unwrap()
        };
        assert_eq!(shown(&mut vm), "\"one!\"");
        let compiled = (vm.request_code.since_collection, vm.heap_stats().collections);
        for _ in 0..50 {
            assert_eq!(shown(&mut vm), "\"one!\"");
        }
        let now = (vm.request_code.since_collection, vm.heap_stats().collections);
        assert_eq!(now, compiled, "a redisplay compiled the :eval form again");
        vm.eval_source_raw("(defmacro ml-probe () \"two\")").unwrap();
        assert_eq!(shown(&mut vm), "\"two!\"");
    }
    /// A native caller's prepared call is made once and reused.
    #[test]
    fn prepared_source_calls_are_made_once() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun prepared-probe (a b) (+ a b))").unwrap();
        let call = "(lambda (a b) (prepared-probe a b))";
        let args = |n| [Value::integer(n).unwrap(), Value::integer(2).unwrap()];
        assert_eq!(vm.call_prepared_source_raw(call, &args(0)).unwrap().as_integer().unwrap(), 2);
        let (functions, requests) = (vm.functions.len(), vm.request_code.since_collection);
        for n in 1..100 {
            assert_eq!(vm.call_prepared_source_raw(call, &args(n)).unwrap().as_integer().unwrap(), n + 2);
        }
        assert_eq!((vm.functions.len(), vm.request_code.since_collection), (functions, requests), "a call compiled code");
    }
    /// The first call prepares its lambda, which runs Lisp; a moving
    /// collection there must not leave the caller's arguments stale.
    #[test]
    fn a_first_prepared_call_keeps_its_arguments_across_collection() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun prepared-pair (a b) (list (car a) (aref b 0)))").unwrap();
        // Garbage below the arguments, so a collection moves them.
        vm.eval_source_raw("(dotimes (_ 100) (list 1 2) (make-vector 2 0))").unwrap();
        let pair = vm.eval_source_raw("(cons (list 'first (make-string 3 ?a)) (vector 'second))").unwrap();
        let (a, b) = vm.cons_pair(pair).unwrap();
        vm.set_gc_stress(1);
        let before = vm.heap_stats().collections;
        let value = vm.call_prepared_source_raw("(lambda (a b) (prepared-pair a b))", &[a, b]).unwrap();
        assert!(vm.heap_stats().collections > before, "the preparation collected");
        assert_eq!(vm.display(value).unwrap(), "(first second)");
    }
    /// Entries released as their requests return need no collection: they
    /// do not count toward the one the 1024th retirable function forces.
    #[test]
    fn released_request_entries_do_not_force_collections() {
        let mut vm = Vm::new();
        // Counts collections, so no forced ones (`ROMACS_GC_STRESS`).
        vm.set_gc_stress(0);
        vm.eval_source_raw("(defun released-probe (n) (eval (list '+ n 1)))").unwrap();
        let before = vm.heap_stats().collections;
        vm.eval_source_raw("(dotimes (i 5000) (released-probe i))").unwrap();
        assert_eq!(vm.heap_stats().collections, before, "5000 released eval entries forced a collection");
        assert!(vm.request_code.since_collection < 16, "{} retirable functions counted", vm.request_code.since_collection);
    }
    /// A compilation that fails after its builder took a cache slot (for
    /// `(foo)') installs no function to own it; collections reclaim it.
    #[test]
    fn failed_compilations_do_not_accumulate_cache_slots() {
        let mut vm = Vm::new();
        for i in 0..2_000 {
            assert!(vm.eval_source_raw("(progn (foo) (let ((nil 1)) nil))").is_err());
            if i % 10 == 0 {
                vm.collect(&mut []);
            }
        }
        vm.collect(&mut []);
        // Slots reclaimed by one collection serve the next ten failures.
        assert!(vm.caches.len() <= 64, "orphaned cache slots accumulate: {}", vm.caches.len());
        assert_eq!(vm.request_code.unclaimed_caches, Vec::<usize>::new(), "every builder's slot is owned or reclaimed");
    }
    /// A persistent IR artifact numbers its caches from 0; installing it
    /// claims the slots it receives, never a failed build's slot that only
    /// shares an artifact ordinal.
    #[test]
    fn a_persistent_artifact_claims_only_the_slots_it_receives() {
        let dir = std::env::temp_dir().join(format!("romacs-artifact-claim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = "(defalias 'gr-art (lambda () (gr-callee 1)))";
        let mut cold = Vm::new();
        cold.set_source_cache_store(dir.clone()).unwrap();
        cold.eval_source_raw(source).unwrap();
        let mut vm = Vm::new();
        vm.set_source_cache_store(dir.clone()).unwrap();
        assert!(vm.eval_source_raw("(progn (foo) (let ((nil 1)) nil))").is_err());
        let orphans = vm.request_code.unclaimed_caches.clone();
        assert!(orphans.contains(&0), "the failed build left slot 0: {orphans:?}");
        vm.eval_source_raw(source).unwrap();
        assert_eq!(vm.source_cache_stats().disk_hits, 1, "the artifact came from the store");
        vm.collect(&mut []);
        // The loaded function stays live through gr-art.
        let owned: Vec<usize> = vm.functions.iter().zip(&vm.request_code.live).filter(|&(_, &live)| live)
            .flat_map(|(f, _)| f.code.iter().filter_map(|op| match *op {
                Op::Direct(_, _, cache, _, _) | Op::Prepared(_, _, _, cache, _, _) => Some(cache),
                _ => None,
            }))
            .collect();
        assert!(!owned.is_empty(), "the artifact's call owns a slot");
        for orphan in &orphans {
            assert!(!owned.contains(orphan) && vm.request_code.free_caches.contains(orphan), "orphan {orphan} reclaimed");
        }
        assert!(owned.iter().all(|c| !vm.request_code.free_caches.contains(c)), "the artifact's slots stay claimed");
        assert_eq!(vm.request_code.unclaimed_caches, Vec::<usize>::new());
        std::fs::remove_dir_all(dir).unwrap();
    }
    /// Cache slots the code of live functions uses.
    fn owned_caches(vm: &Vm) -> Vec<usize> {
        vm.functions.iter().zip(&vm.request_code.live).filter(|&(_, &live)| live)
            .flat_map(|(f, _)| f.code.iter().filter_map(|op| match *op {
                Op::Direct(_, _, cache, _, _) | Op::Prepared(_, _, _, cache, _, _) => Some(cache),
                _ => None,
            }))
            .collect()
    }

    /// A panic while a persistent artifact is installed, before or after its
    /// slots are reserved, leaves nothing to restore: the unclaimed list
    /// holds what it held plus any slots reserved, `compiling' is untouched,
    /// and a collection then reclaims every slot no live code owns.
    #[test]
    fn a_panic_while_installing_an_artifact_leaves_reclaimable_state() {
        let dir = std::env::temp_dir().join(format!("romacs-artifact-panic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = "(defalias 'gr-art (lambda () (list (gr-callee 1) (gr-callee 2))))";
        let mut cold = Vm::new();
        cold.set_source_cache_store(dir.clone()).unwrap();
        cold.eval_source_raw(source).unwrap();
        for fault in [ArtifactFault::Installing, ArtifactFault::Remapped] {
            let mut vm = Vm::new();
            vm.set_source_cache_store(dir.clone()).unwrap();
            assert!(vm.eval_source_raw("(progn (foo) (let ((nil 1)) nil))").is_err());
            let before = vm.request_code.unclaimed_caches.clone();
            assert!(!before.is_empty());
            ARTIFACT_FAULT.set(Some(fault));
            let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| vm.eval_source_raw(source)));
            ARTIFACT_FAULT.set(None);
            assert!(caught.is_err(), "the injected failure unwinds");
            assert!(!vm.request_code.compiling, "compiling is untouched");
            let after = vm.request_code.unclaimed_caches.clone();
            assert_eq!(after[..before.len()], before[..], "the unclaimed list keeps what it held");
            match fault {
                ArtifactFault::Installing => assert_eq!(after, before, "no slot was reserved yet"),
                ArtifactFault::Remapped => assert!(after.len() > before.len(), "the reserved slots stay unclaimed"),
            }
            vm.collect(&mut []);
            assert_eq!(vm.request_code.unclaimed_caches, Vec::<usize>::new());
            let owned = owned_caches(&vm);
            for slot in &after {
                assert!(!owned.contains(slot) && vm.request_code.free_caches.contains(slot), "slot {slot} reclaimed");
            }
            // The VM still loads the artifact and runs it.
            vm.eval_source_raw("(defun gr-callee (x) (* x 10))").unwrap();
            let hits = vm.source_cache_stats().disk_hits;
            vm.eval_source_raw(source).unwrap();
            assert_eq!(vm.source_cache_stats().disk_hits, hits + 1, "the artifact came from the store");
            let value = vm.eval_source_raw("(gr-art)").unwrap();
            assert_eq!(vm.display(value).unwrap(), "(10 20)");
            vm.collect(&mut []);
            let owned = owned_caches(&vm);
            assert!(owned.iter().all(|c| !vm.request_code.free_caches.contains(c)), "live slots stay claimed");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn short_distinct_requests_reuse_code_slots_without_collecting() {
        let mut vm = Vm::new();
        // Counts collections, so no forced ones (`ROMACS_GC_STRESS`).
        vm.set_gc_stress(0);
        for n in 0..100_000 {
            let form = Form::List(vec![Form::Sym("+".into()), Form::Int(n), Form::Int(2)]);
            assert_eq!(vm.eval_form_raw(&form).unwrap().as_integer().unwrap(), n + 2);
        }
        // Each entry is released as its request returns: its slot is reused
        // at once, with nothing left for a collection to retire.
        assert_eq!(vm.heap_stats().collections, 0);
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
