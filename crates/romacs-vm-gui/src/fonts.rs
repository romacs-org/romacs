//! The VM's window system (romacs_vm::DisplayHost, DIVERGENCES.md
//! VM2-WIN5 and VM2-FONT1): frames on the GUI's display are GNU pgtk's,
//! and the fonts Lisp lists, opens and asks about are the ones painting
//! draws with. A character's font is resolved as layout resolves it: the
//! face's family at its pixel size, with no fontset of its own.
use romacs_font::{FontRequest, FontResolver, entity};
use romacs_vm::{FontEntity, FontQuery, OpenedFont};
use std::collections::HashMap;

/// The fonts Romacs ships (assets/fonts, DIVERGENCES.md VM2-FONT-BUNDLED),
/// in the runtime tree: an installation's share/romacs, or the checkout
/// this binary was built from.
fn bundled_fonts_dir() -> std::path::PathBuf {
    romacs_vm::host::runtime_root().join("assets/fonts")
}

/// Make every font Romacs ships a Fontconfig application font, for this
/// process only: the icon font, so a character only it has resolves to it
/// as any font's fallback does, and Iosevka, the default face's family,
/// which wins a tie with a system copy of the same face. Nothing is
/// installed into a font directory. Run before a resolver is built, so
/// none has cached a codepoint or family as missing. The files Fontconfig
/// refused; none when the directory is absent.
pub fn register_bundled_fonts() -> Vec<std::path::PathBuf> {
    let mut fonts: Vec<_> = std::fs::read_dir(bundled_fonts_dir()).into_iter().flatten()
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| matches!(ext, "ttf" | "otf" | "ttc")))
        .collect();
    fonts.sort();
    fonts.into_iter().filter(|font| !romacs_font::register_font_file(font)).collect()
}

/// The decoded pixels this display keeps for the renderer.
#[path = "residency.rs"]
pub mod residency;

