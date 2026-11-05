use super::*;
impl Leaf<'_> {
    pub(crate) fn backtrace_first(&self, base: Value) -> std::result::Result<Option<usize>, Fault> {
        Ok(self
            .vm
            .backtrace_start(base)?
            .map(|i| self.vm.backtrace.frames[i].token))
    }
    pub(crate) fn backtrace_entry(
        &mut self,
        token: usize,
    ) -> std::result::Result<Option<(Vec<Value>, usize)>, Fault> {
        let Some(i) = self.vm.backtrace.index_of(token) else {
            return Ok(None);
        };
        let next = i
            .checked_sub(1)
            .map_or(0, |i| self.vm.backtrace.frames[i].token);
        Ok(Some((self.vm.backtrace_frame_values(i).to_vec(), next)))
    }
    /// The (EVALD FUNCTION ARGS FLAGS) of the activation NFRAMES out from
    /// BASE's, if there is one.
    pub(crate) fn backtrace_frame_at(&mut self, nframes: Value, base: Value) -> std::result::Result<Option<Vec<Value>>, Fault> {
        Ok(self.vm.backtrace_frame_index(nframes, base)?.map(|i| self.vm.backtrace_frame_values(i).to_vec()))
    }
    pub(crate) fn backtrace_locals(&mut self, level: Value, base: Value) -> super::super::Out {
        self.vm.backtrace_locals(level, base)
    }
    pub(crate) fn backtrace_frames_and_locals(&mut self, base: Value) -> super::super::Out {
        self.vm.backtrace_frames_and_locals(base)
    }
    /// GNU Fbacktrace_debug: FLAG, after setting the frame's flag to it.
    pub(crate) fn backtrace_debug(&mut self, level: Value, flag: Value, base: Value) -> super::super::Out {
        if let Some(i) = self.vm.backtrace_frame_index(level, base)? {
            self.vm.backtrace_set_debug(i, flag != Value::NIL);
        }
        Ok(flag)
    }
    /// `backtrace-eval`'s unrewind to the activation NFRAMES out from
    /// BASE's: what was swapped, as four integers for the rewind.
    pub(crate) fn backtrace_unrewind(&mut self, nframes: Value, base: Value) -> super::super::Out {
        let index = self.vm.backtrace_frame_index(nframes, base)?
            .ok_or_else(|| Fault::Error("Activation frame not found!".into()))?;
        let span = self.vm.backtrace_unrewind(index).map_err(Fault::Error)?;
        let span = span.map(|n| Value::integer(n as i64).expect("backtrace span"));
        Ok(self.vm.list_value(&span))
    }
    pub(crate) fn backtrace_rewind(&mut self, span: Value) -> super::super::Out {
        let mut parts = [0usize; 4];
        let mut tail = span;
        for part in &mut parts {
            let (head, rest) = self.vm.cons_pair(tail).map_err(|_| Fault::Wrong("consp", span))?;
            *part = head.integer_value().and_then(|n| usize::try_from(n).ok()).ok_or(Fault::Wrong("wholenump", head))?;
            tail = rest;
        }
        self.vm.backtrace_rewind(parts).map_err(Fault::Error)?;
        Ok(Value::NIL)
    }
}
