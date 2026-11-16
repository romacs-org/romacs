//! `romacs--loads-in-progress': the loader's steps still under way.
use super::*;
use crate::prim::Out;

impl Leaf<'_> {
    /// Each step the loader has entered and not finished, outermost first,
    /// as (FILE LINE ORDINAL BYTE).
    pub(crate) fn loads_in_progress(&mut self) -> Out {
        let entered: Vec<(String, usize, usize, usize)> = self.vm.load_steps().iter()
            .filter(|step| step.status == crate::loader::StepStatus::Entered)
            .map(|step| (self.vm.load_step_file(step).to_string_lossy().into_owned(), step.line, step.ordinal, step.byte))
            .collect();
        let mut loads = Vec::with_capacity(entered.len());
        for (file, line, ordinal, byte) in entered {
            let file = self.string(&file);
            let int = |n: usize| Value::integer(n as i64).map_err(Fault::Error);
            let fields = [file, int(line)?, int(ordinal)?, int(byte)?];
            loads.push(self.list(&fields));
        }
        Ok(self.list(&loads))
    }
}
