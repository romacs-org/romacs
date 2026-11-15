//! A VM booted as the gnu-ert runner boots it: GNU's Lisp as GNU batch
//! has it (gnu_lisp.rs), then help-fns, cl-print and ERT.
use romacs_vm::Vm;
#[path = "gnu_lisp.rs"]
mod gnu_lisp;

pub fn bootstrapped() -> Vm {
    gnu_lisp::gnu_lisp(&["lisp/help-fns.el", "lisp/emacs-lisp/romacs-cl-print.el", "lisp/emacs-lisp/ert.el"])
}
