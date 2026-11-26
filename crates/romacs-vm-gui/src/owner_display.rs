//! An owner's display (ownership O1b §4.1): the layout engine an owner
//! isolate lays its master's windows out with, its fonts interned from the
//! owner's slot, and shared with the layout host its windows' queries
//! measure with (`crate::measure`).
use std::{cell::RefCell, rc::Rc};

/// No engine until the owner starts (`owner-window--start`): a worker
/// that never becomes an owner loads no fonts for it.
#[derive(Default)]
pub struct OwnerDisplay {
    engine: Option<Rc<RefCell<romacs_layout::LayoutEngine>>>,
}

impl romacs_vm::OwnerDisplay for OwnerDisplay {
    fn start(&mut self, slot: u8) -> Result<Box<dyn romacs_vm::LayoutHost>, String> {
        let engine = Rc::new(RefCell::new(romacs_layout::LayoutEngine::with_font_base(slot)?));
        self.engine = Some(Rc::clone(&engine));
        Ok(Box::new(crate::measure::Host { engine }))
    }

    fn lay_out(&mut self, frame: &romacs_layout_input::FrameSnapshot, cursor: Option<(u64, romacs_display_protocol::Color)>)
        -> Result<(romacs_display_protocol::FrameGlyphBuffer, Vec<romacs_vm::WindowEnd>), String> {
        let engine = self.engine.as_ref().ok_or("the owner display has not started")?;
        // Lisp runs only between layouts, never inside one.
        let mut engine = engine.try_borrow_mut().map_err(|_| "the layout engine is measuring".to_owned())?;
        engine.layout_frame(frame, 1, cursor)
    }
}
