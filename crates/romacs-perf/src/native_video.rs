use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

use crate::Frontend;
use serde::{Deserialize, Serialize};

/// Non-zero pixel dimensions of the video presentation exercised by the
/// physical-display benchmark.
///
/// A native-display GUI cannot be sized hermetically by the off-screen GUI
/// adapter.  The benchmark therefore treats the GUI dimensions as its video
/// presentation contract and verifies that the real window can contain it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeVideoPresentationTarget {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl NativeVideoPresentationTarget {
    pub fn from_frontend(frontend: Frontend) -> Result<Self, InvalidPresentationTarget> {
        let Frontend::Gui { width, height } = frontend else {
            return Err(InvalidPresentationTarget::NotGui);
        };
        Ok(Self {
            width: NonZeroU32::new(width).ok_or(InvalidPresentationTarget::ZeroWidth)?,
            height: NonZeroU32::new(height).ok_or(InvalidPresentationTarget::ZeroHeight)?,
        })
    }

    pub const fn width(self) -> u32 {
        self.width.get()
    }

    pub const fn height(self) -> u32 {
        self.height.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidPresentationTarget {
    NotGui,
    ZeroWidth,
    ZeroHeight,
}

impl fmt::Display for InvalidPresentationTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotGui => "sustained native-video performance requires the GUI frontend",
            Self::ZeroWidth => "native-video presentation width must be non-zero",
            Self::ZeroHeight => "native-video presentation height must be non-zero",
        })
    }
}

impl std::error::Error for InvalidPresentationTarget {}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeVideoFrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl NativeVideoFrameRate {
    pub fn is_60_hz_class(self) -> bool {
        matches!(
            (self.numerator, self.denominator),
            (60, 1) | (60_000, 1_001)
        )
    }
}

/// Metadata discovered from the exact compressed input before launching the
/// editor. GStreamer's typed discoverer API avoids parsing a human CLI format.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeVideoMediaMetadata {
    pub width_pixels: u32,
    pub height_pixels: u32,
    pub frame_rate: NativeVideoFrameRate,
    pub codec_caps: String,
}

/// Content and execution inputs that must be identical across every child of
/// one native-video comparison. Editor identity is intentionally excluded:
/// comparing editors is the purpose of the benchmark.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeVideoComparisonIdentity {
    pub workload_fixture_sha256: String,
    pub video_file_sha256: String,
    pub video_file_size_bytes: u64,
    pub media: NativeVideoMediaMetadata,
    pub presentation_width_pixels: u32,
    pub presentation_height_pixels: u32,
    pub display_environment: BTreeMap<String, String>,
    pub gstreamer_environment: BTreeMap<String, String>,
    pub gpu_frame_timing: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeVideoDecoderKind {
    Hardware,
    Software,
    Unknown,
}

impl fmt::Display for NativeVideoDecoderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Hardware => "hardware",
            Self::Software => "software",
            Self::Unknown => "unknown",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeVideoGraphicsBackend {
    Vulkan,
    Metal,
    Dx12,
    Gl,
    BrowserWebgpu,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeVideoFrameFormat {
    Nv12,
    P010,
    Rgba8,
    Bgra8,
}

impl fmt::Display for NativeVideoFrameFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Nv12 => "nv12",
            Self::P010 => "p010",
            Self::Rgba8 => "rgba8",
            Self::Bgra8 => "bgra8",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeVideoGpuTimingStatus {
    Disabled,
    Unsupported,
    Enabled,
}

impl fmt::Display for NativeVideoGraphicsBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Vulkan => "vulkan",
            Self::Metal => "metal",
            Self::Dx12 => "dx12",
            Self::Gl => "gl",
            Self::BrowserWebgpu => "browser-webgpu",
            Self::Other => "other",
        })
    }
}

/// Runtime path identity that must remain fixed across every sample in a
/// comparison. Content identity alone is insufficient: changing decoder,
/// adapter, driver, render node, or display rate changes the experiment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeVideoExecutionIdentity {
    pub decoder_factory: String,
    pub decoder_plugin: String,
    pub decoder_kind: NativeVideoDecoderKind,
    pub gpu_adapter_name: String,
    pub gpu_vendor: u32,
    pub gpu_device: u32,
    pub gpu_device_type: String,
    pub graphics_backend: NativeVideoGraphicsBackend,
    pub gpu_driver: String,
    pub gpu_driver_info: String,
    pub drm_render_node: Option<String>,
    pub display_refresh_hz: Option<u16>,
    pub frame_format: NativeVideoFrameFormat,
    pub gpu_timing_status: NativeVideoGpuTimingStatus,
}

impl NativeVideoMediaMetadata {
    pub fn validate_4k60(&self) -> Result<(), String> {
        if (self.width_pixels, self.height_pixels) != (3840, 2160) {
            return Err(format!(
                "sustained native-video performance requires 3840x2160 input, got {}x{}",
                self.width_pixels, self.height_pixels
            ));
        }
        if !self.frame_rate.is_60_hz_class() {
            return Err(format!(
                "sustained native-video performance requires 60 fps-class input, got {}/{} fps",
                self.frame_rate.numerator, self.frame_rate.denominator
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "native_video/tests/native_video_test.rs"]
mod tests;
