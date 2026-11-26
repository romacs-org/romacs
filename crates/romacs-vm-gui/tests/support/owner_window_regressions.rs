use super::{
    BUFFER, CATALOG, CONTENT, FACE, OTHER, Owner, SIZE, VIEW, WINDOW, assert_same_paint,
    with_owner, with_pinned_fonts,
};
use romacs_display_protocol::{
    Color, CursorStyle, FrameGlyph, FrameGlyphBuffer, frame_glyphs::GlyphRowRole,
    pane_picture::PanePicture,
};
use romacs_layout::LayoutEngine;
use romacs_vm::Vm;

fn request_box(
    owner: &mut Owner,
    revision: u64,
    size: (u32, u32),
    fringes: (u32, u32),
    cell: (u32, u32),
) {
    let (width, height) = size;
    let (left, right) = fringes;
    let (line, column) = cell;
    owner.call(&format!(
        "'owner-window-request :window {WINDOW} :view {VIEW} :buffer {BUFFER:?} :revision {revision}
         :box '({width} . {height}) :fringes '({left} . {right}) :cell '({line} . {column})
         :catalog {CATALOG} :scale 1.0 :selected t :bar nil"
    ));
}

fn fringed_master(left_margin: u32) -> ((u32, u32), FrameGlyphBuffer) {
    let mut vm = Vm::new();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    romacs_vm_gui::startup::drive_loadup(&mut vm, &romacs_vm_gui::startup::lisp_dir()).unwrap();
    vm.set_display_host(Some(Box::new(
        romacs_vm_gui::fonts::Display::new().unwrap(),
    )));
    assert!(vm.open_display());
    vm.set_frame_geometry(400, 260, 20, 10).unwrap();
    vm.eval_source(&format!(
        "{FACE} {CONTENT}
         (set-window-buffer nil test-buffer)
         (set-window-fringes nil 8 4)
         (set-window-margins nil {left_margin} 0)"
    ))
    .unwrap();
    let target = vm.selected_window_id().unwrap();
    let mut snapshot = vm.frame_snapshot(400, 260).unwrap();
    snapshot
        .windows
        .retain(|window| window.snapshot.id == target);
    let window = &mut snapshot.windows[0];
    let left = 8 + left_margin * 10;
    assert_eq!(
        (window.x, window.snapshot.width),
        (left, 396 - left),
        "master text insets"
    );
    window.y = 0;
    snapshot.height = window.snapshot.height;
    snapshot.tab_bar = None;
    snapshot.status_bar = None;
    let [r, g, b] = snapshot.chrome.cursor.unwrap_or_default();
    let color = Color::from_pixel(u32::from_be_bytes([0, r, g, b]));
    let laid = LayoutEngine::new()
        .unwrap()
        .layout_frame(&snapshot, 1, Some((target, color)))
        .unwrap()
        .0;
    ((snapshot.width, snapshot.height), laid)
}

#[test]
fn fringes_and_margin_match_master_layout() {
    with_pinned_fonts(|| {
        let (size, expected) = fringed_master(1);
        let mut owner = Owner::new();
        assert_smaller_pane_preserves_f1_size(&mut owner);
        owner.run("(with-current-buffer test-buffer (setq-local mode-line-format \"pane\"))");
        request_box(&mut owner, 2, size, (8, 4), (20, 10));
        owner.shown(WINDOW);
        owner.run(&format!(
            "(set-window-margins (test-pane-window {WINDOW}) 1 0)"
        ));
        let actual = owner.shown(WINDOW);
        assert_eq!((actual.width, actual.height), size);
        assert_same_paint(actual, expected, true);
    });
}

