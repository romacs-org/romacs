//! Vulkan import of packed and native bi-planar DRM DMA-BUF surfaces.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, IntoRawFd};
use std::sync::Arc;

use ash::vk;

use super::frame::DmaBufSurface;
use crate::sampling::GpuFrameRelease;

struct ImportParams<'a> {
    /// Unique DMA-BUF memory objects supplied by the producer.
    fds: Vec<BorrowedFd<'a>>,
    /// Logical video-plane to memory-object mapping.
    plane_object_indices: Vec<usize>,
    strides: Vec<u32>,
    offsets: Vec<u32>,
    fourcc: u32,
    modifier: u64,
}

struct ImportedResources {
    device: ash::Device,
    image: vk::Image,
    memories: Vec<vk::DeviceMemory>,
    queue_family: u32,
}

impl Drop for ImportedResources {
    fn drop(&mut self) {
        // SAFETY: this owner created the image and its memories on `device`
        // and is their last user: wgpu's texture holds it through its drop
        // callback and each release through an Arc, so no queued work still
        // reads them when the last reference goes.
        unsafe {
            self.device.destroy_image(self.image, None);
            for memory in self.memories.drain(..) {
                self.device.free_memory(memory, None);
            }
        }
    }
}

struct ForeignRelease(Arc<ImportedResources>);

impl GpuFrameRelease for ForeignRelease {
    fn record(&self, encoder: &mut wgpu::CommandEncoder) {
        let resources = &self.0;
        // SAFETY: the closure records one barrier into the encoder's own
        // Vulkan command buffer, on the image `resources` keeps alive until
        // the recorded work is retired; it neither ends nor submits the buffer.
        unsafe {
            encoder.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|hal| {
                let hal = hal.expect("DMA-BUF release requires a Vulkan command encoder");
                let barrier = vk::ImageMemoryBarrier {
                    src_access_mask: vk::AccessFlags::SHADER_READ,
                    dst_access_mask: vk::AccessFlags::empty(),
                    old_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    new_layout: vk::ImageLayout::GENERAL,
                    src_queue_family_index: resources.queue_family,
                    dst_queue_family_index: vk::QUEUE_FAMILY_FOREIGN_EXT,
                    image: resources.image,
                    subresource_range: color_subresource(),
                    ..Default::default()
                };
                resources.device.cmd_pipeline_barrier(
                    hal.raw_handle(),
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
            });
        }
    }
}

/// Vulkan object retained for one stable decoder-pool allocation. Sampling
/// objects may be reused, but every new producer lease must still acquire
/// FOREIGN ownership and every retired frame must release it again.
pub(super) struct ImportedDmaBufSurface {
    resources: Arc<ImportedResources>,
}

