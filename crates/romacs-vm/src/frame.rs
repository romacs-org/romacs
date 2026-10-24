//! Lisp identity and ownership for the single initial display device.
//! No TTY is claimed: the initial terminal has no native TTY top frame.
use crate::{data::Object, *};
pub(super) const SELECTED_FRAME: &str = "#:vm2-frame";
#[derive(Clone)]
pub(super) struct Frame {
    pub root: Value,
    pub selected: Value,
    pub minibuffer: Value,
    pub terminal: Value,
    pub parameters: Vec<(Value, Value)>,
    pub live: bool,
}
impl Frame {
    pub fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) {
        for v in [&mut self.root, &mut self.selected, &mut self.minibuffer,
                  &mut self.terminal] { f(v); }
        for (k, v) in &mut self.parameters { f(k); f(v); }
    }
    pub fn storage_bytes(&self) -> usize {
        size_of::<Self>() + self.parameters.capacity() * size_of::<(Value, Value)>()
    }
}
#[derive(Clone)]
pub(super) struct Terminal {
    pub frame: Value,
    pub live: bool,
}
impl Terminal {
    pub fn visit_values(&mut self, f: &mut impl FnMut(&mut Value)) { f(&mut self.frame); }
    pub fn storage_bytes(&self) -> usize { size_of::<Self>() }
}
impl Vm {
    pub(super) fn frame_value_peek(&self) -> Value {
        self.names.get(SELECTED_FRAME).map_or(Value::UNBOUND, |&i| self.slots[i].value)
    }
    pub(super) fn selected_frame_value(&mut self) -> Result<Value> {
        self.ensure_windows()?;
        Ok(self.frame_value_peek())
    }
    pub(super) fn frame(&self, value: Value) -> Result<&Frame> {
        match self.object_if(value) {
            Some(Object::Frame(f)) => Ok(f),
            _ => Err("frame required".into()),
        }
    }
    pub(super) fn frame_mut(&mut self, value: Value) -> Result<&mut Frame> {
        match self.objects.get_mut(value.index(0)?.saturating_sub(1)) {
            Some(Object::Frame(f)) => Ok(f),
            _ => Err("frame required".into()),
        }
    }
    pub(super) fn allocate_initial_frame(&mut self, root: Value, minibuffer: Value) -> Result<()> {
        let terminal = self.allocate_object(Object::Terminal(Box::new(Terminal {
            frame: Value::NIL, live: true,
        })));
        let frame = self.allocate_object(Object::Frame(Box::new(Frame {
            root, selected: root, minibuffer, terminal,
            parameters: vec![], live: true,
        })));
        let Object::Terminal(t) = &mut self.objects[terminal.index(0)? - 1] else { unreachable!() };
        t.frame = frame;
        let slot = self.slot(SELECTED_FRAME);
        self.slots[slot].value = frame;
        let last = self.slot("#:vm2-last-nonminibuffer-frame");
        self.slots[last].value = frame;
        self.window_mut(root)?.frame = frame;
        self.window_mut(minibuffer)?.frame = frame;
        Ok(())
    }
}
