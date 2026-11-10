//! The renderer's options (rlisp/romacs-effects.el) on the real --loadup
//! chain with a window system: every way of setting one reaches the
//! display as a visual configuration, a refused value warns and leaves the
//! display as it was, and each option starts at the renderer's default.
use romacs_vm_gui::{fonts, startup};
use romacs_display_protocol::VisualConfig;
use romacs_vm::Vm;
use std::{cell::RefCell, rc::Rc, time::Duration};

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

/// A --loadup session whose display records each configuration it is sent.
fn session() -> (Vm, Rc<RefCell<Vec<VisualConfig>>>) {
    let sent = Rc::new(RefCell::new(vec![]));
    let seen = sent.clone();
    let mut vm = Vm::new();
    vm.set_frame_geometry(1280, 800, 20, 10).unwrap();
    vm.set_display_host(Some(Box::new(fonts::Display::new().unwrap()
        .with_visual_hook(move |config| seen.borrow_mut().push(config.clone())))));
    startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
    (vm, sent)
}

#[test]
fn options_reach_the_display_however_they_are_set() {
    let (mut vm, sent) = session();
    let last = |sent: &Rc<RefCell<Vec<VisualConfig>>>| sent.borrow().last().cloned().unwrap_or_default();
    // Loading the options sends nothing: their defaults are the renderer's.
    assert_eq!(sent.borrow().len(), 0);
    assert_eq!(printed(&mut vm, "(get-buffer \"*Warnings*\")"), "nil");

    assert_eq!(printed(&mut vm, "(progn (setq romacs-cursor-motion-enabled nil) (plist-get (romacs-effect-get 'cursor-motion) :enabled))"), "nil");
    assert!(!last(&sent).cursor_motion.enabled, "setq turns the glide off");
    printed(&mut vm, "(setopt romacs-cursor-motion-enabled t)");
    assert!(last(&sent).cursor_motion.enabled);

    assert_eq!(printed(&mut vm, "(let ((romacs-cursor-motion-duration 0.3)) (plist-get (romacs-effect-get 'cursor-motion) :duration))"), "0.3");
    assert_eq!(last(&sent).cursor_motion.duration, Duration::from_millis(100), "let's exit restores it");

    let count = sent.borrow().len();
    let refused = printed(&mut vm, "(list (condition-case e (setq romacs-cursor-motion-style 'bogus) (user-error (cadr e)))
      romacs-cursor-motion-style (plist-get (romacs-effect-get 'cursor-motion) :style))");
    assert!(refused.contains("romacs-cursor-motion-style") && refused.contains("unknown variant `bogus`")
            && refused.ends_with(" neovide neovide)"), "{refused}");
    assert_eq!(sent.borrow().len(), count, "a refused value sends nothing");
    // A binding over the variable unwinds to the value the renderer shows.
    printed(&mut vm, "(let ((romacs-cursor-motion-style 'exponential)) t)");
    assert_eq!(last(&sent).cursor_motion.style, romacs_display_protocol::CursorAnimStyle::Neovide);

    // A decorative effect's plist option goes the same way.
    printed(&mut vm, "(setq romacs-effect-cursor-glow '(:enabled t))");
    assert!(last(&sent).effects.cursor_glow.enabled);
    let radius = last(&sent).effects.cursor_glow.radius;
    printed(&mut vm, "(let ((romacs-effect-cursor-glow '(:enabled nil :radius 61))) (setq romacs-test-glow (plist-get (romacs-effect-get 'cursor-glow) :enabled)))");
    assert_eq!(printed(&mut vm, "romacs-test-glow"), "nil");
    let glow = last(&sent).effects.cursor_glow;
    assert!(glow.enabled && glow.radius == radius, "let's exit restores the whole effect: {glow:?}");
}

#[test]
fn every_option_starts_at_the_renderers_default() {
    let (mut vm, _) = session();
    let differing = printed(&mut vm, "(let (differ)
      (dolist (effect romacs-effects--behavioural differ)
        (let ((values (romacs-effect-get effect)))
          (dolist (entry (romacs-effect-schema effect))
            (let ((option (intern (format \"romacs-%s-%s\" effect (substring (symbol-name (car entry)) 1))))
                  (value (plist-get values (car entry))))
              (unless (if (and (numberp value) (numberp (symbol-value option)))
                          (< (abs (- value (symbol-value option))) 1e-6)
                        (equal value (symbol-value option)))
                (push (list option (symbol-value option) value) differ)))))))");
    assert_eq!(differing, "nil");
    assert_eq!(printed(&mut vm, "(and (boundp 'romacs-effect-cursor-glow) (plist-member romacs-effect-cursor-glow :enabled) t)"), "t");
}

/// `romacs-cursor-shape-mode': a bar where text goes in, a block in a
/// read-only or special buffer, set as a buffer's mode or read-only state
/// changes; a cursor something else chose, or the user's own default, is
/// left alone, and turning the mode off gives every buffer its default back.
#[test]
fn the_cursor_is_a_bar_where_text_goes_in_and_a_block_where_it_does_not() {
    let (mut vm, _) = session();
    let report = printed(&mut vm, "(progn (romacs-cursor-shape-mode 1)
      (list (with-current-buffer (get-buffer-create \"edit\") (text-mode) cursor-type)
            (with-current-buffer (get-buffer-create \"view\") (special-mode) cursor-type)
            (with-current-buffer \"edit\" (read-only-mode 1) (prog1 cursor-type (read-only-mode -1)))
            (with-current-buffer \"edit\" cursor-type)
            (with-current-buffer (get-buffer-create \"mine\") (text-mode) (setq-local cursor-type 'hbar)
              (read-only-mode 1) cursor-type)
            (progn (romacs-cursor-shape-mode -1) (with-current-buffer \"edit\" cursor-type))
            (progn (setq-default cursor-type 'hollow) (romacs-cursor-shape-mode 1)
              (with-current-buffer (get-buffer-create \"other\") (text-mode) cursor-type))))");
    assert_eq!(report, "((bar . 2) box box (bar . 2) hbar t hollow)");
}

/// The desktop's reduced motion turns `romacs-motion-reduced' on, unless the
/// user chose it already.
#[test]
fn reduced_motion_follows_the_desktop_unless_the_user_chose() {
    let (mut vm, sent) = session();
    let follow = |vm: &mut Vm, desktop: &str, chosen: &str| printed(vm, &format!("(progn
      (setq romacs-motion-reduced nil) (put 'romacs-motion-reduced 'customized-value {chosen})
      (cl-letf (((symbol-function 'romacs-motion--desktop-reduced-p) (lambda () {desktop})))
        (romacs-motion--follow-desktop))
      romacs-motion-reduced)"));
    assert_eq!(follow(&mut vm, "nil", "nil"), "nil");
    assert_eq!(follow(&mut vm, "t", "nil"), "t");
    assert!(sent.borrow().last().unwrap().motion.reduced, "it reaches the renderer");
    assert_eq!(follow(&mut vm, "t", "'(nil)"), "nil", "the user's own choice stands");
    printed(&mut vm, "(setq romacs-motion-follow-desktop nil)");
    assert_eq!(follow(&mut vm, "t", "nil"), "nil", "an init file's nil stands with following off");
}
