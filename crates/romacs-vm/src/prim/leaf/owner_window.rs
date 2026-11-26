//! An owner's logical windows from a leaf (`crate::owner::display`).
use super::{Fault, Leaf, Value};
use crate::Vm;
use crate::owner::display::PaneRequest;

type Out = std::result::Result<Value, Fault>;

/// V as a natural number that fits `u64`.
fn natural(v: Value) -> std::result::Result<u64, Fault> {
    v.integer_value().and_then(|n| u64::try_from(n).ok()).ok_or_else(|| Fault::Wrong("natnump", v))
}

/// V as a cell side in px: a positive integer that fits `u32`.
fn px(v: Value) -> std::result::Result<u32, Fault> {
    v.integer_value().and_then(|n| u32::try_from(n).ok()).filter(|&n| n > 0).ok_or_else(|| Fault::Wrong("wholenump", v))
}

impl Leaf<'_> {
    /// t once started; nil, its argument unread, with no owner display.
    pub(crate) fn owner_display_start(&mut self, slot: Value) -> Out {
        if !self.vm.has_owner_display() { return Ok(Value::NIL); }
        let index = u8::try_from(natural(slot)?).ok()
            .filter(|&k| romacs_display_protocol::pane_picture::owner_slot_base(k).is_some())
            .ok_or_else(|| Fault::Wrong("owner-slot-p", slot))?;
        self.vm.owner_display_start(index).map_err(Fault::Error)?;
        Ok(Value::TRUE)
    }

    pub(crate) fn owner_set_cell(&mut self, line: Value, column: Value) -> Out {
        let (line, column) = (px(line)?, px(column)?);
        self.vm.owner_set_cell(line, column).map_err(Fault::Error)?;
        Ok(Value::NIL)
    }

    /// RESULT of a step that ran Lisp: nil, or what the Lisp signalled
    /// (a quit stays a quit), else the error.
    fn ran(&mut self, result: crate::Result<()>) -> Out {
        match result {
            Ok(()) => Ok(Value::NIL),
            Err(_) if self.vm.mutator.propagated_escape.is_some() => Err(Fault::Escape),
            Err(error) => Err(Fault::Error(error)),
        }
    }

    pub(crate) fn owner_panes_prepare(&mut self) -> Out {
        let result = self.vm.leaf_lisp(Vm::panes_prepare);
        self.ran(result)
    }

    pub(crate) fn owner_pane_lay_out(&mut self, window: Value, request: [Value; 5]) -> Out {
        let window = self.vm.decode_window(window, true, false).map_err(|_| Fault::Wrong("window-live-p", window))?;
        let [master, view, revision, catalog, selected] = request;
        let request = PaneRequest {
            master: natural(master)?, view: natural(view)?, revision: natural(revision)?, catalog: natural(catalog)?,
            selected: selected != Value::NIL,
        };
        let result = self.vm.leaf_lisp(|vm| vm.pane_lay_out(window, request));
        self.ran(result)
    }

    pub(crate) fn owner_pane_picture(&mut self, window: Value) -> Out {
        let live = self.vm.decode_window(window, true, false).map_err(|_| Fault::Wrong("window-live-p", window))?;
        let id = self.vm.window(live).map_err(Fault::Error)?.id;
        Ok(match self.vm.pane_picture(id).map_err(Fault::Error)? {
            Some(bytes) => self.vm.make_lisp_string(bytes, false),
            None => Value::NIL,
        })
    }
}
