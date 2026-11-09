use super::*;
/// Opaque and VM-local. Dropping/clearing caches never recycles a handle.
#[derive(Clone, Copy, Debug)]
pub struct PreparedHandle(pub usize);
#[derive(Clone)]
pub(super) struct Prepared {
    source: String,
    pub(super) function: usize,
    guard: Option<Entry>,
}
romacs_snapshot::snapshot_struct!(Prepared { source, function, guard });
impl Vm {
    fn prepared_function(&mut self, source: &str) -> Result<Prepared> {
        let (value, used) = self.read_source_raw(source)?.ok_or("lambda required")?;
        // A form before reading on: the reader can run Lisp (`\N{NAME}').
        let form = self.source_form(value, 0)?;
        if self.read_source_raw(&source[used..])?.is_some() {
            return Err("exactly one lambda required".into());
        }
        let a = form.list()?;
        if a.len() < 3 || a[0].sym()? != "lambda" {
            return Err("lambda required".into());
        }
        if a[1]
            .list()?
            .iter()
            .any(|f| f.sym().is_ok_and(|s| s.starts_with('&')))
        {
            return Err("prepared handles require fixed arity".into());
        }
        let entry = self.cached_source_form(source, &form)?;
        let value = self.execute_raw(entry)?;
        let c = &self.closures[value.index(4)?];
        if !c.captures.is_empty() || self.functions[c.function].arity > 4096 {
            return Err("prepared lambda cannot capture".into());
        }
        Ok(Prepared {
            source: source.into(),
            function: c.function,
            guard: self
                .source_cache
                .entries
                .get(&key(source))
                .cloned()
                .or_else(|| {
                    self.source_cache
                        .last_guard
                        .as_ref()
                        .filter(|e| e.function == entry)
                        .cloned()
                }),
        })
    }
    pub fn prepare_source(&mut self, source: &str) -> Result<PreparedHandle> {
        let prepared = self.prepared_function(source)?;
        let handle = PreparedHandle(self.source_cache.prepared.len());
        self.source_cache.prepared.push(prepared);
        Ok(handle)
    }
    /// Invoke a prepared handle, possibly mark/compacting even after a short
    /// request or error. Only the returned `Value` is rewritten; other raw heap
    /// `Value`s, including values in `args`, must not be reused after this call
    /// unless rooted. Prepared handles themselves remain valid. Embedders pass
    /// `Arg::Fixed`/`Arg::Rooted` through the public wrapper (handle.rs).
    pub(crate) fn invoke_prepared_raw(&mut self, handle: PreparedHandle, args: &[Value]) -> Result<Value> {
        self.invoke_prepared_with(handle, args, false)
    }
    fn invoke_prepared_with(&mut self, handle: PreparedHandle, args: &[Value], propagate: bool) -> Result<Value> {
        let p = self
            .source_cache
            .prepared
            .get(handle.0)
            .ok_or("invalid prepared handle")?;
        if !p.guard.as_ref().is_some_and(|e| self.cache_valid(e)) {
            let source = p.source.clone();
            // Remaking the lambda runs Lisp: ARGS are rooted across it and
            // read back before the call.
            let frame = self.root_frame(args);
            let updated = self.prepared_function(&source);
            let args: Vec<Value> = (0..args.len()).map(|i| self.rooted(&frame, i)).collect();
            self.drop_frame(frame);
            self.source_cache.prepared[handle.0] = updated?;
            return self.call_prepared(handle, &args, propagate);
        }
        self.call_prepared(handle, args, propagate)
    }
    fn call_prepared(&mut self, handle: PreparedHandle, args: &[Value], propagate: bool) -> Result<Value> {
        let function = self.source_cache.prepared[handle.0].function;
        if args.len() != self.functions[function].arity {
            return Err("wrong number of arguments".into());
        }
        self.propagate_next |= propagate;
        let outcome = threaded::execute_args(self, function, args);
        if propagate { self.propagate_next = false; }
        self.finish_request(outcome)
    }
    /// Call the fixed-arity lambda SOURCE with ARGS: a native caller's direct
    /// path into Lisp. Its prepared handle is made on the first call, so no
    /// call compiles a request.
    pub(crate) fn call_prepared_source_raw(&mut self, source: &str, args: &[Value]) -> Result<Value> {
        self.call_prepared_source(source, args, false)
    }
    /// `call_prepared_source_raw' for Lisp a primitive runs with no handler of
    /// its own: what it signals stays structured in `propagated_escape', for
    /// the calling engine to raise, so a quit stays a quit.
    pub(crate) fn call_prepared_source_propagating(&mut self, source: &str, args: &[Value]) -> Result<Value> {
        self.call_prepared_source(source, args, true)
    }
    fn call_prepared_source(&mut self, source: &str, args: &[Value], propagate: bool) -> Result<Value> {
        if let Some(index) = self.source_cache.prepared.iter().position(|p| p.source == source) {
            return self.invoke_prepared_with(PreparedHandle(index), args, propagate);
        }
        // Preparing runs Lisp, which may collect: ARGS wait in a frame.
        let frame = self.root_frame(args);
        let handle = self.prepare_source(source);
        let args: Vec<Value> = (0..args.len()).map(|i| self.rooted(&frame, i)).collect();
        self.drop_frame(frame);
        self.invoke_prepared_with(handle?, &args, propagate)
    }
    pub fn prepared_ir(&self, handle: PreparedHandle) -> Result<String> {
        let p = self
            .source_cache
            .prepared
            .get(handle.0)
            .ok_or("invalid prepared handle")?;
        self.dump_ir(p.function)
    }
}
