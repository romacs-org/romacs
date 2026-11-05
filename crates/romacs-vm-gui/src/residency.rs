//! Image residency: the decoded pixels of every image the VM's cache holds
//! (fonts.rs's `decode_image' keeps them by the image's id), and which of
//! them the renderer has.  An image goes to the renderer the first time a
//! frame about to be submitted draws it, ahead of that frame on the same
//! channel; one the VM's cache releases is retired there, which the
//! renderer defers until no presented frame draws it.  One the renderer
//! evicted, or lost with its GPU, goes again the next time a frame draws it.
//! The channel is the frame's too: a publication sends at most
//! `ASSETS_PER_PUBLICATION` loads and retirements, none while the channel
//! is full, and the frame goes again with the rest.
use romacs_display_protocol::{FrameGlyph, FrameGlyphBuffer};
use romacs_display_runtime::paint_host::PaintSender;
use romacs_renderer_wgpu::image_cache::SemanticImageDecoded;
use std::collections::HashMap;
use std::sync::Arc;

/// Most loads and retirements one publication sends: the paint channel
/// (64 commands) keeps room for the frame that draws them.
const ASSETS_PER_PUBLICATION: usize = 16;

/// Where images go: the renderer, through the paint channel.  `load' and
/// `retire' answer false, sending nothing, while the channel is full.
pub trait Sink {
    fn load(&self, id: u32, attempt: u64, decoded: Arc<SemanticImageDecoded>) -> Result<bool, String>;
    fn retire(&self, id: u32) -> Result<bool, String>;
    /// The images it lost since the last call, and whether it lost them all.
    fn lost(&self) -> (Vec<u32>, bool);
    /// Whether it lost them all since the last call, without taking that.
    fn reset_pending(&self) -> bool;
}

impl Sink for PaintSender {
    fn load(&self, id: u32, attempt: u64, decoded: Arc<SemanticImageDecoded>) -> Result<bool, String> {
        self.load_image(id, attempt, decoded)
    }
    fn retire(&self, id: u32) -> Result<bool, String> { self.retire_image(id) }
    fn lost(&self) -> (Vec<u32>, bool) { self.take_lost_images() }
    fn reset_pending(&self) -> bool { self.images_reset_pending() }
}

#[derive(Default)]
pub struct Images {
    decoded: HashMap<u32, Arc<SemanticImageDecoded>>,
    /// The images the renderer has, by the attempt that sent each.
    resident: HashMap<u32, u64>,
    /// The last attempt sent for each image the VM still holds.
    attempts: HashMap<u32, u64>,
    released: Vec<u32>,
    /// Pixels decoded off the VM landed since the last publication, or a
    /// publication left loads or retirements unsent: the frame goes again.
    landed: bool,
}

pub type Shared = std::rc::Rc<std::cell::RefCell<Images>>;

impl Images {
    /// Keep ID's pixels for the renderer.
    pub fn hold(&mut self, id: u32, decoded: SemanticImageDecoded) {
        self.decoded.insert(id, Arc::new(decoded));
    }

    /// Keep ID's pixels, decoded after a frame may have drawn it.
    pub fn land(&mut self, id: u32, decoded: SemanticImageDecoded) {
        self.hold(id, decoded);
        self.landed = true;
    }

    /// Drop ID's pixels; the renderer's copy goes at the next publication.
    pub fn release(&mut self, id: u32) {
        self.decoded.remove(&id);
        self.attempts.remove(&id);
        if self.resident.remove(&id).is_some() { self.released.push(id); }
    }

    /// Whether pixels wait to go to the renderer: the loop paints again.
    pub fn behind(&self) -> bool {
        self.landed
    }

    /// Whether the frame on screen must be submitted again though it is
    /// unchanged: pixels it lacked landed, or a GPU rebuild lost images it
    /// had.  (An image the renderer evicts is one no presented frame draws,
    /// so a frame that draws it again is a changed one.)
    pub fn must_repaint(&self, sink: &impl Sink) -> bool {
        self.landed || !self.resident.is_empty() && sink.reset_pending()
    }

