//! Passive native tooltip lifetime. Callers own help selection and delay policy.
use crate::presentation::{PopupCommit, PopupHost, PopupRole};
use romacs_display_protocol::tooltip::TooltipRequest;
use romacs_display_protocol::{
    Point, PopupConstraintPolicy, PopupPlacement, PopupPreferredSide, Rect,
};
use romacs_renderer_wgpu::{TooltipLayout, WgpuGlyphAtlas, WgpuRenderer};
use std::{sync::Arc, time::Instant};
use winit::{
    event::WindowEvent,
    window::{Window, WindowId},
};

pub(crate) struct TooltipOwner {
    pub frame: u64,
    pub parent: Arc<dyn Window>,
    pub anchor: Rect,
    pub metrics: (f32, f32, f32),
    pub fonts: romacs_display_protocol::frame_glyphs::FrameGlyphBuffer,
}

struct ReadyTooltip {
    owner: TooltipOwner,
    request: TooltipRequest,
    expires: Instant,
}

fn placement(owner: &TooltipOwner, request: &TooltipRequest) -> PopupPlacement {
    PopupPlacement::new(
        owner.anchor,
        PopupPreferredSide::Below,
        Point::new(request.offset.0 as f32, request.offset.1 as f32),
        PopupConstraintPolicy::FlipAndShift { padding: 0.0 },
    )
}

// A mapped tooltip owns all native/GPU resources together. Drop its child
// surface before releasing the retained parent.
struct MappedTooltip {
    host: PopupHost,
    paint: TooltipLayout,
    atlas: WgpuGlyphAtlas,
    ready: ReadyTooltip,
    reposition: Option<PopupPlacement>,
}

#[derive(Default)]
enum Presentation {
    #[default]
    Hidden,
    Ready(Box<ReadyTooltip>),
    Mapped(Box<MappedTooltip>),
}

#[derive(Default)]
pub(crate) struct Tooltips {
    state: Presentation,
    retired: crate::presentation::Retirements<Box<MappedTooltip>>,
}

impl Tooltips {
    pub fn show(&mut self, owner: TooltipOwner, request: TooltipRequest, now: Instant) {
        let expires = now.checked_add(request.timeout).unwrap_or(now);
        if let Presentation::Mapped(mapped) = &mut self.state
            && mapped.ready.owner.parent.id() == owner.parent.id()
            && mapped.ready.request.same_content(&request)
        {
            mapped.ready.expires = expires;
            if mapped.ready.owner.anchor != owner.anchor
                || mapped.ready.request.offset != request.offset
            {
                mapped.reposition = Some(placement(&owner, &request));
            }
            mapped.ready.owner = owner;
            mapped.ready.request = request;
            return;
        }
        self.hide();
        self.state = Presentation::Ready(Box::new(ReadyTooltip {
            owner,
            request,
            expires,
        }));
    }

    pub fn hide(&mut self) -> bool {
        let visible = matches!(self.state, Presentation::Mapped(_));
        if let Presentation::Mapped(mapped) = std::mem::take(&mut self.state) {
            self.retired.push(mapped);
        }
        visible
    }

    pub(crate) fn commit_retirements(&mut self, _commit: &PopupCommit<'_>) {
        self.retired.commit();
    }

    /// Terminal teardown and device loss cannot wait for another event batch.
    pub fn shutdown(&mut self) {
        self.hide();
        self.retired.commit();
    }

    fn ready(&self) -> Option<&ReadyTooltip> {
        match &self.state {
            Presentation::Hidden => None,
            Presentation::Ready(ready) => Some(&**ready),
            Presentation::Mapped(mapped) => Some(&mapped.ready),
        }
    }

    pub fn owner(&self) -> Option<u64> {
        self.ready().map(|r| r.owner.frame)
    }
    pub(crate) fn owns_parent(&self, id: WindowId) -> bool {
        self.ready().is_some_and(|r| r.owner.parent.id() == id)
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.ready().map(|r| r.expires)
    }

