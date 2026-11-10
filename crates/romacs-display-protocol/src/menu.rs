//! Plain menu presentation data shared by the runtime and painter.

mod item;
mod text;
pub use item::{MenuAvailability, MenuItemKind};
pub use text::MeasuredMenu;

/// Correlates a native heading intent with the evaluator's menu response.
/// Distinct from MenuToken: one intent can produce multiple menu revisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuBarRequestId(pub u64);

impl MenuBarRequestId {
    pub fn fresh() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

/// Identifies one immutable revision of an evaluator-owned menu session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuToken {
    pub session: u64,
    pub revision: u64,
}

impl MenuToken {
    pub fn fresh() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            session: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            revision: 0,
        }
    }
}

/// Lisp-owned button state, distinct from runtime hover or keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCheckState {
    Off,
    On,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MenuIndicator {
    #[default]
    None,
    Toggle(MenuCheckState),
    Radio(MenuCheckState),
}

/// A single item in a popup menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupMenuItem {
    pub kind: MenuItemKind,
    /// Help belongs to the item snapshot, including disabled items.
    pub help: Option<String>,
    /// Display label for the item
    pub label: String,
    /// Keyboard shortcut text (e.g., "C-x C-s"), or empty
    pub shortcut: String,
    /// Nesting depth (0 = top-level, 1 = first submenu, etc.)
    pub depth: u32,
}
