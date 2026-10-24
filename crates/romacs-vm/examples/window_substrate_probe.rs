//! Read-only lane probe: reuse the actual GUI loadup driver without a GUI build.
#[cfg(not(test))]
#[path = "../../romacs-vm-gui/src/loadup.rs"]
mod loadup;

#[cfg(not(test))]
fn main() {
    let root = std::env::args().nth(1).expect("GNU Lisp directory");
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let mut vm = romacs_vm::Vm::new();
            let report = loadup::drive(&mut vm, std::path::Path::new(&root))
                .expect("actual loadup driver must run");
            println!("{}", report.table());
            println!("STATUS {}", report.status());
            for source in [
                "(fboundp 'window-list)",
                "(fboundp 'get-buffer-window)",
                "(fboundp 'minibuffer-selected-window)",
                "(fboundp 'set-window-hscroll)",
                "(fboundp 'window-combination-limit)",
                "(fboundp 'recenter)",
                "(fboundp 'terminal-list)",
                "(fboundp 'tty-top-frame)",
                "(fboundp 'display-buffer)",
                "(fboundp 'quit-restore-window)",
                "(fboundp 'window-height)",
                "(condition-case e (window-list) (error e))",
                "(condition-case e (display-buffer (current-buffer)) (error e))",
            ] {
                let result = vm.eval_source(source).and_then(|value| value.display());
                println!("PROBE {source} => {result:?}");
            }
        })
        .expect("probe thread")
        .join()
        .expect("probe completed");
}
