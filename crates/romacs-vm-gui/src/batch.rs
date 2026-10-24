//! Headless editor host for source-loaded Lisp tests and development probes.
use romacs_vm::Vm;
use std::io::Read;
use std::path::PathBuf;

pub fn run(args: Vec<String>) -> Result<(), String> {
    if args.len() != 1 && !(args.len() == 3 && args[1] == "--init-file") {
        return Err("usage: romacs-vm-gui --batch RLISP_DIRECTORY [--init-file FILE] < forms.el".into());
    }
    let init = args.get(2).map(PathBuf::from);
    let root = PathBuf::from(&args[0]).canonicalize().map_err(|error| error.to_string())?;
    let mut source = String::new();
    std::io::stdin().read_to_string(&mut source).map_err(|error| error.to_string())?;
    if source.trim().is_empty() { return Err("batch input is empty".into()); }
    super::spawn_evaluator(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1000, 700, 20, 10)?;
        super::startup::initialize_files_from_sources(&mut vm, &[], &root)?;
        if let Some(path) = init {
            if !super::user_init::load(&mut vm, &path)? {
                return Err(format!("Explicit init file does not exist: {}", path.display()));
            }
        }
        let value = vm.eval_source(&source)?.display()?;
        println!("{value}");
        vm.shutdown_background_processes()
    })?.join().map_err(|_| "batch evaluator panicked".to_owned())?
}
