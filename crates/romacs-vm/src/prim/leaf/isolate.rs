//! Isolates from a leaf (`crate::isolate`).
use super::*;

type Out = std::result::Result<Value, Fault>;

impl Leaf<'_> {
    fn freeze_error(&mut self, error: crate::frozen::FreezeError) -> Fault {
        self.vm.freeze_fault(error)
    }

    pub(crate) fn isolate_start(&mut self, program: Value, args: Value) -> Out {
        let program = std::path::PathBuf::from(String::from_utf8_lossy(self.string_view(program)?.bytes()).into_owned());
        let mut strings = Vec::new();
        for arg in self.vm.list_values(args).map_err(Fault::Error)? {
            strings.push(std::ffi::OsString::from(String::from_utf8_lossy(self.string_view(arg)?.bytes()).into_owned()));
        }
        let id = self.vm.isolate_start(&program, &strings).map_err(Fault::Error)?;
        Value::integer(id as i64).map_err(Fault::Error)
    }

    pub(crate) fn isolate_post(&mut self, id: u64, kind: Value, generation: Value, value: Value) -> std::result::Result<(), Fault> {
        let name = self.symbol_name(kind)?;
        let kind = crate::isolate::Kind::parse(&name).ok_or(Fault::Wrong("isolate-frame-kind", kind))?;
        let generation = generation.integer_value().and_then(|n| u64::try_from(n).ok()).ok_or(Fault::Wrong("natnump", generation))?;
        self.vm.isolate_post(id, kind, generation, value).map_err(|e| self.freeze_error(e))
    }

    pub(crate) fn isolate_wait(&mut self, timeout: Option<std::time::Duration>) -> Out {
        Ok(if self.vm.isolate_wait(timeout).map_err(Fault::Error)? { Value::TRUE } else { Value::NIL })
    }

    pub(crate) fn isolate_take(&mut self, id: u64) -> Out {
        self.vm.isolate_take(id).map_err(Fault::Error)
    }

    pub(crate) fn isolate_signal(&mut self, id: u64, signal: i64) -> Out {
        self.vm.isolate_signal(id, signal as i32).map_err(Fault::Error)?;
        Ok(Value::NIL)
    }

    pub(crate) fn isolate_reap(&mut self, id: u64) -> Out {
        match self.vm.isolate_reap(id).map_err(Fault::Error)? {
            Some(crate::isolate::Exit::Status(status)) => Value::integer(i64::from(status)).map_err(Fault::Error),
            // Dead, its status known to its zygote alone.
            Some(crate::isolate::Exit::Unknown) => Ok(Value::TRUE),
            None => Ok(Value::NIL),
        }
    }

    pub(crate) fn isolate_signal_children(&mut self, id: u64) -> Out {
        self.vm.isolate_signal_children(id).map_err(Fault::Error)
    }

    pub(crate) fn isolate_children(&mut self, id: u64) -> Out {
        let pids: Vec<Value> = self.vm.isolate_children(id).into_iter().map(|p| Value::integer(i64::from(p)).expect("pid fits")).collect();
        Ok(self.vm.list_value(&pids))
    }

    pub(crate) fn isolate_close(&mut self, id: u64) -> bool {
        self.vm.isolate_close(id)
    }

    pub(crate) fn isolate_pid(&mut self, id: u64) -> Out {
        Ok(self.vm.isolate_pid(id).map_or(Value::NIL, |p| Value::integer(i64::from(p)).expect("pid fits")))
    }

    pub(crate) fn isolate_frames(&self, id: u64) -> usize {
        self.vm.isolate_inbox_len(id)
    }

    pub(crate) fn isolate_ids(&mut self) -> Out {
        let ids: Vec<Value> = self.vm.isolate_ids().into_iter().map(|i| Value::integer(i as i64).expect("id fits")).collect();
        Ok(self.vm.list_value(&ids))
    }

    pub(crate) fn isolate_has_parent(&self) -> bool {
        self.vm.isolate_has_parent()
    }

    pub(crate) fn isolate_parent_closed(&self) -> bool {
        self.vm.isolate_parent_closed()
    }

    pub(crate) fn isolate_set_running(&mut self, generation: Option<u64>) {
        self.vm.isolate_set_running(generation);
    }

    pub(crate) fn isolate_hello_ok(&mut self, text: Value, program: Value) -> std::result::Result<bool, Fault> {
        let theirs = String::from_utf8_lossy(self.string_view(text)?.bytes()).into_owned();
        let program = String::from_utf8_lossy(self.string_view(program)?.bytes()).into_owned();
        let program = crate::host::canonicalize(&program).map(|p| p.display().to_string()).unwrap_or(program);
        let mut lines = theirs.splitn(4, '\n');
        Ok(lines.next() == Some("romacs-isolate")
            && lines.next() == Some(&crate::isolate::PROTOCOL.to_string())
            && lines.next() == Some(program.as_str()))
    }

    pub(crate) fn isolate_hello(&mut self, boot: Value) -> Out {
        let boot = self.symbol_name(boot)?;
        let text = crate::isolate::hello(&boot);
        Ok(self.vm.make_lisp_string(text.into_bytes(), true))
    }

    pub(crate) fn isolate_executable(&mut self) -> Out {
        let exe = crate::host::current_exe().map_err(|e| Fault::Error(e.to_string()))?;
        Ok(self.vm.make_lisp_string(exe.display().to_string().into_bytes(), true))
    }

    pub(crate) fn isolate_fork_request(&mut self, zygote: u64) -> Out {
        let id = self.vm.isolate_fork_request(zygote).map_err(Fault::Error)?;
        Value::integer(id as i64).map_err(Fault::Error)
    }

    pub(crate) fn isolate_fork(&mut self, generation: u64) -> Out {
        match self.vm.isolate_fork(generation).map_err(Fault::Error)? {
            Some(pid) => Value::integer(i64::from(pid)).map_err(Fault::Error),
            None => Ok(Value::NIL),
        }
    }

    pub(crate) fn isolate_release(&mut self, generation: u64) -> Out {
        self.vm.isolate_release(generation).map_err(Fault::Error)?;
        Ok(Value::NIL)
    }

    pub(crate) fn isolate_fork_refusal(&mut self) -> Out {
        Ok(match self.vm.isolate_fork_refusal() {
            Some(reason) => self.vm.make_lisp_string(reason.into_bytes(), true),
            None => Value::NIL,
        })
    }

    pub(crate) fn isolate_compact_next(&mut self) {
        self.vm.isolate_compact_next();
    }

    pub(crate) fn isolate_reserve(&mut self, needed: usize) {
        self.vm.isolate_reserve(needed);
    }

    pub(crate) fn isolate_stamp(&mut self, files: Value) -> Out {
        use std::hash::{Hash, Hasher};
        use std::os::unix::fs::MetadataExt;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for file in self.vm.list_values(files).map_err(Fault::Error)? {
            let name = String::from_utf8_lossy(self.string_view(file)?.bytes()).into_owned();
            let stamp = crate::host::metadata(&name).ok().map(|m| (m.len(), m.ctime(), m.ctime_nsec(), m.mtime_nsec(), m.ino(), m.dev()));
            (name, stamp).hash(&mut hasher);
        }
        // A fixnum: the digest's top bits dropped.
        Value::integer((hasher.finish() >> 3) as i64).map_err(Fault::Error)
    }

    pub(crate) fn isolate_name_process(&mut self, name: Value) -> Out {
        let name = String::from_utf8_lossy(self.string_view(name)?.bytes()).into_owned();
        crate::host::set_process_name(&name).map_err(|e| Fault::Error(e.to_string()))?;
        Ok(Value::NIL)
    }

    pub(crate) fn isolate_forked(&self) -> usize {
        self.vm.isolate_forked()
    }

    pub(crate) fn isolate_job_budget(&mut self, bytes: usize) {
        self.vm.isolate_job_budget(bytes);
    }
}
