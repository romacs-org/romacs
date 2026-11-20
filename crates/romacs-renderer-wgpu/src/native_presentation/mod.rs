//! Opt-in Vulkan presentation observations. No target times or pacing changes.
//! Native first-pixel output is distinct from physical display visibility.
//!
//! This module is the crate's one unsafe wrapper (`#![deny(unsafe_code)]`
//! holds the rest). VK_EXT_present_timing and VK_KHR_present_id2 are newer
//! than ash 0.38, the version wgpu-hal is built on, and wgpu reaches its
//! Vulkan handles only through its unsafe `as_hal`: no crate in Cargo.lock
//! makes these calls safely.
mod abi;
mod clock;
mod query;

use ash::vk;
pub use query::{Observation, SwapchainTiming};
use wgpu::hal::{api::Vulkan, vulkan};

pub fn requested() -> bool {
    std::env::var_os("ROMACS_GUI_PRESENTATION_RECEIPT").is_some()
}

struct DeviceFeatures {
    timing: abi::TimingFeatures,
    ids: abi::IdFeatures,
}
impl DeviceFeatures {
    fn query(adapter: &vulkan::Adapter) -> Option<Self> {
        if adapter.shared_instance().instance_api_version() < vk::API_VERSION_1_1
            || adapter.physical_device_capabilities().properties().api_version
                < vk::API_VERSION_1_1
        {
            return None;
        }
        if !requested()
            || ![abi::TIMING, abi::ID2, ash::khr::calibrated_timestamps::NAME]
                .iter()
                .all(|ext| {
                    adapter
                        .physical_device_capabilities()
                        .supports_extension(ext)
                })
            || !adapter
                .shared_instance()
                .extensions()
                .contains(&ash::khr::get_surface_capabilities2::NAME)
        {
            return None;
        }
        let mut this = Self {
            timing: Default::default(),
            ids: Default::default(),
        };
        this.timing.p_next = (&mut this.ids as *mut abi::IdFeatures).cast();
        let mut features = vk::PhysicalDeviceFeatures2 {
            p_next: (&mut this.timing as *mut abi::TimingFeatures).cast(),
            ..Default::default()
        };
        // SAFETY: both extension structs and output feature storage remain live
        // during this query on the adapter's own physical device, and both the
        // instance and the device are Vulkan 1.1 (checked above), which has
        // the core query.
        // UNSAFE(ash's raw Vulkan call; ash 0.38 has no present-timing features)
        unsafe {
            adapter
                .shared_instance()
                .raw_instance()
                .get_physical_device_features2(adapter.raw_physical_device(), &mut features);
        }
        if this.timing.present_timing == 0 || this.ids.present_id2 == 0 {
            return None;
        }
        this.timing.absolute = 0;
        this.timing.relative = 0;
        this.timing.p_next = std::ptr::null_mut();
        Some(this)
    }

    /// Storage must outlive vkCreateDevice, not just its setup callback.
    fn enable(&mut self, args: &mut vulkan::CreateDeviceCallbackArgs<'_, '_, '_>) {
        for extension in [abi::TIMING, abi::ID2, ash::khr::calibrated_timestamps::NAME] {
            if !args.extensions.contains(&extension) {
                args.extensions.push(extension);
            }
        }
        self.ids.p_next = args.create_info.p_next.cast_mut();
        self.timing.p_next = (&mut self.ids as *mut abi::IdFeatures).cast();
        args.create_info.p_next = (&self.timing as *const abi::TimingFeatures).cast();
    }
}

