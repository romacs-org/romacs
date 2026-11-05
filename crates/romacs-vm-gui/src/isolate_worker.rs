//! `--isolate FD`: this binary as a worker isolate (`romacs_vm::isolate`,
//! rlisp/vm/isolate.el). The worker boots as its spawner did: `--world
//! STORE KEY` adopts that exact world and refuses any other, `--source`
//! runs the GNU loadup chain from source, `--bare` is a VM with the Lisp
//! prelude alone; `--compiled-store DIR` loads compiled Lisp from the
//! store its spawner loads from. It then serves the spawner on descriptor
//! FD until the spawner's bye, its death, or SIGTERM.
use romacs_vm::Vm;
use romacs_vm::world::store::World;
use std::path::PathBuf;

enum Boot { World(PathBuf, [u8; 32]), Source, Bare }

struct Start { fd: i32, parent: Option<i32>, boot: Boot, compiled: Option<PathBuf> }

fn parse(args: Vec<String>) -> Result<Start, String> {
    let mut args = args.into_iter();
    let (mut fd, mut parent, mut boot, mut compiled) = (None, None, None, None);
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
            "--compiled-store" => compiled = Some(PathBuf::from(args.next().ok_or("missing compiled store")?)),
            "--source" => boot = Some(Boot::Source),
            "--bare" => boot = Some(Boot::Bare),
            other => return Err(format!("unsupported worker argument {other}")),
        }
    }
    let fd = fd.ok_or("--isolate FD required")?;
    let boot = boot.ok_or("a boot is required: --world, --source or --bare")?;
    Ok(Start { fd, parent, boot, compiled })
}

/// Run as a worker with ARGS (after `--isolate`, which the caller saw).
/// On the main thread, before the evaluator exists, so it inherits them:
/// the process's name (`romacs-worker`, for `ps`, `pgrep` and any CPU
/// policy that pins processes by name), the worker priority, and, for a
/// world boot, the compiled Lisp its spawner loads.
pub fn run(args: Vec<String>) -> Result<(), String> {
    let Start { fd, parent, boot, compiled } = parse(args)?;
    let socket = romacs_vm::isolate_worker_socket(fd, parent)?;
    let _ = romacs_vm::host::set_thread_name("romacs-worker");
    romacs_vm::host::lower_priority(romacs_vm::host::Priority::Worker);
    if let (Boot::World(..), Some(store)) = (&boot, compiled) {
        super::batch::configure_worker_compiled_lisp(store);
    }
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
                // A worker is batch, whatever its world (an init layer loads as a live session).
                vm.eval_source("(setq noninteractive t)")?;
                (vm, "world")
            }
        };
        vm.eval_source(&format!("(setq isolate-boot '{name})"))?;
        vm.serve_isolate(socket, name)?;
        vm.shutdown_background_processes()
    })?.join().map_err(|_| "isolate worker panicked".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_world_worker_is_given_its_spawners_compiled_store() {
        let key = "ab".repeat(32);
        let start = parse(args(&["--isolate", "0", "--parent", "7", "--world", "/w", &key, "--compiled-store", "/s"])).unwrap();
        assert!(matches!(start.boot, Boot::World(ref store, k) if store == std::path::Path::new("/w") && k == [0xab; 32]));
        assert_eq!(start.compiled.as_deref(), Some(std::path::Path::new("/s")));
        assert_eq!((start.fd, start.parent), (0, Some(7)));
        let start = parse(args(&["--isolate", "0", "--world", "/w", &key])).unwrap();
        assert!(start.compiled.is_none());
        assert!(parse(args(&["--isolate", "0", "--bare", "--compiled-store"])).is_err());
    }
}
