//! `romacs--armillary-frame` returns the armillary sphere's frame (the
//! romacs-armillary crate), its cells faced by brightness when given faces.
use romacs_armillary::{frame, View};

#[test]
fn the_primitive_returns_the_frame() {
    let mut vm = romacs_vm::Vm::new();
    let printed = vm.eval_source("(romacs--armillary-frame 40 12 -0.25 0.4 -0.15 1 0.5 t)").unwrap().display().unwrap();
    let view = View { cols: 40, rows: 12, pitch: -0.25, yaw: 0.4, roll: -0.15, zoom: 1.0, cell_aspect: 0.5, stars: true };
    assert_eq!(printed, format!("{:?}", frame(&view)));
    let faced = "(let ((s (romacs--armillary-frame 40 12 -0.25 0.4 -0.15 1 0.5 t [dim bright])))
                   (list (equal-including-properties s (substring-no-properties s))
                         (let ((i 0) faces) (while (< i (length s)) (let ((f (get-text-property i 'face s))) (and f (not (memq f faces)) (push f faces))) (setq i (1+ i))) (nreverse faces))))";
    assert_eq!(vm.eval_source(faced).unwrap().display().unwrap(), "(nil (dim bright))");
    assert_eq!(vm.eval_source("(condition-case e (romacs--armillary-frame 201 10 0 0 0 1 0.5 nil) (error e))").unwrap().display().unwrap(),
               "(args-out-of-range 201 10)");
}
