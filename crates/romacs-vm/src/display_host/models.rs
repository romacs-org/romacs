//! Native model surfaces on the open display (rlisp/romacs-model.el). The
//! VM names each surface by an integer id it allocates, sends Lisp's
//! requests to the display host, and hands the renderer's events to Lisp as
//! special events `(romacs-model ID TYPE PLIST)`, which `special-event-map'
//! runs at once, inside a read or between commands. Lisp owns each handle
//! and frees its surface when the handle is collected (a finalizer) or
//! destroyed: the VM keeps only the ids still live, to refuse requests to a
//! freed one.
use crate::{Result, Value, Vm, data};
use romacs_display_protocol::model::{ModelEvent, ModelHit, ModelView};

/// The live native surfaces of one display: id -> mesh count.
#[derive(Default)]
pub(crate) struct Surfaces {
    next: u32,
    models: rustc_hash::FxHashMap<u32, usize>,
}

impl Surfaces {
    /// A fresh id, never 0 (layout's "no surface").
    pub(crate) fn allocate(&mut self) -> u32 {
        loop {
            self.next = self.next.wrapping_add(1);
            if self.next != 0 && !self.models.contains_key(&self.next) { return self.next; }
        }
    }
    pub(crate) fn insert(&mut self, id: u32, meshes: usize) {
        self.models.insert(id, meshes);
    }
    /// Model ID's mesh count, None once freed.
    pub(crate) fn meshes(&self, id: u32) -> Option<usize> {
        self.models.get(&id).copied()
    }
    /// Forget ID: whether it was live.
    pub(crate) fn remove(&mut self, id: u32) -> bool {
        self.models.remove(&id).is_some()
    }
}

impl Vm {
    /// The renderer's EVENT about model ID as the special event Lisp reads:
    /// `(romacs-model ID TYPE PLIST)'.
    pub(crate) fn model_event_value(&mut self, id: u32, event: &ModelEvent) -> Value {
        let kind = self.intern("romacs-model");
        let id = Value::integer(i64::from(id)).expect("a surface id is a fixnum");
        let ty = self.intern(event.kind());
        let plist = self.model_event_plist(event);
        self.list_value(&[kind, id, ty, plist])
    }

    fn model_event_plist(&mut self, event: &ModelEvent) -> Value {
        let mut items = vec![];
        match event {
            ModelEvent::Ready { triangles } => {
                let counts: Vec<Value> = triangles.iter().map(|&n| integer(u64::from(n))).collect();
                let counts = self.list_value(&counts);
                self.push(&mut items, ":triangles", counts);
            }
            ModelEvent::Failed { error } => {
                let error = self.text(error);
                self.push(&mut items, ":error", error);
            }
            ModelEvent::Error { req, error } => {
                let req = req.map_or(Value::NIL, integer);
                self.push(&mut items, ":req", req);
                let error = self.text(error);
                self.push(&mut items, ":error", error);
            }
            ModelEvent::View { view, rev } => {
                let view = self.view_value(view);
                self.push(&mut items, ":view", view);
                self.push(&mut items, ":rev", integer(*rev));
            }
            ModelEvent::Readiness { current, rev, req } => {
                self.push(&mut items, ":current", if *current { Value::TRUE } else { Value::NIL });
                self.push(&mut items, ":rev", integer(*rev));
                self.push(&mut items, ":req", integer(*req));
            }
            ModelEvent::Pick { hit, view, rev, req } => {
                let hit = hit.as_ref().map_or(Value::NIL, |hit| self.hit_value(hit));
                self.push(&mut items, ":hit", hit);
                let view = self.view_value(view);
                self.push(&mut items, ":view", view);
                self.push(&mut items, ":rev", integer(*rev));
                self.push(&mut items, ":req", integer(*req));
            }
        }
        self.list_value(&items)
    }

    fn push(&mut self, items: &mut Vec<Value>, key: &str, value: Value) {
        let key = self.intern(key);
        items.extend([key, value]);
    }

    fn text(&mut self, text: &str) -> Value {
        self.make_lisp_string(text.as_bytes().to_vec(), !text.is_ascii())
    }

    fn floats(&mut self, values: &[f64]) -> Vec<Value> {
        values.iter().map(|&n| self.allocate_object(data::Object::Float(n))).collect()
    }

    fn view_value(&mut self, view: &ModelView) -> Value {
        let values = self.floats(&[view.yaw, view.pitch, view.zoom]);
        self.list_value(&values)
    }

    /// `(:mesh INDEX :face N :barycentric [A B C] :point [X Y Z])'.
    fn hit_value(&mut self, hit: &ModelHit) -> Value {
        let barycentric = self.floats(&hit.barycentric);
        let barycentric = self.allocate_object(data::Object::Vector(barycentric));
        let point = self.floats(&hit.point);
        let point = self.allocate_object(data::Object::Vector(point));
        let mut items = vec![];
        self.push(&mut items, ":mesh", integer(u64::from(hit.mesh)));
        self.push(&mut items, ":face", integer(u64::from(hit.face)));
        self.push(&mut items, ":barycentric", barycentric);
        self.push(&mut items, ":point", point);
        self.list_value(&items)
    }

    /// Run the renderer's EVENT about model ID as the special event it is
    /// (keyboard.c read_char): its `special-event-map' binding, here
    /// between commands. Inside a read, the read runs it
    /// ([`crate::HostEvent::Model`]). A freed model's events still go:
    /// Lisp drops what names no live handle.  The embedder's top level
    /// calls it: an error or quit comes back as the result, with no engine
    /// above to hand an escape to.
    pub fn run_model_event(&mut self, id: u32, event: &ModelEvent) -> Result<()> {
        let value = self.model_event_value(id, event);
        self.call_prepared_source_raw(
            "(lambda (event) (romacs--run-special-event event))",
            &[value],
        )?;
        Ok(())
    }
}

fn integer(n: u64) -> Value {
    // Request and revision numbers stay far below the fixnum range.
    Value::integer(i64::try_from(n).unwrap_or(i64::MAX)).unwrap_or(Value::NIL)
}
