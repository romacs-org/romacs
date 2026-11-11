//! One GPU policy for every Romacs entry point: how an instance is made,
//! which adapter is chosen, and how the renderer's device is requested. The
//! program and its tests go through the same three functions, so a test
//! renders on the GPU the program would pick.

use std::sync::Mutex;

/// Instances are made one at a time, process-wide. Making one runs every
/// installed Vulkan driver's setup through the loader, and some drivers'
/// setup is not safe on two threads at once: a driver installed for a GPU
/// that is not even present crashed the loader that way.
static MAKING_INSTANCE: Mutex<()> = Mutex::new(());

/// A new wgpu instance for DESCRIPTOR. The program makes one at startup and
/// one per device-loss recovery, after the last is gone; tests share one
/// (`test_gpu`).
pub fn new_instance(descriptor: wgpu::InstanceDescriptor) -> wgpu::Instance {
    let _one_at_a_time = MAKING_INSTANCE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    wgpu::Instance::new(descriptor)
}

/// The power preference `ROMACS_GPU` asks for: `low` or `integrated` prefer
/// an integrated GPU, anything else (or nothing) a discrete one.
pub(crate) fn power_preference() -> wgpu::PowerPreference {
    match std::env::var("ROMACS_GPU").as_deref() {
        Ok("low" | "integrated") => wgpu::PowerPreference::LowPower,
        Ok("high" | "discrete") | Err(_) => wgpu::PowerPreference::HighPerformance,
        Ok(other) => {
            tracing::warn!("ROMACS_GPU={other}: unrecognized, preferring a discrete GPU");
            wgpu::PowerPreference::HighPerformance
        }
    }
}

/// How well an adapter of KIND fits PREFERENCE: lower is better.
fn adapter_rank(kind: wgpu::DeviceType, preference: wgpu::PowerPreference) -> u8 {
    use wgpu::{DeviceType, PowerPreference};
    match (preference, kind) {
        (PowerPreference::None, _) => 0,
        (PowerPreference::LowPower, DeviceType::IntegratedGpu)
        | (PowerPreference::HighPerformance, DeviceType::DiscreteGpu) => 0,
        (_, DeviceType::IntegratedGpu | DeviceType::DiscreteGpu) => 1,
        (_, DeviceType::Other) => 2,
        (_, DeviceType::VirtualGpu) => 3,
        (_, DeviceType::Cpu) => 4,
    }
}

/// The adapter the program renders with, from ADAPTERS (an instance's
/// `enumerate_adapters`): the best fit for `power_preference`, keeping the
/// enumeration's order among equals, that can present to SURFACE when one
/// is given.
pub fn choose_adapter(
    mut adapters: Vec<wgpu::Adapter>,
    surface: Option<&wgpu::Surface<'_>>,
) -> Option<wgpu::Adapter> {
    let preference = power_preference();
    adapters.sort_by_key(|adapter| adapter_rank(adapter.get_info().device_type, preference));
    adapters
        .into_iter()
        .find(|adapter| surface.is_none_or(|surface| adapter.is_surface_supported(surface)))
}

#[cfg(target_os = "linux")]
pub(crate) const LINUX_DMA_BUF_EXTENSIONS: [&std::ffi::CStr; 4] = [
    ash::khr::external_memory_fd::NAME,
    ash::ext::external_memory_dma_buf::NAME,
    ash::ext::image_drm_format_modifier::NAME,
    ash::ext::queue_family_foreign::NAME,
];

fn requested_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let requested = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
        | wgpu::Features::TEXTURE_FORMAT_NV12
        | wgpu::Features::TEXTURE_FORMAT_P010;
    // Root glyph-pass diagnostics also use timestamp queries without video.
    let requested = if std::env::var_os("ROMACS_GPU_FRAME_TIMING").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        requested | wgpu::Features::TIMESTAMP_QUERY
    } else {
        requested
    };
    adapter.features() & requested
}

