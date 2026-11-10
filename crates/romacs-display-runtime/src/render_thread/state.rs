use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

use romacs_display_protocol::frame_time::EventTime;

use winit::dpi::{LogicalSize, PhysicalSize, Size};

use crate::clipboard::ClipboardService;
use crate::host_types::ResolvedImageMetadata;
pub(crate) use crate::thread_comm::MonitorInfo;
use crate::thread_comm::RenderComms;
pub(super) use romacs_display_protocol::PointerAppearancePhase;
use romacs_display_protocol::{
    EffectsConfig, FrameGlyphBuffer, FrameRect, ImageCacheUsage, ImageId, ImageLoadToken,
    InteractionId, PointerAppearanceId, PointerAppearanceSelection, PresentationId,
    PresentedResizeAxis, VisualConfig,
};
use romacs_renderer_wgpu::{RowRange, WgpuRenderer};

use super::cursor::CursorState;
use super::frame_windows::{
    FrameLifecycle, GuiFrameRenderState, GuiFrameWindowManager, GuiFrameWindowState,
};
use super::render_quality::{RenderBackendProfile, RenderQualityPolicy};
pub(super) use super::toolbar::ToolbarResources;
use super::transitions::TransitionPolicy;

/// Decoded image facts shared from the render thread to the evaluator.
///
/// Only [`Self::Ready`] and [`Self::Failed`] are *terminal*: they are the
/// answers to "what happened to this load", and reading one consumes nothing
/// because the renderer publishes each at most once. [`Self::Band`] is
/// intermediate — rows of an image whose decode is still running, each
/// superseding the last — so a reader asking for the terminal state keeps
/// waiting past any number of bands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImageDecodeTerminal {
    /// One band of a decode still running: rows that exist, in source-row
    /// coordinates.
    Band(RowRange),
    Ready(ResolvedImageMetadata),
    /// The decode ended without pixels.  It carries GNU's diagnostic rather
    /// than a bare string, because the consumer that shows it to a user is the
    /// same one that has to word it.
    Failed(romacs_display_protocol::image_diagnostic::ImageDiagnostic),
}

impl ImageDecodeTerminal {
    #[cfg(test)]
    /// Whether this publication ends a load, as opposed to advancing one.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        match self {
            Self::Band(_) => false,
            Self::Ready(_) | Self::Failed(_) => true,
        }
    }

    #[cfg(test)]
    /// The terminal state of a load, if the terminal one is what is published.
    fn terminal(self) -> Option<Self> {
        self.is_terminal().then_some(self)
    }
}

/// Exclusive renderer publication capability for terminal image results.
///
/// The contained map is deliberately private. Holding this value models the
/// only contention redisplay's [`ImageTerminalProbe::Busy`] path must tolerate;
/// dropping it wakes bounded synchronous waiters once for the whole batch.
pub(crate) struct ImageTerminalPublication<'a> {
    terminals: MutexGuard<'a, HashMap<ImageLoadToken, ImageDecodeTerminal>>,
    terminal_changed: &'a Condvar,
}

impl ImageTerminalPublication<'_> {
    pub fn publish(&mut self, load: ImageLoadToken, terminal: ImageDecodeTerminal) {
        self.terminals.insert(load, terminal);
    }

    /// Publish one band of a decode that is still running.
    ///
    /// A band replaces the load's previous band and is in turn replaced by the
    /// terminal state, so a reader that never looks at bands costs one shared
    /// buffer per in-flight load and nothing else.
    pub fn publish_band(&mut self, load: ImageLoadToken, rows: RowRange) {
        self.publish(load, ImageDecodeTerminal::Band(rows));
    }

    pub(crate) fn clear_image(&mut self, image: ImageId) {
        self.terminals.retain(|load, _| load.image() != image);
    }
}

impl Drop for ImageTerminalPublication<'_> {
    fn drop(&mut self) {
        self.terminal_changed.notify_all();
    }
}

/// Cross-thread image facts published by the renderer and read by Elisp.
///
/// Terminal decode results need a condition variable for bounded synchronous
/// measurement. Cache usage is an independent lock-free snapshot: querying
/// `image-cache-size` never waits for a decode or the render thread.
pub(crate) struct ImageRenderState {
    terminals: Mutex<HashMap<ImageLoadToken, ImageDecodeTerminal>>,
    terminal_changed: Condvar,
    cache_bytes: AtomicU64,
}

impl Default for ImageRenderState {
    fn default() -> Self {
        Self {
            terminals: Mutex::new(HashMap::new()),
            terminal_changed: Condvar::new(),
            cache_bytes: AtomicU64::new(0),
        }
    }
}

