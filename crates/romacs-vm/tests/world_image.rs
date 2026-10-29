//! A world image through the whole launch protocol, in processes of this
//! test binary: a launcher builds its world, runs itself again on the
//! image and takes the VM back, with the executable and every library
//! where the kernel chose and the arena where its world chose. A damaged
//! image is refused and rebuilt; the restoring process refuses a file
//! that is no whole image before mapping any of it.
use romacs_vm::Vm;
use romacs_vm::world::arena::{self, Arena};
use romacs_vm::world::store::{Launch, World};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[global_allocator]
static ALLOCATOR: Arena = Arena;

const ROLE: &str = "ROMACS_WORLD_IMAGE_TEST";
const NAME: &str = "a_world_restores_randomized_and_refuses_a_damaged_image";

/// This test in a process of its own, in ROLE, with STORE.
fn child(role: &str, store: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env(ROLE, role).env("ROMACS_WORLD_IMAGE_TEST_STORE", store);
    command
}

/// What a child printed of its outcome.
fn outcome(command: &mut Command) -> String {
    let output = command.output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{text}{}", String::from_utf8_lossy(&output.stderr));
    text.lines().find_map(|l| l.split_once("WORLD-IMAGE ").map(|(_, o)| o)).unwrap_or_else(|| panic!("no outcome: {text}")).to_owned()
}

/// Wait for the builder a launch that booted from source started, as
/// OUTCOME names it, to exit: it is no child of this process.
fn builder_done(outcome: &str) {
    let outcome: serde_json::Value = serde_json::from_str(outcome).unwrap();
    let pid = outcome["reason"]["builder"].as_u64().unwrap_or_else(|| panic!("no builder started: {outcome}"));
    let started = std::time::Instant::now();
    while Path::new(&format!("/proc/{pid}")).exists() {
        assert!(started.elapsed() < std::time::Duration::from_secs(120), "builder {pid} still runs");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn image(store: &Path) -> PathBuf {
    std::fs::read_dir(store).unwrap().map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "rwi")).expect("an image")
}

/// Rewrite the image in STORE with EDIT, keeping its mode.
fn damage(store: &Path, edit: impl FnOnce(&mut std::fs::File, u64)) {
    let path = image(store);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    let len = file.metadata().unwrap().len();
    edit(&mut file, len);
    drop(file);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
}

/// The launcher's side: restore, build or boot from source, and say which.
fn launch(store: &Path) {
    let world = World::new(store.to_owned(), vec!["consumer world-image-test".into()]).unwrap();
    match world.launch::<u64>(|| std::env::args_os().skip(1).collect(), true) {
        Launch::Restored { mut vm, carry, .. } => {
            vm.world_reinit();
            let value = vm.eval_source("(world-image-probe)").and_then(|v| v.display()).unwrap();
            let builder = arena::mapped_image().unwrap().executable;
            println!("WORLD-IMAGE {}", json!({"mode": "restored", "carry": carry, "value": value,
                                               "moved": builder != arena::executable_base()}));
        }
        Launch::Build => {
            world.begin_build(&world.build_roots(vec![])).unwrap();
            let mut vm = Vm::new();
            vm.world_begin();
            // A hash table, a string literal's copy and a closure.
            vm.eval_source("(defalias 'world-image-probe (let ((table (make-hash-table :test 'equal))) \
                            (puthash \"key\" 'built table) (lambda () (list (gethash \"key\" table) (hash-table-count table)))))")
                .unwrap();
            let len = world.finish_build(vm, 42u64, json!({})).unwrap();
            println!("WORLD-IMAGE {}", json!({"mode": "built", "bytes": len}));
            std::process::exit(0);
        }
        Launch::Chain { reason } => println!("WORLD-IMAGE {}", json!({"mode": "source", "reason": reason})),
    }
}

/// The restoring process's side, on the descriptor a parent left open.
fn probe() {
    println!("WORLD-IMAGE {}", json!({"restore": format!("{:?}", arena::restore_outcome())}));
}

/// Map FILE as an image in a child, on an inherited descriptor: what the
/// child's arena made of it.
fn probe_file(store: &Path, file: &Path) -> String {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    let file = std::fs::File::open(file).unwrap();
    let fd = file.as_raw_fd();
    let mut command = child("probe", store);
    command.env("ROMACS_WORLD_IMAGE", fd.to_string());
    unsafe { command.pre_exec(move || { libc::fcntl(fd, libc::F_SETFD, 0); Ok(()) }) };
    outcome(&mut command)
}

#[test]
fn a_world_restores_randomized_and_refuses_a_damaged_image() {
    let store = std::env::var_os("ROMACS_WORLD_IMAGE_TEST_STORE").map(PathBuf::from);
    match std::env::var(ROLE).as_deref() {
        Ok("launch") => return launch(&store.unwrap()),
        Ok("probe") => return probe(),
        _ => {}
    }
    let scratch = std::env::temp_dir().join(format!("romacs-world-image-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let store = scratch.join("store");
    let restored = json!({"mode": "restored", "carry": 42, "value": "(built 1)", "moved": true}).to_string();

    // A first launch builds its world, then boots from it.
    assert_eq!(outcome(&mut child("launch", &store)), restored);
    assert_eq!(outcome(&mut child("launch", &store)), restored);

    // A byte changed in place: the content is not the one the manifest
    // binds. The launch boots from source and a builder replaces it.
    damage(&store, |file, len| {
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut reader = std::fs::File::open(image(&store)).unwrap();
        let mut byte = [0u8];
        reader.seek(SeekFrom::Start(len / 2)).unwrap();
        reader.read_exact(&mut byte).unwrap();
        file.seek(SeekFrom::Start(len / 2)).unwrap();
        file.write_all(&[byte[0] ^ 1]).unwrap();
    });
    let refused = outcome(&mut child("launch", &store));
    assert!(refused.contains(r#""mode":"source""#) && refused.contains(r#""image""#), "{refused}");
    builder_done(&refused);
    assert_eq!(outcome(&mut child("launch", &store)), restored);

    // Cut short: refused the same way.
    damage(&store, |file, len| file.set_len(len - 4096).unwrap());
    let refused = outcome(&mut child("launch", &store));
    assert!(refused.contains(r#""mode":"source""#) && refused.contains(r#""image""#), "{refused}");
    builder_done(&refused);

    // The restoring process maps only a whole image of this format.
    let whole = image(&store);
    let bytes = std::fs::read(&whole).unwrap();
    let short = scratch.join("short.rwi");
    std::fs::write(&short, &bytes[..bytes.len() - 4096]).unwrap();
    let other = scratch.join("other.rwi");
    std::fs::write(&other, vec![7u8; 8192]).unwrap();
    assert_eq!(probe_file(&store, &short), r#"{"restore":"Format"}"#);
    assert_eq!(probe_file(&store, &other), r#"{"restore":"Format"}"#);
    assert_eq!(probe_file(&store, &whole), r#"{"restore":"Mapped"}"#);
    let _ = std::fs::remove_dir_all(&scratch);
}
