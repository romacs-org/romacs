//! undo.c and buffer.c's undo switches over the VM's undo recording.
use super::*;

impl Leaf<'_> {
    pub(crate) fn undo_boundary(&mut self) -> super::super::Out {
        self.vm.undo_boundary().map_err(Fault::Error)
    }

    /// keyboard.c command_loop_1's point_before_last_command_or_undo:
    /// the current buffer and its point before a command, which
    /// record_point puts in the undo list when the command's first change
    /// is elsewhere.
    pub(crate) fn note_command_point(&mut self) -> super::super::Out {
        self.vm.note_command_point().map_err(Fault::Error)?;
        Ok(Value::NIL)
    }

    /// `Vm::take_undo_offer`.
    pub(crate) fn take_undo_offer(&mut self, again: Value) -> super::super::Out {
        self.vm.take_undo_offer(again != Value::NIL).map_err(Fault::Error)
    }

    /// `Vm::answer_undo_offer`.
    pub(crate) fn answer_undo_offer(&mut self, buffer: Value, answer: Value) -> super::super::Out {
        self.vm.answer_undo_offer(buffer, answer).map_err(Fault::Error)?;
        Ok(Value::NIL)
    }

    /// Fbuffer_enable_undo: Fget_buffer's BUFFER, nil meaning the current
    /// one; a t undo list becomes nil.
    pub(crate) fn buffer_enable_undo(&mut self, buffer: Value) -> super::super::Out {
        let buffer = if buffer == Value::NIL {
            self.current_buffer()?
        } else if matches!(self.kind(buffer), Kind::Buffer | Kind::ByteBuffer) {
            buffer
        } else {
            let name = self.string_text(buffer).map_err(|_| Fault::Wrong("stringp", buffer))?;
            self.vm.named_buffer(&name).ok_or_else(|| Fault::Error(format!("No buffer named {name}")))?
        };
        let index = buffer.index(0).map_err(Fault::Error)? - 1;
        if self.vm.undo_list_of(index) == Value::TRUE {
            self.vm.set_undo_list_of(index, Value::NIL);
        }
        Ok(Value::NIL)
    }
}