/// Create the renderer's device and queue with platform interop enabled when
/// the selected adapter can support it.
///
/// Keeping this policy here prevents standalone and windowed renderer entry
/// points from silently creating devices with different native-video
/// capabilities.
pub async fn request_renderer_device(
    adapter: &wgpu::Adapter,
    label: &'static str,
) -> Result<(wgpu::Device, wgpu::Queue), String> {
    tracing::warn!(adapter = ?adapter.get_info(), "renderer adapter backend fingerprint");
    let descriptor = wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: requested_features(adapter),
        required_limits: wgpu::Limits::default(),
        memory_hints: Default::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        trace: wgpu::Trace::Off,
    };

    std::cfg_select! {
        target_os = "linux" => {
            if linux::supports_dma_buf_extensions(adapter) || crate::native_presentation::requested() {
                match linux::request_interop_device(adapter, &descriptor) {
                    Ok(device) => return Ok(device),
                    Err(error) => tracing::warn!(
                        %error,
                        "failed to enable Vulkan interop device extensions; falling back to a standard renderer device"
                    ),
                }
            } else {
                tracing::info!(
                    "Vulkan adapter does not expose the complete Linux DMA-BUF extension set"
                );
            }
        }
        _ => {}
    }

    adapter
        .request_device(&descriptor)
        .await
        .map_err(|error| format!("failed to create renderer device: {error}"))
}

#[cfg(target_os = "linux")]
mod linux {
    use super::LINUX_DMA_BUF_EXTENSIONS;
    use wgpu::hal::api::Vulkan;

    pub(super) fn supports_dma_buf_extensions(adapter: &wgpu::Adapter) -> bool {
        if !cfg!(feature = "video") {
            return false;
        }
        // SAFETY: the guard is used only for immutable capability inspection
        // and is dropped before this function returns.
        unsafe { adapter.as_hal::<Vulkan>() }.is_some_and(|hal| {
            LINUX_DMA_BUF_EXTENSIONS.iter().all(|extension| {
                hal.physical_device_capabilities()
                    .supports_extension(extension)
            })
        })
    }

    pub(super) fn request_interop_device(
        adapter: &wgpu::Adapter,
        descriptor: &wgpu::DeviceDescriptor<'_>,
    ) -> Result<(wgpu::Device, wgpu::Queue), String> {
        // SAFETY: `hal_adapter` is obtained from this exact wgpu adapter. The
        // callback only adds extensions proven supported above, preserves all
        // wgpu-required features/extensions, and the resulting HAL device is
        // immediately returned to the same adapter for wgpu-core ownership.
        unsafe {
            let hal_adapter = adapter
                .as_hal::<Vulkan>()
                .ok_or_else(|| "selected renderer adapter is not Vulkan".to_owned())?;
            let dma_buf = supports_dma_buf_extensions(adapter);
            let mut timing = crate::native_presentation::DeviceFeatures::query(&hal_adapter);
            let hal_device = hal_adapter
                .open_with_callback(
                    descriptor.required_features,
                    &descriptor.required_limits,
                    &descriptor.memory_hints,
                    Some(Box::new(|mut args| {
                        if let Some(timing) = &mut timing {
                            timing.enable(&mut args);
                        }
                        for extension in LINUX_DMA_BUF_EXTENSIONS.into_iter().filter(|_| dma_buf) {
                            if !args.extensions.contains(&extension) {
                                args.extensions.push(extension);
                            }
                        }
                    })),
                )
                .map_err(|error| format!("failed to open Vulkan interop device: {error:?}"))?;
            adapter
                .create_device_from_hal(hal_device, descriptor)
                .map_err(|error| format!("failed to adopt Vulkan interop device: {error}"))
        }
    }
}

/// One GPU for a whole test process: one instance, made once, and the
/// adapter `choose_adapter` picks from it. Each test asks it for its own
/// device, so tests stay independent and run in parallel without ever making
/// two instances at once.
#[cfg(any(test, feature = "test-gpu"))]
pub mod test_gpu {
    use std::sync::OnceLock;