/// The renderer's device on ADAPTER for DESCRIPTOR, with the present-timing
/// extensions enabled when the adapter supports them: wgpu has no safe way to
/// enable a Vulkan device extension it does not know.
pub(crate) fn request_device(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<(wgpu::Device, wgpu::Queue), String> {
    // SAFETY: `hal_adapter` is obtained from this exact wgpu adapter. The
    // callback only adds extensions proven supported by `DeviceFeatures::query`,
    // preserves all wgpu-required features and extensions, and chains feature
    // storage (`timing`) that outlives the open. The resulting HAL device is
    // immediately returned to the same adapter for wgpu-core ownership.
    // UNSAFE(wgpu opens a device with extra extensions only through its HAL)
    unsafe {
        let hal_adapter = adapter
            .as_hal::<Vulkan>()
            .ok_or_else(|| "selected renderer adapter is not Vulkan".to_owned())?;
        let mut timing = DeviceFeatures::query(&hal_adapter);
        let hal_device = hal_adapter
            .open_with_callback(
                descriptor.required_features,
                &descriptor.required_limits,
                &descriptor.memory_hints,
                Some(Box::new(|mut args| {
                    if let Some(timing) = &mut timing {
                        timing.enable(&mut args);
                    }
                })),
            )
            .map_err(|error| format!("failed to open Vulkan interop device: {error:?}"))?;
        adapter
            .create_device_from_hal(hal_device, descriptor)
            .map_err(|error| format!("failed to adopt Vulkan interop device: {error}"))
    }
}

fn enabled(device: &vulkan::Device) -> bool {
    [abi::TIMING, abi::ID2, ash::khr::calibrated_timestamps::NAME]
        .iter()
        .all(|extension| device.enabled_device_extensions().contains(extension))
}

/// Call before each surface configure. Unsupported devices retain ordinary WSI.
///
/// # Safety
/// Device and surface must originate from the same Vulkan instance, the
/// device's physical device must support presenting to the surface, and the
/// device must be the one used for the subsequent configure. A device carrying
/// the timing extensions must also have the `presentTiming` and `presentId2`
/// features enabled, as `request_device` does. Neither may be
/// reconfigured or destroyed concurrently with this call.
// UNSAFE(wgpu's &self configure lets only the caller pin device and surface)
pub unsafe fn prepare_surface(device: &wgpu::Device, surface: &wgpu::Surface<'_>) -> bool {
    if !requested() {
        return false;
    }
    // SAFETY: immutable HAL guards borrow this live device/surface. No handle is
    // retained; native capability queries finish before either guard is dropped.
    // The surface-capabilities query runs on the device's physical device and
    // instance, which the caller promises the surface belongs to and is
    // supported by; its chained output structs live on this frame through
    // the call. Enabling timing
    // needs the device to carry both extensions (`enabled`, with their feature
    // bits on by the caller's contract) and the surface to report the
    // first-pixel stage and present IDs, both checked first.
    // UNSAFE(wgpu's as_hal, and present-timing calls ash 0.38 does not wrap)
    unsafe {
        let Some(device) = device.as_hal::<Vulkan>() else {
            return false;
        };
        let Some(surface) = surface.as_hal::<Vulkan>() else {
            return false;
        };
        let _ = surface.set_present_timing_ext_enabled(false);
        if !enabled(&device) {
            return false;
        }
        let shared = device.shared_instance();
        if !shared
            .extensions()
            .contains(&ash::khr::get_surface_capabilities2::NAME)
        {
            return false;
        }
        let Some(raw_surface) = surface.raw_native_handle() else {
            return false;
        };
        let mut ids = abi::IdCapabilities::default();
        let mut timing = abi::TimingCapabilities {
            p_next: (&mut ids as *mut abi::IdCapabilities).cast(),
            ..Default::default()
        };
        let mut caps = vk::SurfaceCapabilities2KHR {
            p_next: (&mut timing as *mut abi::TimingCapabilities).cast(),
            ..Default::default()
        };
        let info = vk::PhysicalDeviceSurfaceInfo2KHR::default().surface(raw_surface);
        let query = ash::khr::get_surface_capabilities2::Instance::new(
            shared.entry(),
            shared.raw_instance(),
        );
        if query
            .get_physical_device_surface_capabilities2(
                device.raw_physical_device(),
                &info,
                &mut caps,
            )
            .is_err()
            || timing.timing == 0
            || ids.supported == 0
            || timing.stages & abi::FIRST_PIXEL_OUT == 0
        {
            return false;
        }
        surface.set_present_timing_ext_enabled(true)
    }
}
