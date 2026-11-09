//! Winit + wgpu GPU-accelerated display backend.

mod animation;
mod events;
pub mod toolbar_icons;

pub mod media_budget {
    //! Moved beside the renderer caches (real byte counts live there);
    //! re-exported here so existing `backend::wgpu::media_budget` paths work.
    pub use romacs_renderer_wgpu::media_budget::*;
}

#[cfg(feature = "video")]
pub use romacs_renderer_wgpu::{CachedVideo, VideoCache, VideoState};

pub use romacs_renderer_wgpu::{
    BufferFormat, CachedImage, ExternalBuffer, GlyphKey, GlyphVertex, ImageCache, ImageState,
    PlatformBuffer, SharedMemoryBuffer, WgpuGlyphAtlas, WgpuRenderer,
};

#[cfg(target_os = "linux")]
pub use romacs_renderer_wgpu::DmaBufBuffer;

pub use animation::{AnimatedProperty, Animation, AnimationEngine, AnimationTarget, Easing};
pub use events::{
    EventKind, ROMACS_ALT_MASK, ROMACS_CTRL_MASK, ROMACS_EVENT_BUTTON_PRESS,
    ROMACS_EVENT_BUTTON_RELEASE, ROMACS_EVENT_CLOSE, ROMACS_EVENT_FILE_DROP,
    ROMACS_EVENT_FOCUS_IN, ROMACS_EVENT_FOCUS_OUT, ROMACS_EVENT_IMAGE_DIMENSIONS_READY,
    ROMACS_EVENT_KEY_PRESS, ROMACS_EVENT_KEY_RELEASE, ROMACS_EVENT_MENU_BAR_CLICK,
    ROMACS_EVENT_MENU_SELECTION, ROMACS_EVENT_MOUSE_MOVE, ROMACS_EVENT_RESIZE,
    ROMACS_EVENT_SCROLL, ROMACS_EVENT_TERMINAL_EXITED, ROMACS_EVENT_TERMINAL_TITLE_CHANGED,
    ROMACS_EVENT_TOOL_BAR_CLICK, ROMACS_HYPER_MASK, ROMACS_META_MASK, ROMACS_SHIFT_MASK,
    ROMACS_SUPER_MASK, RomacsInputEvent,
};

// DRM device discovery for GPU device path mapping
#[cfg(target_os = "linux")]
mod drm_device;

#[cfg(target_os = "linux")]
pub use drm_device::render_node_from_device_number;