fn assert_smaller_pane_preserves_f1_size(owner: &mut Owner) {
    owner.seed();
    owner.run("(with-current-buffer test-buffer (setq-local mode-line-format nil))");
    let f1_width = "(frame-pixel-width test-f1)";
    let f1_height = "(frame-pixel-height test-f1)";
    let width: u32 = owner.run(f1_width).parse().unwrap();
    let height: u32 = owner.run(f1_height).parse().unwrap();
    let size = (width - 4, height - 4);
    request_box(owner, 1, size, (0, 0), (8, 6));
    let picture = owner.shown(WINDOW);
    assert_eq!((picture.width, picture.height), size);
    assert_eq!(
        owner.run(&format!("(list {f1_width} {f1_height})")),
        format!("({width} {height})")
    );
    owner.call(&format!("'owner-window-delete :window {WINDOW}"));
}

#[test]
fn pending_resize_repairs_bypassed_split() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        owner.request(WINDOW, BUFFER, 2, (460, 140), true);
        assert_eq!(
            owner.run(
                "(let ((ignore-window-parameters t))
               (window-live-p (split-window nil nil 'right)))"
            ),
            "t"
        );
        let picture = owner.shown(WINDOW);
        assert_eq!(
            (picture.head.request, picture.width, picture.height),
            (2, 460, 140)
        );
        assert_eq!(
            owner.run(&format!(
                "(let* ((pane (test-pane-window {WINDOW})) (frame (window-frame pane)))
               (list (eq (window-buffer pane) test-buffer)
                     (length (window-list frame 'nomini))))"
            )),
            "(t 1)"
        );
    });
}

#[test]
fn directly_deleted_selected_pane_is_rebuilt_selected() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        assert_eq!(
            owner.run(
                "(setq test-old-frame (selected-frame))
             (delete-frame test-old-frame t)
             (not (frame-live-p test-old-frame))"
            ),
            "t"
        );
        owner.shown(WINDOW);
        assert_eq!(
            owner.run(&format!(
                "(and (eq (selected-window) (test-pane-window {WINDOW}))
                  (eq (window-buffer (selected-window)) test-buffer))"
            )),
            "t"
        );
    });
}

#[test]
fn a_hook_cannot_replace_the_buffer_in_the_picture() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        owner.run(
            r#"
            (setq test-intruder (get-buffer-create "intruder") test-swapped nil)
            (with-current-buffer test-intruder (insert "zzzz"))
            (with-current-buffer test-buffer
              (add-hook 'window-size-change-functions
                (lambda (window)
                  (unless test-swapped
                    (let ((ignore-window-parameters t))
                      (set-window-dedicated-p window nil)
                      (set-window-buffer window test-intruder)
                      (setq test-swapped t)))) nil t))
            nil
        "#,
        );
        let picture = owner.show(WINDOW, 2, (460, 120), true);
        assert_eq!(owner.run("test-swapped"), "t", "fixture hook ran");
        let shows = |letter| {
            picture.glyphs.iter().any(|glyph| {
                matches!(glyph, FrameGlyph::Char { row_role: GlyphRowRole::Text, char, .. } if *char == letter)
            })
        };
        assert_eq!(
            (shows('a'), shows('z')),
            (true, false),
            "record text, intruder text"
        );
    });
}

