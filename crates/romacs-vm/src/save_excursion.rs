//! GNU editfns.c:776-840: an excursion is a left-biased point marker,
//! not a numeric position or a binding of the current-buffer slot.
use crate::{Builder, Form, Op, Primitive, Result, Value, Vm, anchors::Bias, annotations::Marker, coordinates::ByteOffset, data::Object};

impl Vm {
    // Use immutable native callable constants: rebinding Lisp function cells
    // inside BODY must not intercept a special form's save/restore machinery.
    fn excursion_call(b: &mut Builder, primitive: Primitive, args: &[usize]) -> usize {
        Self::native_call(b, Value::reference(primitive as usize, 6), args)
    }
    pub(super) fn native_call(b: &mut Builder, callable: Value, args: &[usize]) -> usize {
        let target = b.reg();
        b.emit(Op::Const(target, callable));
        let start = b.next;
        for &arg in args {
            let r = b.reg();
            b.emit(Op::Move(r, arg));
        }
        let dest = b.reg();
        b.emit(Op::Call(dest, target, start, args.len()));
        dest
    }

    pub(super) fn save_excursion(&mut self, b: &mut Builder, body: &[Form]) -> Result<usize> {
        use Primitive::*;
        let subr = |name| crate::prim::subr_named(name).expect("save-excursion subr");
        let marker = Self::native_call(b, subr("romacs--save-excursion"), &[]);
        let window = Self::excursion_call(b, SelectedWindow, &[]);
        let shown = Self::excursion_call(b, WindowBuffer, &[window]);
        let original = Self::excursion_call(b, CurrentBuffer, &[]);
        let was_shown = Self::excursion_call(b, Eq, &[shown, original]);
        let dest = b.reg();
        let protect = b.emit(Op::Protect(dest, 0, 0));
        let result = self.progn(b, body)?;
        b.emit(Op::LeaveProtect(result));
        let cleanup = b.code.len();
        let buffer = Self::native_call(b, subr("romacs--restore-excursion"), &[marker]);
        let dead = b.emit(Op::Unless(buffer, 0));
        let not_shown = b.emit(Op::Unless(was_shown, 0));
        let live = Self::excursion_call(b, WindowLiveP, &[window]);
        let dead_window = b.emit(Op::Unless(live, 0));
        let selected = Self::excursion_call(b, SelectedWindow, &[]);
        let same = Self::excursion_call(b, Eq, &[window, selected]);
        let different = b.emit(Op::Unless(same, 0));
        let same_window = b.emit(Op::Jump(0));
        b.code[different] = Op::Unless(same, b.code.len());
        let shown_now = Self::excursion_call(b, WindowBuffer, &[window]);
        let still_shown = Self::excursion_call(b, Eq, &[shown_now, buffer]);
        let changed = b.emit(Op::Unless(still_shown, 0));
        let restored = Self::excursion_call(b, Point, &[]);
        Self::excursion_call(b, SetWindowPoint, &[window, restored]);
        let end = b.code.len();
        for (branch, reg) in [(dead, buffer), (not_shown, was_shown), (dead_window, live), (changed, still_shown)] {
            b.code[branch] = Op::Unless(reg, end);
        }
        b.code[same_window] = Op::Jump(end);
        b.emit(Op::ResumeExit);
        b.code[protect] = Op::Protect(dest, cleanup, b.code.len());
        Ok(dest)
    }
}

impl Vm {
    /// editfns.c save_excursion_save: a left-biased marker at point.
    pub(crate) fn save_excursion_data(&mut self) -> Result<Value> {
        let index = self.buffer_index()?;
        let buffer = self.slots[self.buffer_slot].value;
        let point = match &self.objects[index] {
            Object::Buffer(t) => t.point,
            Object::ByteBuffer(b) => b.point,
            _ => return Err("buffer required".into()),
        };
        let anchor = self.marker_anchors_mut(buffer)?.create(point.0 as u32, Bias::Left).expect("excursion anchor");
        Ok(self.allocate_object(Object::Marker(Box::new(Marker { buffer, anchor: Some(anchor), bias: Bias::Left }))))
    }

