//! Winit + wgpu display backend glue.

use romacs_display_protocol::TransportModifierBit;

/// The key transport's modifier masks under the names the frontend's key
/// translation uses; the bits are [`TransportModifierBit`]'s.
pub const ROMACS_SHIFT_MASK: u32 = TransportModifierBit::Shift.mask();
pub const ROMACS_CTRL_MASK: u32 = TransportModifierBit::Ctrl.mask();
pub const ROMACS_META_MASK: u32 = TransportModifierBit::Meta.mask();
pub const ROMACS_SUPER_MASK: u32 = TransportModifierBit::Super.mask();
pub const ROMACS_ALT_MASK: u32 = TransportModifierBit::Alt.mask();
pub const ROMACS_HYPER_MASK: u32 = TransportModifierBit::Hyper.mask();