impl ImportedDmaBufSurface {
    pub(super) fn acquire(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(), String> {
        // SAFETY: the image and its device are this surface's, alive while
        // `self` is, and `device` is the wgpu device they were imported into.
        unsafe {
            submit_foreign_acquire(
                device,
                queue,
                &self.resources.device,
                self.resources.queue_family,
                self.resources.image,
            )
        }
    }

    pub(super) fn release(&self) -> Box<dyn GpuFrameRelease> {
        Box::new(ForeignRelease(Arc::clone(&self.resources)))
    }
}

pub(super) fn import_dmabuf(
    device: &wgpu::Device,
    surface: &DmaBufSurface,
    width: u32,
    height: u32,
) -> Result<(wgpu::Texture, ImportedDmaBufSurface), String> {
    let modifier = surface
        .objects
        .first()
        .ok_or_else(|| "DMA-BUF surface has no memory objects".to_owned())?
        .modifier;
    if surface
        .objects
        .iter()
        .any(|object| object.modifier != modifier)
    {
        return Err("packed DMA-BUF objects disagree on their DRM modifier".to_owned());
    }
    let params = ImportParams {
        fds: surface
            .objects
            .iter()
            .map(|object| object.fd.as_fd())
            .collect(),
        plane_object_indices: surface
            .planes
            .iter()
            .map(|plane| plane.object_index)
            .collect(),
        strides: surface.planes.iter().map(|plane| plane.stride).collect(),
        offsets: surface.planes.iter().map(|plane| plane.offset).collect(),
        fourcc: surface.fourcc,
        modifier,
    };
    if params.fds.is_empty() || params.fds.len() > 4 {
        return Err(format!("invalid DMA-BUF plane count {}", params.fds.len()));
    }
    if params
        .plane_object_indices
        .iter()
        .any(|&index| index >= params.fds.len())
    {
        return Err("DMA-BUF plane refers to a missing memory object".to_owned());
    }
    let (vk_format, wgpu_format) = sampled_format(params.fourcc)
        .ok_or_else(|| format!("DRM format {:#010x} is not sampleable", params.fourcc))?;

    use wgpu::hal::api::Vulkan;
    // SAFETY: every raw handle comes from wgpu's Vulkan device and outlives
    // this block. The create-info chains point at locals that live to the
    // calls reading them. The image is destroyed on every error path before
    // anything else owns it, and its memories by `bind_*` on theirs. Once
    // bound, the image passes to wgpu with a drop callback that keeps the
    // owning `ImportedResources` alive as long as the texture.
    unsafe {
        let hal = device
            .as_hal::<Vulkan>()
            .ok_or_else(|| "DMA-BUF import requires wgpu's Vulkan backend".to_string())?;
        let raw_device = hal.raw_device();
        let physical_device = hal.raw_physical_device();
        let instance = hal.shared_instance().raw_instance();
        if !hal
            .enabled_device_extensions()
            .contains(&ash::ext::queue_family_foreign::NAME)
        {
            return Err("Vulkan adapter lacks VK_EXT_queue_family_foreign".into());
        }
        let required_feature = match wgpu_format {
            wgpu::TextureFormat::NV12 => Some(wgpu::Features::TEXTURE_FORMAT_NV12),
            wgpu::TextureFormat::P010 => Some(wgpu::Features::TEXTURE_FORMAT_P010),
            _ => None,
        };
        if required_feature.is_some_and(|feature| !device.features().contains(feature)) {
            return Err(format!(
                "Vulkan adapter did not enable the wgpu feature for {wgpu_format:?} video textures"
            ));
        }
        let external_memory_fd = ash::khr::external_memory_fd::Device::new(instance, raw_device);
        let modifier_plane_count =
            modifier_plane_count(instance, physical_device, vk_format, params.modifier)
                .ok_or_else(|| {
                    format!(
                        "Vulkan adapter cannot sample DRM modifier {:#x} for {vk_format:?}",
                        params.modifier
                    )
                })?;
        if modifier_plane_count == 0 || modifier_plane_count > 4 {
            return Err(format!(
                "Vulkan reported invalid modifier plane count {modifier_plane_count}"
            ));
        }

        let layouts = plane_layouts(&params, modifier_plane_count)?;
        let mut modifier_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT {
            drm_format_modifier: params.modifier,
            drm_format_modifier_plane_count: modifier_plane_count,
            p_plane_layouts: layouts.as_ptr(),
            ..Default::default()
        };
        let mut external_info = vk::ExternalMemoryImageCreateInfo {
            p_next: (&mut modifier_info as *mut _) as *mut std::ffi::c_void,
            handle_types: vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
            ..Default::default()
        };
        let disjoint = fds_are_disjoint(&params.fds);
        let image_info = vk::ImageCreateInfo {
            p_next: (&mut external_info as *mut _) as *mut std::ffi::c_void,
            flags: if disjoint {
                vk::ImageCreateFlags::DISJOINT
            } else {
                vk::ImageCreateFlags::empty()
            },
            image_type: vk::ImageType::TYPE_2D,
            format: vk_format,
            extent: vk::Extent3D {
                width,
                height,
                depth: 1,
            },
            mip_levels: 1,
            array_layers: 1,
            samples: vk::SampleCountFlags::TYPE_1,
            tiling: vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC,
            sharing_mode: vk::SharingMode::EXCLUSIVE,
            initial_layout: vk::ImageLayout::UNDEFINED,
            ..Default::default()
        };
        let image = raw_device
            .create_image(&image_info, None)
            .map_err(|error| format!("vkCreateImage for DMA-BUF failed: {error:?}"))?;
        let memories = match if disjoint {
            bind_disjoint(
                raw_device,
                &external_memory_fd,
                image,
                &params,
                modifier_plane_count,
            )
        } else {
            bind_shared(raw_device, &external_memory_fd, image, &params)
        } {
            Ok(memories) => memories,
            Err(error) => {
                raw_device.destroy_image(image, None);
                return Err(error);
            }
        };

        let resources = Arc::new(ImportedResources {
            device: raw_device.clone(),
            image,
            memories,
            queue_family: hal.queue_family_index(),
        });
        let hal_desc = wgpu_hal::TextureDescriptor {
            label: Some("Romacs imported video DMA-BUF"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu_format,
            usage: wgpu::TextureUses::RESOURCE,
            memory_flags: wgpu_hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        let imported = ImportedDmaBufSurface {
            resources: Arc::clone(&resources),
        };
        let mut resources = Some(resources);
        let drop_callback: wgpu_hal::DropCallback = Box::new(move || drop(resources.take()));
        let hal_texture = hal.texture_from_raw(
            image,
            &hal_desc,
            Some(drop_callback),
            wgpu_hal::vulkan::TextureMemory::External,
        );
        let texture = device.create_texture_from_hal::<Vulkan>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("Romacs zero-copy video DMA-BUF"),
                size: hal_desc.size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            wgpu::TextureUses::RESOURCE,
        );
        Ok((texture, imported))
    }
}

fn sampled_format(fourcc: u32) -> Option<(vk::Format, wgpu::TextureFormat)> {
    match fourcc {
        0x3432_5241 => Some((
            vk::Format::B8G8R8A8_SRGB,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        )),
        0x3432_4241 => Some((
            vk::Format::R8G8B8A8_SRGB,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        )),
        0x3231_564e => Some((
            vk::Format::G8_B8R8_2PLANE_420_UNORM,
            wgpu::TextureFormat::NV12,
        )),
        0x3031_3050 => Some((
            vk::Format::G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16,
            wgpu::TextureFormat::P010,
        )),
        _ => None,
    }
}

#[cfg(test)]
#[path = "dmabuf/tests/dmabuf_test.rs"]
mod tests;

/// The memory-plane count the device reports for FORMAT with MODIFIER, or
/// None when it cannot sample that combination.
///
/// # Safety
/// PHYSICAL_DEVICE belongs to INSTANCE.
unsafe fn modifier_plane_count(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    format: vk::Format,
    modifier: u64,
) -> Option<u32> {
    let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
    let mut properties = vk::FormatProperties2 {
        p_next: (&mut list as *mut _) as *mut std::ffi::c_void,
        ..Default::default()
    };
    // SAFETY: the chain points at the live local `list`, whose array pointer
    // is null, so Vulkan only writes the count.
    unsafe {
        instance.get_physical_device_format_properties2(physical_device, format, &mut properties)
    };
    let mut entries = vec![
        vk::DrmFormatModifierPropertiesEXT::default();
        list.drm_format_modifier_count as usize
    ];
    list.p_drm_format_modifier_properties = entries.as_mut_ptr();
    let mut properties = vk::FormatProperties2 {
        p_next: (&mut list as *mut _) as *mut std::ffi::c_void,
        ..Default::default()
    };
    // SAFETY: as above; `list` now points at `entries`, which holds the
    // count Vulkan reported.
    unsafe {
        instance.get_physical_device_format_properties2(physical_device, format, &mut properties)
    };
    entries
        .iter()
        .find(|entry| entry.drm_format_modifier == modifier)
        .map(|entry| entry.drm_format_modifier_plane_count)
}

fn plane_layouts(
    params: &ImportParams<'_>,
    count: u32,
) -> Result<Vec<vk::SubresourceLayout>, String> {
    if params.offsets.len() < count as usize || params.strides.len() < count as usize {
        return Err(format!(
            "DRM modifier requires {count} memory-plane layouts, but the decoder supplied {} offsets and {} strides",
            params.offsets.len(),
            params.strides.len()
        ));
    }
    Ok((0..count as usize)
        .map(|plane| vk::SubresourceLayout {
            offset: u64::from(params.offsets[plane]),
            row_pitch: u64::from(params.strides[plane]),
            ..Default::default()
        })
        .collect())
}

/// Whether FDS name more than one DMA-BUF (by inode); true when one
/// cannot be told.
fn fds_are_disjoint(fds: &[BorrowedFd<'_>]) -> bool {
    let mut first_inode = None;
    for fd in fds {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: FD is open for the borrow; fstat fills `stat` when it
        // returns 0, and only then is it read.
        let inode = unsafe {
            if libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) != 0 {
                return true;
            }
            stat.assume_init().st_ino
        };
        match first_inode {
            None => first_inode = Some(inode),
            Some(first) if first != inode => return true,
            Some(_) => {}
        }
    }
    false
}

/// Import the single DMA-BUF behind IMAGE's planes and bind it; the image
/// stays the caller's, and the memory is freed again on failure.
///
/// # Safety
/// DEVICE created IMAGE, which has no memory bound yet, and
/// EXTERNAL_MEMORY_FD was loaded for DEVICE.
unsafe fn bind_shared(
    device: &ash::Device,
    external_memory_fd: &ash::khr::external_memory_fd::Device,
    image: vk::Image,
    params: &ImportParams<'_>,
) -> Result<Vec<vk::DeviceMemory>, String> {
    // SAFETY: the caller's contract.
    let requirements = unsafe { device.get_image_memory_requirements(image) };
    // SAFETY: the caller's contract; the size and types are IMAGE's.
    let memory = unsafe {
        import_memory(
            device,
            image,
            params.fds[0],
            requirements.size,
            requirements.memory_type_bits,
            external_memory_fd,
        )?
    };
    // SAFETY: MEMORY was allocated on DEVICE for IMAGE, which has none bound.
    if let Err(error) = unsafe { device.bind_image_memory(image, memory, 0) } {
        // SAFETY: the binding failed, so nothing uses MEMORY.
        unsafe { device.free_memory(memory, None) };
        return Err(format!("binding DMA-BUF memory failed: {error:?}"));
    }
    Ok(vec![memory])
}

/// Import each plane's DMA-BUF and bind them to IMAGE's memory planes; the
/// image stays the caller's, and the memories are freed again on failure.
///
/// # Safety
/// As [`bind_shared`], for an image created DISJOINT.
unsafe fn bind_disjoint(
    device: &ash::Device,
    external_memory_fd: &ash::khr::external_memory_fd::Device,
    image: vk::Image,
    params: &ImportParams<'_>,
    plane_count: u32,
) -> Result<Vec<vk::DeviceMemory>, String> {
    if params.plane_object_indices.len() < plane_count as usize {
        return Err(format!(
            "disjoint DRM modifier requires {plane_count} memory-plane mappings, but the decoder supplied {}",
            params.plane_object_indices.len()
        ));
    }
    let mut memories = Vec::with_capacity(plane_count as usize);
    for plane in 0..plane_count {
        let aspect = plane_aspect(plane)?;
        let mut plane_info = vk::ImagePlaneMemoryRequirementsInfo::default().plane_aspect(aspect);
        let requirements_info = vk::ImageMemoryRequirementsInfo2 {
            p_next: (&mut plane_info as *mut _) as *mut std::ffi::c_void,
            image,
            ..Default::default()
        };
        let mut requirements = vk::MemoryRequirements2::default();
        // SAFETY: the caller's contract; the chain points at the live local
        // `plane_info`.
        unsafe { device.get_image_memory_requirements2(&requirements_info, &mut requirements) };
        let object_index = params.plane_object_indices[plane as usize];
        let fd = *params.fds.get(object_index).ok_or_else(|| {
            format!("DMA-BUF plane {plane} refers to missing object {object_index}")
        })?;
        // SAFETY: the caller's contract; the size and types are this plane's.
        let memory = match unsafe {
            import_memory(
                device,
                image,
                fd,
                requirements.memory_requirements.size,
                requirements.memory_requirements.memory_type_bits,
                external_memory_fd,
            )
        } {
            Ok(memory) => memory,
            Err(error) => {
                for memory in memories.drain(..) {
                    // SAFETY: not yet bound, so nothing uses it.
                    unsafe { device.free_memory(memory, None) };
                }
                return Err(error);
            }
        };
        memories.push(memory);
    }
    let mut plane_infos: Vec<_> = (0..plane_count)
        .map(|plane| Ok(vk::BindImagePlaneMemoryInfo::default().plane_aspect(plane_aspect(plane)?)))
        .collect::<Result<_, String>>()?;
    let bindings: Vec<_> = memories
        .iter()
        .enumerate()
        .map(|(plane, memory)| vk::BindImageMemoryInfo {
            p_next: (&mut plane_infos[plane] as *mut _) as *mut std::ffi::c_void,
            image,
            memory: *memory,
            memory_offset: 0,
            ..Default::default()
        })
        .collect();
    // SAFETY: each binding's chain points at `plane_infos`, alive across the
    // call, and names a memory allocated on DEVICE for IMAGE.
    if let Err(error) = unsafe { device.bind_image_memory2(&bindings) } {
        for memory in memories.drain(..) {
            // SAFETY: the binding failed, so nothing uses it.
            unsafe { device.free_memory(memory, None) };
        }
        return Err(format!("binding disjoint DMA-BUF planes failed: {error:?}"));
    }
    Ok(memories)
}

/// Allocate dedicated memory for IMAGE that imports a duplicate of FD.
///
/// # Safety
/// DEVICE created IMAGE, and EXTERNAL_MEMORY_FD was loaded for DEVICE.
unsafe fn import_memory(
    device: &ash::Device,
    image: vk::Image,
    fd: BorrowedFd<'_>,
    allocation_size: u64,
    image_memory_types: u32,
    external_memory_fd: &ash::khr::external_memory_fd::Device,
) -> Result<vk::DeviceMemory, String> {
    let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
    // SAFETY: FD is open for the borrow and only inspected.
    unsafe {
        external_memory_fd.get_memory_fd_properties(
            vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
            fd.as_raw_fd(),
            &mut fd_properties,
        )
    }
    .map_err(|error| format!("querying DMA-BUF Vulkan memory types failed: {error:?}"))?;
    let memory_type_index = first_memory_type(image_memory_types & fd_properties.memory_type_bits)?;
    let owned = fd
        .try_clone_to_owned()
        .map_err(|error| format!("failed to duplicate DMA-BUF for Vulkan import: {error}"))?;
    let mut import = vk::ImportMemoryFdInfoKHR {
        handle_type: vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
        fd: owned.as_raw_fd(),
        ..Default::default()
    };
    let mut dedicated = vk::MemoryDedicatedAllocateInfo {
        p_next: (&mut import as *mut _) as *mut std::ffi::c_void,
        image,
        ..Default::default()
    };
    let allocate = vk::MemoryAllocateInfo {
        p_next: (&mut dedicated as *mut _) as *mut std::ffi::c_void,
        allocation_size,
        memory_type_index,
        ..Default::default()
    };
    // SAFETY: the caller's contract; the chain points at locals alive across
    // the call. On success Vulkan owns the duplicate, so it is not closed
    // here; on failure `owned` closes it.
    match unsafe { device.allocate_memory(&allocate, None) } {
        Ok(memory) => {
            let _ = owned.into_raw_fd();
            Ok(memory)
        }
        Err(error) => Err(format!("Vulkan DMA-BUF memory import failed: {error:?}")),
    }
}

/// Submit a barrier that takes IMAGE from the foreign queue family into
/// QUEUE_FAMILY for sampling.
///
/// # Safety
/// IMAGE is a live image of DEVICE, which is WGPU_DEVICE's Vulkan device.
unsafe fn submit_foreign_acquire(
    wgpu_device: &wgpu::Device,
    queue: &wgpu::Queue,
    device: &ash::Device,
    queue_family: u32,
    image: vk::Image,
) -> Result<(), String> {
    let mut encoder = wgpu_device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Romacs DMA-BUF foreign ownership acquire"),
    });
    // SAFETY: the caller's contract; the closure only records a barrier into
    // the encoder's own command buffer, which wgpu finishes and submits.
    unsafe {
        encoder.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|hal| {
            if let Some(hal) = hal {
                let barrier = vk::ImageMemoryBarrier {
                    src_access_mask: vk::AccessFlags::MEMORY_WRITE,
                    dst_access_mask: vk::AccessFlags::SHADER_READ,
                    old_layout: vk::ImageLayout::GENERAL,
                    new_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    src_queue_family_index: vk::QUEUE_FAMILY_FOREIGN_EXT,
                    dst_queue_family_index: queue_family,
                    image,
                    subresource_range: color_subresource(),
                    ..Default::default()
                };
                device.cmd_pipeline_barrier(
                    hal.raw_handle(),
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[barrier],
                );
            }
        });
    }
    queue.submit(std::iter::once(encoder.finish()));
    Ok(())
}

const fn color_subresource() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

fn first_memory_type(bits: u32) -> Result<u32, String> {
    (0..32)
        .find(|index| bits & (1 << index) != 0)
        .ok_or_else(|| "DMA-BUF has no compatible Vulkan memory type".to_string())
}

fn plane_aspect(plane: u32) -> Result<vk::ImageAspectFlags, String> {
    match plane {
        0 => Ok(vk::ImageAspectFlags::MEMORY_PLANE_0_EXT),
        1 => Ok(vk::ImageAspectFlags::MEMORY_PLANE_1_EXT),
        2 => Ok(vk::ImageAspectFlags::MEMORY_PLANE_2_EXT),
        3 => Ok(vk::ImageAspectFlags::MEMORY_PLANE_3_EXT),
        _ => Err(format!("unsupported DMA-BUF memory plane {plane}")),
    }
}
