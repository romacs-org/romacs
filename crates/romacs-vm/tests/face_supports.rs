//! `display-supports-face-attributes-p' as xfaces.c answers it: nil in
//! batch and without a graphic display, else gui_supports_face_attributes_p:
//! every specified attribute differs from the default face's, and one that
//! selects a font selects another installed font than the default face's.
//! defface's `(supports ...)' clauses read it, so completion-preview's
//! `completion-preview-common' takes its underline on a GUI as in GNU.
use romacs_vm::{DisplayHost, FontEntity, FontQuery, OpenedFont, Vm};

/// A display whose fonts are "Mono" in regular, bold and italic and
/// "Plain" in regular only.
struct Fonts;
fn font(family: &str, weight: i64, slant: i64) -> FontEntity {
    FontEntity { foundry: None, family: family.into(), weight, slant, width: 100, spacing: Some(100),
                 registry: "iso10646-1".into(), file: format!("{family}-{weight}-{slant}.ttf"), index: 0 }
}
impl DisplayHost for Fonts {
    fn window_system(&self) -> &str { "pgtk" }
    fn display_name(&self) -> String { String::new() }
    fn list_fonts(&mut self, query: &FontQuery) -> Vec<FontEntity> {
        [font("Mono", 80, 100), font("Mono", 200, 100), font("Mono", 80, 200), font("Plain", 80, 100)]
            .into_iter().filter(|f| query.family.as_ref().is_none_or(|family| *family == f.family)).collect()
    }
    fn font_families(&mut self) -> Vec<String> { vec!["Mono".into(), "Plain".into()] }
    fn open_font(&mut self, _: &FontEntity, _: i64) -> Option<OpenedFont> { None }
    fn char_font(&mut self, _: &romacs_vm::FaceFont, _: char) -> Option<(OpenedFont, u32)> { None }
}

fn graphic_vm(family: &str) -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_display_host(Some(Box::new(Fonts)));
    assert!(vm.open_display());
    vm.eval_source(&format!("(progn (setq noninteractive nil romacs-display '(:graphic t))
        (internal-set-lisp-face-attribute 'default :family \"{family}\" (selected-frame))
        (internal-set-lisp-face-attribute 'default :foreground \"black\" (selected-frame)))")).unwrap();
    vm
}

fn answers(vm: &mut Vm, attributes: &[&str]) -> String {
    let forms: Vec<String> = attributes.iter().map(|a| format!("(display-supports-face-attributes-p {a})")).collect();
    vm.eval_source(&format!("(list {})", forms.join(" "))).unwrap().display().unwrap()
}

#[test]
fn decorations_and_colours_are_supported_where_they_differ_from_the_default() {
    let mut vm = graphic_vm("Mono");
    assert_eq!(answers(&mut vm, &["'(:underline t)", "'(:underline nil)", "'(:box t)", "'(:foreground \"red\")",
                                  "'(:foreground \"black\")", "'(:underline reset)", "nil",
                                  "'(:inherit (:underline t))", "'((:foreground \"black\") (:underline t))",
                                  "'(:underline (:style wave))", "'(:underline (:style double-line))"]),
               "(t nil t t nil nil t t nil t nil)");
}

#[test]
fn font_attributes_are_supported_where_they_select_another_font() {
    let mut vm = graphic_vm("Mono");
    assert_eq!(answers(&mut vm, &["'(:weight bold)", "'(:slant italic)", "'(:weight normal)", "'(:family \"Plain\")",
                                  "'(:weight bold :underline t)", "'(:height 2.0)", "'(:height 1.0)"]),
               "(t t nil t t t nil)");
    // "Plain" has no bold or italic face: the default face's font again.
    let mut vm = graphic_vm("Plain");
    assert_eq!(answers(&mut vm, &["'(:weight bold)", "'(:slant italic)", "'(:underline t)"]), "(nil nil t)");
}

#[test]
fn batch_and_a_terminal_display_support_nothing() {
    let mut vm = graphic_vm("Mono");
    assert_eq!(vm.eval_source("(let ((noninteractive t)) (display-supports-face-attributes-p '(:underline t)))")
                   .unwrap().display().unwrap(), "nil");
    vm.eval_source("(setq romacs-display '(:graphic nil))").unwrap();
    assert_eq!(answers(&mut vm, &["'(:underline t)"]), "(nil)");
}
