//! xdisp.c Fcurrent_bidi_paragraph_direction over crate::bidi's search.
use super::*;
use crate::bidi::{self, Direction, Re};
use crate::data::Object;

impl Leaf<'_> {
    /// `current-bidi-paragraph-direction': BUFFER's (default the current
    /// buffer's) `bidi-paragraph-direction' when set, else the direction of
    /// the paragraph at or before its point; left to right when
    /// `bidi-display-reordering' is nil or the buffer is unibyte.
    pub(crate) fn current_bidi_paragraph_direction(&mut self, buffer: Value) -> super::super::Out {
        let buffer = if buffer == Value::NIL { self.vm.slots[self.vm.buffer_slot].value } else { buffer };
        if !matches!(self.vm.object_if(buffer), Some(Object::Buffer(_) | Object::ByteBuffer(_))) {
            return Err(Fault::Wrong("bufferp", buffer));
        }
        let slot = self.vm.buffer_slot;
        let saved = self.vm.slots[slot].value;
        self.vm.slots[slot].value = buffer;
        let answer = self.paragraph_direction_here(buffer);
        self.vm.slots[slot].value = saved;
        answer
    }

    fn paragraph_direction_here(&mut self, buffer: Value) -> super::super::Out {
        let mut var = |leaf: &mut Self, name: &str| -> std::result::Result<Value, Fault> {
            let symbol = leaf.intern(name);
            Ok(leaf.variable_value(symbol)?.unwrap_or(Value::NIL))
        };
        let left = self.intern("left-to-right");
        if var(self, "bidi-display-reordering")? == Value::NIL || matches!(self.vm.object_if(buffer), Some(Object::ByteBuffer(_))) {
            return Ok(left);
        }
        let given = var(self, "bidi-paragraph-direction")?;
        if given != Value::NIL { return Ok(given); }
        let mut regexp = |leaf: &mut Self, name: &str, default: &str| -> std::result::Result<Value, Fault> {
            let value = var(leaf, name)?;
            Ok(if leaf.string_view(value).is_ok() { value }
                else { leaf.vm.allocate_object(Object::String(Box::new(crate::data::LispString::new(default)))) })
        };
        let patterns = [regexp(self, "bidi-paragraph-start-re", bidi::START_RE)?,
            regexp(self, "bidi-paragraph-separate-re", bidi::SEPARATE_RE)?,
            regexp(self, "#:vm2-bidi-trailing-blank", bidi::TRAILING_BLANK_RE)?];
        let direction = self.vm.with_buffer_looking_at(&patterns, |text, point, looks| {
            bidi::paragraph_direction(text, point, &mut |re, at| looks(re as usize, at))
        })?;
        Ok(match direction {
            Direction::LeftToRight => left,
            Direction::RightToLeft => self.intern("right-to-left"),
        })
    }
}

const _: () = assert!(Re::Start as usize == 0 && Re::Separate as usize == 1 && Re::TrailingBlank as usize == 2);
