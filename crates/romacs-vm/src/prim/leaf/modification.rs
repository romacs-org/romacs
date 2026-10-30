use super::*;
impl Leaf<'_> {
    pub fn buffer_modification(&mut self, buffer: Value) -> std::result::Result<crate::modification::Modification, Fault> {
        let index = if buffer == Value::NIL {
            self.vm.buffer_index().map_err(Fault::Error)?
        } else {
            match self.vm.object_if(buffer) {
                Some(data::Object::Buffer(_) | data::Object::ByteBuffer(_)) => buffer.index(0).map_err(Fault::Error)? - 1,
                _ => return Err(Fault::Wrong("bufferp", buffer)),
            }
        };
        Ok(match &self.vm.objects[index] {
            data::Object::Buffer(b) => b.state.modification,
            data::Object::ByteBuffer(b) => b.state.modification,
            _ => unreachable!(),
        })
    }
    /// Change the current buffer's modification state with F.
    pub fn update_modification<R>(&mut self, f: impl FnOnce(&mut crate::modification::Modification) -> R) -> std::result::Result<R, Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        Ok(f(match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => &mut b.state.modification,
            data::Object::ByteBuffer(b) => &mut b.state.modification,
            _ => unreachable!(),
        }))
    }
    pub fn set_buffer_modified(&mut self, flag: Value, redisplay: bool) -> std::result::Result<(), Fault> {
        // Intern before borrowing buffer storage; no raw Value is retained.
        let autosaved = self.intern("autosaved");
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let state = match &mut self.vm.objects[index] {
            data::Object::Buffer(b) => &mut b.state.modification,
            data::Object::ByteBuffer(b) => &mut b.state.modification,
            _ => unreachable!(),
        };
        state.set_modified(flag != Value::NIL);
        if flag == autosaved { state.autosave_modiff = state.modiff; }
        if redisplay { self.vm.display_state.version += 1; }
        Ok(())
    }
}
