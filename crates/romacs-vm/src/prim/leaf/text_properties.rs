use super::*;
use crate::buffer::PropertyEdit;

/// GNU's interval I before the change hooks: the characters-modified count,
/// I's character POSITION and LENGTH (the start of the change and its run)
/// and the run's plist, and in a multibyte buffer markers at both ends (the
/// end one advancing) that show whether the hooks moved or resized I (nil in
/// a unibyte buffer, which has no markers; there any text change counts).
pub(crate) struct Retry { pub chars: u64, pub plist: Value, pub start: Value, pub end: Value, pub position: i64, pub length: i64 }

/// What a property primitive does next: it is finished, or the change hooks
/// must run around it (`romacs--before-change' ... `romacs--after-change').
pub(crate) enum PropertyBegin {
    Done(Value),
    /// RETRY is what GNU's retry check compares after the hooks.
    Hooks { buffer: Value, start: i64, end: i64, prepared: crate::insdel::Prepared, retry: Retry },
}

impl Leaf<'_> {
    /// Run F with BUFFER current, restoring the current buffer after.
    fn in_buffer<T>(&mut self, buffer: Value, f: impl FnOnce(&mut Self) -> std::result::Result<T, Fault>) -> std::result::Result<T, Fault> {
        let current = self.vm.slots[self.vm.buffer_slot].value;
        self.vm.slots[self.vm.buffer_slot].value = buffer;
        let result = f(self);
        self.vm.slots[self.vm.buffer_slot].value = current;
        result
    }
    /// modify_text_properties up to its hooks: plan EDIT; finish it here
    /// when it changes nothing, touches a string, or no change hook can
    /// run; else say which hooks must run first.
    pub(crate) fn property_begin(&mut self, edit: &PropertyEdit, start: Value, end: Value, object: Value) -> std::result::Result<PropertyBegin, Fault> {
        let Some(plan) = self.vm.property_plan(edit, start, end, object, false)? else { return Ok(PropertyBegin::Done(Value::NIL)) };
        let buffer = plan.object;
        if !matches!(self.vm.object_if(buffer), Some(data::Object::Buffer(_) | data::Object::ByteBuffer(_))) {
            self.vm.property_apply(plan, false)?;
            return Ok(PropertyBegin::Done(Value::TRUE));
        }
        let (start, end) = (plan.start, plan.end);
        let prepared = self.in_buffer(buffer, |cx| {
            let prepared = cx.vm.prepare_change_of(start, end, true)?;
            let after = !prepared.inhibited && cx.vm.after_change_runs(false, false)?;
            Ok(if prepared.before || after { Ok(prepared) } else { Err(!prepared.inhibited) })
        })?;
        match prepared {
            Err(hooks) => {
                self.vm.property_apply(plan, hooks)?;
                Ok(PropertyBegin::Done(Value::TRUE))
            }
            Ok(prepared) => {
                let chars = self.vm.buffer_state(buffer).map_err(Fault::Error)?.chars_modiff;
                let (run_end, plist) = self.vm.property_run_at(buffer, plan.first)?;
                let (position, length) = (self.vm.char_position(buffer, plan.first)?, self.vm.char_position(buffer, run_end)?);
                let length = length - position;
                let (start, end) = if matches!(self.vm.object_if(buffer), Some(data::Object::Buffer(_))) {
                    (self.vm.watch_marker(buffer, position, false)?, self.vm.watch_marker(buffer, position + length, true)?)
                } else { (Value::NIL, Value::NIL) };
                let retry = Retry { chars, plist, start, end, position, length };
                Ok(PropertyBegin::Hooks { buffer, start: plan.start, end: plan.end, prepared, retry })
            }
        }
    }

    /// The rest of the primitive once the before hooks ran: the modify tail,
    /// then the change, applied even if the hooks made it moot unless GNU's
    /// interval I changed under it. GNU then re-scans, and if nothing is left
    /// to do returns nil and runs no after-change hooks (None). I changes
    /// when text moved (characters modified), or when the hooks split it:
    /// add, remove and face watch I's position (a hook that changed the
    /// properties at I's start and ended inside it), set also I's length
    /// (any shorter run); remove-list never re-scans (WATCHES false). HOOKS
    /// says modification hooks were not inhibited. Some is the after
    /// hooks' BEG and END.
    pub(crate) fn property_finish(&mut self, edit: &PropertyEdit, start: Value, end: Value, buffer: Value, retry: Retry, watches: bool, hooks: bool) -> std::result::Result<Option<(i64, i64)>, Fault> {
        // I moved or changed length: the markers moved apart, or (unibyte) text changed.
        let moved = if retry.start == Value::NIL {
            self.vm.buffer_state(buffer).map_err(Fault::Error)?.chars_modiff != retry.chars
        } else {
            let (s, e) = (self.vm.release_marker(retry.start)?, self.vm.release_marker(retry.end)?);
            s != retry.position || e - s != retry.length
        };
        let rescan = watches && {
            moved || {
                // Same characters, but their bytes may not be: re-resolve I.
                let first = self.vm.byte_position(buffer, retry.position)?;
                let (run_end, plist) = self.vm.property_run_at(buffer, first)?;
                let shorter = self.vm.char_position(buffer, run_end)? - retry.position < retry.length;
                let same = self.vm.list_values(plist).ok() == self.vm.list_values(retry.plist).ok();
                if matches!(edit, PropertyEdit::Set(_)) { shorter } else { shorter && !same }
            }
        };
        let plan = self.vm.property_plan(edit, start, end, buffer, !rescan)?;
        self.vm.property_modify_tail(buffer, hooks)?;
        let Some(plan) = plan else { return Ok(None) };
        let positions = (plan.start, plan.end);
        self.vm.property_install(plan)?;
        Ok(Some(positions))
    }

    /// add-text-properties on a string (decoding's runs): no change hooks.
    pub(crate) fn add_text_properties(&mut self, start: Value, end: Value, properties: Value, object: Value) -> super::super::Out {
        self.vm.property_edit_now(&PropertyEdit::Add(properties), start, end, object)
    }
    pub(crate) fn next_single_property_change(&mut self, position: Value, prop: Value, object: Value, limit: Value) -> super::super::Out {
        self.vm.next_single_property_change_values(position, prop, object, limit)
    }
    pub(crate) fn previous_property_change(&mut self, position: Value, prop: Option<Value>, object: Value, limit: Value) -> super::super::Out {
        self.vm.previous_property_change_values(position, prop, object, limit)
    }
    pub(crate) fn next_property_change(
        &mut self,
        position: Value,
        object: Value,
        limit: Value,
    ) -> super::super::Out {
        self.vm.next_property_change_values(position, object, limit)
    }
    pub(crate) fn next_single_char_property_change(
        &mut self,
        position: Value,
        property: Value,
        object: Value,
        limit: Value,
    ) -> super::super::Out {
        self.vm
            .next_single_char_property_change_values(position, property, object, limit)
    }
    pub(crate) fn text_property_not_all(&mut self, start: Value, end: Value, property: Value, value: Value, object: Value) -> super::super::Out {
        self.vm.text_property_not_all_values(start, end, property, value, object)
    }
    pub(crate) fn text_property_any(&mut self, start: Value, end: Value, property: Value, value: Value, object: Value) -> super::super::Out {
        self.vm.text_property_any_values(start, end, property, value, object)
    }
}

impl Leaf<'_> {
    pub(crate) fn get_pos_property(&mut self, position: Value, property: Value, object: Value) -> super::super::Out {
        self.vm.get_pos_property_values(position, property, object)
    }
}