    /// Send SINK the images FRAMES draw that it lacks, then retire the
    /// released ones, at most `ASSETS_PER_PUBLICATION` in all; what does
    /// not go waits for the next publication, the frame marked to go again.
    /// A frame drawing an image the renderer lacks draws nothing there.
    pub fn publish<'a>(&mut self, sink: &impl Sink, frames: impl IntoIterator<Item = &'a FrameGlyphBuffer>) -> Result<(), String> {
        self.landed = false;
        let (evicted, reset) = sink.lost();
        if reset { self.resident.clear(); }
        for id in evicted { self.resident.remove(&id); }
        // Without images held, no frame draws one: a session without
        // images scans no glyphs.
        let frames = frames.into_iter().take(if self.decoded.is_empty() { 0 } else { usize::MAX });
        let mut sent = 0;
        'frames: for frame in frames {
            for glyph in &frame.glyphs {
                let FrameGlyph::Image { image_id, .. } = glyph else { continue };
                let id = image_id.get();
                if self.resident.contains_key(&id) { continue; }
                let Some(decoded) = self.decoded.get(&id) else { continue };
                let attempt = self.attempts.get(&id).map_or(1, |a| a + 1);
                if sent == ASSETS_PER_PUBLICATION || !sink.load(id, attempt, decoded.clone())? {
                    self.landed = true;
                    break 'frames;
                }
                sent += 1;
                self.attempts.insert(id, attempt);
                self.resident.insert(id, attempt);
            }
        }
        let released = std::mem::take(&mut self.released);
        for (i, &id) in released.iter().enumerate() {
            if sent == ASSETS_PER_PUBLICATION || !sink.retire(id)? {
                self.released.extend_from_slice(&released[i..]);
                self.landed = true;
                break;
            }
            sent += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A renderer that records what it is sent and loses what it is told,
    /// its channel full after `room' commands when that is set.
    #[derive(Default)]
    struct Renderer { sent: RefCell<Vec<String>>, lost: RefCell<(Vec<u32>, bool)>, room: std::cell::Cell<Option<usize>> }

    impl Renderer {
        fn take(&self) -> bool {
            match self.room.get() {
                Some(0) => false,
                Some(n) => { self.room.set(Some(n - 1)); true }
                None => true,
            }
        }
    }

    impl Sink for Renderer {
        fn load(&self, id: u32, attempt: u64, _: Arc<SemanticImageDecoded>) -> Result<bool, String> {
            if !self.take() { return Ok(false) }
            self.sent.borrow_mut().push(format!("load {id}/{attempt}"));
            Ok(true)
        }
        fn retire(&self, id: u32) -> Result<bool, String> {
            if !self.take() { return Ok(false) }
            self.sent.borrow_mut().push(format!("retire {id}"));
            Ok(true)
        }
        fn lost(&self) -> (Vec<u32>, bool) { std::mem::take(&mut *self.lost.borrow_mut()) }
        fn reset_pending(&self) -> bool { self.lost.borrow().1 }
    }

    fn decoded() -> SemanticImageDecoded {
        use romacs_display_protocol::image_diagnostic::{ImageDiagnosticSubject, ImageFormatName, ImageLoadIdentity};
        use romacs_display_protocol::*;
        use romacs_renderer_wgpu::image_cache::{SemanticImageDecoder, SemanticImageRequest, SemanticImageSource};
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="3"/>"#.to_vec();
        SemanticImageDecoder::default().decode_pixels(SemanticImageRequest {
            source: SemanticImageSource::Data { data: EncodedBytes::new(svg), resources: romacs_renderer_wgpu::SvgResourceContext::Isolated },
            size: ImageSizeSpec::new(AxisSize::resolve(None, None), AxisSize::resolve(None, None)), rotation: ImageRotation::None,
            realization: ImageRealization::new(1.0, 1.0, 1.0), colors: ImageColorContext::from_pixels(0, 0xffffff),
            mask: ImageMaskPolicy::Preserve, animation: ImageAnimationPolicy::disabled(), frame: ImageFrameIndex::new(0),
            sequence: ImageSequenceId::new(1).unwrap(), limit: ImageSizeLimit::UNLIMITED,
            identity: ImageLoadIdentity::new(ImageFormatName::from_lisp_type("svg"), ImageDiagnosticSubject::Spec("test".into())),
        }).unwrap()
    }

    fn drawing(ids: &[u32]) -> FrameGlyphBuffer {
        let mut frame = FrameGlyphBuffer::new();
        for &id in ids { frame.add_image(romacs_display_protocol::ImageId::new(id), 0.0, 0.0, 4.0, 3.0); }
        frame
    }

    #[test]
    fn an_image_goes_once_when_drawn_and_retires_when_released() {
        let (mut images, renderer) = (Images::default(), Renderer::default());
        images.hold(1, decoded());
        images.hold(2, decoded());
        images.publish(&renderer, [&drawing(&[])]).unwrap();
        images.publish(&renderer, [&drawing(&[1])]).unwrap();
        images.publish(&renderer, [&drawing(&[1])]).unwrap();
        // Released before it was ever drawn: nothing to retire.
        images.release(2);
        images.release(1);
        images.publish(&renderer, [&drawing(&[1])]).unwrap();
        assert_eq!(*renderer.sent.borrow(), ["load 1/1", "retire 1"]);
    }

    #[test]
    fn an_evicted_image_goes_again_from_the_vms_pixels() {
        let (mut images, renderer) = (Images::default(), Renderer::default());
        images.hold(1, decoded());
        images.hold(2, decoded());
        images.publish(&renderer, [&drawing(&[1, 2])]).unwrap();
        renderer.lost.borrow_mut().0.push(1);
        images.publish(&renderer, [&drawing(&[1, 2])]).unwrap();
        // A GPU rebuild loses both: the unchanged frame on screen is submitted again.
        assert!(!images.must_repaint(&renderer));
        renderer.lost.borrow_mut().1 = true;
        assert!(images.must_repaint(&renderer));
        images.publish(&renderer, [&drawing(&[2]), &drawing(&[1])]).unwrap();
        assert_eq!(*renderer.sent.borrow(), ["load 1/1", "load 2/1", "load 1/2", "load 2/2", "load 1/3"]);
    }

    /// Forty new images in one frame go sixteen at a time, the frame marked
    /// to go again until all are sent; a full channel sends nothing more
    /// and keeps the retirements for later.
    #[test]
    fn a_frame_of_many_images_goes_in_bounded_publications() {
        let (mut images, renderer) = (Images::default(), Renderer::default());
        let ids: Vec<u32> = (1..=40).collect();
        for &id in &ids { images.hold(id, decoded()); }
        let mut publications = 0;
        loop {
            images.publish(&renderer, [&drawing(&ids)]).unwrap();
            publications += 1;
            if !images.behind() { break }
        }
        assert_eq!((publications, renderer.sent.borrow().len()), (3, 40));
        images.release(1);
        images.release(2);
        images.hold(41, decoded());
        // Full: nothing goes, and the frame goes again.
        renderer.room.set(Some(0));
        images.publish(&renderer, [&drawing(&[41])]).unwrap();
        assert!(images.behind());
        // Room for one: the image; the retirements wait, and go next.
        renderer.room.set(Some(1));
        images.publish(&renderer, [&drawing(&[41])]).unwrap();
        assert!(images.behind());
        renderer.room.set(None);
        images.publish(&renderer, [&drawing(&[41])]).unwrap();
        assert!(!images.behind());
        assert_eq!(renderer.sent.borrow()[40..], ["load 41/1", "retire 1", "retire 2"]);
        // Forty retirements go sixteen at a time too.
        for id in 3..=41 { images.release(id); }
        let mut publications = 0;
        loop {
            images.publish(&renderer, [&drawing(&[])]).unwrap();
            publications += 1;
            if !images.behind() { break }
        }
        assert_eq!((publications, renderer.sent.borrow().len()), (3, 43 + 39));
    }

}