impl ImageRenderState {
    fn lock_terminals(&self) -> MutexGuard<'_, HashMap<ImageLoadToken, ImageDecodeTerminal>> {
        self.terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    /// The terminal state of `load`, if it has one.
    ///
    /// A load whose decode is still running has bands published but no terminal
    /// state, and reports `None` here — a band is not an answer to "how did
    /// this load end".
    pub fn terminal(&self, load: ImageLoadToken) -> Option<ImageDecodeTerminal> {
        self.lock_terminals()
            .get(&load)
            .cloned()
            .and_then(ImageDecodeTerminal::terminal)
    }

    pub(crate) fn begin_terminal_publication(&self) -> ImageTerminalPublication<'_> {
        ImageTerminalPublication {
            terminals: self.lock_terminals(),
            terminal_changed: &self.terminal_changed,
        }
    }

    pub(crate) fn publish_terminal(&self, load: ImageLoadToken, terminal: ImageDecodeTerminal) {
        self.begin_terminal_publication().publish(load, terminal);
    }

    /// Publish one band of a load whose decode is still running.
    pub fn publish_band(&self, load: ImageLoadToken, rows: RowRange) {
        self.begin_terminal_publication().publish_band(load, rows);
    }

    #[cfg(test)]
    /// The newest band published for `load`, if one is.
    ///
    /// Only the newest: a band supersedes the one before it, so what is
    /// observable is how far the decode has come, not its history. The
    /// terminal state supersedes the bands in turn, so a load that has ended
    /// reports `None` here even though it published bands on the way.
    pub fn band(&self, load: ImageLoadToken) -> Option<RowRange> {
        match self.lock_terminals().get(&load) {
            Some(ImageDecodeTerminal::Band(rows)) => Some(*rows),
            Some(ImageDecodeTerminal::Ready(_) | ImageDecodeTerminal::Failed(_)) | None => None,
        }
    }

    pub(crate) fn clear_image_terminals(&self, image: ImageId) {
        self.begin_terminal_publication().clear_image(image);
    }

    #[cfg(test)]
    pub(crate) fn wait_for_terminal(
        &self,
        load: ImageLoadToken,
        timeout: std::time::Duration,
    ) -> Option<ImageDecodeTerminal> {
        let deadline = std::time::Instant::now() + timeout;
        let mut terminals = self.lock_terminals();
        loop {
            // Bands are skipped, not returned: this waits for the load to end,
            // and a decode that is emitting bands has not ended.
            if let Some(terminal) = terminals
                .get(&load)
                .cloned()
                .and_then(ImageDecodeTerminal::terminal)
            {
                return Some(terminal);
            }
            let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
            match self.terminal_changed.wait_timeout(terminals, remaining) {
                Ok((guard, result)) => {
                    terminals = guard;
                    if result.timed_out() {
                        return terminals
                            .get(&load)
                            .cloned()
                            .and_then(ImageDecodeTerminal::terminal);
                    }
                }
                Err(poisoned) => {
                    let (guard, _) = poisoned.into_inner();
                    terminals = guard;
                }
            }
        }
    }

    pub(crate) fn publish_cache_usage(&self, usage: ImageCacheUsage) {
        self.cache_bytes
            .store(usage.total_bytes(), Ordering::Release);
    }

    #[cfg(test)]
    #[must_use]
    pub fn cached_size_bytes(&self) -> u64 {
        self.cache_bytes.load(Ordering::Acquire)
    }
}

pub(crate) type SharedImageRenderState = Arc<ImageRenderState>;

/// Shared storage for monitor info accessible from both threads.
/// The Condvar is notified once monitors have been populated.
pub(crate) type SharedMonitorInfo = Arc<(Mutex<Vec<MonitorInfo>>, std::sync::Condvar)>;

pub(super) fn backend_uses_winit_logical_pixels() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub(super) fn effective_window_scale_factor(raw_scale_factor: f64) -> f64 {
    // On X11 fontconfig already handles DPI — the font metrics returned are
    // already scaled for the display.  Only Wayland needs us to apply the
    // compositor scale factor to rendering.
    if backend_uses_winit_logical_pixels() {
        raw_scale_factor
    } else {
        1.0
    }
}

pub(super) fn window_size_from_emacs_pixels(width: u32, height: u32) -> Size {
    if backend_uses_winit_logical_pixels() {
        Size::Logical(LogicalSize::new(width as f64, height as f64))
    } else {
        // X11: physical pixels as-is, matching GNU Emacs.  fontconfig DPI
        // already scales font sizes, so window dimensions are already at
        // the correct physical size.
        Size::Physical(PhysicalSize::new(width, height))
    }
}

pub(super) fn emacs_pixels_from_window_size(
    width: u32,
    height: u32,
    scale_factor: f64,
) -> (u32, u32) {
    if backend_uses_winit_logical_pixels() {
        (
            (width as f64 / scale_factor).round() as u32,
            (height as f64 / scale_factor).round() as u32,
        )
    } else {
        // X11: fontconfig handles DPI.  Return physical pixels as-is
        // so Emacs computes the correct character grid with the already-
        // scaled font metrics.
        (width, height)
    }
}

