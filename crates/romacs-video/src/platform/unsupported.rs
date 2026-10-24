use crate::backend::{
    DecodedFrame, DecoderBackend, FrameImportOutcome, FrameImporter, Platform, ProductionPlatform,
};
use crate::sampling::GpuVideoContext;
use crate::{GpuVideoFrame, VideoCommand, VideoDecodeBackend, VideoInitError, VideoWake};

pub(crate) struct UnsupportedPlatform;
pub(crate) struct UnsupportedDecoder;
pub(crate) struct UnsupportedFrame;
pub(crate) struct UnsupportedImporter;

impl DecoderBackend for UnsupportedDecoder {
    type Frame = UnsupportedFrame;

    fn command(&mut self, _command: VideoCommand) -> Result<(), crate::VideoCommandError> {
        Err("video is unsupported on this platform".into())
    }

    fn service(
        &mut self,
        _request: &crate::VideoServiceRequest,
    ) -> Vec<crate::backend::BackendEvent<Self::Frame>> {
        Vec::new()
    }
}

impl FrameImporter<UnsupportedFrame> for UnsupportedImporter {
    type Sampled = GpuVideoFrame;

    fn compositor_import(
        &self,
        _frame: &DecodedFrame<UnsupportedFrame>,
    ) -> crate::VideoCompositorImport {
        crate::VideoCompositorImport::CpuUpload
    }

    fn import(
        &mut self,
        _frame: DecodedFrame<UnsupportedFrame>,
    ) -> Result<FrameImportOutcome<Self::Sampled>, String> {
        Err("video is unsupported on this platform".into())
    }
}

impl Platform for UnsupportedPlatform {
    const BACKEND: VideoDecodeBackend = VideoDecodeBackend::Unsupported;
    type Frame = UnsupportedFrame;
    type Sampled = GpuVideoFrame;
    type Decoder = UnsupportedDecoder;
    type Importer = UnsupportedImporter;
}

impl ProductionPlatform for UnsupportedPlatform {
    fn create(
        _gpu: GpuVideoContext,
        _policy: crate::FrameImportPolicy,
        _wake: VideoWake,
    ) -> Result<(Self::Decoder, Self::Importer), VideoInitError> {
        Err(VideoInitError::UnsupportedPlatform)
    }
}
