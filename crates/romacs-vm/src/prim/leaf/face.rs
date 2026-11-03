//! Live Lisp face registry storage. Allocation does not collect inside a leaf.
use super::*;

impl Vm {
    pub(crate) fn register_declared_face(&mut self, face: Value, definition: romacs_face::FaceDef) -> crate::Result<()> {
        use romacs_face::{Attribute, Datum};
        let slots = romacs_face::registry::LFACE_ATTRIBUTES.iter().map(|name| {
            if *name == ":inherit" {
                definition.inherit().map_or("unspecified".to_owned(), |parents| {
                    Datum::List(parents.iter().map(|name| Datum::symbol(name)).collect()).to_lisp()
                })
            } else {
                Attribute::from_keyword(name).map_or("unspecified".to_owned(), |attribute| {
                    definition.attrs().get(attribute).to_lisp()
                })
            }
        }).collect::<Vec<_>>().join(" ");
        let source = format!("[face {slots}]");
        let global = self.read_source_raw(&source)?.ok_or("missing face vector")?.0;
        let local = self.read_source_raw(&source)?.ok_or("missing face vector")?.0;
        let frame = self.selected_frame_value()?;
        let mut leaf = Leaf { vm: self };
        leaf.face_insert(face, Value::NIL, global).map_err(|_| "cannot register declared face")?;
        leaf.face_insert(face, frame, local).map_err(|_| "cannot register frame face")?;
        Ok(())
    }

    // Like obarray, materialize the C-initialized hash on first symbol use,
    // keeping construction heap-free without making boundp observe a void cell.
    pub(crate) fn initialize_face_slot(&mut self, name: &str, slot: usize) {
        if name == "face--new-frame-defaults" {
            let table = hash::HashTable::new(hash::Test::Eq, hash::Weakness::None, 8);
            self.slots[slot].value = self.allocate_object(data::Object::HashTable(Box::new(table)));
            self.specials.insert(name.to_owned());
        }
    }
}

