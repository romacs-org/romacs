use super::*;
use crate::{anchors::Bias, annotations::Marker, data::Object, prim::Out};

impl Vm {
    pub(crate) fn buffer_mark_value(&self, buffer: Value) -> Option<Value> {
        match self.object_if(buffer) {
            Some(Object::Buffer(b)) => b.per_buffer.editor_mark(),
            Some(Object::ByteBuffer(b)) => b.per_buffer.editor_mark(),
            _ => None,
        }
    }
}

impl Leaf<'_> {
    pub(crate) fn editor_mark_marker(&mut self) -> Out {
        let buffer = self.current_buffer()?;
        if let Some(marker) = self.vm.buffer_mark_value(buffer) { return Ok(marker); }
        let marker = self.vm.allocate_object(Object::Marker(Box::new(Marker {
            buffer: Value::NIL, anchor: None, bias: Bias::Left, last_position: 0
        })));
        match self.vm.objects.entry_mut(buffer.index(0).map_err(Fault::Error)? - 1) {
            Object::Buffer(b) => b.per_buffer.set_editor_mark(marker),
            Object::ByteBuffer(b) => b.per_buffer.set_editor_mark(marker),
            _ => unreachable!(),
        }
        Ok(marker)
    }
    pub(crate) fn editor_point_marker(&mut self) -> Out {
        let point = self.point()?;
        self.editor_position_marker(point)
    }
    pub(crate) fn editor_position_marker(&mut self, position: i64) -> Out {
        let buffer = self.current_buffer()?;
        let point = Value::integer(position).map_err(Fault::Error)?;
        let marker = self.vm.allocate_object(Object::Marker(Box::new(Marker {
            buffer: Value::NIL, anchor: None, bias: Bias::Left, last_position: 0
        })));
        self.vm.set_marker_object(marker, point, buffer).map_err(Fault::Error)
    }
    fn editor_check_activation(&mut self) -> std::result::Result<(), Fault> {
        let transient = self.intern("transient-mark-mode");
        let even = self.intern("mark-even-if-inactive");
        let active = self.intern("mark-active");
        if self.variable_value(transient)?.unwrap_or(Value::NIL) != Value::NIL
            && self.variable_value(even)?.unwrap_or(Value::NIL) == Value::NIL
            && self.variable_value(active)?.unwrap_or(Value::NIL) == Value::NIL {
            let condition = self.intern("mark-inactive");
            return Err(Fault::Signal(condition, Value::NIL));
        }
        Ok(())
    }
    pub(crate) fn editor_checked_mark(&mut self, region: bool) -> Out {
        let buffer = self.current_buffer()?;
        let marker = self.editor_mark_marker()?;
        let belongs = matches!(self.vm.object_if(marker), Some(Object::Marker(m)) if m.buffer == buffer);
        if !belongs {
            return Err(Fault::Error(if region { "The mark is not set now, so there is no region" }
                                   else { "The mark is not set now" }.into()));
        }
        self.editor_check_activation()?;
        self.vm.marker_position_value(marker).map_err(Fault::Error)
    }
    pub(crate) fn editor_region_limit(&mut self, beginning: bool) -> Out {
        self.editor_check_activation()?;
        let buffer = self.current_buffer()?;
        let marker = self.vm.buffer_mark_value(buffer).unwrap_or(Value::NIL);
        let mark = self.marker_position(marker).flatten()
            .ok_or_else(|| Fault::Error("The mark is not set now, so there is no region".into()))?;
        let point = self.point()?;
        let shape = self.buffer_shape()?;
        let mark = mark.clamp(shape.begv, shape.zv);
        Value::integer(if beginning { point.min(mark) } else { point.max(mark) }).map_err(Fault::Error)
    }
}