pub struct Display {
    resolver: FontResolver,
    /// Opened faces by file, index and pixel size, and characters' fonts by
    /// family, pixel size and character: `frame-parameters' asks for the
    /// frame's font each time.
    opened: HashMap<(String, u32, i64), Option<OpenedFont>>,
    chars: HashMap<((String, u32, u16, bool, u16), char), Option<(OpenedFont, u32)>>,
    /// Each family's fonts as `family_fonts' listed them, kept as long as
    /// `opened' and `chars': bounded by the families faces name.
    families: HashMap<Option<String>, std::rc::Rc<Vec<FontEntity>>>,
    /// Where `alpha' setters go, with the frame's display id: the window's
    /// opacity state.
    alpha_hook: Option<Box<dyn FnMut(u64, [f32; 2], f32)>>,
    visual_hook: Option<Box<dyn FnMut(&romacs_display_protocol::VisualConfig)>>,
    lose_device_hook: Option<Box<dyn FnMut() -> bool>>,
    /// The window's render thread, which draws native models.
    models: Option<std::sync::Arc<romacs_display_runtime::paint_host::PaintSender>>,
    /// GNU's image loaders: the renderer's semantic decoder, and the pixels
    /// of each image the VM's cache holds, by id, for the renderer
    /// (residency.rs).
    images: std::sync::Arc<romacs_imaging::decode::SemanticImageDecoder>,
    decoded: residency::Shared,
    next_image: u64,
    /// The decode worker: images sized from their headers decode there,
    /// off the VM, and their results wait here until taken.
    worker: Option<Worker>,
    /// Finished decodes not yet taken, by image id.
    finished: HashMap<u64, Result<romacs_imaging::decode::SemanticImageDecoded, String>>,
    /// Images pending on the worker, and those released while pending,
    /// whose results are dropped.
    pending: std::collections::HashSet<u64>,
    released: std::collections::HashSet<u64>,
    /// Wakes the GUI's wait when a decode finishes.
    wake: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    /// Set by the worker after each result it sends: `finished_decodes`
    /// looks at the worker's results only then, not on every call.
    landed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The first 64 KiB of an SVG SOURCE, where its root start tag is.
fn svg_head(source: &romacs_imaging::decode::SemanticImageSource) -> Option<Vec<u8>> {
    use romacs_imaging::decode::SemanticImageSource;
    const HEAD: usize = 64 * 1024;
    match source {
        SemanticImageSource::File(path) => {
            use std::io::Read;
            let mut head = Vec::with_capacity(HEAD);
            std::fs::File::open(path).and_then(|f| f.take(HEAD as u64).read_to_end(&mut head)).ok()?;
            Some(head)
        }
        SemanticImageSource::Data { data, .. } => Some(data.as_slice()[..data.as_slice().len().min(HEAD)].to_vec()),
    }
}

type Decoded = Result<romacs_imaging::decode::SemanticImageDecoded, String>;

/// A decode for the worker: the image id, its request, and its sequence's
/// reservation, held from submission so a flush while it is queued fences
/// it (it cannot repopulate the sequence cache).
type Job = (u64, romacs_imaging::decode::SemanticImageRequest,
            romacs_imaging::decode::SemanticImageSequenceReservation);

/// The decode worker's ends: jobs in, results out.
struct Worker {
    jobs: std::sync::mpsc::Sender<Job>,
    results: std::sync::mpsc::Receiver<(u64, Decoded)>,
}

impl Worker {
    /// A thread decoding with DECODER, setting LANDED and waking WAKE
    /// after each result.
    fn spawn(decoder: std::sync::Arc<romacs_imaging::decode::SemanticImageDecoder>,
             wake: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
             landed: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Option<Worker> {
        let (jobs, inbox) = std::sync::mpsc::channel::<Job>();
        let (outbox, results) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("image-decode".into()).spawn(move || {
            // ROMACS_TEST_IMAGE_HOLD: while that file exists no decode starts
            // (tests observe the pending state).
            let hold = std::env::var_os("ROMACS_TEST_IMAGE_HOLD").map(std::path::PathBuf::from);
            for (id, request, reservation) in inbox {
                while hold.as_ref().is_some_and(|p| p.exists()) { std::thread::sleep(std::time::Duration::from_millis(10)); }
                let result = decoder.decode_pixels(request).map_err(|diagnostic| diagnostic.message());
                drop(reservation);
                if outbox.send((id, result)).is_err() { break; }
                landed.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Some(wake) = &wake { wake(); }
            }
        }).ok()?;
        Some(Worker { jobs, results })
    }
}

impl Display {
    pub fn new() -> Result<Self, String> {
        Ok(Self { resolver: FontResolver::new().map_err(|e| e.to_string())?,
                  opened: HashMap::new(), chars: HashMap::new(), families: HashMap::new(), alpha_hook: None, visual_hook: None, lose_device_hook: None, models: None,
                  images: Default::default(), decoded: Default::default(), next_image: 0, worker: None,
                  finished: HashMap::new(), pending: Default::default(), released: Default::default(), wake: None,
                  landed: std::sync::Arc::default() })
    }
    /// This display, keeping decoded images' pixels in IMAGES.
    pub fn with_images(self, images: residency::Shared) -> Self {
        Self { decoded: images, ..self }
    }
    /// This display, waking the GUI's wait with WAKE when a decode finishes.
    pub fn with_wake(self, wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> Self {
        Self { wake: Some(wake), ..self }
    }
    /// The final facts of DECODED, image ID, its pixels kept for the
    /// renderer: LANDED when a frame may have drawn it pending.
    fn decoded_facts(&mut self, id: u64, decoded: romacs_imaging::decode::SemanticImageDecoded, landed: bool) -> romacs_vm::ImageFacts {
        let metadata = decoded.metadata();
        let (width, height) = metadata.reported.dimensions();
        let facts = romacs_vm::ImageFacts { id, width, height, mask: metadata.mask.has_clipping_mask(),
                                            metadata: metadata.embedded.clone(),
                                            background: (!metadata.background_transparent).then_some(metadata.background),
                                            pending: false };
        if landed { self.decoded.borrow_mut().land(id as u32, decoded) } else { self.decoded.borrow_mut().hold(id as u32, decoded) }
        facts
    }
    /// A finished decode of image ID, taken: its facts, or the loader's error.
    fn take_result(&mut self, id: u64, result: Decoded) -> Result<romacs_vm::ImageFacts, String> {
        self.pending.remove(&id);
        match result {
            Ok(decoded) => Ok(self.decoded_facts(id, decoded, true)),
            Err(message) => { romacs_vm::DisplayHost::release_image(self, id); Err(message) }
        }
    }
    /// The image id, the decode request and its source for QUERY, or the
    /// error GNU's loader gives for bytes plainly of another type.
    fn prepare(&mut self, query: romacs_vm::ImageQuery) -> Result<(u64, romacs_imaging::decode::SemanticImageRequest), String> {
        use romacs_display_protocol::image_diagnostic::{ImageDiagnosticSubject, ImageFormatName, ImageLoadIdentity};
        use romacs_display_protocol::{EncodedBytes, ImageAnimationPolicy, ImageSequenceId};
        use romacs_imaging::decode::{SemanticImageRequest, SemanticImageSource};
        // The renderer's image ids are 32 bits; the VM's take the upper half,
        // apart from those the renderer allocates for itself (its toolbar).
        if self.next_image >= u64::from(u32::MAX >> 1) { return Err("too many images loaded".into()); }
        self.next_image += 1;
        let id = (1 << 31) + self.next_image;
        let (source, subject) = match query.source {
            romacs_vm::ImageSource::File(path) => {
                let path = path.to_string_lossy().into_owned();
                (SemanticImageSource::File(path.clone()), ImageDiagnosticSubject::File(path))
            }
            romacs_vm::ImageSource::Data { bytes, base_uri } => {
                let resources = base_uri.map_or(romacs_imaging::SvgResourceContext::Isolated,
                                                romacs_imaging::SvgResourceContext::BaseUri);
                (SemanticImageSource::Data { data: EncodedBytes::new(bytes), resources },
                 ImageDiagnosticSubject::Spec(format!("(image :type {} :data ...)", query.kind)))
            }
        };
        // GNU's loaders read only their own format: bytes that are plainly
        // another's fail as the declared type's loader reports them.
        let head = match &source {
            SemanticImageSource::File(path) => {
                use std::io::Read;
                let mut head = Vec::with_capacity(512);
                std::fs::File::open(path).and_then(|f| f.take(512).read_to_end(&mut head)).ok().map(|_| head)
            }
            SemanticImageSource::Data { data, .. } => Some(data.as_slice()[..data.as_slice().len().min(512)].to_vec()),
        };
        if let Some(found) = head.as_deref().and_then(romacs_imaging::image_probe::sniff_format) && found != query.kind {
            let diagnostic = romacs_display_protocol::image_diagnostic::ImageDiagnostic::NotAFormat {
                format: ImageFormatName::from_lisp_type(&query.kind), subject: subject.clone() };
            return Err(diagnostic.message());
        }
        Ok((id, SemanticImageRequest {
            source, size: query.size, rotation: query.rotation, realization: query.realization, colors: query.colors,
            mask: query.mask, animation: ImageAnimationPolicy::disabled(), frame: query.frame,
            sequence: ImageSequenceId::new(id).expect("image ids start at 1"), limit: query.limit,
            identity: ImageLoadIdentity::new(ImageFormatName::from_lisp_type(&query.kind), subject),
        }))
    }
    /// This display, sending each `alpha' setter to HOOK.
    pub fn with_alpha_hook(self, hook: impl FnMut(u64, [f32; 2], f32) + 'static) -> Self {
        Self { alpha_hook: Some(Box::new(hook)), ..self }
    }
    /// Call HOOK with each visual configuration Lisp asks for.
    pub fn with_visual_hook(self, hook: impl FnMut(&romacs_display_protocol::VisualConfig) + 'static) -> Self {
        Self { visual_hook: Some(Box::new(hook)), ..self }
    }
    /// This display, drawing native models on SENDER's render thread.
    #[must_use]
    pub fn with_models(self, sender: std::sync::Arc<romacs_display_runtime::paint_host::PaintSender>) -> Self {
        Self { models: Some(sender), ..self }
    }
    /// Call HOOK for `romacs--debug-lose-device`: whether the request went.
    pub fn with_lose_device_hook(self, hook: impl FnMut() -> bool + 'static) -> Self {
        Self { lose_device_hook: Some(Box::new(hook)), ..self }
    }
    fn open_uncached(entity: &FontEntity, pixel_size: i64) -> Option<OpenedFont> {
        let metrics = romacs_font::probe::probe_font_px_metrics(&entity.file, entity.index, u32::try_from(pixel_size).ok()?.max(1), None)?;
        Some(OpenedFont {
            entity: entity.clone(),
            pixel_size: i64::from(metrics.pixel_size),
            ascent: i64::from(metrics.ascent),
            descent: i64::from(metrics.descent),
            max_width: i64::from(metrics.max_width),
            space_width: i64::from(metrics.space_width),
            average_width: i64::from(metrics.average_width),
            otf: romacs_font::probe::otf_capability(&entity.file, entity.index & 0xffff)
                .map_or_else(Default::default, |otf| (otf_table(otf.gsub), otf_table(otf.gpos))),
        })
    }
    fn char_font_uncached(&mut self, face: &romacs_vm::FaceFont, ch: char) -> Option<(OpenedFont, u32)> {
        // The request painting makes (romacs-layout `font_request').
        let slant = if face.italic { romacs_display_protocol::font::FontSlantKind::Italic } else { romacs_display_protocol::font::FontSlantKind::Normal };
        let request = FontRequest { weight: face.weight, slant, width: face.width, ..FontRequest::new(&face.family, face.pixel_size) };
        let resolved = self.resolver.resolve_char(&request, ch).ok()?;
        let file = resolved.font.identity.file_path.clone()?;
        let index = resolved.font.identity.freetype_selector()?;
        let entity = entity::find(&resolved.font.family, &file, index).map(font_entity)?;
        let opened = romacs_vm::DisplayHost::open_font(self, &entity, resolved.font.pixel_size.round() as i64)?;
        Some((opened, resolved.glyph.glyph_id.get()))
    }
}

/// A model command's outcome: a full channel refuses it, so Lisp's request
/// number stays unissued.
fn sent(result: Result<bool, String>) -> Result<(), String> {
    if result? { Ok(()) } else { Err("the renderer is busy; try again".into()) }
}

fn font_entity(entity: entity::Entity) -> FontEntity {
    FontEntity {
        foundry: entity.foundry,
        family: entity.family,
        weight: entity.weight,
        slant: entity.slant,
        width: entity.width,
        spacing: entity.spacing,
        registry: "iso10646-1".to_owned(),
        file: entity.file,
        index: entity.index,
    }
}

fn otf_table(scripts: Vec<romacs_font::probe::OtfScript>) -> romacs_vm::OtfTable {
    scripts.into_iter().map(|script| (script.tag, script.lang_syses.into_iter().map(|lang| (lang.tag, lang.features)).collect())).collect()
}

/// Whether ENTITY has each property QUERY gives (the pattern ftfont.c
/// hands FcFontList). Every face Romacs lists is a Unicode face, so any
/// registry matches.
fn matches(query: &FontQuery, entity: &FontEntity) -> bool {
    query.foundry.as_ref().is_none_or(|foundry| entity.foundry.as_ref().is_some_and(|f| f.eq_ignore_ascii_case(foundry)))
        && query.weight.is_none_or(|weight| weight == entity.weight)
        && query.slant.is_none_or(|slant| slant == entity.slant)
        && query.width.is_none_or(|width| width == entity.width)
        && query.spacing.is_none_or(|spacing| Some(spacing) == entity.spacing)
}

impl romacs_vm::DisplayHost for Display {
    fn window_system(&self) -> &str {
        "pgtk"
    }
    fn display_name(&self) -> String {
        std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".to_owned())
    }
    fn list_fonts(&mut self, query: &FontQuery) -> Vec<FontEntity> {
        entity::list(query.family.as_deref()).into_iter().map(font_entity).filter(|entity| matches(query, entity)).collect()
    }
    fn family_fonts(&mut self, family: Option<&str>) -> std::rc::Rc<Vec<FontEntity>> {
        let key = family.map(str::to_owned);
        if let Some(found) = self.families.get(&key) { return found.clone(); }
        let fonts = std::rc::Rc::new(entity::list(family).into_iter().map(font_entity).collect::<Vec<_>>());
        self.families.insert(key, fonts.clone());
        fonts
    }
    fn font_families(&mut self) -> Vec<String> {
        entity::families()
    }
    fn open_font(&mut self, entity: &FontEntity, pixel_size: i64) -> Option<OpenedFont> {
        self.opened.entry((entity.file.clone(), entity.index, pixel_size))
            .or_insert_with(|| Self::open_uncached(entity, pixel_size))
            .clone()
            .map(|opened| OpenedFont { entity: entity.clone(), ..opened })
    }
    fn char_font(&mut self, face: &romacs_vm::FaceFont, ch: char) -> Option<(OpenedFont, u32)> {
        let key = (face.key(), ch);
        if let Some(found) = self.chars.get(&key) { return found.clone(); }
        let found = self.char_font_uncached(face, ch);
        self.chars.insert(key, found.clone());
        found
    }
    fn glyph_metrics(&mut self, font: &OpenedFont, ch: char) -> Option<romacs_vm::GlyphMetrics> {
        let (glyph, [width, lbearing, rbearing, ascent, descent]) = self.resolver.glyph_extents(&font.entity.file, font.entity.index, font.pixel_size as f32, ch)?;
        Some(romacs_vm::GlyphMetrics { code: glyph.get(), width: width as i64, lbearing: lbearing as i64, rbearing: rbearing as i64, ascent: ascent as i64, descent: descent as i64 })
    }
    fn clear_font_cache(&mut self) {
        self.resolver.clear_fonts();
        self.opened.clear();
        self.chars.clear();
    }
    fn set_fontsets(&mut self, state: std::sync::Arc<romacs_font::FontsetState>) {
        self.resolver.set_fontsets(state);
        self.chars.clear();
    }
    fn opened_def_font(&mut self, font: &romacs_font::OpenedDefFont) -> Option<OpenedFont> {
        let entity = entity::find(&font.family, &font.file, font.selector).map(font_entity)?;
        romacs_vm::DisplayHost::open_font(self, &entity, i64::from(font.pixel_size))
    }
    fn set_frame_alpha(&mut self, frame: u64, alpha: [f32; 2], limit: f32) {
        if let Some(hook) = &mut self.alpha_hook { hook(frame, alpha, limit); }
    }
    fn set_visual_config(&mut self, config: &romacs_display_protocol::VisualConfig) {
        if let Some(hook) = &mut self.visual_hook { hook(config); }
    }
    fn lose_device(&mut self) -> bool {
        self.lose_device_hook.as_mut().is_some_and(|hook| hook())
    }
    fn decode_image(&mut self, query: romacs_vm::ImageQuery) -> Result<romacs_vm::ImageFacts, String> {
        let (id, request) = self.prepare(query)?;
        let decoded = self.images.decode_pixels(request).map_err(|diagnostic| {
            self.release_image(id);
            diagnostic.message()
        })?;
        Ok(self.decoded_facts(id, decoded, false))
    }
    fn begin_decode(&mut self, query: romacs_vm::ImageQuery) -> Result<romacs_vm::ImageFacts, String> {
        use romacs_imaging::decode::SemanticImageSource;
        use romacs_imaging::image_probe::{ImageProbeSource, admit, probe_image_reported, probe_svg_reported};
        // Rasters size from their header, SVGs from their root tag; the
        // rest (XBM, XPM, PBM: small, and sized by Lisp-side data) decode
        // here, as before.
        let svg = query.kind == "svg";
        if !svg && !matches!(query.kind.as_str(), "png" | "jpeg" | "gif" | "webp" | "tiff") { return self.decode_image(query); }
        let (size, rotation, realization, limit) = (query.size, query.rotation, query.realization, query.limit);
        let (id, request) = self.prepare(query)?;
        let probe = |source: &SemanticImageSource| match source {
            SemanticImageSource::File(_) | SemanticImageSource::Data { .. } if svg => (Ok(()), svg_head(source).and_then(|head| {
                probe_svg_reported(&head, size, rotation, realization)
            })),
            SemanticImageSource::File(path) => (admit(ImageProbeSource::File(path), limit),
                                                 probe_image_reported(ImageProbeSource::File(path), size, rotation, realization)),
            SemanticImageSource::Data { data, .. } => (admit(ImageProbeSource::Data(data.as_slice()), limit),
                                                        probe_image_reported(ImageProbeSource::Data(data.as_slice()), size, rotation, realization)),
        };
        let (admitted, reported) = probe(&request.source);
        // Refused at admission, as the decode would refuse it.
        if admitted.is_err() {
            return Err(romacs_display_protocol::image_diagnostic::ImageDiagnostic::InvalidSize.message());
        }
        let Some((width, height)) = reported else {
            let decoded = self.images.decode_pixels(request).map_err(|d| { self.release_image(id); d.message() })?;
            return Ok(self.decoded_facts(id, decoded, false));
        };
        if self.worker.is_none() { self.worker = Worker::spawn(self.images.clone(), self.wake.clone(), self.landed.clone()); }
        let Some(worker) = &self.worker else {
            let decoded = self.images.decode_pixels(request).map_err(|d| { self.release_image(id); d.message() })?;
            return Ok(self.decoded_facts(id, decoded, false));
        };
        let reservation = self.images.reserve_sequence(request.sequence);
        worker.jobs.send((id, request, reservation)).map_err(|e| e.to_string())?;
        self.pending.insert(id);
        Ok(romacs_vm::ImageFacts { id, width, height, mask: false, metadata: romacs_display_protocol::ImageEmbeddedMetadata::EMPTY,
                                   background: None, pending: true })
    }
    fn finish_decode(&mut self, id: u64, quit: &dyn Fn() -> bool) -> Result<romacs_vm::ImageFacts, String> {
        loop {
            if let Some(result) = self.finished.remove(&id) { return self.take_result(id, result); }
            let Some(worker) = &self.worker else { return Err("no decode worker".into()) };
            match worker.results.recv_timeout(std::time::Duration::from_millis(40)) {
                Ok((done, result)) => { self.finished.insert(done, result); }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => if quit() { return Err(romacs_vm::DECODE_QUIT.into()) },
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err("the decode worker stopped".into()),
            }
        }
    }
    fn finished_decodes(&mut self) -> Vec<(u64, Result<romacs_vm::ImageFacts, String>)> {
        // Asked on every pass of the loop and of a read: nothing finished
        // since the last call unless the worker said so, or a wait for
        // another image (`finish_decode') took a result on the way.
        if !self.landed.swap(false, std::sync::atomic::Ordering::SeqCst) && self.finished.is_empty() { return vec![]; }
        romacs_display_protocol::call_count::count("decode.results");
        if let Some(worker) = &self.worker { while let Ok((id, result)) = worker.results.try_recv() { self.finished.insert(id, result); } }
        let ids: Vec<u64> = self.finished.keys().copied().collect();
        let mut out = vec![];
        for id in ids {
            let result = self.finished.remove(&id).expect("listed");
            // Released while it decoded: nobody shows it.
            if self.released.remove(&id) { self.pending.remove(&id); continue; }
            out.push((id, self.take_result(id, result)));
        }
        out
    }
    fn image_header_size(&mut self, path: &str) -> Option<(u32, u32)> {
        romacs_imaging::image_probe::probe_file_pixels(path)
    }
    fn model_available(&self) -> bool {
        self.models.as_ref().is_some_and(|sender| sender.model_renderer_live())
    }
    fn create_model(&mut self, id: u32, model: romacs_vm::ModelCreate) -> Result<(), String> {
        let sender = self.models.as_ref().ok_or(romacs_vm::NO_MODELS)?;
        sent(sender.create_model(id, model.meshes, model.view, model.background, model.size))
    }
    fn update_model(&mut self, id: u32, req: u64, change: romacs_display_protocol::model::ModelChange) -> Result<(), String> {
        let sender = self.models.as_ref().ok_or(romacs_vm::NO_MODELS)?;
        sent(sender.update_model(id, req, change))
    }
    fn free_surface(&mut self, id: u32) {
        // Sent after the next frame, which no longer draws it (residency.rs).
        self.decoded.borrow_mut().free_surface(id);
    }
    fn release_image(&mut self, id: u64) {
        // Still decoding: its result is dropped when it comes.
        if self.pending.contains(&id) { self.released.insert(id); }
        // A multi-frame decode keeps its frames in the decoder's sequence
        // cache under the image's id, animation off or not.
        self.decoded.borrow_mut().release(id as u32);
        if let Some(sequence) = romacs_display_protocol::ImageSequenceId::new(id) {
            self.images.retire(romacs_display_protocol::ImageSequenceRetirement::One(sequence));
        }
    }
}

#[cfg(test)]
mod tests {
    //! The GUI's frame on its display against pinned GNU pgtk's, `emacs -Q
    //! --font "PlemolJP Console NF-14"' (19 pixels) with that one font file
    //! added to the gate's Fontconfig: a face no host lists, which the test
    //! registers, so its answers are the fixture's own.
    use super::*;
    use romacs_vm::Vm;

