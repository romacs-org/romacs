//! A world image through the whole launch protocol, in processes of this
//! test binary: a launcher builds its world, maps the image and takes the
//! VM back, with the executable and every library where the kernel chose
//! and the arena where its world chose. A damaged image is refused and
//! rebuilt; the arena refuses a file that is no whole image before mapping
//! any of it.
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
const INTERRUPTED: &str = "an_interrupted_build_leaves_nothing_in_the_way";
const PAGES: &str = "capture_reads_no_heap_byte_so_a_page_zeroed_by_a_store_is_data";

/// Test NAME in a process of its own, in ROLE, with STORE.
fn child_of(name: &str, role: &str, store: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(ROLE, role).env("ROMACS_WORLD_IMAGE_TEST_STORE", store);
    command
}

fn child(role: &str, store: &Path) -> Command {
    child_of(NAME, role, store)
}

/// Act in the role a parent gave this process, if any: true when it did.
fn act_in_role() -> bool {
    let store = std::env::var_os("ROMACS_WORLD_IMAGE_TEST_STORE").map(PathBuf::from);
    match std::env::var(ROLE).as_deref() {
        Ok("launch") => launch(&store.unwrap()),
        Ok("probe") => probe(),
        Ok("pages") => pages(&store.unwrap()),
        _ => return false,
    }
    true
}

/// A fresh scratch directory for test NAME.
fn scratch(name: &str) -> PathBuf {
    let scratch = std::env::temp_dir().join(format!("romacs-world-image-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    scratch
}

fn restored() -> String {
    json!({"mode": "restored", "carry": 42, "value": "(built 1)", "moved": true}).to_string()
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
fn world(store: &Path) -> World {
    World::new(store.to_owned(), vec!["consumer world-image-test".into()]).unwrap()
}

fn launch(store: &Path) {
    let world = world(store);
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

/// Adopt the file the parent named: what the arena made of it.
fn probe() {
    let file = std::fs::File::open(std::env::var_os("ROMACS_WORLD_IMAGE_TEST_FILE").unwrap()).unwrap();
    println!("WORLD-IMAGE {}", json!({"restore": format!("{:?}", arena::adopt(&file))}));
}

/// Adopt FILE as an image in a child: what its arena made of it.
fn probe_file(store: &Path, file: &Path) -> String {
    outcome(child("probe", store).env("ROMACS_WORLD_IMAGE_TEST_FILE", file))
}

#[test]
fn a_world_restores_randomized_and_refuses_a_damaged_image() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(NAME);
    let store = scratch.join("store");
    let restored = restored();

    // A first launch builds its world, then boots from it.
    assert_eq!(outcome(&mut child("launch", &store)), restored);
    assert_eq!(outcome(&mut child("launch", &store)), restored);
    // The libraries were hashed as mapped at each launch: nothing of
    // theirs is kept in the store to stand in for that.
    let kept: Vec<String> = std::fs::read_dir(store.join("ids")).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert!(kept.iter().all(|name| !name.starts_with("library-")), "{kept:?}");

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

/// A builder's capture of three pages of one block: the first stored to
/// with a zero, the second with bytes, the third never. Where each is in
/// the image, and whether it holds data there.
fn pages(scratch: &Path) {
    const PAGE: usize = 4096;
    // A page run, not a slab block: page-aligned.
    let mut block = vec![0u8; 16 * PAGE];
    unsafe { std::ptr::write_volatile(block.as_mut_ptr(), 0) };
    block[PAGE..2 * PAGE].fill(0xab);
    let out = std::fs::File::create(scratch.join("pages.rwi")).unwrap();
    let fields = arena::ImageFields { len: 0, root: block.as_ptr() as usize, counters: [0; 4], executable: 0, key: [0; 32] };
    let len = unsafe { arena::write_image(&out, fields) }.unwrap();
    let bytes = std::fs::read(scratch.join("pages.rwi")).unwrap();
    assert_eq!(bytes.len(), len);
    let second = (0..len / PAGE).map(|p| p * PAGE).find(|&at| bytes[at..at + PAGE].iter().all(|&b| b == 0xab)).unwrap();
    use std::os::fd::AsRawFd;
    let data_from = |at: usize| unsafe { libc::lseek(out.as_raw_fd(), at as i64, libc::SEEK_DATA) };
    println!("WORLD-IMAGE {}", json!({"first": data_from(second - PAGE) == (second - PAGE) as i64,
                                       "second": data_from(second) == second as i64,
                                       "third": data_from(second + PAGE) == (second + PAGE) as i64}));
    drop(block);
}

#[test]
fn capture_reads_no_heap_byte_so_a_page_zeroed_by_a_store_is_data() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(PAGES);
    let mut command = child_of(PAGES, "pages", &scratch);
    command.env("ROMACS_WORLD_BUILD", &scratch);
    // The page a store touched is data although all of it is zero: capture
    // no longer looks at a page's bytes, which may be padding no value
    // initialized. The untouched page is a hole.
    assert_eq!(outcome(&mut command), json!({"first": true, "second": true, "third": false}).to_string());
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn an_interrupted_build_leaves_nothing_in_the_way() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(INTERRUPTED);
    let store = scratch.join("store");
    let key: String = world(&store).key.iter().map(|b| format!("{b:02x}")).collect();
    // What a builder interrupted between writing and publishing leaves:
    // its temporaries, read-only already.
    let left = ["rwi.new", "json.new", "rwi.Ab12Cd", "json.x9Y8z7"].map(|t| store.join(format!("{key}.{t}")));
    for path in &left {
        std::fs::write(path, b"partial").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o400)).unwrap();
    }
    assert_eq!(outcome(&mut child_of(INTERRUPTED, "launch", &store)), restored());
    for path in &left {
        assert!(!path.exists(), "{} is left", path.display());
    }
    let names: Vec<String> = std::fs::read_dir(&store).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with(&key)).collect();
    assert!(names.iter().all(|n| [".rwi", ".json", ".lock", ".log"].iter().any(|s| *n == format!("{key}{s}"))), "{names:?}");
    let _ = std::fs::remove_dir_all(&scratch);
}
