//! The lines a face draws and the face a row paints past its text, as the
//! display receives them: GNU's `:underline' (t, a colour, a plist of
//! `:color' and `:style'), `:overline', `:strike-through' and
//! `:inverse-video' realised per face (xfaces.c realize_gui_face); xdisp.c
//! extend_face_to_end_of_line's end-of-line face (the merge of only the
//! faces there whose `:extend' is non-nil, none for a newline in the
//! default face or a row ending at point-max); the region merged over the
//! text's own face, so what it leaves unspecified shows through.
use romacs_layout_input::{Face, Line, WindowSnapshot};
use romacs_vm::Vm;
#[path = "support/gui_probe.rs"]
mod gui_probe;
use gui_probe::gnu_session;

fn eval(vm: &mut Vm, form: &str) -> String {
    vm.eval_source(form).unwrap_or_else(|e| panic!("{form}: {e}")).display().unwrap()
}

fn window(vm: &mut Vm) -> WindowSnapshot {
    vm.frame_snapshot(1216, 876).unwrap().windows[0].snapshot.clone()
}

/// The face the character at BYTE of the window's text shows in.
fn face_at(snapshot: &WindowSnapshot, byte: usize) -> &Face {
    let run = snapshot.text.face_runs.iter().find(|run| run.bytes.contains(&byte)).expect("a face run");
    &snapshot.faces[run.face.0 as usize]
}

/// The end-of-line face of the row the newline at BYTE ends, if any.
fn extend_at(snapshot: &WindowSnapshot, byte: usize) -> Option<&Face> {
    snapshot.text.extends.iter().find(|e| e.byte as usize == byte).map(|e| &snapshot.faces[e.face.0 as usize])
}

