//! A buffer records that a window named it (`BufferState::window_named`),
//! through every Rust path that puts a buffer in a window: its buffer, its
//! previous and next buffers, and its `quit-restore` parameters. A kill reads
//! the record (`romacs--windows-may-name-p`) to skip
//! `replace-buffer-in-windows` for a buffer no window named, while
//! window.el's definition stands.
use romacs_vm::Vm;

fn printed(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("VM-ERROR {error}"),
    }
}

/// Each way a window comes to name a buffer, one fresh buffer each; a
/// parameter other than `quit-restore`'s names none; the record stays with
/// the buffer object across `buffer-swap-text`; a new frame's window names
/// the buffer it is made in (window allocation); with no window.el the kill
/// has nothing to call, and with its function replaced it calls it for every
/// buffer.
#[test]
fn every_way_a_window_names_a_buffer_is_recorded() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    assert_eq!(printed(&mut vm, r#"(let* ((w (selected-window))
             (named (lambda (b) (romacs--windows-may-name-p b)))
             (shown (get-buffer-create "wn-shown")))
        (set-window-buffer w shown)
        (list
          (funcall named shown)
          (progn (fset 'replace-buffer-in-windows (lambda (&optional _) nil))
                 (romacs--window-replacer-defined)
                 (funcall named (get-buffer-create "wn-none")))
          (funcall named shown)
          (let ((b (get-buffer-create "wn-set"))) (set-window-buffer w b) (funcall named b))
          (let ((b (get-buffer-create "wn-prev"))) (set-window-prev-buffers w (list (list b 1 1))) (funcall named b))
          (let ((b (get-buffer-create "wn-next"))) (set-window-next-buffers w (list b)) (funcall named b))
          (let ((b (get-buffer-create "wn-qr"))) (set-window-parameter w 'quit-restore (list 'window 'window w b)) (funcall named b))
          (let ((b (get-buffer-create "wn-qrp"))) (set-window-parameter w 'quit-restore-prev (list 'same (list b 1 1 10) w shown)) (funcall named b))
          (let ((b (get-buffer-create "wn-other"))) (set-window-parameter w 'other (list b)) (funcall named b))
          (let ((b (get-buffer-create "wn-swap")) (o (get-buffer-create "wn-swap-other")))
            (set-window-buffer w b)
            (save-current-buffer (set-buffer b) (buffer-swap-text o))
            (list (funcall named b) (funcall named o)))
          (let ((b (get-buffer-create "wn-child")))
            (save-current-buffer
              (set-buffer b)
              (romacs--create-frame (list (cons 'parent-frame (selected-frame)) (cons 'visibility nil) (cons 'width 5) (cons 'height 2))))
            (funcall named b))
          (progn (fset 'replace-buffer-in-windows (lambda (&optional _) t))
                 (funcall named (get-buffer-create "wn-redefined")))))"#),
        "(nil nil t t t t t t nil (t nil) t t)");
}