fn inject_layout_error(owner: &mut Owner, bad: u64) {
    owner.run(&format!(
        r#"
        (setq test-real-layout (symbol-function 'romacs--owner-pane-lay-out)
              test-failed-layouts nil)
        (fset 'romacs--owner-pane-lay-out
          (lambda (window master &rest args)
            (if (= master {bad})
                (progn (push master test-failed-layouts) (error "test pane layout fault"))
              (apply test-real-layout window master args))))
        nil
    "#
    ));
}

#[test]
fn failed_refresh_neither_returns_a_picture_nor_advances_v() {
    with_owner(|owner| {
        owner.seed();
        let before = owner.show(WINDOW, 1, SIZE, true);
        inject_layout_error(owner, WINDOW);
        let failed = owner.picture(WINDOW);
        assert_eq!(owner.run("test-failed-layouts"), format!("({WINDOW})"));
        owner.run("(fset 'romacs--owner-pane-lay-out test-real-layout) nil");
        let recovered = owner.shown(WINDOW);
        assert_eq!(
            (failed.is_none(), recovered.head.v),
            (true, before.head.v + 1)
        );
    });
}

#[test]
fn another_pane_refreshes_when_one_layout_fails() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        owner.show(OTHER, 2, SIZE, false);
        // Try both orders so a global abort cannot pass by laying the healthy pane first.
        for (bad, healthy, revision) in [(WINDOW, OTHER, 3), (OTHER, WINDOW, 4)] {
            inject_layout_error(owner, bad);
            owner.request(healthy, BUFFER, revision, (460, 120), false);
            let picture = owner.shown(healthy);
            assert_eq!(owner.run("test-failed-layouts"), format!("({bad})"));
            owner.run("(fset 'romacs--owner-pane-lay-out test-real-layout) nil");
            assert_eq!(
                picture.head.request, revision,
                "pane {healthy} returned a stale layout after pane {bad} failed"
            );
        }
    });
}