/// FPS counter and frame time tracking state.
#[derive(Clone)]
pub(super) struct FpsCounter {
    pub(super) enabled: bool,
    pub(super) last_instant: EventTime,
    pub(super) frame_count: u32,
    pub(super) display_value: f32,
    pub(super) frame_time_ms: f32,
    /// Wall-clock start of this frame's CPU render span. Not frame time:
    /// this measures work, and a frame-domain clock would read zero.
    pub(super) cpu_span_start: Instant,
}

/// Typing-speed overlay state for one native GUI frame window.
#[derive(Default)]
pub(super) struct TypingSpeedState {
    /// Key press timestamps for WPM calculation.
    pub(super) key_press_times: Vec<EventTime>,
    /// Smoothed WPM value for display.
    pub(super) displayed_wpm: f32,
}

/// Idle dim overlay state for one native GUI frame window.
pub(super) struct IdleDimState {
    pub(super) last_activity_time: EventTime,
    pub(super) current_alpha: f32,
    pub(super) active: bool,
}

impl IdleDimState {
    /// Idle state for a window that has just been created.
    ///
    /// There is no `Default`: "no activity recorded" has no honest zero value.
    /// Seeding with `None` and treating it as "idle forever" would dim a
    /// freshly-opened window immediately, so creation counts as activity and
    /// the caller must say when it happened.
    pub(super) fn new(at: EventTime) -> Self {
        Self {
            last_activity_time: at,
            current_alpha: 0.0,
            active: false,
        }
    }
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self {
            enabled: false,
            last_instant: romacs_display_protocol::frame_time::observe_platform_now(),
            frame_count: 0,
            display_value: 0.0,
            frame_time_ms: 0.0,
            cpu_span_start: Instant::now(),
        }
    }
}

/// One resolved native cursor policy, ordered from strongest to weakest by
/// [`Self::resolve`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PointerCursorIntent {
    Default,
    ChromeAction,
    PresentedWindowResize(PresentedResizeAxis),
    NativeFrameResize(winit::window::ResizeDirection),
}

impl PointerCursorIntent {
    #[must_use]
    pub(super) const fn resolve(
        native_resize: Option<winit::window::ResizeDirection>,
        presented_resize: Option<PresentedResizeAxis>,
        chrome_action: bool,
    ) -> Self {
        if let Some(direction) = native_resize {
            Self::NativeFrameResize(direction)
        } else if let Some(axis) = presented_resize {
            Self::PresentedWindowResize(axis)
        } else if chrome_action {
            Self::ChromeAction
        } else {
            Self::Default
        }
    }

    #[must_use]
    pub(super) fn icon(self) -> winit::cursor::CursorIcon {
        match self {
            Self::Default => winit::cursor::CursorIcon::Default,
            Self::ChromeAction => winit::cursor::CursorIcon::Pointer,
            Self::PresentedWindowResize(axis) => match axis {
                PresentedResizeAxis::Horizontal => winit::cursor::CursorIcon::EwResize,
                PresentedResizeAxis::Vertical => winit::cursor::CursorIcon::NsResize,
            },
            Self::NativeFrameResize(direction) => winit::cursor::CursorIcon::from(direction),
        }
    }
}

/// Borderless native-window chrome state (title bar, resize edges, decorations).
#[derive(Clone)]
pub(super) struct WindowChrome {
    pub(super) decorations_enabled: bool,
    pub(super) resize_edge: Option<winit::window::ResizeDirection>,
    pub(super) cursor_intent: PointerCursorIntent,
    pub(super) title: String,
    pub(super) titlebar_height: f32,
    pub(super) titlebar_hover: u32,
    /// When the titlebar was last clicked, for double-click detection.
    /// `None` until the first click: a window has not been clicked at creation,
    /// and pretending otherwise made the first click within the double-click
    /// interval of opening a window maximize it instead of dragging it.
    pub(super) last_titlebar_click: Option<EventTime>,
    pub(super) corner_radius: f32,
}

