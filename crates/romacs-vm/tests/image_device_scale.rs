//! A HiDPI output's device scale: an image shown in a window is decoded
//! again at the new scale when the scale changes, even with the frame's
//! logical size unchanged, and keeps its logical size.
use romacs_vm::Vm;
use std::cell::RefCell;
use std::rc::Rc;

/// A window system that decodes every image as 40x24 logical pixels and
/// records the device scale each decode asks for.
struct ScaleDisplay(Rc<RefCell<Vec<f32>>>);

impl romacs_vm::DisplayHost for ScaleDisplay {
    fn window_system(&self) -> &str { "pgtk" }
    fn display_name(&self) -> String { String::new() }
    fn list_fonts(&mut self, _: &romacs_vm::FontQuery) -> Vec<romacs_vm::FontEntity> { vec![] }
    fn font_families(&mut self) -> Vec<String> { vec![] }
    fn open_font(&mut self, _: &romacs_vm::FontEntity, _: i64) -> Option<romacs_vm::OpenedFont> { None }
    fn char_font(&mut self, _: &romacs_vm::FaceFont, _: char) -> Option<(romacs_vm::OpenedFont, u32)> { None }
    fn decode_image(&mut self, query: romacs_vm::ImageQuery) -> Result<romacs_vm::ImageFacts, String> {
        let mut decodes = self.0.borrow_mut();
        decodes.push(query.realization.device_scale());
        Ok(romacs_vm::ImageFacts { id: decodes.len() as u64, width: 40, height: 24, mask: false,
                                   metadata: romacs_display_protocol::ImageEmbeddedMetadata::EMPTY, background: None, pending: false })
    }
}

#[test]
fn a_device_scale_change_decodes_shown_images_again() {
    let decodes = Rc::new(RefCell::new(vec![]));
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_display_host(Some(Box::new(ScaleDisplay(decodes.clone()))));
    assert!(vm.open_display());
    vm.eval_source(r#"(with-current-buffer (window-buffer (selected-window))
                        (insert "a" (propertize " " 'display '(image :type svg :data "<svg/>")) "b"))"#).unwrap();
    let image = |vm: &mut Vm| vm.frame_snapshot(800, 600).unwrap().windows[0].snapshot.images[0];
    let before = image(&mut vm);
    assert!(!vm.set_device_scale(1.0), "the scale is 1 already");
    assert!(vm.set_device_scale(2.0));
    let after = image(&mut vm);
    assert_eq!(*decodes.borrow(), [1.0, 2.0]);
    assert_ne!(before.image, after.image, "the image is the one decoded at 2");
    assert_eq!((before.width, before.ascent + before.descent), (after.width, after.ascent + after.descent));
    assert_eq!(after.width, 40);
}
