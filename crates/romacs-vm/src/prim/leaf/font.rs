//! What prims/font.rs reads and writes: font records' slots and the open
//! display's font host (display_host.rs), with the frame's default face.
use super::*;

impl Leaf<'_> {
    /// A record's slots, its type first; None for anything else.
    pub(crate) fn record_slots(&self, value: Value) -> Option<Vec<Value>> {
        match self.vm.object_if(value) {
            Some(data::Object::Record(slots)) => Some(slots.clone()),
            _ => None,
        }
    }
    /// Store VALUE in slot INDEX of the record RECORD.
    pub(crate) fn record_set(&mut self, record: Value, index: usize, value: Value) -> std::result::Result<(), Fault> {
        if !matches!(self.vm.object_if(record), Some(data::Object::Record(_))) {
            return Err(Fault::Wrong("recordp", record));
        }
        let object = self.writable_object(record)?;
        self.vm.objects.update_element(object, index, value).map_err(|_| Fault::Range(record, Value::integer(index as i64).unwrap()))
    }
    /// The open display's host; None while the frame is on the initial terminal.
    pub(crate) fn font_host(&mut self) -> Option<&mut (dyn crate::DisplayHost + 'static)> {
        self.vm.fontset_state();
        self.vm.window_system_host()
    }
    /// The open display's cached ASCII font name for KEY (a face's family
    /// and pixel-size bits), and its recording.
    pub(crate) fn cached_face_font(&self, key: &(String, u32, u16, bool, u16)) -> Option<String> {
        self.vm.display.as_ref().filter(|d| d.open).and_then(|d| d.face_fonts.get(key).cloned())
    }
    pub(crate) fn cache_face_font(&mut self, key: (String, u32, u16, bool, u16), name: String) {
        if let Some(display) = self.vm.display.as_mut() { display.face_fonts.insert(key, name); }
    }
    /// The font-object the open display made for ENTITY at its pixel size
    /// before, as font.c `font_open_entity` finds it on the entity's
    /// object list.
    pub(crate) fn cached_font_object(&mut self, entity: &crate::FontEntity, pixel_size: i64) -> Option<Value> {
        let index = *self.vm.display.as_ref().filter(|d| d.open)?.font_objects.index.get(entity)?.get(&pixel_size)?;
        let slot = self.vm.slot(crate::prim::FONT_OBJECTS);
        match self.vm.object_if(self.vm.slots[slot].value) {
            Some(data::Object::Vector(objects)) => objects.get(index).copied(),
            _ => None,
        }
    }
    /// Keep OBJECT as ENTITY's font-object at its pixel size while the
    /// display keeps its fonts.
    pub(crate) fn cache_font_object(&mut self, entity: crate::FontEntity, pixel_size: i64, object: Value) {
        let Some(len) = self.vm.display.as_ref().filter(|d| d.open).map(|d| d.font_objects.len) else { return };
        let slot = self.vm.slot(crate::prim::FONT_OBJECTS);
        let vector = self.vm.slots[slot].value;
        let room = matches!(self.vm.object_if(vector), Some(data::Object::Vector(objects)) if objects.len() > len);
        if !room {
            let mut grown = match self.vm.object_if(vector) {
                Some(data::Object::Vector(objects)) => objects[..len].to_vec(),
                _ => Vec::new(),
            };
            grown.resize((2 * len).max(8), Value::NIL);
            self.vm.slots[slot].value = self.vm.allocate_object(data::Object::Vector(grown));
        }
        let (objects, Some(index)) = (self.vm.slots[slot].value, i64::try_from(len).ok().and_then(|n| Value::integer(n).ok())) else { return };
        // A vector a world holds may refuse the write: the font goes uncached.
        if self.vector_set(objects, index, object).is_err() { return; }
        if let Some(display) = self.vm.display.as_mut() {
            display.font_objects.index.entry(entity).or_default().insert(pixel_size, len);
            display.font_objects.len = len + 1;
        }
    }
    /// The display forgot its fonts (`Fclear_font_cache`): so do their objects.
    pub(crate) fn forget_font_objects(&mut self) {
        let slot = self.vm.slot(crate::prim::FONT_OBJECTS);
        self.vm.slots[slot].value = Value::NIL;
        if let Some(display) = self.vm.display.as_mut() { display.font_objects = crate::display_host::FontObjects::default(); }
    }
    /// The frame's default face's font request, as painting realizes it.
    pub(crate) fn default_font_request(&self) -> std::result::Result<crate::FaceFont, Fault> {
        self.face_font_request("default")
    }
    /// FACE's font request (family, pixel size, weight, slant, width), as
    /// painting realizes it.
    pub(crate) fn face_font_request(&self, face: &str) -> std::result::Result<crate::FaceFont, Fault> {
        let face = self.vm.display_face(face).map_err(Fault::Error)?;
        Ok(crate::FaceFont { family: face.family, pixel_size: face.pixel_size, weight: face.weight, italic: face.italic, width: face.width })
    }
}

impl Vm {
    /// xdisp.c decode_mode_spec_coding: CODING's mnemonic (`-' while it is
    /// undecided, a space in a unibyte buffer) and, with EOL, its
    /// end-of-line mnemonic, for a window-system frame's `%z' and `%Z'.
    pub(crate) fn mode_spec_coding(&mut self, coding: Value, multibyte: bool, eol: bool) -> crate::Result<String> {
        let mut leaf = Leaf { vm: self };
        let spec = if coding == Value::NIL { Value::NIL } else { leaf.coding_spec(coding).unwrap_or(Value::NIL) };
        let index = |n: i64| Value::integer(n).unwrap();
        let (mnemonic, eol_type) = if spec == Value::NIL { (None, Value::NIL) } else {
            let attrs = leaf.vector_get(spec, index(0)).map_err(|_| "coding spec")?;
            let mnemonic = leaf.vector_get(attrs, index(2)).ok().and_then(|c| c.integer_value()).and_then(|c| char::from_u32(c as u32));
            (mnemonic, leaf.vector_get(spec, index(2)).unwrap_or(Value::NIL))
        };
        let mut out = String::new();
        out.push(match (multibyte, spec != Value::NIL) { (false, _) => ' ', (true, false) => '-', (true, true) => mnemonic.unwrap_or('-') });
        if eol {
            let variable = if spec == Value::NIL || leaf.kind(eol_type) != Kind::Symbol || eol_type == Value::NIL { "eol-mnemonic-undecided" }
                else if eol_type == leaf.intern("unix") { "eol-mnemonic-unix" }
                else if eol_type == leaf.intern("dos") { "eol-mnemonic-dos" } else { "eol-mnemonic-mac" };
            let symbol = leaf.intern(variable);
            let value = leaf.variable_value(symbol).ok().flatten().unwrap_or(Value::NIL);
            match (leaf.string_view(value), value.integer_value().and_then(|c| char::from_u32(c as u32))) {
                (Ok(text), _) => out.push_str(&String::from_utf8_lossy(text.bytes())),
                (_, Some(c)) => out.push(c),
                _ => out.push('?'),
            }
        }
        Ok(out)
    }
}