    const FONT: &str = "-TWR-PlemolJP Console NF-regular-normal-normal-*-19-*-*-*-d-0-iso10646-1";

    fn displayed() -> Vm {
        assert!(romacs_font::register_font_file(romacs_test_fonts::plemol_jp_console_nf_regular()));
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 24, 10).unwrap();
        vm.set_display_host(Some(Box::new(Display::new().unwrap())));
        vm.eval_source("(progn (internal-set-lisp-face-attribute 'default :family \"PlemolJP Console NF\" (selected-frame))
                               (internal-set-lisp-face-attribute 'default :height 140 (selected-frame)))").unwrap();
        vm
    }
    fn printed(vm: &mut Vm, source: &str) -> String {
        vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}")).display().unwrap()
    }

    /// A character in the icons' range resolves to Romacs's icon font
    /// (RomacsIcons.ttf, DIVERGENCES.md VM2-MODE-LINE), for a face of any
    /// family: it is the only font with it.
    #[test]
    fn an_icon_resolves_to_the_icon_font() {
        assert_eq!(register_bundled_fonts(), Vec::<std::path::PathBuf>::new());
        let icon_font = || bundled_fonts_dir().join("RomacsIcons.ttf");
        assert!(icon_font().is_file());
        let mut resolver = FontResolver::new().unwrap();
        for family in ["Liberation Mono", "PlemolJP Console NF"] {
            let resolved = resolver.resolve_char(&FontRequest::new(family, 15.0), '\u{FE001}').unwrap();
            let file = resolved.font.identity.file_path.clone().unwrap();
            assert_eq!(std::path::Path::new(&file).canonicalize().unwrap(), icon_font().canonicalize().unwrap(), "{family}");
            assert_ne!(resolved.glyph.glyph_id.get(), 0, "{family}: the glyph, not .notdef");
        }
    }

