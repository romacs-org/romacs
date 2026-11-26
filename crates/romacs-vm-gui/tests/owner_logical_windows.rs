//! O1b-1a-3 through a real vm-gui owner. See O1b.md §§2.1–2.3 and 4.1.
//!
//! Interface (each form is called in the owner via `owner-call`):
//!   (owner-window--start SLOT)
//!   (owner-window-request :window W :view G :buffer B :revision R
//!     :box (WIDTH . HEIGHT) :fringes (LEFT . RIGHT) :cell (LINE . COLUMN)
//!     :catalog CAT :scale S :selected BOOL :bar COLUMNS-OR-NIL)
//!   (owner-window-delete :window W)
//!   (owner--picture W) => unibyte PanePicture wire string, or nil if absent.
//!
//! Request and delete take keyword arguments. For this slice B is an
//! owner-side buffer name, before stand-ins supply owner buffer ids.
//! Rust supplies the start/display bridge and picture bytes through these
//! Lisp calls. The test consumes them with `PanePicture::decode(bytes, SLOT)`.

use romacs_display_protocol::{
    Color, DisplayWindowId, FaceId, FrameGlyph, FrameGlyphBuffer,
    pane_picture::PanePicture,
    text_animation::{TextAnimationKind, located},
};
use romacs_layout::LayoutEngine;
use romacs_layout_input::FrameSnapshot;
use romacs_vm::{Local, Vm};
use std::{collections::HashMap, path::PathBuf, process::Command};

const SLOT: u8 = 3;
const WINDOW: u64 = 9001;
const OTHER: u64 = 9002;
const BUFFER: &str = "pane text";
const SIZE: (u32, u32) = (400, 120);
const VIEW: u64 = 11;
const CATALOG: u64 = 13;
const FACE: &str = r#"(set-face-attribute 'default nil :family "Iosevka" :height 120)"#;
const CONTENT: &str = r#"
    (setq test-buffer (get-buffer-create "pane text"))
    (with-current-buffer test-buffer
      (setq-local mode-line-format "pane"
                  header-line-format nil
                  cursor-type 'box
                  cursor-in-non-selected-windows t)
      (insert "alpha\nbeta\n")
      (goto-char (point-min)))
    nil
"#;
const CHILD: &str = "ROMACS_OWNER_LOGICAL_WINDOWS_TEST";

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn with_owner(test: impl FnOnce(&mut Owner)) {
    with_pinned_fonts(|| test(&mut Owner::new()));
}

