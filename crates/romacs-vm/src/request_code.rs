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
    /// Request functions that may have become garbage since the last
    /// collection, which only a collection can retire (`counted').
    pub(super) since_collection: usize,
    /// By function, whether it counts in `since_collection'. An install
    /// counts until its entry is released as its request returns
    /// (`release_request_entry') or it becomes a definition (with the
    /// closures its code makes), which its symbol keeps live; a lazily held
    /// body never counts, being live with its function. A definition
    /// another one replaces counts again (`charge_definition').
    counted: Vec<bool>,
    /// Cache slots a builder took that no installed function's code owns
    /// yet: at a collection, where no builder is open, those of a failed or
    /// discarded build.
    unclaimed_caches: Vec<usize>,
}
romacs_snapshot::snapshot_struct!(RequestCode { permanent, image_caches, live, free, free_caches, compiling, compiled, since_collection, counted, unclaimed_caches });

impl RequestCode {
    /// Whether every cache slot a builder took is owned: no build is open
    /// and none left one behind.
    pub(super) fn unclaimed_caches_empty(&self) -> bool {
        self.unclaimed_caches.is_empty()
    }
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
        let caches: Vec<usize> = owned_caches(&function).collect();
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
        #[cfg(test)]
        INSTALLS.set(INSTALLS.get() + 1);
        let code = &mut self.request_code;
        let id = if request
            && let Some(id) = code.free.pop()
        {
            self.functions.set(id, Arc::new(function));
            code.live[id] = true;
            id
        } else {
            let id = self.functions.len();
            self.functions.push(Arc::new(function));
            code.permanent.push(!request);
            code.live.push(true);
            code.counted.push(false);
            id
        };
        if request {
            // Only an enclosing compile_request collects what it compiled.
            if code.compiling {
                code.compiled.push(id);
            }
            self.count_code(id);
        }
        id
    }
    /// Request function ID may be garbage at the next collection: it counts
    /// toward the one `finish_request' makes for code.
    fn count_code(&mut self, id: usize) {
        let code = &mut self.request_code;
        if code.live[id] && !code.permanent[id] && !std::mem::replace(&mut code.counted[id], true) {
            code.since_collection += 1;
        }
    }
    /// Request function ID is live by construction, or gone: it no longer
    /// counts toward a collection.
    fn uncount_code(&mut self, id: usize) {
        let code = &mut self.request_code;
        if std::mem::replace(&mut code.counted[id], false) {
            code.since_collection -= 1;
        }
    }
    /// A collection with no request running retired the code that was
    /// garbage: what is left counts toward no collection.
    pub(super) fn reset_code_count(&mut self) {
        let code = &mut self.request_code;
        code.counted.fill(false);
        code.since_collection = 0;
    }
    /// FUNCTION and the functions its code makes closures of, transitively:
    /// a definition's code, live while its closure is.
    fn code_tree(&self, function: usize) -> Vec<usize> {
        let mut tree = vec![function];
        let mut seen = HashSet::default();
        seen.insert(function);
        let mut next = 0;
        while let Some(&id) = tree.get(next) {
            next += 1;
            let f = &self.functions[id];
            let children = f.code.iter().filter_map(|op| match *op {
                Op::Closure(_, child, _) => Some(child),
                _ => None,
            });
            let pending = f.pending.iter().flat_map(|body| body.children.iter().copied());
            for child in children.chain(pending).collect::<Vec<_>>() {
                if child < self.functions.len() && seen.insert(child) {
                    tree.push(child);
                }
            }
        }
        tree
    }
    /// A function cell went from OLD to NEW (`define_function'). NEW's
    /// request code is a definition now, which the symbol keeps live, so it
    /// counts toward no collection; OLD's, superseded, may be garbage and
    /// counts again: redefining a function again and again (an
    /// `eval-buffer' of an init, a test rerun) still brings a collection,
    /// which object pacing, counting heap objects only, would not.
    pub(super) fn charge_definition(&mut self, old: Value, new: Value) {
        let (old, new) = (self.cell_code(old), self.cell_code(new));
        if old == new {
            return;
        }
        if let Some(function) = new {
            for id in self.code_tree(function) {
                self.uncount_code(id);
            }
        }
        if let Some(function) = old {
            for id in self.code_tree(function) {
                self.count_code(id);
            }
        }
    }
    /// The request function a function cell's VALUE runs: a private closure's,
    /// also as a macro's `(macro . CLOSURE)'.
    fn cell_code(&self, value: Value) -> Option<usize> {
        let value = match self.cons_pair(value) {
            Ok((head, tail)) if head == Value::reference(self.names.get("macro").copied()? + 1, 7) => tail,
            _ => value,
        };
        if Self::image_value(value) {
            return None;
        }
        let closure = value.index_if(4)?;
        let function = self.closures.get(closure)?.function;
        (!self.request_code.permanent.get(function).copied().unwrap_or(true)).then_some(function)
    }

    /// Run BUILD outside any request's compilation: what it installs is
    /// permanent, owned by no request.
    pub(super) fn compile_permanent<T>(&mut self, build: impl FnOnce(&mut Self) -> T) -> T {
        let compiling = std::mem::replace(&mut self.request_code.compiling, false);
        let result = build(self);
        self.request_code.compiling = compiling;
        result
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
        let start = self.request_code.compiled.len();
        self.request_code.compiling = true;
        let result = self.function(args, body, outer, local_specials);
        if result.is_err() {
            self.request_code.since_collection += 1;
        }
        // The body is live with the function that held it.
        for id in self.request_code.compiled[start..].to_vec() {
            self.uncount_code(id);
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
            Arc::get_mut(self.functions.change(id)).expect("fresh code is private"),
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
                pending: None,
                dynamic: None,
                params: None,
            },
        );
        if self.request_code.live[id] && !self.request_code.permanent[id] {
            self.uncount_code(id);
            self.request_code.live[id] = false;
            self.request_code.free.push(id);
        }
        function
    }

    pub(super) fn reserve_request_artifact(
        &mut self,
        functions: impl IntoIterator<Item = Function>,
        caches: usize,
    ) -> (Vec<usize>, Vec<usize>) {
        // Request-owned placeholders holding no code, so installing them
        // claims nothing and a failed install leaves nothing naming a slot.
        // The slots stay unclaimed until the caller has rewritten and stored
        // the functions that use them (`claim_caches'). No state is changed and
        // restored here, so an unwinding failure leaves only unreachable
        // placeholders and unclaimed slots, which a collection reclaims.
        let ids = functions
            .into_iter()
            .map(|f| {
                #[cfg(test)]
                artifact_fault(ArtifactFault::Installing);
                self.install_slot(f, true)
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
        // A compiled-Lisp entry whose run failed before its first call still
        // holds its body pending, and the caches that body names.
        self.uncount_code(id);
        self.drop_request_function(id);
        for program in self.programs_mut() {
            program.release_entry(id);
        }
    }

    pub(super) fn finish_request(&mut self, mut outcome: Result<Value>) -> Result<Value> {
        if self.eval_depth == 0 {
            self.propagated_escape = None;
            self.propagated_escape_hooked = false;
            self.propagated_eval_depth = None;
            self.drop_deferred_escapes();
        }
        // A request boundary: no engine's registers are live here, and any
        // suspended frame or caller below holds its Values in traced storage
        // unless it holds collection (`Vm::may_collect`). Only an outermost
        // request retires request code, so only one collects for its count.
        let outermost = self.suspended_stacks.is_empty();
        if self.may_collect(0)
            && self.stack.is_empty()
            && (self.collection_due()
                || (outermost && self.request_code.since_collection >= self.code_collection_threshold()))
        {
            match &mut outcome {
                Ok(value) => self.collect(std::slice::from_mut(value)),
                Err(_) => self.collect(&mut []),
            }
            // A nested request's caller polls soon for what this left.
            if self.eval_depth > 0 && self.aftermath_pending() { self.request_poll(); }
        }
        // An embedder's request ends here: a safe point for what collections
        // left pending, its result rooted (VM2-GC-FINALIZER). The run's
        // failures are its own (its finalizers' are logged).
        debug_assert!(self.eval_depth > 0 || !self.aftermath_held(), "no engine runs at a request's end");
        if self.eval_depth == 0 && let Some(runner) = self.safe_runner(0) {
            let frame = self.root_frame(&[*outcome.as_ref().unwrap_or(&Value::NIL)]);
            let _ = self.run_aftermath(runner, false);
            let [value] = self.release_frame(frame);
            if let Ok(kept) = &mut outcome { *kept = value; }
        }
        outcome
    }

    /// How much request code a request boundary lets accumulate before it
    /// collects: 1024 functions, or a quarter of the code installed, so that,
    /// as with objects (`gc_threshold`), the collections a load pays grow
    /// with what it adds rather than with what the heap already holds.
    pub(super) fn code_collection_threshold(&self) -> usize {
        ((self.functions.len() - self.request_code.free.len()) / 4).max(1024)
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
        // The work follows what died, not what lives.
        for id in 0..marks.len() {
            if !marks[id] && self.request_code.live[id] {
                self.drop_request_function(id);
            }
        }
        if self.gc_stress.verify {
            self.verify_cache_owners();
        }
    }
    /// Free request function ID and what only it owns: its code, any lazy
    /// or pending body, and its cache slots. A slot has one owner
    /// (`claim_caches'); image ordinals stay reserved for undecoded bodies.
    fn drop_request_function(&mut self, id: usize) {
        let caches: Vec<usize> = owned_caches(&self.functions[id]).collect();
        for cache in caches {
            if cache >= self.request_code.image_caches {
                self.caches[cache] = Cache { version: 0, target: Value::UNBOUND };
                self.request_code.free_caches.push(cache);
            }
        }
        let f = Arc::get_mut(self.functions.change(id)).expect("request code is private at a wait point");
        f.code = Vec::new();
        f.sites = Vec::new();
        f.capture_mutations = Vec::new();
        f.lazy = None;
        f.pending = None;
        self.request_code.live[id] = false;
        self.request_code.free.push(id);
    }
    /// `ROMACS_GC_VERIFY': every cache slot past the image's has exactly
    /// one owner, a live function or the free list.
    fn verify_cache_owners(&self) {
        let mut owners = vec![0u32; self.caches.len()];
        let live = self.functions.iter().zip(&self.request_code.live).filter(|&(_, &live)| live);
        for cache in live.flat_map(|(function, _)| owned_caches(function)).chain(self.request_code.free_caches.iter().copied()) {
            owners[cache] += 1;
        }
        let image = self.request_code.image_caches;
        if let Some(cache) = (image..owners.len()).find(|&cache| owners[cache] != 1) {
            panic!("cache slot {cache} has {} owners", owners[cache]);
        }
    }
}

/// The cache slots FUNCTION's code, compiled or pending, names.
fn owned_caches(function: &Function) -> impl Iterator<Item = usize> + '_ {
    let direct = function.code.iter().filter_map(|op| match *op {
        Op::Direct(_, _, cache, _, _) => Some(cache),
        _ => None,
    });
    direct.chain(function.pending.iter().flat_map(|body| body.caches.iter().copied()))
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
    /// Functions installed on this thread, request-owned or not.
    pub(super) static INSTALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
    /// `(foo)'; `(lambda)' fails at compile time) installs no function to
    /// own it; collections reclaim it.
    #[test]
    fn failed_compilations_do_not_accumulate_cache_slots() {
        let mut vm = Vm::new();
        for i in 0..2_000 {
            assert!(vm.eval_source_raw("(progn (foo) (lambda))").is_err());
            if i == 0 {
                assert!(!vm.request_code.unclaimed_caches.is_empty(), "the failure left its slot unowned");
            }
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
        assert!(vm.eval_source_raw("(progn (foo) (lambda))").is_err());
        let orphans = vm.request_code.unclaimed_caches.clone();
        assert!(orphans.contains(&0), "the failed build left slot 0: {orphans:?}");
        vm.eval_source_raw(source).unwrap();
        assert_eq!(vm.source_cache_stats().disk_hits, 1, "the artifact came from the store");
        vm.collect(&mut []);
        // The loaded function stays live through gr-art.
        let owned: Vec<usize> = vm.functions.iter().zip(&vm.request_code.live).filter(|&(_, &live)| live)
            .flat_map(|(f, _)| f.code.iter().filter_map(|op| match *op {
                Op::Direct(_, _, cache, _, _) => Some(cache),
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
                Op::Direct(_, _, cache, _, _) => Some(cache),
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
            assert!(vm.eval_source_raw("(progn (foo) (lambda))").is_err());
            let before = vm.request_code.unclaimed_caches.clone();
            assert!(!before.is_empty());
            ARTIFACT_FAULT.set(Some(fault));
            let caught = vm.catching(|vm| vm.eval_source_raw(source));
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
    /// A loaded file's top-level entries go as they return and its
    /// definitions are live: the load leaves no code a collection must
    /// retire, and its function slots grow by the definitions only.
    #[test]
    fn a_load_counts_no_code_toward_a_collection() {
        let path = std::env::temp_dir().join(format!("romacs-load-count-{}.el", std::process::id()));
        let mut text = String::new();
        for i in 0..400 {
            text.push_str(&format!("(defvar load-count-v{i} (list {i}))\n(put 'load-count-v{i} 'probe (list {i} {i}))\n"));
            text.push_str(&format!("(defun load-count-f{i} (x) (funcall (lambda (y) (+ x y {i})) x))\n"));
        }
        std::fs::write(&path, text).unwrap();
        let mut vm = Vm::new();
        // Counts collections, so no forced ones (`ROMACS_GC_STRESS`).
        vm.set_gc_stress(0);
        let live = |vm: &Vm| vm.request_code.live.iter().filter(|&&live| live).count();
        let (before, collections) = (live(&vm), vm.heap_stats().collections);
        vm.load_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(vm.request_code.since_collection, 0, "the load left code counted toward a collection");
        assert_eq!(vm.heap_stats().collections, collections);
        let grown = live(&vm) - before;
        assert!(grown <= 800, "{grown} functions live after loading 400 definitions");
        let value = vm.eval_source_raw("(load-count-f399 1)").unwrap();
        assert_eq!(value.as_integer().unwrap(), 401);
    }
    /// A definition another replaces may be garbage: redefining a function
    /// again and again brings code collections, so the old bodies do not
    /// pile up, and a closure still holding an old body runs it.
    #[test]
    fn superseded_definitions_count_toward_a_collection() {
        let mut vm = Vm::new();
        // Counts collections, so no forced ones (`ROMACS_GC_STRESS`).
        vm.set_gc_stress(0);
        let define = |n: i64| form(&format!(r#"["defalias",["quote","redef-probe"],["function",["lambda",[],{n}]]]"#));
        vm.eval_form_raw(&define(0)).unwrap();
        vm.eval_form_raw(&form(r#"["setq","redef-keeper",["symbol-function",["quote","redef-probe"]]]"#)).unwrap();
        let collections = vm.heap_stats().collections;
        for n in 1..5000 {
            vm.eval_form_raw(&define(n)).unwrap();
        }
        assert!(vm.heap_stats().collections > collections, "5000 redefinitions brought no collection");
        assert!(vm.functions.len() < 2500, "{} function slots after 5000 redefinitions", vm.functions.len());
        assert_eq!(vm.eval_form_raw(&form(r#"["funcall","redef-keeper"]"#)).unwrap().as_integer().unwrap(), 0);
        assert_eq!(vm.eval_form_raw(&form(r#"["redef-probe"]"#)).unwrap().as_integer().unwrap(), 4999);
    }
    /// Retiring code frees its sites and cache slots where they are: rounds
    /// of dropped definitions take them back, the tables stay flat, and a
    /// function that lives through it keeps its code and its warm call.
    #[test]
    fn retired_code_gives_its_sites_and_caches_back() {
        let mut vm = Vm::new();
        vm.set_gc_stress(0);
        vm.eval_source_raw("(defun retire-callee (x) (1+ x)) (defun retire-keeper (x) (retire-callee x))").unwrap();
        let mut sizes = vec![];
        for round in 0..6 {
            for i in 0..200 {
                let value = 1000 * round + i;
                vm.eval_source_raw(&format!("(defun retire-probe (x) (retire-callee (+ x {value}))) (retire-probe 0)")).unwrap();
            }
            assert_eq!(vm.eval_source_raw("(retire-keeper 1)").unwrap().as_integer().unwrap(), 2);
            vm.collect(&mut []);
            sizes.push((vm.execution.sites(), vm.caches.len()));
        }
        // Natural collections land anywhere in a round; kept code would add 200 caches a round.
        assert!(sizes[5].0 < sizes[1].0 * 2 && sizes[5].1 < sizes[1].1 * 2, "sites and caches grew across rounds: {sizes:?}");
        assert_eq!(vm.eval_source_raw("(retire-keeper 2)").unwrap().as_integer().unwrap(), 3);
        vm.gc_stress.verify = true;
        vm.collect(&mut []);
    }
    /// A native call into Lisp (a macro expander, a reader hook, a watcher)
    /// enters a permanent trampoline: no call installs a function, and a
    /// macro call's expansion installs only its own request's code.
    #[test]
    fn native_calls_into_lisp_install_no_function() {
        let mut vm = Vm::new();
        vm.eval_source_raw("(defun native-probe (&rest args) (length args)) (defmacro native-twice (x) (list 'progn x x))").unwrap();
        let probe = vm.intern("native-probe");
        for n in [0, 3, 8, 9, 12] {
            let args: Vec<Value> = (0..n).map(|i| Value::integer(i).unwrap()).collect();
            vm.reader_call(probe, &args).unwrap();
            let installs = INSTALLS.get();
            for _ in 0..50 {
                assert_eq!(vm.reader_call(probe, &args).unwrap().as_integer().unwrap(), n);
            }
            assert_eq!(INSTALLS.get(), installs, "a native call with {n} arguments installed a function");
        }
        vm.eval_form_raw(&form(r#"["native-twice",1]"#)).unwrap();
        let installs = INSTALLS.get();
        for n in 0..20 {
            let value = vm.eval_form_raw(&form(&format!(r#"["native-twice",{n}]"#))).unwrap();
            assert_eq!(value.as_integer().unwrap(), n);
        }
        assert_eq!(INSTALLS.get(), installs + 20, "an expansion installed more than its request's entry");
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
