//! Live Lisp face registry storage. Allocation does not collect inside a leaf.
use super::*;

/// The symbols the face primitives compare against, interned once: a
/// recalculation runs through thousands of these per theme switch, and
/// interning by name hashes a string each time. Symbols never move.
#[derive(Clone, Copy)]
pub(crate) struct FaceKeys {
    pub unspecified: Value,
    pub ignore_defface: Value,
    pub reset: Value,
    pub normal: Value,
    pub bold: Value,
    pub italic: Value,
    pub default: Value,
    pub scroll_bar: Value,
    pub border: Value,
    pub cursor: Value,
    pub mouse: Value,
    pub face_alias: Value,
    pub face_no_inherit: Value,
    pub theme_face: Value,
    pub defface_spec: Value,
    pub override_spec: Value,
    pub customized_face: Value,
    pub saved_face: Value,
    pub bold_key: Value,
    pub italic_key: Value,
    pub background_mode: Value,
    pub display_type: Value,
    pub reverse: Value,
    pub face_x_resources: Value,
    pub type_condition: Value,
    pub min_colors: Value,
    pub class: Value,
    pub background: Value,
    pub supports: Value,
    pub graphic: Value,
    /// The lface vector's attribute keywords, by slot less one.
    pub keywords: [Value; LFACE_VECTOR_SLOTS],
    /// The symbol slots of the registries and the display record; DEFAULTS
    /// is 0 until first used.
    pub frame_tables: usize,
    pub defaults: usize,
    pub display_record: usize,
    pub memos: usize,
    /// What a memo holds for a slot the replay did not write.
    pub unwritten: Value,
}
romacs_snapshot::snapshot_struct!(FaceKeys { unspecified, ignore_defface, reset, normal, bold, italic, default, scroll_bar, border, cursor, mouse, face_alias, face_no_inherit, theme_face, defface_spec, override_spec, customized_face, saved_face, bold_key, italic_key, background_mode, display_type, reverse, face_x_resources, type_condition, min_colors, class, background, supports, graphic, keywords, frame_tables, defaults, display_record, memos, unwritten });

pub(crate) const LFACE_VECTOR_SLOTS: usize = romacs_face::registry::LFACE_VECTOR_SIZE - 1;

/// What `face-spec-recalc' reads of a face's symbol, in one walk of its plist.
#[derive(Clone, Copy)]
pub(crate) struct FaceProps {
    pub alias: Value,
    pub no_inherit: Value,
    pub theme_face: Value,
    pub defface: Value,
    pub over: Value,
    pub customized: Value,
    pub saved: Value,
}

impl FaceKeys {
    /// Nil keys, for a VM that has not interned any yet.
    pub(crate) const EMPTY: FaceKeys = FaceKeys {
        unspecified: Value::NIL, ignore_defface: Value::NIL, reset: Value::NIL, normal: Value::NIL, bold: Value::NIL,
        italic: Value::NIL, default: Value::NIL, scroll_bar: Value::NIL, border: Value::NIL, cursor: Value::NIL,
        mouse: Value::NIL, face_alias: Value::NIL, face_no_inherit: Value::NIL, theme_face: Value::NIL,
        defface_spec: Value::NIL, override_spec: Value::NIL, customized_face: Value::NIL, saved_face: Value::NIL, bold_key: Value::NIL, italic_key: Value::NIL,
        background_mode: Value::NIL, display_type: Value::NIL, reverse: Value::NIL, face_x_resources: Value::NIL,
        type_condition: Value::NIL, min_colors: Value::NIL, class: Value::NIL, background: Value::NIL,
        supports: Value::NIL, graphic: Value::NIL, keywords: [Value::NIL; LFACE_VECTOR_SLOTS],
        frame_tables: 0, defaults: 0, display_record: 0, memos: 0, unwritten: Value::NIL,
    };

    pub(crate) fn new(vm: &mut Vm) -> FaceKeys {
        let frame_tables = vm.slot("#:vm2-frame-face-tables");
        let display_record = vm.slot("romacs-display");
        let memos = vm.slot("#:vm2-frame-face-memos");
        let mut key = |name: &str| vm.property_key_symbol(name);
        FaceKeys {
            frame_tables, defaults: 0, display_record, memos, unwritten: key("#:vm2-face-unwritten"),
            unspecified: key("unspecified"), ignore_defface: key(":ignore-defface"), reset: key("reset"),
            normal: key("normal"), bold: key("bold"), italic: key("italic"), default: key("default"),
            scroll_bar: key("scroll-bar"), border: key("border"), cursor: key("cursor"), mouse: key("mouse"),
            face_alias: key("face-alias"), face_no_inherit: key("face-no-inherit"), theme_face: key("theme-face"),
            defface_spec: key("face-defface-spec"), override_spec: key("face-override-spec"),
            customized_face: key("customized-face"), saved_face: key("saved-face"),
            bold_key: key(":bold"), italic_key: key(":italic"), background_mode: key("background-mode"),
            display_type: key("display-type"), reverse: key("reverse"), face_x_resources: key("face-x-resources"),
            type_condition: key("type"), min_colors: key("min-colors"), class: key("class"),
            background: key("background"), supports: key("supports"), graphic: key("graphic"),
            keywords: romacs_face::registry::LFACE_ATTRIBUTES.map(|name| key(name)),
        }
    }