/// Set Fontconfig before either process starts using fonts, even in `all`.
fn with_pinned_fonts(test: impl FnOnce()) {
    if std::env::var_os(CHILD).is_some() {
        test();
        return;
    }
    let output = Command::new("mktemp")
        .args(["-d", "--tmpdir", "romacs-logical-windows.XXXXXXXX"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let scratch = Scratch(PathBuf::from(
        std::str::from_utf8(&output.stdout).unwrap().trim(),
    ));
    let fonts = romacs_test_fonts::bundled_iosevka();
    let config =
        romacs_test_fonts::isolated_fontconfig(&scratch.0, &[fonts[0].parent().unwrap()]).unwrap();
    let current = std::thread::current();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", current.name().unwrap(), "--nocapture"])
        .env(CHILD, "1")
        .env("FONTCONFIG_FILE", config)
        .env("HOME", &scratch.0)
        .env("XDG_CONFIG_HOME", scratch.0.join("config"))
        .env("XDG_CACHE_HOME", scratch.0.join("cache"))
        .env("XDG_DATA_HOME", scratch.0.join("data"))
        .env("XDG_STATE_HOME", scratch.0.join("state"))
        .env("XDG_RUNTIME_DIR", &scratch.0)
        .output()
        .unwrap();
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "child exit {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

struct Owner(Vm);

impl Owner {
    fn new() -> Self {
        // The worker's hello checks the GUI build id, as a GUI master does.
        romacs_vm::isolate_set_build_id(env!("ROMACS_BUILD_ID"));
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let mut owner = Self(vm);
        owner.eval(&format!(
            "(setq isolate-worker-program {:?} isolate-boot 'source)
             (setq test-owner (owner--spawn))",
            env!("CARGO_BIN_EXE_romacs-vm-gui"),
        ));
        assert_eq!(
            owner.run("(list isolate-boot (fboundp 'split-window))"),
            "(source t)",
            "source-booted vm-gui owner",
        );
        owner.run("(setq test-f1 (selected-frame)) nil");
        owner.call(&format!("'owner-window--start {SLOT}"));
        owner.run(&format!(
            "{FACE}
             (defun test-pane-window (id)
               (catch 'window
                 (dolist (frame (frame-list))
                   (when (eql id (frame-parameter frame 'owner-window))
                     (throw 'window (frame-root-window frame))))))
             nil"
        ));
        owner
    }

    fn eval(&mut self, form: &str) -> Local<'_> {
        self.0
            .eval_source(form)
            .unwrap_or_else(|error| panic!("{form}: {error}"))
    }

    fn await_call(&mut self, call: &str) -> Local<'_> {
        self.eval(&format!(
            "(isolate-await (owner-call test-owner {call}) 60)"
        ))
    }

    fn call(&mut self, call: &str) -> String {
        self.await_call(call).display().unwrap()
    }

    fn run(&mut self, body: &str) -> String {
        self.call(&format!("'(lambda () {body})"))
    }

    fn seed(&mut self) {
        self.run(CONTENT);
    }

    fn show(&mut self, id: u64, revision: u64, size: (u32, u32), selected: bool) -> PanePicture {
        self.request(id, BUFFER, revision, size, selected);
        let picture = self.shown(id);
        assert_eq!((picture.width, picture.height), size);
        assert_eq!(picture.head.request, revision);
        picture
    }

    fn shown(&mut self, id: u64) -> PanePicture {
        self.picture(id).expect("requested pane was laid out")
    }

    fn request(&mut self, id: u64, buffer: &str, revision: u64, size: (u32, u32), selected: bool) {
        let (width, height) = size;
        self.call(&format!(
            "'owner-window-request :window {id} :view {VIEW} :buffer {buffer:?} :revision {revision}
             :box '({width} . {height}) :fringes '(0 . 0) :cell '(20 . 10)
             :catalog {CATALOG} :scale 1.0 :selected {} :bar nil",
            if selected { "t" } else { "nil" },
        ));
    }

    fn picture(&mut self, id: u64) -> Option<PanePicture> {
        let value = self.await_call(&format!("'owner--picture {id}"));
        if value.is_nil() {
            return None;
        }
        let picture = PanePicture::decode(value.bytes().expect("picture wire string"), SLOT)
            .expect("valid owner picture for the configured slot");
        assert_eq!(picture.head.window, id);
        assert_eq!(picture.head.view, VIEW);
        assert_eq!(picture.catalog.get(), CATALOG);
        Some(picture)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.eval_source("(isolate-kill-all)");
    }
}

#[test]
fn unheld_buffer_creates_no_window() {
    with_owner(|owner| {
        owner.request(WINDOW, "absent pane buffer", 1, SIZE, true);
        assert!(owner.picture(WINDOW).is_none());
        assert_eq!(owner.run("(length (frame-list))"), "1");
    });
}

/// The master lays out its own ordinary window with the same text and box.
/// Isolate that window in a pane-sized snapshot to compare local coordinates.
fn master_pane(vm: &mut Vm, target: u64) -> FrameSnapshot {
    let mut frame = vm.frame_snapshot(400, 260).unwrap();
    frame.windows.retain(|window| window.snapshot.id == target);
    let [window] = frame.windows.as_mut_slice() else {
        panic!("master window {target} is absent from the snapshot");
    };
    (window.x, window.y) = (0, 0);
    (frame.width, frame.height) = (window.snapshot.width, window.snapshot.height);
    frame.tab_bar = None;
    frame.status_bar = None;
    frame.children.clear();
    frame.chrome.internal_border = 0;
    frame
}

fn normalize_glyphs(glyphs: &mut [FrameGlyph]) {
    let mut faces = HashMap::new();
    for glyph in glyphs {
        if let Some(id) = glyph.window_id_mut() {
            *id = DisplayWindowId::new(1);
        }
        if let Some(slot) = glyph.slot_id_mut() {
            slot.window_id = DisplayWindowId::new(1);
        }
        if let Some(id) = glyph.face_id_mut() {
            let next = FaceId::new(u32::try_from(faces.len() + 1).unwrap());
            *id = *faces.entry(*id).or_insert(next);
        }
    }
}

fn assert_same_paint(mut picture: PanePicture, mut master: FrameGlyphBuffer, selected: bool) {
    assert!(
        picture
            .glyphs
            .iter()
            .any(|glyph| matches!(glyph, FrameGlyph::Char { char: 'a', .. })),
        "the pane painted the fixture text"
    );
    normalize_glyphs(&mut picture.glyphs);
    normalize_glyphs(&mut master.glyphs);
    assert_eq!(picture.glyphs, master.glyphs, "glyphs, selected={selected}");
    let mut cursor = picture.cursor.expect("both states paint a cursor");
    let [expected] = master.window_cursors.as_mut_slice() else {
        panic!("the master painted {} cursors", master.window_cursors.len());
    };
    cursor.window_id = expected.window_id;
    cursor.slot_id.window_id = expected.slot_id.window_id;
    assert_eq!(cursor.active, selected);
    assert_eq!(cursor, *expected);
}

#[test]
fn layout_matches_the_master_with_either_selection() {
    with_pinned_fonts(|| {
        let reference = master_layouts();
        let mut owner = Owner::new();
        owner.seed();
        for ((revision, selected), (size, expected)) in
            [(1, true), (2, false)].into_iter().zip(reference)
        {
            let picture = owner.show(WINDOW, revision, size, selected);
            assert_same_paint(picture, expected, selected);
        }
    });
}

fn master_layouts() -> [((u32, u32), FrameGlyphBuffer); 2] {
    let mut master = Vm::new();
    master.set_frame_geometry(80, 25, 1, 1).unwrap();
    let lisp = romacs_vm_gui::startup::lisp_dir();
    romacs_vm_gui::startup::drive_loadup(&mut master, &lisp).unwrap();
    master.set_frame_geometry(400, 260, 20, 10).unwrap();
    master
        .eval_source(&format!(
            "{FACE} {CONTENT}
         (setq test-master-window (selected-window))
         (set-window-buffer test-master-window test-buffer)
         (set-window-fringes test-master-window 0 0)
         (setq test-master-other (split-window test-master-window 6 'below))"
        ))
        .unwrap();
    let target = master.selected_window_id().unwrap();
    let mut layout = LayoutEngine::new().unwrap();
    [true, false].map(|selected| {
        let snapshot = master_pane(&mut master, target);
        let active = master.selected_window_id().unwrap();
        assert_eq!(active == target, selected);
        let [r, g, b] = snapshot.chrome.cursor.unwrap_or_default();
        let color = Color::from_pixel(u32::from_be_bytes([0, r, g, b]));
        let expected = layout
            .layout_frame(&snapshot, 1, Some((active, color)))
            .unwrap()
            .0;
        assert!(!expected.glyphs.is_empty());
        assert_eq!(
            expected.window_cursors.len(),
            1,
            "master cursor, selected={selected}"
        );
        assert_eq!(expected.window_cursors[0].active, selected);
        master
            .eval_source("(select-window test-master-other)")
            .unwrap();
        ((snapshot.width, snapshot.height), expected)
    })
}

#[test]
fn retained_bytes_keep_their_own_text_revision_and_coverage() {
    with_owner(|owner| {
        owner.seed();
        owner.request(WINDOW, BUFFER, 1, SIZE, true);
        let bytes = owner
            .await_call(&format!("'owner--picture {WINDOW}"))
            .bytes()
            .unwrap()
            .to_vec();
        let before = PanePicture::decode(&bytes, SLOT).unwrap();
        let old = before.submitted.as_ref().expect("coverage before insert");
        assert!(!old.visible_text_bytes.is_empty());
        owner.run(
            r#"(with-current-buffer test-buffer
                        (save-excursion (goto-char (point-min)) (insert "Longer prefix: ")))"#,
        );
        let after = owner.shown(WINDOW);
        let new = after.submitted.expect("coverage after insert");
        assert!(new.text_revision > old.text_revision);
        assert_ne!(new.visible_text_bytes, old.visible_text_bytes);
        assert_ne!(after.glyphs, before.glyphs);
        let retained = PanePicture::decode(&bytes, SLOT).unwrap();
        assert_eq!(&retained.submitted.unwrap(), old);
    });
}

#[test]
fn two_windows_have_independent_widths() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        owner.show(OTHER, 2, (260, 120), false);
        assert_eq!(
            owner.run(&format!(
                "(list (window-body-width (test-pane-window {WINDOW}))
                       (window-body-width (test-pane-window {OTHER})))"
            )),
            "(40 26)"
        );
        assert_eq!(
            owner.run(&format!(
                "(with-current-buffer test-buffer
                   (let ((windows (get-buffer-window-list nil nil t)))
                     (and (= (length windows) 2)
                          (memq (test-pane-window {WINDOW}) windows)
                          (memq (test-pane-window {OTHER}) windows)
                          t)))"
            )),
            "t"
        );
    });
}

/// An element in the existing display input format. A batch owner has no
/// animation clock, so provide the moving element and its still text.
fn working_thread(owner: &mut Owner) -> PanePicture {
    owner.seed();
    owner.run(
        r#"(with-current-buffer test-buffer
                    (goto-char (point-max))
                    (insert "⠿⠿⠿\n")
                    (goto-char (point-min)))"#,
    );
    owner.show(WINDOW, 1, SIZE, true);
    owner.run(&format!(
        "(setq romacs-animate--elements
           (list (list 27 (test-pane-window {WINDOW}) test-buffer
                       12 3 1 20 t '(working running 0 default 2))))
         nil"
    ));
    owner.shown(WINDOW)
}

#[test]
fn every_wire_window_id_is_the_masters() {
    with_owner(|owner| {
        let picture = working_thread(owner);
        let master = DisplayWindowId::new(WINDOW.cast_signed());
        let ids: Vec<_> = picture
            .glyphs
            .iter()
            .filter_map(FrameGlyph::window_id)
            .collect();
        let slots: Vec<_> = picture
            .glyphs
            .iter()
            .filter_map(FrameGlyph::slot_id)
            .collect();
        assert!(
            !ids.is_empty() && ids.iter().all(|&id| id == master),
            "{ids:?}"
        );
        assert!(
            !slots.is_empty() && slots.iter().all(|slot| slot.window_id == master),
            "{slots:?}"
        );
        let cursor = picture.cursor.unwrap();
        assert_eq!(
            (cursor.window_id, cursor.slot_id.window_id),
            (master, master)
        );
        let [(key, animation)] = picture.animations.as_slice() else {
            panic!("expected the working thread, got {:?}", picture.animations);
        };
        assert_eq!((*key, animation.window), ((27 << 32) | WINDOW, WINDOW));
        let submitted = picture.submitted.unwrap();
        assert_eq!(submitted.window_id, WINDOW);
        assert_eq!(submitted.view_generation, VIEW);
    });
}

#[test]
fn a_working_thread_is_located_on_the_pane_glyphs() {
    with_owner(|owner| {
        let picture = working_thread(owner);
        let [(_, working)] = picture.animations.as_slice() else {
            panic!("expected one working thread, got {:?}", picture.animations);
        };
        assert!(matches!(working.kind, TextAnimationKind::Working { .. }));
        assert_eq!(working.anchor, Some(11));
        let found = located(&picture.clone().into_frame(), working)
            .expect("the thread is placed on the pane's braille row");
        assert_eq!(found.top, 2);
    });
}

#[test]
fn a_box_larger_than_f1_is_not_clamped() {
    with_owner(|owner| {
        let width = owner
            .run("(frame-pixel-width test-f1)")
            .parse::<u32>()
            .unwrap()
            + 100;
        let height = owner
            .run("(frame-pixel-height test-f1)")
            .parse::<u32>()
            .unwrap()
            + 100;
        owner.seed();
        let picture = owner.show(WINDOW, 1, (width, height), true);
        assert_eq!((picture.width, picture.height), (width, height));
        assert_eq!(
            owner.run(&format!(
                "(let ((pane (test-pane-window {WINDOW})))
                   (list (window-pixel-width pane) (window-pixel-height pane)))"
            )),
            format!("({width} {height})")
        );
    });
}

#[test]
fn clearing_selected_only_clears_that_logical_window() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        assert_eq!(
            owner.run(&format!(
                "(eq (selected-window) (test-pane-window {WINDOW}))"
            )),
            "t"
        );
        owner.show(WINDOW, 2, SIZE, false);
        assert_eq!(
            owner.run("(eq (selected-window) (frame-root-window test-f1))"),
            "t"
        );
        owner.show(OTHER, 3, SIZE, true);
        assert_eq!(
            owner.run(&format!(
                "(eq (selected-window) (test-pane-window {OTHER}))"
            )),
            "t"
        );
        owner.show(WINDOW, 4, SIZE, false);
        assert_eq!(
            owner.run(&format!(
                "(eq (selected-window) (test-pane-window {OTHER}))"
            )),
            "t"
        );
    });
}

#[test]
fn owner_cursor_blink_leaves_the_picture_unchanged() {
    with_owner(|owner| {
        owner.seed();
        let before = owner.show(WINDOW, 1, SIZE, true);
        assert!(before.cursor.is_some());
        assert_eq!(
            owner.run(&format!(
                "(let ((pane (test-pane-window {WINDOW})))
               (internal-show-cursor pane nil)
               (internal-show-cursor-p pane))"
            )),
            "nil"
        );
        let mut after = owner.shown(WINDOW);
        (after.head.v, after.head.c) = (before.head.v, before.head.c);
        assert_eq!(after, before);
    });
}