#[test]
fn a_face_s_lines_and_inverse_video_reach_the_display() {
    let mut vm = gnu_session();
    eval(&mut vm, r##"(progn
      (defface deco-wave '((t :underline (:color "red" :style wave))) "")
      (defface deco-over '((t :overline t :strike-through "blue")) "")
      (defface deco-inverse '((t :inverse-video t :foreground "#111111" :background "#eeeeee")) "")
      (erase-buffer)
      (insert (propertize "aa" 'face 'deco-wave) (propertize "bb" 'face 'deco-over)
              (propertize "cc" 'face 'deco-inverse) (propertize "dd" 'face '(:underline t))
              (propertize "ee" 'face '(:underline (:style double-line))) "ff")
      (goto-char (point-min)))"##);
    let w = window(&mut vm);
    assert_eq!(face_at(&w, 0).decor.underline, Some(Line { position: None, style: 3, color: Some([255, 0, 0]) }));
    let over = face_at(&w, 2).decor;
    assert_eq!((over.overline, over.strike, over.underline),
               (Some(Line { position: None, style: 1, color: None }), Some(Line { position: None, style: 1, color: Some([0, 0, 255]) }), None));
    let inverse = face_at(&w, 4);
    assert_eq!((inverse.foreground, inverse.background), ([0xee; 3], [0x11; 3]));
    assert_eq!(face_at(&w, 6).decor.underline, Some(Line { position: None, style: 1, color: None }));
    assert_eq!(face_at(&w, 8).decor.underline, Some(Line { position: None, style: 2, color: None }));
    assert_eq!(face_at(&w, 10).decor, Default::default());
}

#[test]
fn a_row_paints_past_its_text_only_in_the_faces_that_extend() {
    let mut vm = gnu_session();
    eval(&mut vm, r##"(progn
      (defface deco-extend '((t :extend t :background "#203040")) "")
      (defface deco-line '((t :underline t :background "#405060")) "")
      (defface deco-line-extend '((t :extend t :underline t)) "")
      (erase-buffer)
      (insert (propertize "a\n" 'face 'deco-extend)
              (propertize "b\n" 'face '(deco-line deco-extend))
              (propertize "c\n" 'face 'deco-line)
              "d\n"
              (propertize "e\n" 'face 'deco-line-extend)
              "f\n"
              (propertize "g" 'face 'deco-extend))
      (overlay-put (make-overlay 11 13) 'face 'deco-extend)
      (goto-char (point-min)))"##);
    let w = window(&mut vm);
    // An extending face: its background reaches the edge.
    assert_eq!(extend_at(&w, 1).map(|f| (f.background, f.decor.underline)), Some(([0x20, 0x30, 0x40], None)));
    // Under a face that does not extend, only the extending one's
    // attributes do: no underline, the extending background.
    assert_eq!(extend_at(&w, 3).map(|f| (f.background, f.decor.underline)), Some(([0x20, 0x30, 0x40], None)));
    // A face that does not extend paints nothing past the text; nor does
    // the default face.
    assert!(extend_at(&w, 5).is_none() && extend_at(&w, 7).is_none());
    // An extending face's lines extend with it.
    assert_eq!(extend_at(&w, 9).map(|f| f.decor.underline), Some(Some(Line { position: None, style: 1, color: None })));
    // An overlay's extending face, over text in the default face.
    assert_eq!(extend_at(&w, 11).map(|f| f.background), Some([0x20, 0x30, 0x40]));
    // The last row ends at point-max with no newline: nothing to extend.
    assert_eq!(w.text.extends.iter().map(|e| e.byte).collect::<Vec<_>>(), [1, 3, 9, 11]);
}

#[test]
fn the_region_merges_over_the_text_s_face() {
    let mut vm = gnu_session();
    eval(&mut vm, r##"(progn
      (transient-mark-mode 1)
      (set-face-attribute 'region nil :background "#102030" :foreground 'unspecified :extend t)
      (erase-buffer)
      (insert (propertize "xyz\n" 'face '(:underline t :foreground "#00ff00")) "next")
      (goto-char 1) (push-mark 1 t t) (goto-char 5))"##);
    let w = window(&mut vm);
    let selected = face_at(&w, 1);
    // The region's background over the text's own underline and foreground.
    assert_eq!((selected.background, selected.foreground, selected.decor.underline),
               ([0x10, 0x20, 0x30], [0, 255, 0], Some(Line { position: None, style: 1, color: None })));
    // The region's `:extend' carries its background past the newline.
    assert_eq!(extend_at(&w, 3).map(|f| f.background), Some([0x10, 0x20, 0x30]));
}

#[test]
fn the_default_face_s_lines_extend_and_strings_extend_their_own_face() {
    let mut vm = gnu_session();
    eval(&mut vm, r##"(progn
      (defface deco-string '((t :background "#304050")) "")
      (erase-buffer)
      (insert "a\nb\n" "c")
      (let ((o (make-overlay 4 5))) (overlay-put o 'display (propertize "s\n" 'face 'deco-string)))
      (goto-char (point-min)))"##);
    // A string's newline extends the string's own face, unfiltered.
    let w = window(&mut vm);
    let string = w.text.extends.iter().map(|e| (e.byte, w.faces[e.face.0 as usize].background)).collect::<Vec<_>>();
    assert!(string.contains(&(4, [0x30, 0x40, 0x50])), "{string:?}");
    // The default face draws nothing past a row until it has a line; then
    // every row's end, point-max's too, extends it.
    assert!(w.text.extends.iter().all(|e| e.face.0 != 0));
    eval(&mut vm, "(set-face-attribute 'default nil :underline t)");
    let w = window(&mut vm);
    let defaults: Vec<u32> = w.text.extends.iter().filter(|e| e.face.0 == 0).map(|e| e.byte).collect();
    assert_eq!(defaults, [1, w.text.utf8.len() as u32], "{:?}", w.text.extends);
}

#[test]
fn only_a_face_s_own_extend_lets_it_extend() {
    let mut vm = gnu_session();
    eval(&mut vm, r##"(progn
      (defface deco-parent '((t :extend t :background "#203040")) "")
      (set-face-attribute 'default nil :extend t)
      (erase-buffer)
      (insert (propertize "a\n" 'face '(:underline t))
              (propertize "b\n" 'face '(:inherit deco-parent :underline t)))
      (goto-char (point-min)))"##);
    let w = window(&mut vm);
    // An anonymous face that neither says `:extend' nor inherits it does
    // not extend, whatever the default face says.
    assert!(extend_at(&w, 1).is_none(), "{:?}", w.text.extends);
    // One that inherits an extending face does, its own lines included.
    assert_eq!(extend_at(&w, 3).map(|f| (f.background, f.decor.underline.is_some())), Some(([0x20, 0x30, 0x40], true)));
}

#[test]
fn an_overlay_of_higher_priority_outranks_the_region() {
    let mut vm = gnu_session();
    eval(&mut vm, r##"(progn
      (transient-mark-mode 1)
      (set-face-attribute 'region nil :background "#102030" :foreground "#ffffff" :extend t)
      (erase-buffer)
      (insert "xyz abc\n")
      (overlay-put (make-overlay 1 9) 'face '(:foreground "#ff0000"))
      (let ((o (make-overlay 5 8))) (overlay-put o 'face '(:foreground "#00ff00")) (overlay-put o 'priority 1))
      (goto-char 1) (push-mark 1 t t) (goto-char 9))"##);
    let w = window(&mut vm);
    // An overlay of no priority over the region's very text is under the
    // region, whose priority is (nil . 100); one of priority 1 is over it
    // (buffer.c compare_overlays).
    assert_eq!((face_at(&w, 1).foreground, face_at(&w, 1).background), ([255; 3], [0x10, 0x20, 0x30]));
    assert_eq!((face_at(&w, 5).foreground, face_at(&w, 5).background), ([0, 255, 0], [0x10, 0x20, 0x30]));
}

#[test]
fn an_underline_s_position_and_the_buffer_s_rules_reach_the_display() {
    let mut vm = gnu_session();
    eval(&mut vm, r##"(progn
      (erase-buffer)
      (insert (propertize "aa" 'face '(:underline (:position 3))) (propertize "bb" 'face '(:underline (:position t))))
      (setq-local underline-minimum-offset 4 x-underline-at-descent-line t x-use-underline-position-properties nil))"##);
    let w = window(&mut vm);
    assert_eq!((face_at(&w, 0).decor.underline.unwrap().position, face_at(&w, 2).decor.underline.unwrap().position), (Some(3), Some(0)));
    assert_eq!(w.underline_rules, romacs_layout_input::UnderlineRules { minimum_offset: 4, at_descent_line: true, use_position: false });
    // Each rule changed alone, the text untouched, reaches the next snapshot.
    for (form, rules) in [("(setq-local underline-minimum-offset 2)", (2, true, false)),
                          ("(setq-local x-underline-at-descent-line nil)", (2, false, false)),
                          ("(setq-local x-use-underline-position-properties t)", (2, false, true))] {
        eval(&mut vm, form);
        let r = window(&mut vm).underline_rules;
        assert_eq!((r.minimum_offset, r.at_descent_line, r.use_position), rules, "{form}");
    }
}
