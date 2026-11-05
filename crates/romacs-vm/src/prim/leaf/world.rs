//! World builds: the startup phase (`crate::world`).
use super::*;

impl Leaf<'_> {
    pub(crate) fn startup_phase(&mut self, on: bool) {
        self.vm.startup_phase(on);
    }

    /// `symbol-plist`: a read of every property of SYMBOL, which startup
    /// makes an input.
    pub(crate) fn observed_symbol_plist(&mut self, symbol: Value) -> crate::prim::Out {
        if self.vm.fence.is_some() {
            self.vm.fence_property(symbol, None, crate::world::Access::Read, None);
        }
        self.symbol_plist(symbol)
    }

    /// The variable NAME's value read past the fence, by a caller whose
    /// result this value does not decide; nil when void.
    pub(crate) fn unfenced_variable(&mut self, name: &str) -> crate::prim::Out {
        let symbol = self.intern(name);
        let slot = self.symbol_slot(symbol)?;
        let value = self.vm.unfenced(slot, |vm| vm.variable_value(slot)).map_err(Fault::Error)?;
        Ok(if value == Value::UNBOUND { Value::NIL } else { value })
    }

    /// The environment list NAME as its Rust owners read it: one named
    /// entry, which the host records, not a read of the fenced list.
    pub(crate) fn environment_list(&mut self, name: &str) -> crate::prim::Out {
        let symbol = self.intern(name);
        let slot = self.symbol_slot(symbol)?;
        let value = self.vm.unfenced(slot, |vm| vm.variable_value(slot)).map_err(Fault::Error)?;
        if value == Value::UNBOUND { Err(Fault::VoidVariable(symbol)) } else { Ok(value) }
    }

    /// `romacs--world-layer-deferred-by`.
    pub(crate) fn world_layer_deferred_by(&mut self, deferred: Value, loaded: Value) -> crate::prim::Out {
        let mut libraries = vec![];
        for library in self.vm.list_values(deferred).map_err(Fault::Error)? {
            let [name, file] = self.vm.list_values(library).map_err(Fault::Error)?[..] else {
                return Err(Fault::Error("romacs--world-layer-deferred-by: a library is (NAME FILE)".into()));
            };
            let file = if file == Value::NIL { None } else { Some(self.text(file)?) };
            libraries.push((self.text(name)?, file));
        }
        let mut loads = vec![];
        for entry in self.vm.list_values(loaded).map_err(Fault::Error)? {
            let [features, files] = self.vm.list_values(entry).map_err(Fault::Error)?[..] else {
                return Err(Fault::Error("romacs--world-layer-deferred-by: an entry is (FEATURES FILES)".into()));
            };
            loads.push((self.texts(features)?, self.texts(files)?));
        }
        let found: Vec<Value> = crate::world::layer::deferred_by(&libraries, &loads).into_iter()
            .map(|name| name.map_or(Value::NIL, |name| self.make_string(name.into_bytes(), true)))
            .collect();
        Ok(self.vm.list_value(&found))
    }

    /// The text of the string or symbol VALUE.
    fn text(&mut self, value: Value) -> std::result::Result<String, Fault> {
        if let Some(name) = self.vm.symbol_name_of(value) { return Ok(name); }
        Ok(String::from_utf8_lossy(self.string_view(value)?.bytes()).into_owned())
    }

    /// The texts of the list LIST's strings or symbols.
    fn texts(&mut self, list: Value) -> std::result::Result<Vec<String>, Fault> {
        self.vm.list_values(list).map_err(Fault::Error)?.into_iter().map(|v| self.text(v)).collect()
    }

    /// `romacs--note-init-after-load`: FILE is a feature or a file name.
    pub(crate) fn note_init_after_load(&mut self, file: Value, printed: Value) -> std::result::Result<(), Fault> {
        let printed = String::from_utf8_lossy(self.string_view(printed)?.bytes()).into_owned();
        let (name, file) = match self.vm.symbol_name_of(file) {
            Some(name) => (name, None),
            None => {
                let file = String::from_utf8_lossy(self.string_view(file)?.bytes()).into_owned();
                (file.clone(), Some(file))
            }
        };
        self.vm.note_init_after_load(name, file, printed);
        Ok(())
    }

    /// `romacs--world-pend`.
    pub(crate) fn pend_variable(&mut self, symbol: Value, index: u32) -> std::result::Result<bool, Fault> {
        let slot = self.symbol_slot(symbol)?;
        Ok(self.vm.pend_variable(slot, index))
    }

    /// `autoload` defined FUNCTION's autoload from FILE: a declared library
    /// when the user's init did it (`world::layer`).
    pub(crate) fn note_init_declared(&mut self, file: Value) {
        if self.vm.loader.recording.is_some() && let Some(file) = self.vm.str_view(file).map(|v| String::from_utf8_lossy(v.bytes()).into_owned()) {
            self.vm.note_init_declared(file);
        }
    }

    /// `romacs--init-recording-hold`.
    pub(crate) fn hold_init_recording(&mut self, on: bool) {
        self.vm.hold_init_recording(on);
    }

    /// `romacs--init-after-load-ordinal`.
    pub(crate) fn init_after_load_ordinal(&mut self, file: Value) -> crate::prim::Out {
        let ordinal = match self.vm.symbol_name_of(file) {
            Some(name) => self.vm.init_after_load_ordinal(&name, None),
            None => {
                let file = String::from_utf8_lossy(self.string_view(file)?.bytes()).into_owned();
                self.vm.init_after_load_ordinal(&file, Some(&file))
            }
        };
        Ok(match ordinal { Some(n) => Value::integer(n as i64).map_err(Fault::Error)?, None => Value::NIL })
    }
}
