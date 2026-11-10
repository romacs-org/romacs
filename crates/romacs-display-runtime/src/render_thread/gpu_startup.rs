//! Driver discovery and device acquisition without worker-owned native surfaces.

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use winit::{
    event_loop::EventLoopProxy,
    window::{Window, WindowId},
};

type Reply<T> = Receiver<Result<T, String>>;

struct WorkerReply<T> {
    sender: Option<Sender<Result<T, String>>>,
    proxy: EventLoopProxy,
}

impl<T> Drop for WorkerReply<T> {
    fn drop(&mut self) {
        // Panic/disconnection must become visible before the native-loop wake.
        drop(self.sender.take());
        self.proxy.wake_up();
    }
}

fn spawn<T: Send + 'static>(
    name: &str,
    proxy: EventLoopProxy,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<Reply<T>, String> {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let reply = WorkerReply {
        sender: Some(sender),
        proxy,
    };
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let result = work();
            let _ = reply.sender.as_ref().unwrap().send(result);
            drop(reply);
        })
        .map_err(|error| error.to_string())?;
    Ok(receiver)
}

pub(super) struct PreparedGpu {
    pub(super) window: Arc<dyn Window>,
    pub(super) instance: wgpu::Instance,
    pub(super) surface: wgpu::Surface<'static>,
    pub(super) adapter: wgpu::Adapter,
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
}

pub(super) enum PendingGpu {
    SurfaceSize(PreparedGpu),
    Adapters {
        window: Arc<dyn Window>,
        reply: Reply<(wgpu::Instance, Vec<wgpu::Adapter>)>,
    },
    Device {
        window: Arc<dyn Window>,
        instance: wgpu::Instance,
        surface: wgpu::Surface<'static>,
        adapter: wgpu::Adapter,
        reply: Reply<(wgpu::Device, wgpu::Queue)>,
    },
}

pub(super) enum GpuPoll {
    Pending(PendingGpu),
    Ready(PreparedGpu),
}

impl PendingGpu {
    pub(super) fn start(
        window: Arc<dyn Window>,
        descriptor: wgpu::InstanceDescriptor,
        proxy: EventLoopProxy,
    ) -> Result<Self, String> {
        crate::paint_host::trace_gpu("adapters-start");
        let reply = spawn("gpu-adapters", proxy, move || {
            let instance = romacs_renderer_wgpu::new_instance(descriptor);
            crate::paint_host::trace_gpu("instance");
            let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
            crate::paint_host::trace_gpu("adapters");
            Ok((instance, adapters))
        })?;
        Ok(Self::Adapters { window, reply })
    }

    pub(super) fn window_id(&self) -> WindowId {
        self.window().id()
    }

    pub(super) fn window(&self) -> &dyn Window {
        match self {
            Self::SurfaceSize(prepared) => prepared.window.as_ref(),
            Self::Adapters { window, .. } | Self::Device { window, .. } => window.as_ref(),
        }
    }

    pub(super) fn poll(self, proxy: &EventLoopProxy) -> Result<GpuPoll, String> {
        match self {
            Self::SurfaceSize(prepared) => Ok(Self::when_sized(prepared)),
            Self::Adapters { ref reply, .. } => {
                let result = match reply.try_recv() {
                    Ok(result) => result?,
                    Err(TryRecvError::Empty) => return Ok(GpuPoll::Pending(self)),
                    Err(TryRecvError::Disconnected) => {
                        return Err("GPU adapter worker exited without a result".into());
                    }
                };
                let Self::Adapters { window, .. } = self else {
                    unreachable!()
                };
                let (instance, adapters) = result;
                // The pinned desktop HALs do not use enumeration's surface
                // hint, so the surface is made here on the main thread and
                // the choice filters by it.
                let surface = instance
                    .create_surface(window.clone())
                    .map_err(|error| error.to_string())?;
                let adapter = romacs_renderer_wgpu::choose_adapter(adapters, Some(&surface))
                    .ok_or_else(|| "no GPU adapter supports the native surface".to_owned())?;
                let worker_adapter = adapter.clone();
                crate::paint_host::trace_gpu("adapter-chosen");
                let reply = spawn("gpu-device", proxy.clone(), move || {
                    let device = pollster::block_on(romacs_renderer_wgpu::request_renderer_device(
                        &worker_adapter,
                        "Romacs Render Thread Device",
                    ))
                    .map_err(|error| error.to_string());
                    crate::paint_host::trace_gpu("device");
                    device
                })?;
                Ok(GpuPoll::Pending(Self::Device {
                    window,
                    instance,
                    surface,
                    adapter,
                    reply,
                }))
            }
            Self::Device { ref reply, .. } => {
                let (device, queue) = match reply.try_recv() {
                    Ok(result) => result?,
                    Err(TryRecvError::Empty) => return Ok(GpuPoll::Pending(self)),
                    Err(TryRecvError::Disconnected) => {
                        return Err("GPU device worker exited without a result".into());
                    }
                };
                let Self::Device {
                    window,
                    instance,
                    surface,
                    adapter,
                    ..
                } = self
                else {
                    unreachable!()
                };
                Ok(Self::when_sized(PreparedGpu {
                    window,
                    instance,
                    surface,
                    adapter,
                    device,
                    queue,
                }))
            }
        }
    }

    fn when_sized(prepared: PreparedGpu) -> GpuPoll {
        let size = prepared.window.surface_size();
        if size.width == 0 || size.height == 0 {
            GpuPoll::Pending(Self::SurfaceSize(prepared))
        } else {
            GpuPoll::Ready(prepared)
        }
    }
}
