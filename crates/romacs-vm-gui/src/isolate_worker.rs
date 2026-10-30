//! `--isolate FD`: this binary as a worker isolate (`romacs_vm::isolate`,
//! rlisp/vm/isolate.el). The worker boots as its spawner did: `--world
//! STORE KEY` adopts that exact world and refuses any other, `--source`
//! runs the GNU loadup chain from source, `--bare` is a VM with the Lisp
//! prelude alone. It then serves the spawner on descriptor FD until the
//! spawner's bye, its death, or SIGTERM.
use romacs_vm::Vm;
use romacs_vm::world::store::World;
use std::path::PathBuf;

enum Boot { World(PathBuf, [u8; 32]), Source, Bare }

fn parse(args: Vec<String>) -> Result<(i32, Option<i32>, Boot), String> {
    let mut args = args.into_iter();
    let (mut fd, mut parent, mut boot) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--isolate" => fd = Some(args.next().ok_or("missing descriptor")?.parse::<i32>().map_err(|e| e.to_string())?),
            "--parent" => parent = Some(args.next().ok_or("missing parent pid")?.parse::<i32>().map_err(|e| e.to_string())?),
            "--world" => {
                let store = PathBuf::from(args.next().ok_or("missing world store")?);
                let hex = args.next().ok_or("missing world key")?;
                let mut key = [0u8; 32];
                if hex.len() != 64 { return Err("world key must be 64 hexadecimal digits".into()); }
                for (i, byte) in key.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|e| e.to_string())?;
                }
                boot = Some(Boot::World(store, key));
            }
            "--source" => boot = Some(Boot::Source),
            "--bare" => boot = Some(Boot::Bare),
            other => return Err(format!("unsupported worker argument {other}")),
        }
    }
    Ok((fd.ok_or("--isolate FD required")?, parent, boot.ok_or("a boot is required: --world, --source or --bare")?))
}

/// Run as a worker with ARGS (after `--isolate`, which the caller saw).
pub fn run(args: Vec<String>) -> Result<(), String> {
    let (fd, parent, boot) = parse(args)?;
    let socket = romacs_vm::isolate_worker_socket(fd, parent)?;
    super::spawn_evaluator(move || -> Result<(), String> {
        let (mut vm, name) = match boot {
            Boot::Bare => {
                let mut vm = Vm::new();
                vm.install_lisp_prelude()?;
                (vm, "bare")
            }
            Boot::Source => {
                let mut vm = Vm::new();
                vm.set_frame_geometry(80, 25, 1, 1)?;
                super::startup::drive_loadup(&mut vm, &super::startup::lisp_dir())?;
                (vm, "source")
            }
            Boot::World(store, key) => {
                let world = World { store, key, recipe: Vec::new() };
                let (vm, _carry) = world.restore_only::<super::world::Carry>().map_err(|reason| format!("isolate world unavailable: {reason}"))?;
                let mut vm = *vm;
                vm.world_reinit();
                vm.eval_source("(romacs-world-restart)")?;
                (vm, "world")
            }
        };
        vm.eval_source(&format!("(setq isolate-boot '{name})"))?;
        vm.serve_isolate(socket, name)?;
        vm.shutdown_background_processes()
    })?.join().map_err(|_| "isolate worker panicked".to_owned())?
}