impl Default for WindowChrome {
    fn default() -> Self {
        Self {
            decorations_enabled: true,
            resize_edge: None,
            cursor_intent: PointerCursorIntent::Default,
            title: String::from("romacs"),
            titlebar_height: 30.0,
            titlebar_hover: 0,
            last_titlebar_click: None,
            corner_radius: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ImeCursorArea {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PresentedAppearanceKey {
    frame_id: u64,
    presentation: PresentationId,
    appearance: PointerAppearanceId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PendingPointerDamage {
    key: PresentedAppearanceKey,
    rect: FrameRect,
}

impl PendingPointerDamage {
    pub(super) const fn new(key: PresentedAppearanceKey, rect: FrameRect) -> Self {
        Self { key, rect }
    }

    pub(super) const fn key(self) -> PresentedAppearanceKey {
        self.key
    }

    #[cfg(test)]
    pub(super) const fn rect(self) -> FrameRect {
        self.rect
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PresentedPointerHit {
    frame_id: u64,
    presentation: PresentationId,
    interaction: Option<InteractionId>,
    appearance: Option<PointerAppearanceId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PresentedInteractionKey {
    frame_id: u64,
    presentation: PresentationId,
    interaction: InteractionId,
}

impl PresentedInteractionKey {
    pub(super) const fn new(presentation: PresentationId, interaction: InteractionId) -> Self {
        Self {
            frame_id: 0,
            presentation,
            interaction,
        }
    }

    pub(super) const fn for_frame(
        frame_id: u64,
        presentation: PresentationId,
        interaction: InteractionId,
    ) -> Self {
        Self {
            frame_id,
            presentation,
            interaction,
        }
    }

    pub(super) const fn frame_id(self) -> u64 {
        self.frame_id
    }

    pub(super) const fn presentation(self) -> PresentationId {
        self.presentation
    }

    pub(super) const fn interaction(self) -> InteractionId {
        self.interaction
    }
}

/// A left-button press owned by a presented interaction. A `None` target is
/// reserved for blank chrome space whose release must not leak to the buffer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PresentedPressCapture {
    target: Option<PresentedInteractionKey>,
    surface_origin: (f32, f32),
}

impl PresentedPressCapture {
    // Only `GuiFrameWindowState::capture_presented`, itself `#[cfg(test)]`,
    // captures a press without a surface origin.
    #[cfg(test)]
    pub(super) const fn new(target: Option<PresentedInteractionKey>) -> Self {
        Self {
            target,
            surface_origin: (0.0, 0.0),
        }
    }

    pub(super) const fn with_surface_origin(
        target: PresentedInteractionKey,
        surface_origin: (f32, f32),
    ) -> Self {
        Self {
            target: Some(target),
            surface_origin,
        }
    }

    pub(super) const fn target(self) -> Option<PresentedInteractionKey> {
        self.target
    }

    pub(super) fn local_coordinates(self, surface_x: f32, surface_y: f32) -> (f32, f32) {
        (
            surface_x - self.surface_origin.0,
            surface_y - self.surface_origin.1,
        )
    }
}

impl PresentedPointerHit {
    pub(super) const fn new(
        frame_id: u64,
        presentation: PresentationId,
        interaction: Option<InteractionId>,
        appearance: Option<PointerAppearanceId>,
    ) -> Self {
        Self {
            frame_id,
            presentation,
            interaction,
            appearance,
        }
    }

    pub(super) const fn presentation(self) -> PresentationId {
        self.presentation
    }

    pub(super) const fn interaction(self) -> Option<InteractionId> {
        self.interaction
    }

    pub(super) fn appearance_key(self) -> Option<PresentedAppearanceKey> {
        self.appearance.map(|appearance| {
            PresentedAppearanceKey::for_frame(self.frame_id, self.presentation, appearance)
        })
    }
}

impl PresentedAppearanceKey {
    #[cfg(test)]
    pub(super) const fn new(presentation: PresentationId, appearance: PointerAppearanceId) -> Self {
        Self {
            frame_id: 0,
            presentation,
            appearance,
        }
    }

    pub(super) const fn for_frame(
        frame_id: u64,
        presentation: PresentationId,
        appearance: PointerAppearanceId,
    ) -> Self {
        Self {
            frame_id,
            presentation,
            appearance,
        }
    }

    pub(super) const fn frame_id(self) -> u64 {
        self.frame_id
    }

    pub(super) const fn presentation(self) -> PresentationId {
        self.presentation
    }

    pub(super) const fn appearance(self) -> PointerAppearanceId {
        self.appearance
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ActivePointerAppearance {
    key: PresentedAppearanceKey,
    phase: PointerAppearancePhase,
}

impl ActivePointerAppearance {
    pub(super) const fn new(key: PresentedAppearanceKey, phase: PointerAppearancePhase) -> Self {
        Self { key, phase }
    }

    pub(super) const fn key(self) -> PresentedAppearanceKey {
        self.key
    }

    pub(super) const fn presentation(self) -> PresentationId {
        self.key.presentation()
    }

    pub(super) const fn appearance(self) -> PointerAppearanceId {
        self.key.appearance()
    }

    #[cfg(test)]
    pub(super) const fn phase(self) -> PointerAppearancePhase {
        self.phase
    }

    /// Produce renderer state only for the exact immutable presentation that
    /// owns both the appearance id and its primitive spans.
    pub(super) fn selection_for(
        self,
        frame: &FrameGlyphBuffer,
    ) -> Option<PointerAppearanceSelection> {
        if self.presentation() != frame.presentation_id
            || frame
                .presented_pointer()
                .appearance(self.appearance())
                .is_none()
        {
            return None;
        }
        Some(PointerAppearanceSelection::new(
            self.appearance(),
            self.phase,
        ))
    }
}

/// Snapshot-qualified pointer visual selection.
///
/// The pressed key is intentionally independent from the currently hovered
/// key: capture may remain on one visual range while the pointer hovers
/// another, and returning to the captured range restores its pressed phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PointerAppearanceState {
    active: Option<ActivePointerAppearance>,
    pressed: Option<PresentedAppearanceKey>,
}

impl PointerAppearanceState {
    pub(super) const fn active(self) -> Option<ActivePointerAppearance> {
        self.active
    }

    pub(super) fn selection_for(
        self,
        frame: &FrameGlyphBuffer,
    ) -> Option<PointerAppearanceSelection> {
        self.active.and_then(|active| active.selection_for(frame))
    }

    #[cfg(test)]
    pub(super) const fn pressed(self) -> Option<PresentedAppearanceKey> {
        self.pressed
    }

    pub(super) fn hover(&mut self, key: Option<PresentedAppearanceKey>) -> bool {
        let previous = *self;
        let next = key.map(|key| {
            let phase = if self.pressed == Some(key) {
                PointerAppearancePhase::Pressed
            } else {
                PointerAppearancePhase::Hover
            };
            ActivePointerAppearance::new(key, phase)
        });
        self.active = next;
        *self != previous
    }

    pub(super) fn hover_would_change(self, key: Option<PresentedAppearanceKey>) -> bool {
        let mut next = self;
        next.hover(key)
    }

    pub(super) fn button_would_change(
        self,
        key: Option<PresentedAppearanceKey>,
        pressed: bool,
    ) -> bool {
        let mut next = self;
        let mut changed = next.hover(key);
        changed |= if pressed {
            next.press()
        } else {
            next.release()
        };
        changed
    }

    pub(super) fn press(&mut self) -> bool {
        let previous = *self;
        self.pressed = self.active.map(ActivePointerAppearance::key);
        if let Some(active) = self.active.as_mut() {
            active.phase = PointerAppearancePhase::Pressed;
        }
        *self != previous
    }

    pub(super) fn release(&mut self) -> bool {
        let previous = *self;
        self.pressed = None;
        if let Some(active) = self.active.as_mut() {
            active.phase = PointerAppearancePhase::Hover;
        }
        *self != previous
    }

    pub(super) fn retire(&mut self, presentation: PresentationId) -> bool {
        let previous = *self;
        if self
            .active
            .is_some_and(|active| active.presentation() == presentation)
        {
            self.active = None;
        }
        if self
            .pressed
            .is_some_and(|pressed| pressed.presentation() == presentation)
        {
            self.pressed = None;
        }
        *self != previous
    }

    pub(super) fn cancel(&mut self) -> bool {
        let changed = self.active.is_some() || self.pressed.is_some();
        *self = Self::default();
        changed
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GuiChromeInteractionState {
    pub(super) menu_bar_hovered: Option<u32>,
    pub(super) menu_bar_active: Option<u32>,
    pub(super) toolbar_hovered: Option<u32>,
    pub(super) toolbar_pressed: Option<u32>,
    pub(super) toolbar_press_captured: bool,
    pub(super) compact_bar_menu_hovered: Option<u32>,
    pub(super) compact_bar_menu_active: Option<u32>,
    pub(super) compact_bar_tool_hovered: Option<u32>,
    pub(super) compact_bar_tool_pressed: Option<u32>,
}

impl GuiChromeInteractionState {
    pub(super) fn clear_menu_bar(&mut self) {
        self.menu_bar_hovered = None;
        self.menu_bar_active = None;
    }

    pub(super) fn clear_tab_bar(&mut self) {
        // Preserve press capture across chrome removal so a chrome press does
        // not leak a buffer release if the tab bar disappears mid-click.
    }

    pub(super) fn clear_toolbar(&mut self) {
        self.toolbar_hovered = None;
        self.toolbar_pressed = None;
        // Preserve press capture across chrome removal so a chrome press does
        // not leak a buffer release if the toolbar disappears mid-click.
    }

    pub(super) fn clear_compact_bar(&mut self) {
        self.compact_bar_menu_hovered = None;
        self.compact_bar_menu_active = None;
        self.compact_bar_tool_hovered = None;
        self.compact_bar_tool_pressed = None;
    }
}

pub(super) struct ChildFrameStyle {
    pub(super) corner_radius: f32,
    pub(super) shadow_enabled: bool,
    pub(super) shadow_layers: u32,
    pub(super) shadow_offset: f32,
    pub(super) shadow_opacity: f32,
}

impl Default for ChildFrameStyle {
    fn default() -> Self {
        Self {
            corner_radius: 0.0,
            shadow_enabled: true,
            shadow_layers: 4,
            shadow_offset: 2.0,
            shadow_opacity: 0.3,
        }
    }
}

pub(super) struct RenderGpuContext {
    pub(super) instance: wgpu::Instance,
    pub(super) adapter: wgpu::Adapter,
    pub(super) device: Arc<wgpu::Device>,
    pub(super) queue: Arc<wgpu::Queue>,
}

/// Where the evaluator's last scene is on its way to the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SceneDraw {
    Drawn,
    /// Just ingested: it draws this event-loop pass if it can.
    Arrived,
    /// It takes the platform's pace, like any redraw.
    Waiting,
}

pub(super) struct RenderApp {
    pub(super) comms: RenderComms,
    pub(super) menu_bar: crate::menu_bar::MenuBar,

    /// Display-lifetime owner for decoded application icon data and native
    /// Wayland toplevel-icon protocol state.
    pub(super) window_icon: crate::window_icon::WindowIconService,
    pub(super) presentation_observer: crate::presentation_feedback::PresentationObserver,

    /// Non-blocking handle to the clipboard worker that owns its display.
    pub(super) clipboard: Result<ClipboardService, String>,

    pub(super) gpu: Option<RenderGpuContext>,
    pub(super) gpu_startup: Option<super::gpu_startup::PendingGpu>,
    /// Last usable editor area for recreating a window during GPU startup.
    pub(super) pending_content_size: super::startup::InitialWindowSize,
    pub(super) gpu_startup_cancelled: Arc<std::sync::atomic::AtomicBool>,
    pub(super) startup_error: Option<String>,
    pub(super) startup_commands: std::collections::VecDeque<crate::thread_comm::RenderCommand>,
    pub(super) frame_leases: HashMap<u64, Arc<std::sync::atomic::AtomicBool>>,
    pub(super) renderer: Option<WgpuRenderer>,
    /// Native decoder workers signal this callback after replacing a latest
    /// frame or publishing control state. Production installs a winit proxy;
    /// tests deliberately retain the no-op callback.
    #[cfg(feature = "video")]
    pub(super) video_wake: romacs_video::VideoWake,
    /// Native browser callbacks wake winit after publishing lifecycle, script,
    /// or frame work. Production replaces the no-op test value before run.
    /// Device generation attached to every imported native video surface.
    #[cfg(feature = "video")]
    pub(super) video_gpu_generation: romacs_video::GpuGeneration,
    /// GPU-independent playback intent parked while a lost device and its
    /// renderer-owned VideoSystem are rebuilt.
    #[cfg(feature = "video")]
    pub(super) pending_video_recovery: Vec<romacs_renderer_wgpu::VideoRecoveryManifest>,
    pub(super) backend_profile: RenderBackendProfile,
    pub(super) render_policy: RenderQualityPolicy,

    /// Shared device-lost latch (`Arc<AtomicBool>` inside) plus the
    /// consecutive surface-Lost streak. Fed by the wgpu device-lost callback
    /// and the surface-acquire path; drained at the top of
    /// `handle_about_to_wait`, which runs `recover_from_device_loss`.
    pub(super) device_lost: super::device_loss::DeviceLossDetector,

    /// Shared media memory accounting. Fed from the asset-command choke
    /// point (`asset_commands.rs`); shader surfaces only so far — see the
    pub(super) faces: romacs_display_protocol::FrameFaceMap,
    /// Sorted (frame_id, ingest_seq) fingerprint of the frames the current
    /// `faces` map was aggregated from; unchanged fingerprint skips the
    /// per-render rebuild entirely.
    pub(super) faces_signature: Vec<(u64, u64)>,
    pub(super) modifiers: u32,
    /// The compiled NS modifier policy (issue #442).
    ///
    /// Defaults to GNU's `syms_of_nsterm` values so a session where Lisp has
    /// not pushed a policy yet cooks identically to the pre-policy
    /// hardcode: Option -> meta, Command -> super (Cocoa defaults,
    /// `src/nsterm.m:11576-11643`).
    pub(super) modifier_policy: romacs_display_protocol::ModifierPolicy,
    /// winit's aggregate answer to "which modifiers are down", kept so a
    /// key event can be re-cooked with its own GNU kind at send time.
    pub(super) modifier_state: winit::keyboard::ModifiersState,
    /// Per-side down/up tracking for the command/option/control families,
    /// which winit's aggregate `ModifiersState` cannot express; updated
    /// from the physical modifier keys' own key events.
    pub(super) modifier_sides: super::modifier_sides::ModifierSides,
    pub(super) key_repeats: super::key_repeat::KeyRepeats,
    /// The evaluator's busy indicator (busy_toast.rs).
    pub(super) busy_toast: super::busy_toast::BusyToast,
    /// How far the last scene from the evaluator got (`draw_fresh_scene').
    pub(super) scene: SceneDraw,
    pub(super) pending_file_drops: std::collections::HashSet<winit::event_loop::AsyncRequestSerial>,

    pub(super) image_metadata: SharedImageRenderState,

    pub(super) cursor_defaults: CursorState,

    pub(super) requested_visual_config: VisualConfig,
    pub(super) effects: EffectsConfig,

    pub(super) transition_policy: TransitionPolicy,

    pub(super) frame_windows: GuiFrameWindowManager,
    /// The child frames the last paint scene showed (core-free hosts).
    pub(super) scene_children: Vec<u64>,

    pub(super) child_frame_style: ChildFrameStyle,
    pub(super) toolbar: ToolbarResources,

    pub(super) scroll_indicators_enabled: bool,

    pub(super) extra_line_spacing: f32,
    pub(super) extra_letter_spacing: f32,

    pub(super) shared_monitors: Option<SharedMonitorInfo>,
    pub(super) monitors_populated: bool,
    pub(super) last_monitor_snapshot: Vec<MonitorInfo>,
    pub(super) debug_first_frame_readback_pending: bool,
    pub(super) debug_surface_readback_frames_remaining: u32,
    pub(super) lifecycle_flags: RenderLifecycle,
    /// Demand-driven frame scheduler: owns per-window redraw coalescing and
    /// wake deadlines (frame scheduling plan, Stage 2).
    pub(super) frame_coordinator: super::frame_sched::FrameCoordinator,
    /// Native models (design §4.1): survives renderer loss.
    pub(super) models: romacs_renderer_wgpu::model::ModelStore,
    /// Fractional wheel input over a model (design §5.1).
    pub(super) model_wheel: super::pointer_events::ModelWheel,
}

/// Only terminal causes may authorize display teardown. A native close
/// request is intentionally absent: live-frame close decisions belong to Lisp.
/// The render loop's exit names the one that ended it (`RenderLoopExit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderShutdownReason {
    EvaluatorShutdown,
    NativeWindowDestroyed,
    StartupCancelled,
    /// The GPU device was lost and rebuilding it kept failing.
    GpuLost,
}

pub(super) struct RenderLifecycle {
    pub resumed_seen: bool,
    pub about_to_wait_seen: bool,
    pub poll_when_idle: bool,
    shutdown_reason: Option<RenderShutdownReason>,
}

impl RenderLifecycle {
    pub(crate) fn request_shutdown(&mut self, reason: RenderShutdownReason) {
        self.shutdown_reason.get_or_insert(reason);
    }

    pub(crate) fn is_shutting_down(&self) -> bool {
        self.shutdown_reason.is_some()
    }

    pub(crate) fn shutdown_reason(&self) -> Option<RenderShutdownReason> {
        self.shutdown_reason
    }

    pub fn new(poll_when_idle: bool) -> Self {
        Self {
            resumed_seen: false,
            about_to_wait_seen: false,
            poll_when_idle,
            shutdown_reason: None,
        }
    }
}

/// The `ROMACS_MEDIA_BUDGET_MB` override (a decimal megabyte count) lets
/// the eviction driver be exercised interactively without allocating
/// hundreds of megabytes of surfaces. Unset or unparseable values fall back
/// to the renderer's 256MB default. Applied to the renderer (which owns the
/// budget, beside the caches) at bootstrap.
pub(super) fn media_budget_env_limit() -> Option<usize> {
    std::env::var("ROMACS_MEDIA_BUDGET_MB")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .map(|mb| mb.saturating_mul(1024 * 1024))
}

impl RenderApp {
    pub(super) fn install_backend_profile(&mut self, profile: RenderBackendProfile) {
        self.backend_profile = profile;
        self.apply_requested_visual_config();
    }

    pub(super) fn apply_requested_visual_config(&mut self) {
        let next_policy =
            RenderQualityPolicy::negotiate(self.backend_profile, &self.requested_visual_config);
        tracing::info!(
            adapter_class = ?next_policy.profile().adapter_class(),
            quality_mode = ?next_policy.mode(),
            "applying render-quality policy"
        );
        if !next_policy.allows_dynamic_effects() {
            self.frame_windows.discard_top_level_renderer_effects();
        }
        let effective = next_policy.effective_visual_config();
        self.cursor_defaults.apply_visual_config(effective);
        self.transition_policy = next_policy.transition_policy();
        self.frame_windows
            .apply_top_level_transition_policy(self.transition_policy);
        self.frame_windows
            .apply_top_level_pane_motion(next_policy.pane_motion());
        self.frame_windows
            .apply_top_level_child_frame_motion(next_policy.child_frame_motion());
        self.effects = effective.effects.clone();
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.effects = self.effects.clone();
            renderer.cursor_look = effective.cursor_look.clone();
        }
        // Motion turned off ends what runs, not only what starts, however
        // it was turned off: reduced motion, the user, or the adapter.
        if effective.child_frame_animations.off {
            self.frame_windows.finish_top_level_child_frame_animations();
        }
        if effective.window_animations.off {
            self.frame_windows.settle_top_level_pane_layouts();
        }
        self.frame_windows
            .sync_top_level_cursor_config(&self.cursor_defaults, true);
        self.render_policy = next_policy;
    }

    pub(super) fn new(
        comms: RenderComms,
        width: u32,
        height: u32,
        title: String,
        image_metadata: SharedImageRenderState,
        shared_monitors: SharedMonitorInfo,
        poll_when_idle: bool,
    ) -> Self {
        let mut frame_windows = GuiFrameWindowManager::new();
        // A deferred connection is not a frame transaction. Only a consumed
        // RealizeFrame may publish its primary identity and cancellation lease.
        if !comms.keep_alive_without_frames {
            frame_windows.set_primary_pending(GuiFrameWindowState {
                pending_scale_factor: None,
                lifecycle: FrameLifecycle::Pending {
                    width,
                    height,
                    scale_factor: 1.0,
                    mouse_hidden_for_typing: false,
                    ime_enabled: false,
                    last_ime_cursor_area: None,
                    chrome: WindowChrome {
                        title,
                        ..WindowChrome::default()
                    },
                    geometry_hints: None,
                },
                render: GuiFrameRenderState::new_without_device(
                    0,
                    false,
                    romacs_display_protocol::frame_time::observe_platform_now(),
                ),
            });
        }

        let requested_visual_config = VisualConfig::default();
        let backend_profile = RenderBackendProfile::pending();
        let render_policy =
            RenderQualityPolicy::negotiate(backend_profile, &requested_visual_config);

        Self {
            comms,
            window_icon: crate::window_icon::WindowIconService::new(),
            presentation_observer: crate::presentation_feedback::PresentationObserver::new(),
            clipboard: Err("clipboard is unavailable before display initialization".to_owned()),
            gpu: None,
            gpu_startup: None,
            pending_content_size: super::startup::InitialWindowSize { width, height },
            gpu_startup_cancelled: Default::default(),
            startup_error: None,
            startup_commands: Default::default(),
            frame_leases: Default::default(),
            menu_bar: crate::menu_bar::MenuBar::default(),
            renderer: None,
            #[cfg(feature = "video")]
            video_wake: romacs_video::VideoWake::noop(),
            #[cfg(feature = "video")]
            video_gpu_generation: romacs_video::GpuGeneration::INITIAL,
            #[cfg(feature = "video")]
            pending_video_recovery: Vec::new(),
            backend_profile,
            render_policy,
            device_lost: super::device_loss::DeviceLossDetector::new(),
            faces: rustc_hash::FxHashMap::default(),
            faces_signature: Vec::new(),
            modifiers: 0,
            modifier_policy: romacs_display_protocol::ModifierPolicy::gnu_ns_default(),
            modifier_state: winit::keyboard::ModifiersState::empty(),
            modifier_sides: super::modifier_sides::ModifierSides::default(),
            key_repeats: super::key_repeat::KeyRepeats::default(),
            busy_toast: super::busy_toast::BusyToast::default(),
            scene: SceneDraw::Drawn,
            pending_file_drops: Default::default(),
            image_metadata,
            cursor_defaults: CursorState::new(
                romacs_display_protocol::frame_time::observe_platform_now(),
            ),
            requested_visual_config,
            effects: EffectsConfig::default(),
            transition_policy: TransitionPolicy::default(),
            frame_windows,
            scene_children: Vec::new(),
            child_frame_style: ChildFrameStyle::default(),
            toolbar: ToolbarResources::default(),
            scroll_indicators_enabled: false,
            extra_line_spacing: 0.0,
            extra_letter_spacing: 0.0,
            shared_monitors: Some(shared_monitors),
            monitors_populated: false,
            last_monitor_snapshot: Vec::new(),
            debug_first_frame_readback_pending: std::env::var_os(
                "ROMACS_DEBUG_FIRST_FRAME_READBACK",
            )
            .is_some(),
            debug_surface_readback_frames_remaining: std::env::var("ROMACS_DEBUG_SURFACE_READBACK")
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|count| *count > 0)
                .unwrap_or_else(|| {
                    if std::env::var_os("ROMACS_DEBUG_SURFACE_READBACK").is_some() {
                        32
                    } else {
                        0
                    }
                }),
            lifecycle_flags: RenderLifecycle::new(poll_when_idle),
            frame_coordinator: super::frame_sched::FrameCoordinator::new(),
            models: romacs_renderer_wgpu::model::ModelStore::new(),
            model_wheel: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests;
