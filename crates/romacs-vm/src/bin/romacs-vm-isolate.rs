//! A bare worker isolate: a `Vm::new` with the Lisp prelude, serving the
//! spawner on descriptor 3 (`romacs_vm::isolate`). The GUI binary is the
//! worker for its own launches; this one is for tests and embedders whose
//! VM is bare.
use romacs_vm::Vm;

fn main() {
    if let Err(error) = run() {
        eprintln!("romacs-vm-isolate: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (mut fd, mut parent) = (None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--isolate" => fd = Some(args.next().ok_or("missing descriptor")?.parse::<i32>().map_err(|e| e.to_string())?),
            "--parent" => parent = Some(args.next().ok_or("missing parent pid")?.parse::<i32>().map_err(|e| e.to_string())?),
            "--bare" => {}
            other => return Err(format!("unsupported worker argument {other}")),
        }
    }
    let socket = romacs_vm::isolate_worker_socket(fd.ok_or("--isolate FD required")?, parent)?;
    let mut vm = Vm::new();
    vm.install_lisp_prelude()?;
    vm.serve_isolate(socket, "bare")?;
    vm.shutdown_background_processes()
}
