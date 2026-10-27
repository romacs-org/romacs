//! GNU editfns.c:776-840: an excursion is a left-biased point marker,
//! not a numeric position or a binding of the current-buffer slot.
use crate::{Builder, Form, Op, Primitive, Result, Value, Vm};

impl Vm {
    // Use immutable native callable constants: rebinding Lisp function cells
    // inside BODY must not intercept a special form's save/restore machinery.
    fn excursion_call(b: &mut Builder, primitive: Primitive, args: &[usize]) -> usize {
        let target = b.reg();
        b.emit(Op::Const(target, Value::reference(primitive as usize, 6)));
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
        let point = Self::excursion_call(b, Point, &[]);
        let marker = Self::excursion_call(b, CopyMarker, &[point]);
        let window = Self::excursion_call(b, SelectedWindow, &[]);
        let shown = Self::excursion_call(b, WindowBuffer, &[window]);
        let original = Self::excursion_call(b, CurrentBuffer, &[]);
        let was_shown = Self::excursion_call(b, Eq, &[shown, original]);
        let dest = b.reg();
        let protect = b.emit(Op::Protect(dest, 0, 0));
        let result = self.progn(b, body)?;
        b.emit(Op::LeaveProtect(result));
        let cleanup = b.code.len();
        let buffer = Self::excursion_call(b, MarkerBuffer, &[marker]);
        let dead = b.emit(Op::Unless(buffer, 0));
        Self::excursion_call(b, SetBuffer, &[buffer]);
        let position = Self::excursion_call(b, MarkerPosition, &[marker]);
        Self::excursion_call(b, GotoChar, &[position]);
        let nil = b.reg();
        b.emit(Op::Const(nil, Value::NIL));
        Self::excursion_call(b, SetMarker, &[marker, nil]);
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
    /// GNU editfns.c save_restriction_save/restore: an unnarrowed buffer is
    /// saved as itself and restored by widening it if it is live; a
    /// restriction as a left-biased start and right-biased end marker,
    /// restored and detached. Cleanup does not change the current buffer.
    pub(super) fn save_restriction(&mut self, b: &mut Builder, body: &[Form]) -> Result<usize> {
        use Primitive::*;
        let min = Self::excursion_call(b, PointMin, &[]);
        let max = Self::excursion_call(b, PointMax, &[]);
        let size = Self::excursion_call(b, BufferSize, &[]);
        let z = Self::excursion_call(b, Inc, &[size]);
        let one = b.reg();
        b.emit(Op::Const(one, Value::integer(1)?));
        let at_beg = Self::excursion_call(b, Eq, &[min, one]);
        let at_end = Self::excursion_call(b, Eq, &[max, z]);
        let saved = Self::excursion_call(b, CurrentBuffer, &[]);
        let (start, end, t) = (b.reg(), b.reg(), b.reg());
        b.emit(Op::Const(start, Value::NIL));
        b.emit(Op::Const(end, Value::NIL));
        b.emit(Op::Const(t, Value::TRUE));
        let narrowed_beg = b.emit(Op::Unless(at_beg, 0));
        let narrowed_end = b.emit(Op::Unless(at_end, 0));
        let whole = b.emit(Op::Jump(0));
        let narrowed = b.code.len();
        b.code[narrowed_beg] = Op::Unless(at_beg, narrowed);
        b.code[narrowed_end] = Op::Unless(at_end, narrowed);
        let marker = Self::excursion_call(b, CopyMarker, &[min]);
        b.emit(Op::Move(start, marker));
        let marker = Self::excursion_call(b, CopyMarker, &[max, t]);
        b.emit(Op::Move(end, marker));
        b.code[whole] = Op::Jump(b.code.len());
        let dest = b.reg();
        let protect = b.emit(Op::Protect(dest, 0, 0));
        let result = self.progn(b, body)?;
        b.emit(Op::LeaveProtect(result));
        let cleanup = b.code.len();
        let current = Self::excursion_call(b, CurrentBuffer, &[]);
        let unnarrowed = b.emit(Op::Unless(start, 0));
        let buffer = Self::excursion_call(b, MarkerBuffer, &[start]);
        let dead = b.emit(Op::Unless(buffer, 0));
        Self::excursion_call(b, SetBuffer, &[buffer]);
        let from = Self::excursion_call(b, MarkerPosition, &[start]);
        let to = Self::excursion_call(b, MarkerPosition, &[end]);
        Self::excursion_call(b, Narrow, &[from, to]);
        Self::excursion_call(b, SetBuffer, &[current]);
        let nil = b.reg();
        b.emit(Op::Const(nil, Value::NIL));
        Self::excursion_call(b, SetMarker, &[start, nil]);
        Self::excursion_call(b, SetMarker, &[end, nil]);
        let restored = b.emit(Op::Jump(0));
        b.code[unnarrowed] = Op::Unless(start, b.code.len());
        let live_p = b.reg();
        b.emit(Op::Const(live_p, crate::prim::subr_named("buffer-live-p").expect("buffer-live-p subr")));
        let arg = b.reg();
        b.emit(Op::Move(arg, saved));
        let live = b.reg();
        b.emit(Op::Call(live, live_p, arg, 1));
        let killed = b.emit(Op::Unless(live, 0));
        Self::excursion_call(b, SetBuffer, &[saved]);
        Self::excursion_call(b, Widen, &[]);
        Self::excursion_call(b, SetBuffer, &[current]);
        b.code[killed] = Op::Unless(live, b.code.len());
        b.code[restored] = Op::Jump(b.code.len());
        b.code[dead] = Op::Unless(buffer, b.code.len());
        b.emit(Op::ResumeExit);
        b.code[protect] = Op::Protect(dest, cleanup, b.code.len());
        Ok(dest)
    }
}
