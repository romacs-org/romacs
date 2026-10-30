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
            Some(status) => Value::integer(i64::from(status)).map_err(Fault::Error),
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
}