    #[test]
    fn a_frame_on_the_display_is_a_pgtk_frame() {
        let mut vm = displayed();
        // Until the session opens the display: the batch chain's initial
        // terminal.
        assert_eq!(printed(&mut vm, "(list (framep (selected-frame)) (window-system) (frame-initial-p) (frame-parameter nil 'font))"),
                   r#"(t nil t "tty")"#);
        assert!(vm.open_display());
        assert_eq!(printed(&mut vm, "(list (framep (selected-frame)) (frame-live-p (selected-frame)) (window-system)
                                          (terminal-live-p nil) (frame-initial-p) (frame-parameter nil 'window-system)
                                          (frame-parameter nil 'font-backend) (frame-parameter nil 'font))"),
                   format!(r#"(pgtk pgtk pgtk pgtk nil pgtk (ftcrhb) "{FONT}")"#));
    }

    /// `alpha' and `alpha-background' against pinned GNU's X11 frame, whose
    /// setters are pgtk's (frame.c gui_set_alpha, gui_set_alpha_background):
    /// each value is stored, then validated; painting gets the background
    /// opacity the last valid setter applied, and every valid `alpha' setter
    /// reaches the window, as GNU's set_frame_alpha_hook. Without a window
    /// `romacs--debug-lose-device' asks the open display to lose its device:
    /// t when the display took it, nil before the display opens or when it
    /// has no device to lose.
    #[test]
    fn losing_the_device_is_asked_of_the_open_display() {
        let asked = std::rc::Rc::new(std::cell::Cell::new(0));
        let seen = asked.clone();
        let mut vm = displayed();
        vm.set_display_host(Some(Box::new(Display::new().unwrap().with_lose_device_hook(move || {
            seen.set(seen.get() + 1);
            true
        }))));
        assert_eq!((printed(&mut vm, "(romacs--debug-lose-device)"), asked.get()), ("nil".into(), 0));
        assert!(vm.open_display());
        assert_eq!((printed(&mut vm, "(romacs--debug-lose-device)"), asked.get()), ("t".into(), 1));
        vm.set_display_host(Some(Box::new(Display::new().unwrap())));
        assert!(vm.open_display());
        assert_eq!(printed(&mut vm, "(romacs--debug-lose-device)"), "nil");
    }

    /// system they are only stored.
    #[test]
    fn opacity_parameters_validate_as_gnu_and_reach_the_window() {
        let setters = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let seen = setters.clone();
        let mut vm = displayed();
        vm.set_display_host(Some(Box::new(Display::new().unwrap().with_alpha_hook(move |frame, alpha, limit| seen.borrow_mut().push((frame, alpha, limit))))));
        let set = |vm: &mut Vm, alist: &str| printed(vm, &format!(
            "(list (condition-case e (modify-frame-parameters nil '{alist}) (error e))
                   (frame-parameter nil 'alpha-background) (frame-parameter nil 'alpha))"));
        let background = |vm: &mut Vm| vm.frame_snapshot(1200, 800).unwrap().background_alpha;
        assert_eq!(set(&mut vm, "((alpha-background . 101) (alpha . 300))"), "(nil 101 300)");
        assert_eq!((background(&mut vm), setters.borrow().len()), (1.0, 0));
        assert!(vm.open_display());
        assert_eq!(printed(&mut vm, "(list frame-alpha-lower-limit (assq 'alpha (frame-parameters))
                                          (assq 'borders-respect-alpha-background (frame-parameters)))"),
                   "(20 (alpha . 300) (borders-respect-alpha-background))");
        assert_eq!(set(&mut vm, "((alpha-background . 80))"), "(nil 80 300)");
        assert_eq!(background(&mut vm), 0.8);
        assert_eq!(set(&mut vm, "((alpha-background . 101))"), "((args-out-of-range 0 100) 101 300)");
        assert_eq!(set(&mut vm, "((alpha-background . 1.5))"), "((args-out-of-range 0.0 1.0) 1.5 300)");
        assert_eq!(set(&mut vm, "((alpha-background . \"80\"))"), r#"((wrong-type-argument numberp "80") "80" 300)"#);
        assert_eq!(background(&mut vm), 0.8, "a refused value leaves the applied one");
        // Both setters of one call reach the window, the later last.
        assert_eq!(set(&mut vm, "((alpha) (alpha . (50 . 70)))"), "(nil \"80\" nil)");
        assert_eq!(*setters.borrow(), [(1, [0.5, 0.7], 0.2), (1, [-1.0, -1.0], 0.2)]);
        // Stored in reverse order, each before its handler: `alpha' signals
        // and `alpha-background' is never reached.
        assert_eq!(set(&mut vm, "((alpha-background . 40) (alpha . (a . 50)))"),
                   "((wrong-type-argument numberp a) \"80\" (a . 50))");
        assert_eq!(setters.borrow().len(), 2);
        assert_eq!(set(&mut vm, "((alpha-background))"), "(nil nil (a . 50))");
        assert_eq!(background(&mut vm), 1.0);
    }

    #[test]
    fn fonts_open_and_answer_as_gnu_pgtk_s() {
        let mut vm = displayed();
        vm.open_display();
        // The frame's font, its file by base name.
        assert_eq!(printed(&mut vm, "(let ((info (font-info (frame-parameter nil 'font))))
            (aset info 12 (file-name-nondirectory (aref info 12))) (aset info 13 (car (aref info 13))) info)"),
            format!(r#"["{FONT}" "PlemolJP Console NF:pixelsize=19:foundry=TWR:weight=regular:slant=normal:width=normal:spacing=90:scalable=true" 19 24 0 0 0 10 19 5 10 10 "PlemolJPConsoleNF-Regular-v3.1.0.ttf" opentype]"#));
        assert_eq!(printed(&mut vm, "(let ((info (font-info \"PlemolJP Console NF:pixelsize=30\")) (i 12) items)
            (while (> i 2) (setq i (1- i) items (cons (aref info i) items))) items)"),
                   "(30 36 0 0 0 16 29 7 16 16)");
        assert_eq!(printed(&mut vm, "(let ((otf (font-get (car (internal-char-font nil ?a)) :otf))) (list (length (car otf)) (car (car (car otf))) (car (car (cdr otf)))))"),
                   "(6 DFLT DFLT)");
        // λ and a: the frame's font and GNU's glyph codes.
        assert_eq!(printed(&mut vm, "(let ((lambda (internal-char-font nil 955)) (a (internal-char-font nil 97)) (font (frame-parameter nil 'font)))
            (list (list (fontp (car lambda) 'font-object) (equal (font-xlfd-name (car lambda)) font) (cdr lambda))
                  (list (fontp (car a) 'font-object) (equal (font-xlfd-name (car a)) font) (cdr a))))"),
                   "((t t 439) (t t 68))");
        assert_eq!(printed(&mut vm, "(let ((e (find-font (font-spec :family \"PlemolJP Console NF\"))))
            (list (fontp e 'font-entity) (font-get e :family) (font-get e :weight) (font-get e :slant) (font-get e :foundry)
                  (font-get e :spacing) (font-get e :registry) (font-get e :size)
                  (find-font (font-spec :family \"PlemolJP Console NF\" :weight 'bold))))"),
                   "(t PlemolJP Console NF regular normal TWR 90 iso10646-1 0 nil)");
        assert_eq!(printed(&mut vm, "(list (find-font (font-spec :family \"NoSuchFontXYZ\")) (font-info \"NoSuchFontXYZ-12\")
            (and (member \"PlemolJP Console NF\" (font-family-list)) t))"), "(nil nil t)");
        assert_eq!(printed(&mut vm, "(equal (font-xlfd-name (font-at 1 nil \"a\\u03bb\")) (frame-parameter nil 'font))"), "t");
    }

    /// xfaces.c Fface_font: on the display every face names the font
    /// painting draws it with, from the face's family and pixel size, and
    /// that font opens, so `window-font-width' (font.c `font-info' of
    /// `face-font'), by which hermes sizes its tables in `fixed-pitch',
    /// measures a face other than `default' instead of signalling. A face
    /// the display theme does not hold paints, and so names, the default
    /// face's font.
    #[test]
    fn every_face_names_the_font_it_draws() {
        let mut vm = displayed();
        vm.open_display();
        assert_eq!(printed(&mut vm, "(progn (internal-make-lisp-face 'rt-big) (internal-make-lisp-face 'rt-big (selected-frame))
            (internal-set-lisp-face-attribute 'rt-big :height 280 (selected-frame))
            (internal-set-lisp-face-attribute 'rt-big :weight 'bold t)
            (let ((font (face-font 'rt-big)))
              (list font (aref (font-info font) 2) (aref (font-info font) 11)
                    (equal (face-font \"default\" nil ?a) (frame-parameter nil 'font)) (face-font 'rt-big t)
                    (progn (internal-make-lisp-face 'rt-empty) (equal (face-font 'rt-empty) (frame-parameter nil 'font)))
                    (condition-case e (face-font 'rt-big nil 'x) (error e)) (condition-case e (face-font 'rt-nosuch) (error e)))))"),
                   r#"("-TWR-PlemolJP Console NF-regular-normal-normal-*-37-*-*-*-d-0-iso10646-1" 37 20 t (bold) t (wrong-type-argument characterp x) (error "Invalid face" rt-nosuch))"#);
    }

    /// The init's `initial-frame-alist' and `default-frame-alist' reach the
    /// display's frame once, before `window-setup-hook', on both of
    /// startup's paths, so the hook's own setting stays: pinned GNU X11
    /// with this init answers (80 90 50) with and without the screen.
    #[test]
    fn startup_applies_the_inits_frame_alists_once() {
        for screen in ["nil", "t"] {
            let answer = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
                let mut vm = displayed();
                let home = std::env::temp_dir().join(format!("romacs-fonts-alists-{}-{screen}", std::process::id()));
                std::fs::create_dir_all(&home).unwrap();
                vm.eval_source(&format!("(setq process-environment (append (list \"HOME={0}\" \"XDG_CONFIG_HOME={0}/config\") process-environment))",
                                        home.display())).unwrap();
                let started = crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::Loadup, None).unwrap();
                printed(&mut vm, &format!("(progn (setq inhibit-startup-screen {screen})
                    (setf (alist-get 'alpha-background default-frame-alist) 80)
                    (setq initial-frame-alist '((alpha . 90)))
                    (defvar probe-seen nil)
                    (add-hook 'window-setup-hook
                      (lambda ()
                        (setq probe-seen (list (frame-parameter nil 'alpha-background) (frame-parameter nil 'alpha)))
                        (set-frame-parameter nil 'alpha-background 50)))
                    t)"));
                crate::startup::finish_loadup(&mut vm, &started).unwrap();
                let answer = printed(&mut vm, "(append probe-seen (list (frame-parameter nil 'alpha-background)))");
                let background = vm.frame_snapshot(1200, 800).unwrap().background_alpha;
                let _ = std::fs::remove_dir_all(&home);
                (answer, background)
            }).unwrap().join().unwrap();
            assert_eq!(answer, ("(80 90 50)".to_owned(), 0.5), "inhibit-startup-screen {screen}");
        }
    }

    /// `--loadup` on the GUI's display: the session puts the frame on it, a
    /// `pgtk' frame as pinned GNU pgtk's, and a user's init, which turns
    /// prettify-symbols on in Lisp modes, lets startup finish: prettify asks
    /// `char-displayable-on-frame-p' about λ, which needs `fontp', when
    /// *scratch* takes `initial-major-mode', and then `emacs-startup-hook'
    /// runs; and the mode line's `%z%Z' show a buffer's coding, as on a
    /// window-system frame. Pinned GNU pgtk with this init: (pgtk pgtk pgtk
    /// t (lisp-interaction-mode t) (t t t) "UU:"). Then pgtk-win.el's
    /// selection methods keep this session's CLIPBOARD until another
    /// program copies, and frame.el and browse-url dispatch to the pgtk
    /// primitives: GNU pgtk on Wayland reports an undecorated frame at
    /// (0 . 0) and "GdkWaylandDisplay".
    #[test]
    fn loadup_on_the_display_finishes_startup_with_the_prettify_init() {
        let answer = std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
            let mut vm = displayed();
            let home = std::env::temp_dir().join(format!("romacs-fonts-prettify-{}", std::process::id()));
            std::fs::create_dir_all(&home).unwrap();
            vm.eval_source(&format!("(setq process-environment (append (list \"HOME={0}\" \"XDG_CONFIG_HOME={0}/config\") process-environment))",
                                    home.display())).unwrap();
            let started = crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::Loadup, None).unwrap();
            printed(&mut vm, "(progn (add-hook 'emacs-lisp-mode-hook #'prettify-symbols-mode)
                (add-hook 'emacs-startup-hook (lambda () (setq prettify-startup-ran t))) t)");
            crate::startup::finish_loadup(&mut vm, &started).unwrap();
            let answer = printed(&mut vm, "(list window-system initial-window-system (framep (selected-frame)) (bound-and-true-p prettify-startup-ran)
                (with-current-buffer \"*scratch*\" (list major-mode prettify-symbols-mode))
                (list (fontp (char-displayable-p 955)) (char-displayable-on-frame-p 955) (char-displayable-p ?a))
                (with-temp-buffer (setq buffer-file-coding-system 'utf-8-unix) (format-mode-line \"%z%Z\")))");
            // pgtk-win.el's selection methods: this session's CLIPBOARD until
            // another program copies; frame.el's and browse-url's pgtk
            // dispatch on an undecorated Wayland frame.
            let pgtk = printed(&mut vm, "(let ((desktop nil))
                (cl-letf (((symbol-function 'romacs--clipboard-set) (lambda (text) (setq desktop text)))
                          ((symbol-function 'romacs--clipboard-text) (lambda () desktop)))
                  (gui-set-selection 'CLIPBOARD \"A\")
                  (let ((mine (list desktop (gui-get-selection 'CLIPBOARD) (gui-backend-selection-owner-p 'CLIPBOARD))))
                    (setq desktop \"B\")
                    (list mine (gui-get-selection 'CLIPBOARD) (gui-backend-selection-owner-p 'CLIPBOARD)
                          ;; Copied again, then the desktop holds no text.
                          (progn (gui-set-selection 'CLIPBOARD \"C\") (setq desktop nil)
                                 (list (gui-get-selection 'CLIPBOARD) (gui-backend-selection-owner-p 'CLIPBOARD)))
                          (frame-edges nil 'outer-edges) (cdr (assq 'outer-size (frame-geometry)))
                          (mouse-absolute-pixel-position) (let ((process-environment (cons \"WAYLAND_DISPLAY=wayland-0\" process-environment))) (pgtk-backend-display-class))))))");
            let answer = format!("{answer} {pgtk}");
            let _ = std::fs::remove_dir_all(&home);
            answer
        }).unwrap().join().unwrap();
        assert_eq!(answer, r#"(pgtk pgtk pgtk t (lisp-interaction-mode t) (t t t) "UU:") (("A" "A" t) "B" nil (nil nil) (0 0 1200 800) (1200 . 800) (0 . 0) "GdkWaylandDisplay")"#);
    }

    /// A queued decode of a multi-frame image flushed before it starts
    /// leaves nothing in the decoder's sequence cache: its reservation,
    /// taken when it was queued, fences it.
    #[test]
    fn a_flushed_queued_decode_does_not_refill_the_sequence_cache() {
        let root = std::env::temp_dir().join(format!("romacs-hold-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let hold = root.join("hold");
        std::fs::write(&hold, "").unwrap();
        // SAFETY: nextest runs this test alone in its process, before the
        // worker thread that reads it exists.
        unsafe { std::env::set_var("ROMACS_TEST_IMAGE_HOLD", &hold) };
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("harness/fixtures/images");
        let mut vm = displayed();
        let display = Display::new().unwrap();
        let decoder = display.images.clone();
        vm.set_display_host(Some(Box::new(display)));
        assert!(vm.open_display());
        vm.eval_source(&format!(r#"(insert (propertize " " 'display '(image :type gif :file "{}")))"#,
                                fixtures.join("anim.gif").display())).unwrap();
        let shown = vm.frame_snapshot(1200, 800).unwrap().windows[0].snapshot.images[0];
        assert!(shown.width > 0, "laid out from its header while queued");
        printed(&mut vm, "(clear-image-cache)");
        std::fs::remove_file(&hold).unwrap();
        // The worker decodes in order: a later decode waited for means the
        // flushed one ran.
        printed(&mut vm, &format!(r#"(image-size '(image :type png :file "{}") t)"#, fixtures.join("plain.png").display()));
        vm.land_images().unwrap();
        assert_eq!(decoder.cached_size_bytes(), 0);
    }


    /// `romacs-image-header-size' answers what `image-size' does in pixels
    /// for every fixture whose header gives its size, and nil for the rest,
    /// without decoding.
    #[test]
    fn the_header_size_is_the_image_size() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("harness/fixtures/images");
        let mut vm = displayed();
        assert!(vm.open_display());
        let mut rows = vec![];
        let mut names: Vec<_> = std::fs::read_dir(&fixtures).unwrap().map(|e| e.unwrap().path()).collect();
        names.sort();
        for path in names {
            let (file, kind) = (path.display().to_string(), path.extension().unwrap().to_string_lossy().replace("jpg", "jpeg"));
            let probed = printed(&mut vm, &format!(r#"(romacs-image-header-size "{file}")"#));
            let size = printed(&mut vm, &format!(r#"(image-size '(image :type {kind} :file "{file}") t)"#));
            rows.push(format!("{} {probed} {size}", path.file_name().unwrap().to_string_lossy()));
        }
        assert_eq!(rows, [
            "alpha.png (32 . 20) (32 . 20)", "anim.gif (16 . 12) (16 . 12)", "anim.webp (16 . 12) (16 . 12)",
            "corrupt.png nil (30 . 30)", "current.svg (20 . 20) (20 . 20)", "dot.xpm nil (6 . 4)",
            "photo.jpg (48 . 30) (48 . 30)", "plain.png (40 . 24) (40 . 24)", "sized.svg (60 . 30) (60 . 30)",
            "star.svg (16 . 16) (16 . 16)", "star.xbm nil (8 . 4)", "viewbox.svg (120 . 40) (120 . 40)",
        ]);
        assert_eq!(printed(&mut vm, r#"(romacs-image-header-size "/nonexistent.png")"#), "nil");
    }

}