    /// editfns.c save_excursion_restore's buffer and point half: unless the
    /// buffer was killed, make it current and move point to the saved
    /// position (clipped to the accessible portion, as goto-char does), then
    /// detach the marker. Returns the buffer, or nil when it was killed.
    pub(crate) fn restore_excursion_data(&mut self, data: Value) -> Result<Value> {
        let buffer = self.marker(data)?.buffer;
        if buffer == Value::NIL { return Ok(Value::NIL); }
        self.require_live_buffer(buffer)?;
        self.slots[self.buffer_slot].value = buffer;
        let at = ByteOffset(self.marker_byte(self.marker(data)?)?.ok_or("invalid marker anchor")?);
        match self.object_mut(buffer)? {
            Object::Buffer(t) => {
                let accessible = t.accessible();
                t.point = at.clamp(accessible.start, accessible.end);
            }
            Object::ByteBuffer(b) => b.point = at.clamp(b.restriction.start, b.restriction.end),
            _ => unreachable!(),
        }
        self.set_marker_object(data, Value::NIL, Value::NIL)?;
        self.display_state.version += 1;
        Ok(buffer)
    }

    /// GNU editfns.c Fsave_restriction: save the restriction, run BODY, and
    /// restore it on every exit. Cleanup does not change the current buffer.
    pub(super) fn save_restriction(&mut self, b: &mut Builder, body: &[Form]) -> Result<usize> {
        let subr = |name| crate::prim::subr_named(name).expect("save-restriction subr");
        let saved = Self::native_call(b, subr("romacs--save-restriction"), &[]);
        let dest = b.reg();
        let protect = b.emit(Op::Protect(dest, 0, 0));
        let result = self.progn(b, body)?;
        b.emit(Op::LeaveProtect(result));
        let cleanup = b.code.len();
        Self::native_call(b, subr("romacs--restore-restriction"), &[saved]);
        b.emit(Op::ResumeExit);
        b.code[protect] = Op::Protect(dest, cleanup, b.code.len());
        Ok(dest)
    }

    /// editfns.c save_restriction_save_1: the current buffer when it is not
    /// narrowed, else (BEG . END), a left-biased marker at BEGV and a
    /// right-biased one at ZV.
    pub(crate) fn save_restriction_data(&mut self) -> Result<Value> {
        let index = self.buffer_index()?;
        let buffer = self.slots[self.buffer_slot].value;
        let bounds = match &self.objects[index] {
            Object::Buffer(t) if t.accessible() != (ByteOffset(0)..ByteOffset(t.len_bytes())) => t.accessible(),
            Object::ByteBuffer(b) if b.restriction != (ByteOffset(0)..ByteOffset(b.bytes.len())) => b.restriction.clone(),
            _ => return Ok(buffer),
        };
        let tree = self.marker_anchors_mut(buffer)?;
        let beg = tree.create(bounds.start.0 as u32, Bias::Left).expect("restriction anchor");
        let end = tree.create(bounds.end.0 as u32, Bias::Right).expect("restriction anchor");
        let [beg, end] = [(beg, Bias::Left), (end, Bias::Right)].map(|(anchor, bias)| {
            self.allocate_object(Object::Marker(Box::new(Marker { buffer, anchor: Some(anchor), bias })))
        });
        Ok(self.alloc_cons(beg, end))
    }

    /// editfns.c save_restriction_restore_1: widen a buffer saved unnarrowed,
    /// or restore a saved restriction and detach its markers, unless the
    /// buffer was killed. Point is clipped into the restored bounds.
    pub(crate) fn restore_restriction_data(&mut self, data: Value) -> Result<()> {
        let Ok((car, cdr)) = self.cons_pair(data) else {
            if self.buffer_state(data)?.live {
                match self.object_mut(data)? {
                    Object::Buffer(t) => t.widen(),
                    Object::ByteBuffer(b) => b.restriction = ByteOffset(0)..ByteOffset(b.bytes.len()),
                    _ => unreachable!(),
                }
                self.display_state.version += 1;
            }
            return Ok(());
        };
        let buffer = self.marker(car)?.buffer;
        if let (Some(beg), Some(end)) = (self.marker_byte(self.marker(car)?)?, self.marker_byte(self.marker(cdr)?)?) {
            let bounds = ByteOffset(beg.min(end))..ByteOffset(beg.max(end));
            match self.object_mut(buffer)? {
                Object::Buffer(t) => t.narrow(bounds)?,
                Object::ByteBuffer(b) => {
                    b.point = b.point.clamp(bounds.start, bounds.end);
                    b.restriction = bounds;
                }
                _ => unreachable!(),
            }
            self.display_state.version += 1;
        }
        for marker in [car, cdr] { self.set_marker_object(marker, Value::NIL, Value::NIL)?; }
        Ok(())
    }
}