    /// The process's instance, without a display: offscreen rendering only.
    pub fn instance() -> &'static wgpu::Instance {
        static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            super::new_instance(wgpu::InstanceDescriptor::new_without_display_handle_from_env())
        })
    }

    /// The adapter the program would choose, None without a usable GPU.
    pub fn adapter() -> Option<&'static wgpu::Adapter> {
        static ADAPTER: OnceLock<Option<wgpu::Adapter>> = OnceLock::new();
        ADAPTER
            .get_or_init(|| {
                let adapters = pollster::block_on(instance().enumerate_adapters(wgpu::Backends::all()));
                super::choose_adapter(adapters, None)
            })
            .as_ref()
    }

    /// The software (CPU) Vulkan adapter on the same instance, for tests
    /// that pin their pixels to a rasterizer every machine shares.
    pub fn software_adapter() -> Option<&'static wgpu::Adapter> {
        static SOFTWARE: OnceLock<Option<wgpu::Adapter>> = OnceLock::new();
        SOFTWARE
            .get_or_init(|| {
                pollster::block_on(instance().enumerate_adapters(wgpu::Backends::VULKAN))
                    .into_iter()
                    .find(|adapter| adapter.get_info().device_type == wgpu::DeviceType::Cpu)
            })
            .as_ref()
    }

    /// A device of its own on the shared adapter, as DESCRIPTOR asks.
    pub fn device(descriptor: &wgpu::DeviceDescriptor<'_>) -> Option<(wgpu::Device, wgpu::Queue)> {
        pollster::block_on(adapter()?.request_device(descriptor)).ok()
    }

    /// A device as the renderer requests it (`request_renderer_device`).
    pub fn renderer_device(label: &'static str) -> Option<(wgpu::Device, wgpu::Queue)> {
        pollster::block_on(super::request_renderer_device(adapter()?, label)).ok()
    }

    /// An offscreen renderer WIDTH by HEIGHT on a device of its own.
    pub fn renderer(width: u32, height: u32) -> Result<crate::WgpuRenderer, String> {
        let (device, queue) =
            renderer_device("Romacs Test Device").ok_or("no usable GPU adapter for a test renderer")?;
        Ok(crate::WgpuRenderer::offscreen(std::sync::Arc::new(device), std::sync::Arc::new(queue), width, height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{DeviceType, PowerPreference};

    #[test]
    fn the_preferred_kind_ranks_first_and_software_last() {
        let kinds = [DeviceType::Cpu, DeviceType::IntegratedGpu, DeviceType::VirtualGpu, DeviceType::DiscreteGpu];
        let ranked = |preference| {
            let mut kinds = kinds;
            kinds.sort_by_key(|kind| adapter_rank(*kind, preference));
            kinds
        };
        assert_eq!(ranked(PowerPreference::HighPerformance),
                   [DeviceType::DiscreteGpu, DeviceType::IntegratedGpu, DeviceType::VirtualGpu, DeviceType::Cpu]);
        assert_eq!(ranked(PowerPreference::LowPower),
                   [DeviceType::IntegratedGpu, DeviceType::DiscreteGpu, DeviceType::VirtualGpu, DeviceType::Cpu]);
    }

    /// Making instances on many threads at once crashed the Vulkan loader on
    /// a machine with a driver installed for absent hardware; they are made
    /// one at a time.
    #[test]
    fn instances_made_on_many_threads_at_once_all_arrive() {
        let made: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| {
                new_instance(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
            }))
            .collect();
        for thread in made {
            thread.join().expect("an instance was made");
        }
    }

    #[test]
    fn the_test_gpu_is_the_programs_choice() {
        let Some(adapter) = test_gpu::adapter() else { return };
        let adapters = pollster::block_on(test_gpu::instance().enumerate_adapters(wgpu::Backends::all()));
        let chosen = choose_adapter(adapters, None).expect("an adapter");
        assert_eq!(adapter.get_info(), chosen.get_info());
    }
}
