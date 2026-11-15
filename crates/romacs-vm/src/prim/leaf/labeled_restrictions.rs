//! editfns.c's labeled restrictions, the `:label` of `with-restriction`. A buffer
//! in one holds a stack of (LABEL BEGV ZV), innermost first, BEGV and ZV
//! markers at its bounds, over (nil BEGV ZV), the stack's last entry: the bounds
//! it had when the first was entered. While it holds one, `narrow-to-region`
//! narrows within the innermost and `widen` widens to it; `save-restriction`
//! saves and restores the stack with the bounds.
//!
//! GNU keeps an alist keyed by buffer (editfns.c `labeled_restrictions`). Here
//! each buffer keeps its own list (`PerBuffer`), so it is traced with the
//! buffer, goes with it, and an indirect buffer starts without one. A list
//! is never changed in place: a push conses onto it, a pop takes its tail.
use super::{Fault, Leaf, Primitive, Result, Value, Vm};
use crate::{anchors::Bias, annotations::Marker, data::Object, prim::Out};

impl Vm {
    /// Buffer INDEX's labeled restrictions, nil when it has none.
    pub(crate) fn labeled_restrictions(&self, index: usize) -> Value {
        self.per_buffer(index).map_or(Value::NIL, crate::forwarded::PerBuffer::labeled_restrictions)
    }

    pub(crate) fn set_labeled_restrictions(&mut self, index: usize, list: Value) {
        if self.labeled_restrictions(index) != list
            && let Some(p) = self.per_buffer_mut(index) {
            p.set_labeled_restrictions(list);
        }
    }

    /// The innermost labeled restriction of buffer INDEX: its label (None
    /// for the outermost bounds, which GNU labels with an uninterned symbol
    /// no caller can name) and its bounds as character positions
    /// (`labeled_restrictions_peek_label` and `labeled_restrictions_get_bound`).
    pub(crate) fn innermost_labeled(&self, index: usize) -> Result<Option<(Option<Value>, i64, i64)>> {
        let list = self.labeled_restrictions(index);
        if list == Value::NIL { return Ok(None); }
        let (entry, under) = self.cons_pair(list)?;
        let (label, bounds) = self.cons_pair(entry)?;
        let (begv, rest) = self.cons_pair(bounds)?;
        let zv = self.cons_pair(rest)?.0;
        let label = (under != Value::NIL).then_some(label);
        Ok(Some((label, self.marker_position_value(begv)?.as_integer()?, self.marker_position_value(zv)?.as_integer()?)))
    }

    /// `labeled_restrictions_pop`: drop the innermost, or the whole stack when
    /// only the outermost bounds are left.
    pub(crate) fn pop_labeled(&mut self, index: usize) -> Result<()> {
        let list = self.labeled_restrictions(index);
        if list == Value::NIL { return Ok(()); }
        let rest = self.cons_pair(list)?.1;
        self.set_labeled_restrictions(index, rest);
        Ok(())
    }

    /// What `narrow-to-region` (P `Narrow`, ARGS START END) and `widen` do in
    /// buffer INDEX while it holds a labeled restriction (editfns.c
    /// `Fnarrow_to_region`, `Fwiden`): a narrowing within the buffer is clipped to the innermost's
    /// bounds, and a widening narrows to them, dropping the outermost bounds
    /// once they are reached. The primitive to run and its arguments.
    pub(crate) fn labeled_bounds_call(&mut self, index: usize, p: Primitive, args: Vec<Value>) -> Result<(Primitive, Vec<Value>)> {
        let Some((label, begv, zv)) = self.innermost_labeled(index)? else { return Ok((p, args)) };
        if p == Primitive::Widen {
            if label.is_none() { self.pop_labeled(index)?; }
            return Ok((Primitive::Narrow, vec![Value::integer(begv)?, Value::integer(zv)?]));
        }
        let (Ok(s), Ok(e)) = (args[0].as_integer(), args[1].as_integer()) else { return Ok((p, args)) };
        let z = match &self.objects[index] {
            Object::Buffer(t) => i64::try_from(t.len_chars()).map_err(|_| "buffer too large")? + 1,
            Object::ByteBuffer(b) => i64::try_from(b.bytes.len()).map_err(|_| "buffer too large")? + 1,
            _ => return Ok((p, args)),
        };
        // A position outside the buffer is refused unclipped, as GNU refuses it.
        if s.min(e) < 1 || s.max(e) > z { return Ok((p, args)); }
        let clip = |n: i64| Value::integer(n.clamp(begv, zv));
        Ok((p, vec![clip(s)?, clip(e)?]))
    }

    /// (LABEL BEGV ZV) at buffer INDEX's bounds now, its markers of
    /// insertion type nil, as `point-min-marker` and `point-max-marker` make them.
    fn labeled_entry(&mut self, index: usize, label: Value) -> Result<Value> {
        let bounds = match &self.objects[index] {
            Object::Buffer(t) => t.accessible(),
            Object::ByteBuffer(b) => b.restriction.clone(),
            _ => return Err("buffer required".into()),
        };
        let buffer = self.slots[self.buffer_slot].value;
        let tree = self.marker_anchors_mut(buffer)?;
        let [begv, zv] = [bounds.start, bounds.end].map(|at| tree.create(u32::try_from(at.0).expect("an anchor offset"), Bias::Left).expect("restriction anchor"));
        let [begv, zv] = [begv, zv].map(|anchor| {
            self.allocate_object(Object::Marker(Box::new(Marker { buffer, anchor: Some(anchor), bias: Bias::Left, last_position: 0 })))
        });
        Ok(self.list_value(&[label, begv, zv]))
    }
}

impl Leaf<'_> {
    /// The outermost bounds a first labeled restriction would keep: buffer
    /// INDEX's now, or nil when it is in one already.
    pub(crate) fn labeled_outermost(&mut self) -> Out {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        if self.vm.labeled_restrictions(index) != Value::NIL { return Ok(Value::NIL); }
        self.vm.labeled_entry(index, Value::NIL).map_err(Fault::Error)
    }

    /// `labeled_restrictions_push`: LABEL over the current buffer's bounds,
    /// above OUTERMOST when that is the first.
    pub(crate) fn push_labeled(&mut self, label: Value, outermost: Value) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        let mut list = self.vm.labeled_restrictions(index);
        if list == Value::NIL && outermost != Value::NIL { list = self.vm.list_value(&[outermost]); }
        let entry = self.vm.labeled_entry(index, label).map_err(Fault::Error)?;
        let list = self.vm.alloc_cons(entry, list);
        self.vm.set_labeled_restrictions(index, list);
        Ok(())
    }

    /// The first half of `Finternal__labeled_widen`: drop the innermost labeled
    /// restriction when it is LABEL's.
    pub(crate) fn pop_labeled_if(&mut self, label: Value) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        match self.vm.innermost_labeled(index).map_err(Fault::Error)? {
            Some((Some(innermost), ..)) if innermost == label => self.vm.pop_labeled(index).map_err(Fault::Error),
            _ => Ok(()),
        }
    }

    /// `labeled_restrictions_remove_in_current_buffer`.
    pub(crate) fn drop_labeled(&mut self) -> std::result::Result<(), Fault> {
        let index = self.vm.buffer_index().map_err(Fault::Error)?;
        self.vm.set_labeled_restrictions(index, Value::NIL);
        Ok(())
    }
}