#[test]
fn global_hook_not_run() {
    with_owner(|owner| {
        owner.seed();
        owner.run(
            r"
            (romacs--run-window-change-functions)
            (setq test-global 0 test-local 0 test-config nil)
            (add-hook 'window-buffer-change-functions
                      (lambda (_frame) (setq test-global (1+ test-global))))
            (with-current-buffer test-buffer
              (add-hook 'window-buffer-change-functions
                        (lambda (window)
                          (when (eq (current-buffer) (window-buffer window))
                            (setq test-local (1+ test-local)))) nil t)
              (add-hook 'window-configuration-change-hook
                        (lambda ()
                          (push (window-body-width (selected-window) t) test-config)) nil t))
            nil
        ",
        );
        owner.show(WINDOW, 1, SIZE, true);
        assert_eq!(owner.run("(list test-global (> test-local 0))"), "(0 t)");
        owner.run("(setq test-config nil)");
        owner.show(WINDOW, 2, (460, 120), true);
        assert_eq!(
            owner.run("(list test-global (and (memq 460 test-config) t))"),
            "(0 t)"
        );
    });
}

fn assert_signals(owner: &mut Owner, operation: &str) {
    let condition = owner.run(&format!(
        "(condition-case err (progn {operation} 'returned) (error (car err)))"
    ));
    assert!(
        !matches!(
            condition.as_str(),
            "returned" | "void-function" | "void-variable"
        ),
        "{operation}: {condition}"
    );
}