#[test]
fn resize_hook_cannot_take_a_recursive_picture() {
    with_owner(|owner| {
        owner.seed();
        let before = owner.show(WINDOW, 1, SIZE, true);
        owner.run(&format!(
            "(setq test-recursive 'not-called)
             (with-current-buffer test-buffer
               (add-hook 'window-size-change-functions
                 (lambda (_) (setq test-recursive (if (owner--picture {WINDOW}) 'picture nil)))
                 nil t))
             nil"
        ));
        let after = owner.show(WINDOW, 2, (460, 120), true);
        assert_eq!(
            (owner.run("test-recursive"), after.head.v),
            ("nil".to_owned(), before.head.v + 1)
        );
    });
}

#[test]
fn delete_retry_reaps_a_frame_after_a_hook_signals() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        assert_eq!(
            owner.run(&format!(
                r#"
            (setq test-old-frame (window-frame (test-pane-window {WINDOW}))
                  test-delete-hook-ran nil)
            (add-hook 'delete-frame-functions
              (lambda (frame)
                (when (and (eq frame test-old-frame) (not test-delete-hook-ran))
                  (setq test-delete-hook-ran t)
                  (error "test refuses the first delete"))))
            (condition-case nil (owner-window-delete :window {WINDOW}) (error nil))
            test-delete-hook-ran
        "#
            )),
            "t"
        );
        owner.call(&format!("'owner-window-delete :window {WINDOW}"));
        let old_live = owner.run("(and (frame-live-p test-old-frame) t)");
        owner.show(WINDOW, 2, SIZE, true);
        let count = owner.run(&format!(
            "(let ((n 0)) (dolist (frame (frame-list) n)
               (when (eql (frame-parameter frame 'owner-window) {WINDOW}) (setq n (1+ n)))))"
        ));
        assert_eq!((old_live.as_str(), count.as_str()), ("nil", "1"));
    });
}

#[test]
fn first_refresh_and_f1_selection_run_only_local_hooks() {
    with_owner(|owner| {
        owner.seed();
        owner.run(
            r"
            (setq test-global-hooks nil test-local-hooks nil)
            (dolist (hook '(window-buffer-change-functions window-configuration-change-hook
                            window-selection-change-functions window-state-change-functions
                            window-state-change-hook))
              (add-hook hook (eval `(lambda (&rest _) (push ',hook test-global-hooks)))))
            (with-current-buffer test-buffer
              (dolist (hook '(window-buffer-change-functions window-configuration-change-hook
                              window-selection-change-functions window-state-change-functions))
                (add-hook hook (eval `(lambda (&rest _) (push ',hook test-local-hooks))) nil t)))
        ",
        );
        let mut globals = Vec::new();
        for (revision, selected) in [(1, true), (2, false), (3, true)] {
            owner.show(WINDOW, revision, SIZE, selected);
            globals.push(owner.run("(prog1 test-global-hooks (setq test-global-hooks nil))"));
            if revision == 1 {
                assert_eq!(owner.run("(and (memq 'window-buffer-change-functions test-local-hooks)
                                            (memq 'window-configuration-change-hook test-local-hooks) t)"),
                    "t", "initial buffer and configuration hooks");
            }
            assert_eq!(owner.run("(prog1 (and (memq 'window-selection-change-functions test-local-hooks)
                                              (memq 'window-state-change-functions test-local-hooks) t)
                                      (setq test-local-hooks nil))"), "t", "local selection and state hooks at revision {revision}");
        }
        assert_eq!(
            globals, ["nil"; 3],
            "globals on first refresh, pane to F1, F1 to pane"
        );
    });
}

#[test]
fn renaming_the_held_buffer_preserves_its_pane() {
    with_owner(|owner| {
        owner.seed();
        let before = owner.show(WINDOW, 1, SIZE, true);
        owner.run(r#"(with-current-buffer test-buffer (rename-buffer "renamed pane text"))"#);
        let picture = owner.shown(WINDOW);
        assert_eq!(picture.glyphs, before.glyphs);
        assert_eq!(
            owner.run(&format!(
                "(eq (window-buffer (test-pane-window {WINDOW})) test-buffer)"
            )),
            "t"
        );
    });
}

#[test]
fn cursor_kind_follows_the_request_after_owner_selection_changes() {
    with_owner(|owner| {
        owner.seed();
        owner.show(WINDOW, 1, SIZE, true);
        owner.show(OTHER, 2, SIZE, false);
        assert_eq!(
            owner.run(&format!(
                "(select-window (test-pane-window {OTHER}))
             (eq (selected-window) (test-pane-window {OTHER}))"
            )),
            "t"
        );
        let active = owner.shown(WINDOW).cursor.unwrap();
        let inactive = owner.shown(OTHER).cursor.unwrap();
        assert_eq!(
            (active.style, inactive.style),
            (CursorStyle::FilledBox, CursorStyle::Hollow)
        );
    });
}

#[test]
fn display_buffer_allows_the_caller_to_request_no_window() {
    with_owner(|owner| {
        owner.seed();
        assert_eq!(
            owner.run(
                "(condition-case err (display-buffer test-buffer '(nil (allow-no-window . t)))
               (error (car err)))"
            ),
            "nil"
        );
    });
}

mod round_three {
    use super::{
        BUFFER, FrameGlyph, GlyphRowRole, Owner, PanePicture, SIZE, WINDOW, assert_same_paint,
        fringed_master, request_box, with_owner, with_pinned_fonts,
    };

    fn applied_request(owner: &mut Owner) -> String {
        owner.run(&format!(
            "(owner-window--request (gethash {WINDOW} owner-window--records))"
        ))
    }

    fn picture_facts(picture: &PanePicture) -> (u64, u64, u32, u32) {
        (
            picture.head.request,
            picture.head.v,
            picture.width,
            picture.height,
        )
    }

    #[test]
    fn failed_apply_keeps_the_applied_request_and_waits_for_a_valid_one() {
        with_owner(|owner| {
            owner.seed();
            let before = owner.show(WINDOW, 1, SIZE, true);
            let applied = applied_request(owner);
            request_box(owner, 2, (460, 140), (0, 0), (2_147_483_648, 10));

            let rejected = owner.picture(WINDOW);
            let after_failure = applied_request(owner);
            let without_new_request = owner.picture(WINDOW);
            let recovered = owner.show(WINDOW, 3, (460, 140), true);

            assert_eq!(
                (
                    rejected.as_ref().map(picture_facts),
                    without_new_request.as_ref().map(picture_facts),
                    after_failure,
                    picture_facts(&recovered),
                ),
                (None, None, applied, (3, before.head.v + 1, 460, 140))
            );
        });
    }

    #[test]
    fn direct_frame_resize_is_repaired_to_the_requested_box() {
        with_owner(|owner| {
            owner.seed();
            owner.show(WINDOW, 1, SIZE, true);
            assert_eq!(
                owner.run(&format!(
                    "(let ((window (test-pane-window {WINDOW})))
                       (set-frame-size (window-frame window) 340 100 t)
                       (list (window-pixel-width window) (window-pixel-height window)))"
                )),
                "(340 100)"
            );
            let picture = owner.shown(WINDOW);
            assert_eq!(
                (picture.head.request, (picture.width, picture.height)),
                (1, SIZE)
            );
        });
    }

    #[test]
    fn failed_drop_never_paints_the_old_buffer_under_the_new_request() {
        with_owner(|owner| {
            owner.seed();
            owner.show(WINDOW, 1, SIZE, true);
            owner.run(&format!(
                r#"
                (setq test-drop-frame (window-frame (test-pane-window {WINDOW}))
                      test-drop-refused nil)
                (add-hook 'delete-frame-functions
                  (lambda (frame)
                    (when (and (eq frame test-drop-frame) (not test-drop-refused))
                      (setq test-drop-refused t)
                      (error "test refuses the first drop"))))
                nil
                "#
            ));
            owner.request(WINDOW, "absent pane buffer", 2, SIZE, true);
            let failed_drop = owner.picture(WINDOW);
            assert_eq!(owner.run("test-drop-refused"), "t", "the deletion hook ran");
            let next = owner.picture(WINDOW);
            let facts = |picture: &PanePicture| (picture_facts(picture), text_in(picture));
            assert_eq!(
                (failed_drop.as_ref().map(facts), next.as_ref().map(facts)),
                (None, None)
            );
        });
    }

    #[test]
    fn a_new_fringe_request_supersedes_a_window_override() {
        with_pinned_fonts(|| {
            let (size, expected) = fringed_master(0);
            let mut owner = Owner::new();
            owner.seed();
            request_box(&mut owner, 1, size, (4, 2), (20, 10));
            owner.shown(WINDOW);
            owner.run(&format!(
                "(set-window-fringes (test-pane-window {WINDOW}) 0 0)"
            ));

            request_box(&mut owner, 2, size, (8, 4), (20, 10));
            let picture = owner.shown(WINDOW);
            assert_eq!(
                (picture.head.request, (picture.width, picture.height)),
                (2, size)
            );
            assert_same_paint(picture, expected, true);
        });
    }

    fn text_in(picture: &PanePicture) -> (bool, bool) {
        let shows = |letter| {
            picture.glyphs.iter().any(|glyph| {
                matches!(glyph, FrameGlyph::Char { row_role: GlyphRowRole::Text, char, .. } if *char == letter)
            })
        };
        (shows('a'), shows('z'))
    }

    #[test]
    fn a_buffer_swap_during_snapshot_disqualifies_the_picture() {
        with_owner(|owner| {
            owner.seed();
            owner.show(WINDOW, 1, SIZE, true);
            owner.run(&format!(
                r#"
                (setq test-snapshot-swapped nil test-intruder (get-buffer-create "snapshot intruder"))
                (with-current-buffer test-intruder (insert "zzzz"))
                (with-current-buffer test-buffer
                  (setq-local mode-line-format
                    '(:eval
                      (progn
                        (unless test-snapshot-swapped
                          (let ((window (test-pane-window {WINDOW}))
                                (ignore-window-parameters t))
                            (set-window-dedicated-p window nil)
                            (set-window-buffer window test-intruder)
                            (setq test-snapshot-swapped t)))
                        "pane"))))
                nil
                "#
            ));
            let swapped = owner.picture(WINDOW);
            assert_eq!(
                owner.run("test-snapshot-swapped"),
                "t",
                "mode-line swap ran"
            );
            let repaired = owner.shown(WINDOW);
            assert_eq!(
                (swapped.as_ref().map(text_in), text_in(&repaired)),
                (None, (true, false))
            );
        });
    }
}
