// The native adapter: GStreamer decoding with DMA-BUF or upload import into
// Vulkan. Codec, decoder, format, device and import capabilities are runtime
// decisions inside it.
mod linux;
pub(crate) use linux::LinuxPlatform as CurrentPlatform;