#[test]
fn window_management_refused() {
    with_owner(|owner| {
        owner.seed();
        owner.run(r#"(setq test-replacement (get-buffer-create "replacement")) nil"#);
        let original = owner.show(WINDOW, 1, SIZE, true);
        for operation in [
            "(split-window)",
            "(delete-other-windows)",
            "(switch-to-buffer test-replacement)",
            "(display-buffer test-replacement)",
            "(quit-window)",
        ] {
            assert_eq!(
                owner.run(&format!(
                    "(eq (selected-window) (test-pane-window {WINDOW}))"
                )),
                "t"
            );
            assert_signals(owner, operation);
            let picture = owner.shown(WINDOW);
            assert_eq!((picture.width, picture.height), SIZE, "{operation}");
            assert_eq!(picture.glyphs, original.glyphs, "{operation}");
            assert_eq!(
                owner.run(&format!(
                    "(let* ((pane (test-pane-window {WINDOW})) (frame (window-frame pane)))
                       (list (eq (window-buffer pane) test-buffer)
                             (length (window-list frame 'nomini))
                             (window-pixel-width pane) (window-pixel-height pane)))"
                )),
                "(t 1 400 120)",
                "{operation}"
            );
        }
        owner.run("(select-frame test-f1) nil");
        for operation in [
            "(display-buffer test-buffer)",
            "(pop-to-buffer test-buffer)",
        ] {
            assert_signals(owner, operation);
            assert_eq!(
                owner.run(
                    "(list (length (window-list test-f1 'nomini))
                                 (buffer-name (window-buffer (frame-root-window test-f1))))"
                ),
                r#"(1 " *owner-root*")"#
            );
            assert_eq!(owner.run("(length (frame-list))"), "2");
        }
    });
}

#[test]
fn delete_then_request() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        owner.run(&format!(
            "(setq test-deleted (test-pane-window {WINDOW})) nil"
        ));
        owner.call(&format!("'owner-window-delete :window {WINDOW}"));
        assert_eq!(owner.run("(window-live-p test-deleted)"), "nil");
        owner.show(WINDOW, 2, (360, 120), true);
        assert_eq!(
            owner.run(&format!(
                "(let ((fresh (test-pane-window {WINDOW})))
                   (and (window-live-p fresh)
                        (not (eq fresh test-deleted))
                        (eq (window-buffer fresh) test-buffer)))"
            )),
            "t"
        );
    });
}

#[test]
fn d1_frame_iteration() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        owner.show(OTHER, 2, (260, 120), false);
        let result = owner.run(&format!(
            r"
            (select-frame test-f1)
            (let ((frames (frame-list))
                  (root (frame-root-window test-f1))
                  (left (test-pane-window {WINDOW}))
                  (right (test-pane-window {OTHER}))
                  (seen nil)
                  (f1-buffers nil))
              (walk-windows
                (lambda (window)
                  (push window seen)
                  (when (eq (window-frame window) test-f1)
                    (push (buffer-name (window-buffer window)) f1-buffers)))
                'nomini t)
              (list (= (length frames) 3)
                    (and (memq test-f1 frames)
                         (memq (window-frame left) frames)
                         (memq (window-frame right) frames) t)
                    (= (length seen) 3)
                    (and (memq root seen) (memq left seen) (memq right seen) t)
                    f1-buffers))
        "
        ));
        assert_eq!(result, r#"(t t t t (" *owner-root*"))"#);
    });
}

#[path = "support/owner_window_regressions.rs"]
mod regressions;