impl Leaf<'_> {
    fn face_table_new(&mut self, weakness: hash::Weakness) -> Value {
        let table = hash::HashTable::new(hash::Test::Eq, weakness, 8);
        self.vm.allocate_object(data::Object::HashTable(Box::new(table)))
    }

    /// Nil denotes the global registry. Frame registries are ephemeron values:
    /// a dead frame cannot be kept alive by its own face attributes.
    pub(crate) fn face_table(&mut self, frame: Value, create: bool) -> super::super::Out {
        let root_name = if frame == Value::NIL { "face--new-frame-defaults" }
            else { "#:vm2-frame-face-tables" };
        let root = self.vm.slot(root_name);
        if frame == Value::NIL {
            let table = self.vm.variable_value(root).map_err(Fault::Error)?;
            return if self.kind(table) == Kind::HashTable { Ok(table) }
                else { Err(Fault::Wrong("hash-table-p", table)) };
        }
        let mut table = self.vm.slots[root].value;
        if table == Value::UNBOUND || table == Value::NIL {
            if !create { return Ok(Value::NIL); }
            table = self.face_table_new(hash::Weakness::Key);
            self.vm.slots[root].value = table;
        }
        if let Some(local) = self.vm.declaration_hash_get(table, frame).map_err(Fault::Error)? {
            return Ok(local);
        }
        if !create { return Ok(Value::NIL); }
        let local = self.face_table_new(hash::Weakness::None);
        self.vm.declaration_hash_put(table, frame, local).map_err(Fault::Error)?;
        Ok(local)
    }

    pub(crate) fn face_frame(&mut self, frame: Value, selected: bool) -> super::super::Out {
        if frame == Value::NIL {
            if !selected { return Ok(Value::NIL); }
            return self.vm.selected_frame_value().map_err(Fault::Error);
        }
        match self.vm.object_if(frame) {
            Some(data::Object::Frame(f)) if f.live => Ok(frame),
            _ => Err(Fault::Wrong("frame-live-p", frame)),
        }
    }

    /// The frame whose faces FRAME shows: FRAME, unless it is a child
    /// frame that never had faces of its own, which shows its parent's
    /// (DIVERGENCES.md VM2-CHILD-FRAME).
    pub(crate) fn face_owner(&mut self, mut frame: Value) -> super::super::Out {
        loop {
            if self.face_table(frame, false)? != Value::NIL { return Ok(frame); }
            let parent = self.vm.frame(frame).map_or(Value::NIL, |f| f.parent);
            if parent == Value::NIL { return Ok(frame); }
            frame = parent;
        }
    }

    /// Whether FRAME shows another frame's faces: a child frame whose own
    /// were never set.
    pub(crate) fn frame_shares_faces(&mut self, frame: Value) -> std::result::Result<bool, Fault> {
        Ok(self.face_owner(frame)? != frame)
    }

    /// Give FRAME faces of its own, a copy of each of those it shows, as
    /// make-terminal-frame copies the selected frame's (frame.c:1879): the
    /// first time anything sets a face on FRAME alone.
    pub(crate) fn face_own(&mut self, frame: Value) -> super::super::Out {
        let owner = self.face_owner(frame)?;
        if owner == frame { return Ok(frame); }
        // The default colours it showed stay its own, as its default face's.
        let entries = self.windows().frame_parameter_entries(owner)?;
        let own = self.windows().frame_parameter_entries(frame)?;
        for name in ["foreground-color", "background-color"] {
            let key = self.intern(name);
            let colour = |cx: &Self, entries: &[(Value, Value)]| entries.iter().find(|(k, v)| *k == key && matches!(cx.kind(*v), Kind::String | Kind::Bytes)).map(|&(_, v)| v);
            if colour(self, &own).is_none() && let Some(value) = colour(self, &entries) {
                self.windows().store_frame_parameter(frame, key, value)?;
            }
        }
        let from = self.face_table(owner, false)?;
        let to = self.face_table(frame, true)?;
        if from == Value::NIL { return Ok(frame); }
        for (face, lface) in self.vm.declaration_hash_entries(from).map_err(Fault::Error)? {
            let values = self.vector_values(lface)?;
            let copy = self.vector_from(values);
            self.vm.declaration_hash_put(to, face, copy).map_err(Fault::Error)?;
        }
        Ok(frame)
    }

    pub(crate) fn face_lookup(&mut self, face: Value, frame: Value) -> super::super::Out {
        let frame = if frame == Value::NIL { frame } else { self.face_owner(frame)? };
        let table = self.face_table(frame, false)?;
        if table == Value::NIL { return Ok(Value::NIL); }
        let value = self.vm.declaration_hash_get(table, face).map_err(Fault::Error)?.unwrap_or(Value::NIL);
        Ok(if frame == Value::NIL { self.pair(value).map_or(Value::NIL, |(_, face)| face) } else { value })
    }

    pub(crate) fn face_insert(&mut self, face: Value, frame: Value, vector: Value) -> super::super::Out {
        if frame != Value::NIL { self.face_own(frame)?; }
        let table = self.face_table(frame, true)?;
        let value = if frame == Value::NIL {
            let counter = self.vm.slot("#:vm2-next-face-id");
            let id = self.vm.slots[counter].value.integer_value().unwrap_or(0);
            let value = Value::integer(id).map_err(Fault::Error)?;
            self.vm.slots[counter].value = Value::integer(id + 1).map_err(Fault::Error)?;
            let key = self.intern("face");
            self.vm.put_symbol_property(face, key, value).map_err(Fault::Error)?;
            // Glyph codes name faces by these ids.
            let names = self.vm.slot("#:vm2-face-names");
            let entry = self.cons(value, face);
            self.vm.slots[names].value = { let rest = self.vm.slots[names].value; let rest = if rest == Value::UNBOUND { Value::NIL } else { rest }; self.cons(entry, rest) };
            self.cons(value, vector)
        } else { vector };
        self.vm.declaration_hash_put(table, face, value).map_err(Fault::Error)?;
        Ok(vector)
    }

    /// The face whose `face-id' is ID, else nil.
    pub(crate) fn face_named_by_id(&mut self, id: i64) -> Value {
        let names = self.vm.slot("#:vm2-face-names");
        let mut list = self.vm.slots[names].value;
        while let Some((entry, rest)) = self.pair(list) {
            if let Some((key, face)) = self.pair(entry) && key.integer_value() == Some(id) { return face; }
            list = rest;
        }
        Value::NIL
    }

    pub(crate) fn face_property(&mut self, face: Value, property: Value) -> super::super::Out {
        self.vm.symbol_property(face, property).map_err(Fault::Error)
    }
}
