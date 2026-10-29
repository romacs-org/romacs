//! Redisplay runs `fontification-functions' over the text a window shows,
//! as GNU's display iterator does in `handle_fontified_prop' (xdisp.c).
//!
//! GNU batch never redisplays its initial frame, but `window-text-pixel-size'
//! walks the same iterator over FROM..TO, so the oracle runs it over exactly
//! the rows a Romacs window of ten rows displays (window start through the
//! end of its tenth line). To find FROM's line start it first moves back to
//! the line before (`move_it_by_lines (&it, 0)`), which a redisplay from
//! window start never visits, so that line is fontified in advance in both.
//! The walk makes three iterator passes from FROM,
//! and GNU calls again at a still-unfontified position on each pass; a
//! Romacs redisplay is one pass, so the call logs compare as their distinct
//! positions in order. Properties and the current buffer compare exactly.
//! The GNU log text (*Messages*) is not compared: Romacs has no *Messages*
//! buffer and prints closures differently.
use romacs_vm::Vm;

/// Ten rows of 20 pixels, no mode line: the window shows lines 6 to 15.
const SETUP: &str = r#"
(progn
  (setq romacs-fl-log nil)
  (defface romacs-fl-face '((t (:foreground "red"))) "Fontified.")
  (defun romacs-fl-log (entry) (setq romacs-fl-log (cons entry romacs-fl-log)))
  (defun romacs-fl-runs (property)
    (with-current-buffer "fl"
      (let ((pos (point-min)) runs)
        (while (< pos (point-max))
          (setq runs (cons (cons pos (get-text-property pos property)) runs))
          (setq pos (next-single-property-change pos property nil (point-max))))
        (nreverse runs))))
  (defun romacs-fl-distinct (list)
    (let (seen)
      (while list
        (unless (member (car list) seen) (setq seen (cons (car list) seen)))
        (setq list (cdr list)))
      (nreverse seen)))
  (set-window-buffer nil (get-buffer-create "fl"))
  (set-buffer "fl")
  (set (make-local-variable 'mode-line-format) nil)
  (let ((i 0))
    (while (< i 60)
      (insert (format "line %02d %s\n" i (make-string 30 ?x)))
      (setq i (1+ i))))
  (setq romacs-fl-start (save-excursion (goto-char 1) (forward-line 5) (point)))
  (setq romacs-fl-end (save-excursion (goto-char romacs-fl-start) (forward-line 10) (point)))
  (put-text-property (save-excursion (goto-char romacs-fl-start) (forward-line -1) (point)) romacs-fl-start 'fontified t)
  (set-window-start nil romacs-fl-start)
  (goto-char romacs-fl-start)
  (set-window-point nil romacs-fl-start))
"#;

const REPORT: &str = r#"(list (romacs-fl-distinct (reverse romacs-fl-log)) (romacs-fl-runs 'fontified) (romacs-fl-runs 'face) (buffer-name))"#;

/// (name, form run in the displayed buffer after SETUP).
const CASES: &[(&str, &str)] = &[
    ("chunks", "(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (put-text-property pos (min (point-max) (+ pos 50)) 'fontified t) (put-text-property pos (+ pos 4) 'face 'romacs-fl-face))))"),
    ("never-fontifies", "(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos))))"),
    ("signals", "(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (error \"boom %d\" pos))))"),
    ("signal-then-fontify", "(setq fontification-functions (list (lambda (pos) (romacs-fl-log (list 'a pos)) (error \"boom\")) (lambda (pos) (romacs-fl-log (list 'b pos)) (put-text-property pos (+ pos 120) 'fontified t) (put-text-property pos (+ pos 5) 'face 'romacs-fl-face))))"),
    ("whole-function", "(setq fontification-functions (lambda (pos) (romacs-fl-log pos) (put-text-property pos (+ pos 200) 'fontified t)))"),
    ("local-t", "(progn (set-default 'fontification-functions (list (lambda (pos) (romacs-fl-log (list 'global pos)) (put-text-property pos (+ pos 150) 'fontified t)))) (set (make-local-variable 'fontification-functions) (list (lambda (pos) (romacs-fl-log (list 'local pos fontification-functions))) t)))"),
    ("fontifies-ahead", "(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (put-text-property (+ pos 10) (+ pos 300) 'fontified t))))"),
    ("defer", "(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (put-text-property pos (+ pos 70) 'fontified 'defer))))"),
    ("switches-buffer", "(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (set-buffer (get-buffer-create \"other\")) (put-text-property pos (+ pos 100) 'fontified t))))"),
    ("bindings", "(setq fontification-functions (list (lambda (pos) (romacs-fl-log (list pos inhibit-redisplay inhibit-quit fontification-functions)) (put-text-property pos (+ pos 400) 'fontified t))))"),
    ("shortens-text", "(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (when (> (point-max) (+ pos 100)) (delete-region (+ pos 100) (point-max))) (put-text-property pos (min (point-max) (+ pos 50)) 'fontified t))))"),
];

fn romacs_vm(case: &str) -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(400, 200, 20, 10).unwrap();
    vm.eval_source(SETUP).unwrap();
    vm.eval_source(case).unwrap();
    vm
}

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap()
}

fn gnu(case: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let walk = "(window-text-pixel-size nil romacs-fl-start romacs-fl-end)";
    let source = format!("(prin1 (progn {SETUP} {case} {walk} {REPORT}))");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", &source]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn redisplay_fontification_matches_pinned_gnu_walk() {
    let mut failures = vec![];
    for (name, case) in CASES {
        let expected = gnu(case);
        let mut vm = romacs_vm(case);
        vm.display_snapshot(400, 200).unwrap();
        let actual = printed(&mut vm, REPORT);
        if actual != expected {
            failures.push(format!("{name}\n  GNU    {expected}\n  Romacs {actual}"));
        }
        assert_eq!(vm.take_display_diagnostics(), Vec::<String>::new(), "{name}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn unchanged_redisplay_calls_nothing() {
    let mut vm = romacs_vm(CASES[0].1);
    vm.display_snapshot(400, 200).unwrap();
    let first = printed(&mut vm, "(reverse romacs-fl-log)");
    assert_eq!(first, "(196 246 296 346 396 446 496 546)");
    vm.display_snapshot(400, 200).unwrap();
    assert_eq!(printed(&mut vm, "(reverse romacs-fl-log)"), first);
    // Scrolling shows unfontified rows: only they are fontified.
    vm.eval_source("(set-window-start nil romacs-fl-end)").unwrap();
    vm.display_snapshot(400, 200).unwrap();
    assert_eq!(printed(&mut vm, "(reverse romacs-fl-log)"), "(196 246 296 346 396 446 496 546 596 646 696 746 796 846 896 946)");
}

/// The snapshot carries the faces fontification put there, on its first display.
#[test]
fn first_snapshot_shows_the_fontified_faces() {
    let mut vm = romacs_vm(CASES[0].1);
    let snapshot = vm.display_snapshot(400, 200).unwrap();
    let red = snapshot.faces.iter().position(|face| face.foreground == [255, 0, 0]).expect("the red face") as u32;
    let start = snapshot.start_byte;
    let run = snapshot.text.face_runs.iter().find(|run| run.bytes.start == start).unwrap();
    assert_eq!((run.bytes.clone(), run.face.0), (start..start + 4, red));
}

/// Collections inside the functions move nothing redisplay still reads.
#[test]
fn collections_during_fontification_keep_the_snapshot_intact() {
    let mut vm = romacs_vm("(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (let ((i 0)) (while (< i 2000) (vector i (make-string 8 ?y)) (setq i (1+ i)))) (put-text-property pos (min (point-max) (+ pos 50)) 'fontified t) (put-text-property pos (+ pos 4) 'face 'romacs-fl-face))))");
    vm.set_collection_floor(1);
    let before = vm.heap_stats().collections;
    let snapshot = vm.display_snapshot(400, 200).unwrap();
    assert!(vm.heap_stats().collections > before);
    assert!(snapshot.text.utf8.starts_with("line 00 "));
    assert_eq!(printed(&mut vm, "(reverse romacs-fl-log)"), "(196 246 296 346 396 446 496 546)");
    assert_eq!(printed(&mut vm, "(buffer-name)"), "\"fl\"");
}

/// GNU calls a function that leaves `fontified' nil at every iterator stop
/// (property boundaries, at most 100 characters apart); Romacs once per nil
/// stretch and redisplay (DIVERGENCES.md VM2-FONTIFY1). Either way it ends.
#[test]
fn a_function_that_never_fontifies_runs_once_per_stretch() {
    let mut vm = romacs_vm("(progn (put-text-property 250 260 'face 'romacs-fl-face) (put-text-property 300 320 'fontified t) (setq fontification-functions (list (lambda (pos) (romacs-fl-log pos)))))");
    vm.display_snapshot(400, 200).unwrap();
    assert_eq!(printed(&mut vm, "(reverse romacs-fl-log)"), "(196 320)");
    vm.display_snapshot(400, 200).unwrap();
    assert_eq!(printed(&mut vm, "(reverse romacs-fl-log)"), "(196 320 196 320)");
}

/// Text a function inserts cannot keep the walk going: a walk ends at the
/// end the window had before it ran, and changed text starts at most three.
/// Each call inserts two characters and fontifies the one after them, so a
/// walk advances one original character in three per call: 130 calls over
/// the 390 the window first showed, then the lines they lengthened.
#[test]
fn a_function_that_grows_the_text_ends() {
    let mut vm = romacs_vm("(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (save-excursion (goto-char pos) (insert \"++\")) (put-text-property (+ pos 2) (+ pos 3) 'fontified t))))");
    vm.display_snapshot(400, 200).unwrap();
    assert_eq!(printed(&mut vm, "(length romacs-fl-log)"), "589");
}

#[test]
fn killing_the_displayed_buffer_ends_fontification_not_redisplay() {
    let mut vm = romacs_vm("(setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (kill-buffer \"fl\"))))");
    let snapshot = vm.display_snapshot(400, 200).unwrap();
    assert_eq!(printed(&mut vm, "romacs-fl-log"), "(196)");
    assert_eq!(printed(&mut vm, "(buffer-live-p (window-buffer))"), "t");
    assert!(!snapshot.text.utf8.starts_with("line 00"));
    assert_eq!(vm.take_display_diagnostics(), Vec::<String>::new());
}

#[test]
fn deleting_the_window_drops_it_from_the_frame() {
    // VM2's window primitives, as GNU's window.el is not loaded.
    let mut vm = romacs_vm("(progn \
        (setq romacs-fl-a (selected-window)) \
        (let ((half (/ (window-pixel-height romacs-fl-a) 2))) \
          (set-window-new-pixel romacs-fl-a half) \
          (setq romacs-fl-b (split-window-internal romacs-fl-a half nil 0.5))) \
        (set-window-buffer romacs-fl-b (get-buffer-create \"other\")) \
        (setq fontification-functions (list (lambda (pos) (romacs-fl-log pos) (put-text-property pos (point-max) 'fontified t) (let ((parent (window-parent romacs-fl-a))) (set-window-new-pixel parent (window-pixel-height parent)) (set-window-new-pixel romacs-fl-b (window-pixel-height parent))) (delete-window-internal romacs-fl-a)))))");
    let frame = vm.frame_snapshot(400, 400).unwrap();
    assert_eq!(printed(&mut vm, "(list (window-live-p romacs-fl-a) (window-live-p romacs-fl-b))"), "(nil t)");
    // The minibuffer and the remaining window.
    assert_eq!(frame.windows.len(), 2);
    assert_eq!(printed(&mut vm, "(length romacs-fl-log)"), "1");
    assert_eq!(vm.take_display_diagnostics(), Vec::<String>::new());
}

/// A failure outside the functions themselves is a display diagnostic.
#[test]
fn a_failing_walk_is_a_diagnostic() {
    let mut vm = romacs_vm("(progn (setq fontification-functions (list 'ignore)) (fset 'romacs--redisplay-fontify (lambda (start end) (error \"walk %d %d\" start end))))");
    let snapshot = vm.display_snapshot(400, 200).unwrap();
    assert!(snapshot.text.utf8.starts_with("line 00"));
    assert_eq!(vm.take_display_diagnostics(), ["fontification: walk 196 586"]);
}

/// One line per call: the position of each line redisplay fontifies.
const LINE_PER_CALL: &str = "(defun romacs-fl-line (pos) (romacs-fl-log pos) (put-text-property pos (save-excursion (goto-char pos) (forward-line 1) (point)) 'fontified t))";

/// No `fontified` nil in the ROWS lines from window start.
const ROWS_FONTIFIED: &str = "(lambda (rows) (not (text-property-any (window-start) (save-excursion (goto-char (window-start)) (forward-line rows) (point)) 'fontified nil)))";

/// Layout paints from -vscroll: a 7-pixel vscroll over ten 20-pixel rows
/// shows part of an eleventh, which redisplay fontifies too.
#[test]
fn a_vscrolled_window_fontifies_its_partial_last_row() {
    let mut vm = romacs_vm(&format!("(progn {LINE_PER_CALL} (setq fontification-functions (list 'romacs-fl-line)) (set-window-vscroll nil 7 t))"));
    let snapshot = vm.display_snapshot(400, 200).unwrap();
    assert_eq!(snapshot.vscroll_px, 7);
    assert_eq!(printed(&mut vm, "(length romacs-fl-log)"), "11");
    assert_eq!(printed(&mut vm, &format!("(funcall {ROWS_FONTIFIED} 11)")), "t");
}

/// Four short lines a function deletes bring four long ones into the
/// window: redisplay fontifies the rows it then shows, not the shorter range
/// it began with.
#[test]
fn rows_brought_into_view_by_a_deletion_are_fontified() {
    let mut vm = romacs_vm(&format!("(progn {LINE_PER_CALL} \
        (save-excursion (goto-char romacs-fl-start) (insert \"s\\ns\\ns\\ns\\n\")) \
        (setq fontification-functions (list (lambda (pos) (unless romacs-fl-log (delete-region pos (+ pos 8))) (romacs-fl-line pos)))))"));
    let snapshot = vm.display_snapshot(400, 200).unwrap();
    assert_eq!(snapshot.text.utf8[snapshot.start_byte..].lines().next(), Some(format!("line 05 {}", "x".repeat(30)).as_str()));
    assert_eq!(printed(&mut vm, &format!("(funcall {ROWS_FONTIFIED} 10)")), "t");
    assert_eq!(printed(&mut vm, "(length romacs-fl-log)"), "10");
}
