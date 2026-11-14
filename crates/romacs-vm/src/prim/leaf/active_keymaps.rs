//! The active keymaps for `prims/keymap.rs` (`src/active_keymaps.rs`).
use super::{Fault, Leaf, Value};

type Out = std::result::Result<Value, Fault>;

impl Leaf<'_> {
    /// `current-active-maps`.
    pub(crate) fn current_active_maps(&mut self, olp: bool, position: Value) -> Out {
        self.vm.current_active_maps(olp, position)
    }
    /// The keymaps of the current buffer's active minor modes, highest
    /// priority first; with MODES, each as (MODE . KEYMAP).
    pub(crate) fn current_minor_mode_maps(&mut self, modes: bool) -> Out {
        let found = self.vm.current_minor_mode_maps()?;
        let items: Vec<Value> = found.into_iter().map(|(mode, map)| if modes { self.vm.alloc_cons(mode, map) } else { map }).collect();
        Ok(self.vm.list_value(&items))
    }
    /// PROPERTY's keymap at point, as the active keymaps take it.
    pub(crate) fn keymap_at_point(&mut self, property: Value) -> Out {
        self.vm.keymap_at_point(property)
    }
    /// Whether OBJECT is a keymap, a symbol whose function is one, or one's
    /// autoload.
    pub(crate) fn keymapp(&self, object: Value) -> bool {
        self.keymap(object).is_some() || self.vm.autoloaded_keymap(object)
    }
}
