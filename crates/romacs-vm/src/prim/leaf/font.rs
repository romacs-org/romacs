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
        let data::Object::Record(slots) = self.object_mut(record)? else { unreachable!() };
        *slots.get_mut(index).ok_or(Fault::Range(record, Value::integer(index as i64).unwrap()))? = value;
        Ok(())
    }
    /// The open display's host; None while the frame is on the initial terminal.
    pub(crate) fn font_host(&mut self) -> Option<&mut (dyn crate::DisplayHost + 'static)> {
        self.vm.window_system_host()
    }
    /// The open display's cached ASCII font name for KEY (a face's family
    /// and pixel-size bits), and its recording.
    pub(crate) fn cached_face_font(&self, key: &(String, u32)) -> Option<String> {
        self.vm.display.as_ref().filter(|d| d.open).and_then(|d| d.face_fonts.get(key).cloned())
    }
    pub(crate) fn cache_face_font(&mut self, key: (String, u32), name: String) {
        if let Some(display) = self.vm.display.as_mut() { display.face_fonts.insert(key, name); }
    }
    /// The frame's default face's family and pixel size, as painting
    /// realizes it.
    pub(crate) fn default_font_request(&self) -> std::result::Result<(String, f32), Fault> {
        self.face_font_request("default")
    }
    /// FACE's family and pixel size, as painting realizes it.
    pub(crate) fn face_font_request(&self, face: &str) -> std::result::Result<(String, f32), Fault> {
        let face = self.vm.display_face(face).map_err(Fault::Error)?;
        Ok((face.family, face.pixel_size))
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
