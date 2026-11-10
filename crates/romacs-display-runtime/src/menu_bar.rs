//! The frame chrome's menu bar: which heading is open, and the input it owns
//! while it is.
//!
//! A heading press or hover asks the evaluator for that heading's menu
//! (`InputEvent::MenuBarClick` with a fresh request id). Until the heading is
//! closed, the menu bar owns its window's pointer and keyboard: Escape or
//! C-g closes it, the arrows step between headings, and the release of the
//! press that opened or closed a heading never reaches the editor.

use romacs_display_protocol::menu::MenuBarRequestId;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::WindowId;

#[derive(Clone, Debug)]
pub(crate) struct MenuHeading {
    pub frame: u64,
    pub parent: WindowId,
    pub key: String,
    pub index: u32,
    pub compact: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HeadingAction {
    Keep,
    Close,
    Request(MenuBarRequestId),
}

#[derive(Default)]
pub(crate) struct MenuBar {
    open: Option<MenuHeading>,
    modifiers: ModifiersState,
    /// A press on a heading still owns its matching release after the
    /// heading closes.
    release_owner: Option<(WindowId, MouseButton)>,
}

impl MenuBar {
    /// Open HEADING, or keep or close it if it is the open one.
    pub(crate) fn select(&mut self, heading: MenuHeading, pressed: bool) -> HeadingAction {
        if pressed {
            self.release_owner = Some((heading.parent, MouseButton::Left));
        }
        if self
            .open
            .as_ref()
            .is_some_and(|open| open.frame == heading.frame && open.key == heading.key)
        {
            if !pressed {
                return HeadingAction::Keep;
            }
            self.open = None;
            return HeadingAction::Close;
        }
        self.open = Some(heading);
        HeadingAction::Request(MenuBarRequestId::fresh())
    }

    pub(crate) fn heading(&self) -> Option<&MenuHeading> {
        self.open.as_ref()
    }

    pub(crate) fn close(&mut self) {
        self.open = None;
    }

    /// The frame whose heading is open.
    pub(crate) fn owner(&self) -> Option<u64> {
        self.open.as_ref().map(|heading| heading.frame)
    }

    pub(crate) fn owns_parent(&self, id: WindowId) -> bool {
        self.open
            .as_ref()
            .is_some_and(|heading| heading.parent == id)
    }

    /// Left and Right step to the neighbouring heading while one is open.
    pub(crate) fn heading_step(&self, id: WindowId, key: &Key) -> Option<i32> {
        if !self.owns_parent(id) {
            return None;
        }
        match key {
            Key::Named(NamedKey::ArrowLeft) => Some(-1),
            Key::Named(NamedKey::ArrowRight) => Some(1),
            _ => None,
        }
    }

    /// Take EVENT on window ID if the menu bar owns it.
    pub(crate) fn event(&mut self, id: WindowId, event: &WindowEvent) -> bool {
        if let WindowEvent::PointerButton { state, button, .. } = event {
            if *state == ElementState::Pressed {
                // A new press supersedes capture whose release was lost.
                self.release_owner = None;
            } else if self
                .release_owner
                .is_some_and(|(_, pressed)| button.clone().mouse_button() == Some(pressed))
            {
                self.release_owner = None;
                return true;
            }
        }
        if matches!(event, WindowEvent::Destroyed)
            && self.release_owner.is_some_and(|(owner, _)| owner == id)
        {
            self.release_owner = None;
        }
        if let WindowEvent::ModifiersChanged(modifiers) = event {
            self.modifiers = modifiers.state();
        }
        if !self.owns_parent(id) {
            return false;
        }
        match event {
            // Hover switching between headings is routed by the frame chrome
            // first; other motion must not highlight editor content.
            WindowEvent::PointerMoved { .. } | WindowEvent::PointerEntered { .. } => true,
            WindowEvent::PointerButton {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                self.release_owner = button.clone().mouse_button().map(|button| (id, button));
                self.close();
                true
            }
            // The heading press belongs to the menu bar, including while the
            // evaluator has not answered it yet.
            WindowEvent::PointerButton { .. } => true,
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.close();
                false
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && cancels(&event.logical_key, self.modifiers)
                {
                    self.close();
                }
                true
            }
            _ => false,
        }
    }
}

fn cancels(key: &Key, modifiers: ModifiersState) -> bool {
    matches!(key, Key::Named(NamedKey::Escape))
        || (modifiers.control_key() && matches!(key.as_ref(), Key::Character("g" | "G")))
}

#[cfg(test)]
mod tests;