    /// The lface slot of the attribute KEYWORD.
    pub(crate) fn slot_of(&self, keyword: Value) -> Option<usize> {
        self.keywords.iter().position(|&k| k == keyword).map(|i| i + 1)
    }
}

impl Vm {
    /// `#:vm2-defface': FACE from SPEC, without faces.el.
    pub(crate) fn declare_face(&mut self, face: Value, spec: Value) -> crate::Result<()> {
        let mut leaf = Leaf { vm: self };
        crate::prim::prims::face_spec::declare(&mut leaf, face, spec).map_err(|_| "cannot declare face".to_owned())?;
        self.display_state.faces_changed();
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
    pub(crate) fn face_keys(&self) -> FaceKeys { self.vm.face_keys }

    /// FACE's properties as `get' finds them: the first of each.
    pub(crate) fn face_props(&self, face: Value) -> std::result::Result<FaceProps, Fault> {
        let keys = &self.vm.face_keys;
        let mut props = FaceProps { alias: Value::NIL, no_inherit: Value::NIL, theme_face: Value::NIL, defface: Value::NIL,
                                    over: Value::NIL, customized: Value::NIL, saved: Value::NIL };
        let mut seen = 0u8;
        let mut tail = self.vm.symbol_plist(face).map_err(Fault::Error)?;
        // A plist is short; this bounds one that is circular.
        for _ in 0..4096 {
            let Some((key, rest)) = self.pair(tail) else { break };
            let Some((value, next)) = self.pair(rest) else { break };
            let (bit, slot) = if key == keys.theme_face { (1, &mut props.theme_face) }
                else if key == keys.defface_spec { (2, &mut props.defface) }
                else if key == keys.override_spec { (4, &mut props.over) }
                else if key == keys.face_alias { (8, &mut props.alias) }
                else if key == keys.face_no_inherit { (16, &mut props.no_inherit) }
                else if key == keys.customized_face { (32, &mut props.customized) }
                else if key == keys.saved_face { (64, &mut props.saved) }
                else { tail = next; continue };
            if seen & bit == 0 { seen |= bit; *slot = value; }
            tail = next;
        }
        Ok(props)
    }

    /// The vector's slots, read in place.
    pub(crate) fn vector_slots(&self, vector: Value) -> Option<&[Value]> {
        match self.vm.object_if(vector) {
            Some(data::Object::Vector(slots)) => Some(&slots[..]),
            _ => None,
        }
    }

    /// Whether romacs-frame.el's display record is missing: GNU's Lisp is
    /// not in this VM.
    pub(crate) fn display_record_missing(&self) -> bool {
        self.vm.slots[self.vm.face_keys.display_record].value == Value::UNBOUND
    }

    /// FRAME's memo table, made if CREATE (see `face_spec::recalc').
    pub(crate) fn face_memos(&mut self, frame: Value, create: bool) -> super::super::Out {
        let root = self.vm.face_keys.memos;
        let mut table = self.vm.slots[root].value;
        if table == Value::UNBOUND || table == Value::NIL {
            if !create { return Ok(Value::NIL); }
            table = self.face_table_new(hash::Weakness::Key);
            self.vm.slots[root].value = table;
        }
        if let Some(local) = self.vm.declaration_hash_get(table, frame).map_err(Fault::Error)? { return Ok(local); }
        if !create { return Ok(Value::NIL); }
        let local = self.face_table_new(hash::Weakness::None);
        self.vm.declaration_hash_put(table, frame, local).map_err(Fault::Error)?;
        Ok(local)
    }

    /// TABLE's entry for FACE: a memo, or a frame's lface; nil for none.
    /// A table a collection left stale is repaired, not scanned per face.
    pub(crate) fn face_memo(&mut self, table: Value, face: Value) -> Value {
        self.vm.declaration_hash_get(table, face).ok().flatten().unwrap_or(Value::NIL)
    }

    pub(crate) fn face_memo_put(&mut self, memos: Value, face: Value, memo: Value) -> std::result::Result<(), Fault> {
        self.vm.declaration_hash_put(memos, face, memo).map(|_| ()).map_err(Fault::Error)
    }

    /// FRAME's own lface of FACE, nil for none; None while FRAME shows
    /// another frame's faces.
    pub(crate) fn owned_lface(&mut self, face: Value, frame: Value) -> std::result::Result<Option<Value>, Fault> {
        let table = self.face_table(frame, false)?;
        if table == Value::NIL {
            if self.vm.frame(frame).map_or(Value::NIL, |f| f.parent) != Value::NIL { return Ok(None); }
            return Ok(Some(Value::NIL));
        }
        Ok(Some(self.vm.eq_hash_get(table, face).unwrap_or(Value::NIL)))
    }

    /// calln (FUNCTION, ARGS...) from a face write: an exit from FUNCTION
    /// goes on through the caller, as GNU's call1 lets it.
    pub(crate) fn call_propagating(&mut self, function: Value, args: &[Value]) -> std::result::Result<(), Fault> {
        let called = self.vm.call_lisp_at_write(function, args);
        if self.vm.propagated_escape.is_some() { return Err(Fault::Escape); }
        called.map_err(Fault::Error)
    }

    /// `maybe_quit' between the items of a face loop: a quit goes on through
    /// the primitive as it would through GNU's Lisp loop.
    pub(crate) fn face_loop_quit(&mut self) -> std::result::Result<(), Fault> {
        if !self.vm.quit_poll_due() { return Ok(()); }
        match self.vm.maybe_quit() {
            None => Ok(()),
            Some(crate::threaded::Escape::Signal(condition, data)) => Err(Fault::Signal(condition, data)),
            Some(escape) => { self.vm.propagated_escape = Some(escape); Err(Fault::Escape) }
        }
    }

    /// `(condition-case () ... (error nil))' around FAULT: Ok when FAULT is
    /// an error, and FAULT again when it is a quit or a throw.
    pub(crate) fn swallow_face_error(&mut self, fault: Fault) -> std::result::Result<(), Fault> {
        use crate::threaded::Escape;
        let error = self.intern("error");
        match fault {
            Fault::Signal(condition, _) if !self.vm.condition_matches(condition, error) => Err(fault),
            Fault::Escape => {
                let swallowed = match self.vm.propagated_escape {
                    Some(Escape::Signal(condition, _)) => self.vm.condition_matches(condition, error),
                    Some(Escape::Error { .. }) => true,
                    _ => false,
                };
                if !swallowed { return Err(Fault::Escape); }
                self.vm.propagated_escape = None;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn face_put(&mut self, symbol: Value, key: Value, value: Value) -> std::result::Result<(), Fault> {
        self.vm.put_symbol_property(symbol, key, value).map(|_| ()).map_err(Fault::Error)
    }

    /// The stored frame parameter KEY of FRAME, without the computed ones
    /// `frame-parameter' adds: a read a face recalculation makes per spec.
    pub(crate) fn stored_frame_parameter(&self, frame: Value, key: Value) -> Value {
        self.vm.frame(frame).ok().and_then(|f| f.parameters.iter().find(|(k, _)| *k == key).map(|&(_, v)| v)).unwrap_or(Value::NIL)
    }

    /// Every face of the new-frame registry with its `face' id.
    pub(crate) fn face_ids(&mut self) -> std::result::Result<Vec<(i64, Value)>, Fault> {
        let table = self.face_table(Value::NIL, false)?;
        let entries = self.vm.declaration_hash_entries(table).map_err(Fault::Error)?;
        Ok(entries.into_iter().filter_map(|(face, entry)| Some((self.pair(entry)?.0.integer_value()?, face))).collect())
    }

    fn face_table_new(&mut self, weakness: hash::Weakness) -> Value {
        let table = hash::HashTable::new(hash::Test::Eq, weakness, 8);
        self.vm.allocate_object(data::Object::HashTable(Box::new(table)))
    }

    /// Nil denotes the global registry. Frame registries are ephemeron values:
    /// a dead frame cannot be kept alive by its own face attributes.
    pub(crate) fn face_table(&mut self, frame: Value, create: bool) -> super::super::Out {
        // The defaults' symbol makes its table when first named, so not at startup.
        if frame == Value::NIL && self.vm.face_keys.defaults == 0 { self.vm.face_keys.defaults = self.vm.slot("face--new-frame-defaults"); }
        let root = if frame == Value::NIL { self.vm.face_keys.defaults } else { self.vm.face_keys.frame_tables };
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
        // So does the mode its faces were computed for: its own parameter
        // stood still while it shared, and the next flip would see no change.
        let keys = self.face_keys();
        for key in [keys.background_mode, keys.display_type] {
            let value = self.stored_frame_parameter(owner, key);
            self.windows().store_frame_parameter(frame, key, value)?;
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
        // The display may have asked for this name while it was no face.
        self.vm.display_state.faces_changed();
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
