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
        let Some(i) = self
            .vm
            .backtrace
            .frames
            .iter()
            .position(|frame| frame.token == token)
        else {
            return Ok(None);
        };
        let function = self.vm.backtrace.frames[i].function;
        let args = self.vm.backtrace_arguments(i).to_vec();
        let args = self.vm.list_value(&args);
        let next = i
            .checked_sub(1)
            .map_or(0, |i| self.vm.backtrace.frames[i].token);
        Ok(Some((vec![Value::TRUE, function, args, Value::NIL], next)))
    }
    pub(crate) fn backtrace_locals(&mut self, level: Value, base: Value) -> super::super::Out {
        self.vm.backtrace_locals(level, base)
    }
}