    pub fn sync(
        &mut self,
        now: Instant,
        commit: &PopupCommit<'_>,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Result<(), String> {
        if self.deadline().is_some_and(|d| d <= now) {
            self.hide();
        }
        self.commit_retirements(commit);
        self.state = match std::mem::take(&mut self.state) {
            Presentation::Ready(ready) => Presentation::Mapped(Box::new(MappedTooltip::create(
                *ready, commit, instance, adapter, device, queue, format,
            )?)),
            state @ (Presentation::Hidden | Presentation::Mapped(_)) => state,
        };
        if let Presentation::Mapped(mapped) = &mut self.state
            && let Some(placement) = mapped.reposition.take()
        {
            mapped.host.reposition(0, placement);
        }
        Ok(())
    }

    pub fn event(
        &mut self,
        id: WindowId,
        event: &WindowEvent,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut WgpuRenderer,
    ) -> bool {
        let Presentation::Mapped(mapped) = &mut self.state else {
            return false;
        };
        let Some(depth) = mapped.host.depth(id) else {
            return false;
        };
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.hide();
            }
            WindowEvent::SurfaceResized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                let size = match event {
                    WindowEvent::SurfaceResized(size) => Some(*size),
                    _ => None,
                };
                let geometry = mapped.host.resize(depth, device, size);
                mapped.remeasure(
                    geometry.device_scale().get(),
                    geometry.logical_size().width(),
                    geometry.logical_size().height(),
                    device,
                    queue,
                );
            }
            WindowEvent::RedrawRequested => {
                mapped.host.draw(depth, device, queue, |target| {
                    renderer.render_native_tooltip(target, &mapped.paint, &mut mapped.atlas)
                });
            }
            _ => {}
        }
        true
    }
}

impl MappedTooltip {
    fn remeasure(
        &mut self,
        scale: f32,
        width: f32,
        height: f32,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        self.atlas.set_scale_factor(scale);
        self.paint = TooltipLayout::measure_with_atlas(
            &self.ready.request,
            self.ready.owner.metrics.2,
            self.ready.owner.metrics.1,
            scale,
            &mut self.atlas,
            device,
            queue,
        );
        self.paint.fit_surface(width, height);
    }

    fn create(
        ready: ReadyTooltip,
        commit: &PopupCommit<'_>,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Result<Self, String> {
        let request = &ready.request;
        let owner = &ready.owner;
        let mut atlas = WgpuGlyphAtlas::new_with_scale(device, owner.parent.scale_factor() as f32);
        atlas.set_metrics(owner.metrics.0, owner.metrics.1);
        let mut bindings =
            romacs_display_protocol::frame_glyphs::FrameGlyphBuffer::with_size(0.0, 0.0);
        bindings.clone_font_bindings_from(&owner.fonts);
        for run in &request.runs {
            bindings.faces.insert(run.face.id, run.face.clone());
            if let Some(font) = &run.font {
                bindings.fonts.insert(font.id, font.clone());
            }
        }
        atlas.set_current_frame_fonts(bindings.font_bindings());
        let paint = TooltipLayout::measure_with_atlas(
            request,
            owner.metrics.2,
            owner.metrics.1,
            owner.parent.scale_factor() as f32,
            &mut atlas,
            device,
            queue,
        );
        let (width, height) = paint.extent();
        let placement = placement(owner, request);
        let mut host = PopupHost::default();
        let geometry = host
            .open_with_role(
                commit,
                owner.parent.clone(),
                placement,
                (width, height),
                instance,
                adapter,
                device,
                format,
                PopupRole::Tooltip,
            )?
            .ok_or("tooltip owner is not mapped")?;
        let size = geometry.logical_size();
        let mut mapped = Self {
            host,
            paint,
            atlas,
            ready,
            reposition: None,
        };
        mapped.remeasure(
            geometry.device_scale().get(),
            size.width(),
            size.height(),
            device,
            queue,
        );
        Ok(mapped)
    }
}

impl Drop for Tooltips {
    fn drop(&mut self) {
        self.shutdown();
    }
}
