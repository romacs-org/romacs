//! A world image through the whole launch protocol, in processes of this
//! test binary: a launcher builds its world, maps the image and takes the
//! VM back, with the executable and every library where the kernel chose
//! and the arena where its world chose. A damaged image is refused and
//! rebuilt; the arena refuses a file that is no whole image before mapping
//! any of it. Failed builds are recorded and retried with a backoff,
//! refusals kept until their inputs change or a reset, and concurrent
//! launches share one builder.
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
const FAILED: &str = "a_failed_build_is_retried_after_a_backoff";
const PUBLISH: &str = "a_build_that_fails_to_publish_records_its_failure";
const REFUSED: &str = "a_refused_capture_stays_refused_until_its_inputs_change_or_a_reset";
const FOREIGN: &str = "a_launch_on_a_directory_that_is_no_store_boots_from_source_and_says_why";
const PAUSED: &str = "pruning_waits_for_a_launcher_between_opening_and_locking";
const PAUSED_BUILDER: &str = "pruning_waits_for_a_builder_between_opening_and_locking";
const CONCURRENT: &str = "concurrent_first_launches_share_one_builder_and_say_they_wait";
const PAGES: &str = "capture_reads_no_heap_byte_so_a_page_zeroed_by_a_store_is_data";
const ANCESTORS: &str = "a_store_is_refused_under_a_home_or_cache_others_could_replace";
const UMASK: &str = "a_store_made_under_umask_000_is_private_all_the_way_down";
const RESERVED: &str = "an_images_range_is_reserved_before_anything_else_maps_and_the_restore_succeeds";
const TAKEN: &str = "an_image_whose_range_was_taken_before_the_process_started_is_refused_and_says_why";

/// Before the runtime's first allocation, so before the arena reserves its
/// window: a page mapped at the address ROMACS_WORLD_IMAGE_TEST_PREMAP
/// names (hexadecimal), as a library's or the loader's own mapping would be.
#[used]
#[unsafe(link_section = ".init_array")]
static PREMAP: unsafe extern "C" fn() = premap;
unsafe extern "C" fn premap() {
    let at = unsafe { libc::getenv(c"ROMACS_WORLD_IMAGE_TEST_PREMAP".as_ptr()) };
    if at.is_null() {
        return;
    }
    let at = unsafe { libc::strtoull(at, std::ptr::null_mut(), 16) } as usize;
    let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE;
    unsafe { libc::mmap(at as *mut libc::c_void, 4096, libc::PROT_NONE, flags, -1, 0) };
}

/// COMMAND run under a stack limit of STACK bytes and an address-space
/// limit of ADDRESS_SPACE bytes. An unlimited stack makes the kernel lay
/// the child's mappings out bottom-up, from inside the window; an
/// address-space limit leaves the arena nothing to reserve the window with.
fn with_limits(mut command: Command, stack: u64, address_space: u64) -> Command {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(move || {
            for (resource, limit) in [(libc::RLIMIT_STACK, stack), (libc::RLIMIT_AS, address_space)] {
                let limit = libc::rlimit { rlim_cur: limit, rlim_max: limit };
                if libc::setrlimit(resource, &limit) != 0 { return Err(std::io::Error::last_os_error()); }
            }
            Ok(())
        })
    };
    command
}

/// Whether a child of this process may run without an address-space
/// limit: the hard limit is none. Nextest's wrapper sets one (8 GiB) on
/// every test process, and a child cannot raise it.
fn address_space_unlimited() -> bool {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    let asked = unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limit) };
    asked == 0 && limit.rlim_max == libc::RLIM_INFINITY
}

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
        Ok("open-launch") => {
            // As a user whose umask lets everyone write: the builder this
            // spawns inherits it.
            unsafe { libc::umask(0) };
            launch(&store.unwrap())
        }
        Ok("probe") => probe(),
        Ok("launch-reserved") => launch_reserved(&store.unwrap()),
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
    let world = match World::new(store.to_owned(), vec!["consumer world-image-test".into()]) {
        Ok(world) => world,
        Err(error) => {
            println!("WORLD-IMAGE {}", json!({"mode": "source", "reason": error.to_string()}));
            return;
        }
    };
    match world.launch::<u64>(|| std::env::args_os().skip(1).collect(), true, || println!("WAITING-FOR-BUILD")) {
        Launch::Restored { mut vm, carry, .. } => {
            vm.world_reinit();
            let value = vm.eval_source("(world-image-probe)").and_then(|v| v.display()).unwrap();
            let builder = arena::mapped_image().unwrap().executable;
            println!("WORLD-IMAGE {}", json!({"mode": "restored", "carry": carry, "value": value,
                                               "moved": builder != arena::executable_base()}));
        }
        Launch::Build => {
            // An input beside the store: the word "fail" in it fails the build.
            let input = store.parent().unwrap().join("input");
            world.begin_build(&world.build_roots(vec![input.clone()])).unwrap();
            // As the GUI and the runner build: steps given to World::build.
            let built = world.build(|| {
                let flag = romacs_vm::host::read_to_string(input.join("flag")).unwrap_or_default();
                if flag.trim() == "fail" {
                    return Err("the input says fail".into());
                }
                if flag.trim() == "slow" {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                let mut vm = Vm::new();
                if flag.trim() == "sqlite" {
                    // A resource of this process: the capture refuses it.
                    vm.eval_source("(defvar world-image-db (sqlite-open))").unwrap();
                }
                vm.world_begin();
                // A hash table, a string literal's copy and a closure.
                vm.eval_source("(defalias 'world-image-probe (let ((table (make-hash-table :test 'equal))) \
                                (puthash \"key\" 'built table) (lambda () (list (gethash \"key\" table) (hash-table-count table)))))")
                    .unwrap();
                Ok((vm, 42u64, json!({})))
            });
            let len = match built {
                Ok(len) => len,
                Err(error) => {
                    println!("WORLD-IMAGE {}", json!({"mode": "failed", "error": error}));
                    std::process::exit(1);
                }
            };
            println!("WORLD-IMAGE {}", json!({"mode": "built", "bytes": len}));
            std::process::exit(0);
        }
        Launch::Chain { reason } => println!("WORLD-IMAGE {}", json!({"mode": "source", "reason": reason})),
    }
}

/// Adopt the file the parent named: what the arena made of it, and of a
/// second adoption.
fn probe() {
    let file = std::fs::File::open(std::env::var_os("ROMACS_WORLD_IMAGE_TEST_FILE").unwrap()).unwrap();
    let first = format!("{:?}", arena::adopt(&file));
    let again = format!("{:?}", arena::adopt(&file));
    println!("WORLD-IMAGE {}", json!({"restore": first, "again": again, "failure": arena::map_failure(),
                                       "kept": arena::mapped_image().is_some()}));
}

/// Where the image in STORE was made: its arena's base, from its header.
fn image_base(store: &Path) -> usize {
    let bytes = std::fs::read(image(store)).unwrap();
    usize::from_le_bytes(bytes[8..16].try_into().unwrap())
}

/// Whether a page can be mapped at AT: the error that refuses it, if any.
fn mapping_at(at: usize) -> Option<i32> {
    let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE;
    let mapped = unsafe { libc::mmap(at as *mut libc::c_void, 4096, libc::PROT_NONE, flags, -1, 0) };
    if mapped == libc::MAP_FAILED {
        return std::io::Error::last_os_error().raw_os_error();
    }
    unsafe { libc::munmap(mapped, 4096) };
    None
}

/// Launch as `launch` does, after showing what the arena made of the
/// address space: the image's range reserved, disjoint from the first
/// arena's and taken by nothing, as the mapping something of the process
/// would make there finds it; and after, whether the image's base is still
/// taken: by the image, or by what refused it.
fn launch_reserved(store: &Path) {
    let base = image_base(store);
    let (first, reserve) = arena::current_range().unwrap();
    let disjoint = base + reserve <= first || first + reserve <= base;
    println!("WORLD-IMAGE {}", json!({"reserved": arena::reserved(base, reserve), "disjoint": disjoint,
                                       "taken": mapping_at(base) == Some(libc::EEXIST)}));
    launch(store);
    println!("WORLD-IMAGE {}", json!({"taken_after": mapping_at(base) == Some(libc::EEXIST)}));
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
    // binds. The launch rebuilds the world, a new image, and boots from it.
    use std::os::unix::fs::MetadataExt;
    let inode = || std::fs::metadata(image(&store)).unwrap().ino();
    let damaged = inode();
    damage(&store, |file, len| {
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut reader = std::fs::File::open(image(&store)).unwrap();
        let mut byte = [0u8];
        reader.seek(SeekFrom::Start(len / 2)).unwrap();
        reader.read_exact(&mut byte).unwrap();
        file.seek(SeekFrom::Start(len / 2)).unwrap();
        file.write_all(&[byte[0] ^ 1]).unwrap();
    });
    assert_eq!(outcome(&mut child("launch", &store)), restored);
    assert_ne!(inode(), damaged, "the damaged image was replaced");

    // Cut short: the same.
    let cut = inode();
    damage(&store, |file, len| file.set_len(len - 4096).unwrap());
    assert_eq!(outcome(&mut child("launch", &store)), restored);
    assert_ne!(inode(), cut);

    // The restoring process maps only a whole image of this format.
    let whole = image(&store);
    let bytes = std::fs::read(&whole).unwrap();
    let short = scratch.join("short.rwi");
    std::fs::write(&short, &bytes[..bytes.len() - 4096]).unwrap();
    let other = scratch.join("other.rwi");
    std::fs::write(&other, vec![7u8; 8192]).unwrap();
    assert_eq!(probe_file(&store, &short), r#"{"again":"Format","failure":null,"kept":false,"restore":"Format"}"#);
    assert_eq!(probe_file(&store, &other), r#"{"again":"Format","failure":null,"kept":false,"restore":"Format"}"#);
    // Adopted once per process: a second adoption is refused, says so, and
    // the first image stays the process's.
    assert_eq!(probe_file(&store, &whole), r#"{"again":"Map","failure":["adopted already",0,false],"kept":true,"restore":"Mapped"}"#);
    let _ = std::fs::remove_dir_all(&scratch);
}

/// What a child printed of its outcomes: every outcome line.
fn outcomes(command: &mut Command) -> Vec<String> {
    let output = command.output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{text}{}", String::from_utf8_lossy(&output.stderr));
    text.lines().filter_map(|l| l.split_once("WORLD-IMAGE ").map(|(_, o)| o.to_owned())).collect()
}

#[test]
fn an_images_range_is_reserved_before_anything_else_maps_and_the_restore_succeeds() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(RESERVED);
    let store = scratch.join("store");
    assert_eq!(outcome(&mut child_of(RESERVED, "launch", &store)), restored());
    // In a process started afterwards, with the image's range in its
    // reservation: nothing of the process can map there, and the image
    // maps over the reservation, which no mapping made since can refuse.
    // So in a process with an unlimited stack, whose kernel maps its
    // libraries bottom-up from a sixth of the address space, inside the
    // window: the image's range lies above them. Under an address-space
    // limit nothing is reserved: the image maps where nothing is, and
    // finds its range free.
    for stack in [8 << 20, libc::RLIM_INFINITY] {
        let limited = outcomes(&mut with_limits(child_of(RESERVED, "launch-reserved", &store), stack, 8 << 30));
        assert_eq!(limited, [json!({"reserved": false, "disjoint": true, "taken": false}).to_string(), restored(),
                             json!({"taken_after": true}).to_string()], "stack limit {stack}, address space limited");
        if !address_space_unlimited() {
            println!("under an address-space limit: the reservation is not exercised");
            continue;
        }
        let printed = outcomes(&mut with_limits(child_of(RESERVED, "launch-reserved", &store), stack, libc::RLIM_INFINITY));
        assert_eq!(printed, [json!({"reserved": true, "disjoint": true, "taken": true}).to_string(), restored(),
                             json!({"taken_after": true}).to_string()], "stack limit {stack}");
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn an_image_whose_range_was_taken_before_the_process_started_is_refused_and_says_why() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(TAKEN);
    let store = scratch.join("store");
    assert_eq!(outcome(&mut child_of(TAKEN, "launch", &store)), restored());
    use std::os::unix::fs::MetadataExt;
    let inode = std::fs::metadata(image(&store)).unwrap().ino();
    // A page mapped at the image's base before the process's first
    // allocation: the window cannot be reserved, and the image's range is
    // mapped only where nothing is, which finds the page. The launch boots
    // from source, says at which step and why, leaves the page where it
    // was, and the image as it is: a later process restores it.
    let base = image_base(&store);
    let printed = outcomes(child_of(TAKEN, "launch-reserved", &store).env("ROMACS_WORLD_IMAGE_TEST_PREMAP", format!("{base:x}")));
    assert_eq!(printed, [json!({"reserved": false, "disjoint": true, "taken": true}).to_string(),
                         json!({"mode": "source", "reason": {"image": "Map", "step": "image", "error": "File exists (os error 17)", "reserved": false}}).to_string(),
                         json!({"taken_after": true}).to_string()]);
    assert_eq!(std::fs::metadata(image(&store)).unwrap().ino(), inode, "the image was kept");
    assert_eq!(outcome(&mut child_of(TAKEN, "launch", &store)), restored());
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
    assert!(names.iter().all(|n| [".rwi", ".json", ".lock", ".log", ".use"].iter().any(|s| *n == format!("{key}{s}"))), "{names:?}");
    let _ = std::fs::remove_dir_all(&scratch);
}

/// The store's files, by name, with their modification times.
fn files(store: &Path) -> std::collections::BTreeMap<String, std::time::SystemTime> {
    std::fs::read_dir(store).unwrap().map(|e| e.unwrap())
        .filter(|e| e.file_type().unwrap().is_file())
        .map(|e| (e.file_name().into_string().unwrap(), e.metadata().unwrap().modified().unwrap())).collect()
}

/// The transient failure record of the world in STORE, if any.
fn failure(store: &Path) -> Option<serde_json::Value> {
    let path = std::fs::read_dir(store).unwrap().map(|e| e.unwrap().path()).find(|p| p.extension().is_some_and(|x| x == "failed"))?;
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).ok()
}

/// Pretend STORE's last transient failure was SECONDS ago.
fn age_failure(store: &Path, seconds: u64) {
    let path = std::fs::read_dir(store).unwrap().map(|e| e.unwrap().path()).find(|p| p.extension().is_some_and(|x| x == "failed")).unwrap();
    let mut record = failure(store).unwrap();
    record["at"] = json!(record["at"].as_u64().unwrap() - seconds);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&path, record.to_string()).unwrap();
}

#[test]
fn a_failed_build_is_retried_after_a_backoff() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(FAILED);
    let store = scratch.join("store");
    std::fs::create_dir_all(scratch.join("input")).unwrap();
    std::fs::write(scratch.join("input/flag"), "fail").unwrap();
    // The launch waits for its builder, which fails: from source, and the
    // failure counted.
    let first = outcome(&mut child_of(FAILED, "launch", &store));
    assert!(first.contains(r#""mode":"source""#), "{first}");
    assert_eq!(failure(&store).unwrap()["attempts"], 1);
    // Within the minute: from source at once, starting no builder.
    let before = files(&store);
    let second = outcome(&mut child_of(FAILED, "launch", &store));
    assert!(second.contains(r#""mode":"source""#) && second.contains("retry_in"), "{second}");
    assert_eq!(files(&store), before, "no builder ran");
    // A minute on: tried again, failing again, and the next wait is ten.
    age_failure(&store, 61);
    outcome(&mut child_of(FAILED, "launch", &store));
    let record = failure(&store).unwrap();
    assert_eq!(record["attempts"], 2);
    let third = outcome(&mut child_of(FAILED, "launch", &store));
    let retry = serde_json::from_str::<serde_json::Value>(&third).unwrap()["reason"]["retry_in"].as_u64().unwrap();
    assert!((500..=600).contains(&retry), "{third}");
    // Fixed, and due: built, booted from, and the failures forgotten.
    std::fs::write(scratch.join("input/flag"), "ok").unwrap();
    age_failure(&store, 601);
    assert_eq!(outcome(&mut child_of(FAILED, "launch", &store)), restored());
    assert!(failure(&store).is_none());
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_build_that_fails_to_publish_records_its_failure() {
    if act_in_role() {
        return;
    }
    use std::os::unix::process::CommandExt;
    let scratch = scratch(PUBLISH);
    let store = scratch.join("store");
    std::fs::create_dir_all(scratch.join("input")).unwrap();
    // No file past a megabyte: the image cannot be written, as on a full
    // disk, but a failure record can.
    let full = || {
        let mut command = child_of(PUBLISH, "launch", &store);
        unsafe { command.pre_exec(|| {
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            let limit = libc::rlimit { rlim_cur: 1 << 20, rlim_max: libc::RLIM_INFINITY };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
        }) };
        command
    };
    let first = outcome(&mut full());
    assert!(first.contains(r#""mode":"source""#), "{first}");
    let record = failure(&store).expect("the publication failure is recorded");
    assert_eq!(record["attempts"], 1);
    assert!(record["error"].as_str().unwrap().contains("world image"), "{record}");
    // So the next launch does not pay for the build again.
    let before = files(&store);
    let second = outcome(&mut full());
    assert!(second.contains("retry_in"), "{second}");
    assert_eq!(files(&store), before);
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_refused_capture_stays_refused_until_its_inputs_change_or_a_reset() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(REFUSED);
    let store = scratch.join("store");
    std::fs::create_dir_all(scratch.join("input")).unwrap();
    std::fs::write(scratch.join("input/flag"), "sqlite").unwrap();
    let first = outcome(&mut child_of(REFUSED, "launch", &store));
    assert!(first.contains(r#""mode":"source""#), "{first}");
    assert!(failure(&store).is_none(), "a refusal is no transient failure");
    // However long since: no retry while the inputs hold.
    let before = files(&store);
    let second = outcome(&mut child_of(REFUSED, "launch", &store));
    assert!(second.contains("refused") && second.contains("a live sqlite object") && !second.contains("retry_in"), "{second}");
    assert_eq!(files(&store), before, "no builder ran");
    // A reset forgets the refusal: the next launch builds again.
    assert_eq!(romacs_vm::world::store::reset(&store, false).unwrap(), 1);
    let third = outcome(&mut child_of(REFUSED, "launch", &store));
    assert!(third.contains("refused"), "{third}");
    assert_ne!(files(&store), before, "a builder ran");
    // Another input: built and booted from.
    std::fs::write(scratch.join("input/flag"), "ok").unwrap();
    assert_eq!(outcome(&mut child_of(REFUSED, "launch", &store)), restored());
    // Every world and cache goes with `all`, and only the store's own
    // files: the marker, the store's lock and directories, and a stranger stay.
    std::fs::write(store.join("unrelated"), "kept").unwrap();
    assert!(romacs_vm::world::store::reset(&store, true).unwrap() > 0);
    let mut left: Vec<String> = std::fs::read_dir(&store).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    left.sort();
    assert_eq!(left, [".romacs-world-store", "ids", "p", "store.lock", "unrelated"]);
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn concurrent_first_launches_share_one_builder_and_say_they_wait() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(CONCURRENT);
    let store = scratch.join("store");
    std::fs::create_dir_all(scratch.join("input")).unwrap();
    // A builder slow enough that the second launch meets it.
    std::fs::write(scratch.join("input/flag"), "slow").unwrap();
    let spawn = || child_of(CONCURRENT, "launch", &store).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let first = spawn();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let second = spawn();
    for child in [first, second] {
        let output = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(text.contains(&format!("WORLD-IMAGE {}", restored())), "{text}");
        assert!(text.contains("WAITING-FOR-BUILD"), "it said it waits: {text}");
    }
    // One builder built: its log has one build in it, and no refusal.
    let log = std::fs::read_dir(&store).unwrap().map(|e| e.unwrap().path()).find(|p| p.extension().is_some_and(|x| x == "log")).unwrap();
    let log = std::fs::read_to_string(log).unwrap();
    assert_eq!(log.matches(r#""mode":"built""#).count(), 1, "{log}");
    // A later launch finds its world: no wait.
    let later = child_of(CONCURRENT, "launch", &store).output().unwrap();
    let text = String::from_utf8_lossy(&later.stdout);
    assert!(text.contains(&format!("WORLD-IMAGE {}", restored())) && !text.contains("WAITING-FOR-BUILD"), "{text}");
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn pruning_waits_for_a_launcher_between_opening_and_locking() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(PAUSED);
    let store = scratch.join("store");
    std::fs::create_dir_all(scratch.join("input")).unwrap();
    // A world, built and restored once, then left a week unused, and its
    // image gone: the next launch takes its lock to build it again.
    assert_eq!(outcome(&mut child_of(PAUSED, "launch", &store)), restored());
    let key: String = world(&store).key.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::remove_file(store.join(format!("{key}.rwi"))).unwrap();
    let week_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86400);
    for (name, _) in files(&store) {
        if name.starts_with(&key) {
            let path = store.join(&name);
            let mode = std::fs::metadata(&path).unwrap().permissions();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(week_ago).unwrap();
            std::fs::set_permissions(&path, mode).unwrap();
        }
    }
    // That launch pauses between opening the world's lock and locking it.
    let pause = scratch.join("pause");
    std::fs::create_dir_all(&pause).unwrap();
    let launcher = child_of(PAUSED, "launch", &store).env("ROMACS_WORLD_TEST_PAUSE_LOCK", &pause)
        .stdout(std::process::Stdio::piped()).spawn().unwrap();
    let started = std::time::Instant::now();
    while !pause.join("paused").exists() {
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "the launcher never paused");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Another recipe's launch prunes the store meanwhile: the paused world
    // is old and not its own, so it would go, lock file and all.
    let other = World::new(store.clone(), vec!["consumer another".into()]).unwrap();
    let pruner = std::thread::spawn(move || other.prune().unwrap());
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!pruner.is_finished(), "pruning waits for the store lock the launcher holds");
    std::fs::write(pause.join("resume"), "").unwrap();
    let output = launcher.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains(&format!("WORLD-IMAGE {}", restored())), "{text}");
    // Once the launcher held the world's lock, pruning found it held.
    let pruned = pruner.join().unwrap();
    assert!(!pruned.contains(&key), "{pruned:?}");
    assert!(store.join(format!("{key}.lock")).exists() && store.join("store.lock").exists());
    let _ = std::fs::remove_dir_all(&scratch);
}

/// Leave the world of this test's recipe in STORE a week unused, its image
/// gone: whoever comes next takes its lock to build it again.
fn stale(store: &Path) -> String {
    let key: String = world(store).key.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::remove_file(store.join(format!("{key}.rwi"))).unwrap();
    let week_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86400);
    for (name, _) in files(store) {
        if name.starts_with(&key) {
            let path = store.join(&name);
            let mode = std::fs::metadata(&path).unwrap().permissions();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(week_ago).unwrap();
            std::fs::set_permissions(&path, mode).unwrap();
        }
    }
    key
}

#[test]
fn pruning_waits_for_a_builder_between_opening_and_locking() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(PAUSED_BUILDER);
    let store = scratch.join("store");
    std::fs::create_dir_all(scratch.join("input")).unwrap();
    assert_eq!(outcome(&mut child_of(PAUSED_BUILDER, "launch", &store)), restored());
    let key = stale(&store);
    // A builder started with no lock handed on opens the world's lock
    // itself, and pauses before locking it.
    let pause = scratch.join("pause");
    std::fs::create_dir_all(&pause).unwrap();
    let builder = child_of(PAUSED_BUILDER, "launch", &store).env("ROMACS_WORLD_BUILD", &store)
        .env("ROMACS_WORLD_TEST_PAUSE_LOCK", &pause).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let started = std::time::Instant::now();
    while !pause.join("paused").exists() {
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "the builder never paused");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let other = World::new(store.clone(), vec!["consumer another".into()]).unwrap();
    let pruner = std::thread::spawn(move || other.prune().unwrap());
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!pruner.is_finished(), "pruning waits for the store lock the builder holds");
    std::fs::write(pause.join("resume"), "").unwrap();
    let output = builder.wait_with_output().unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains(r#""mode":"built""#), "{output:?}");
    assert!(!pruner.join().unwrap().contains(&key));
    // What it built is the world's: a launch boots from it.
    assert_eq!(outcome(&mut child_of(PAUSED_BUILDER, "launch", &store)), restored());
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_launch_on_a_directory_that_is_no_store_boots_from_source_and_says_why() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(FOREIGN);
    let store = scratch.join("store");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(store.join("letter.txt"), "mine").unwrap();
    let said = outcome(&mut child_of(FOREIGN, "launch", &store));
    assert!(said.contains(r#""mode":"source""#) && said.contains("is no world store"), "{said}");
    assert_eq!(std::fs::read_dir(&store).unwrap().count(), 1, "nothing written in it");
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_store_made_under_umask_000_is_private_all_the_way_down() {
    if act_in_role() {
        return;
    }
    let scratch = scratch(UMASK);
    std::fs::set_permissions(&scratch, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = scratch.join("cache/romacs/worlds");
    assert_eq!(outcome(&mut child_of(UMASK, "open-launch", &store)), restored());
    // Each directory made on the way, and the store's own, 0700; no file
    // in it readable or writable by anyone else.
    for dir in ["cache", "cache/romacs", "cache/romacs/worlds"] {
        let mode = std::fs::metadata(scratch.join(dir)).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o700, "{dir}: {mode:o}");
    }
    let mut pending = vec![store.clone()];
    let mut seen = 0;
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            assert_eq!(meta.permissions().mode() & 0o077, 0, "{}: {:o}", path.display(), meta.permissions().mode());
            if meta.is_dir() {
                pending.push(path);
            }
            seen += 1;
        }
    }
    assert!(seen > 3, "a built store: {seen} entries");
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_store_is_refused_under_a_home_or_cache_others_could_replace() {
    if act_in_role() {
        return;
    }
    use std::os::unix::fs::symlink;
    let scratch = scratch(ANCESTORS);
    let mode = |dir: &Path, mode: u32| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
    let dirs = |dirs: &[&Path]| for dir in dirs {
        std::fs::create_dir_all(dir).unwrap();
        mode(dir, 0o700);
    };
    // A launch whose HOME and XDG_CACHE_HOME are HOME and CACHE, with its
    // store in CACHE: it boots from source, saying why, and makes nothing.
    let refused = |home: &Path, cache: &Path, why: &str| {
        let store = cache.join("romacs/worlds");
        let said = outcome(child_of(ANCESTORS, "launch", &store).env("HOME", home).env("XDG_CACHE_HOME", cache));
        assert!(said.contains(r#""mode":"source""#) && said.contains(why), "{said}");
        assert!(!store.exists() && !cache.join("romacs").exists(), "nothing made in {}", cache.display());
    };
    // A HOME others may write, under a private cache of its own.
    let home = scratch.join("open-home");
    let cache = home.join("cache");
    dirs(&[&home, &cache]);
    mode(&home, 0o770);
    refused(&home, &cache, "open-home is writable by others");
    // A symlinked HOME, and a symlinked cache under a real one.
    let real = scratch.join("real-home");
    dirs(&[&real, &real.join("cache")]);
    symlink(&real, scratch.join("linked-home")).unwrap();
    refused(&scratch.join("linked-home"), &scratch.join("linked-home/cache"), "a symlink");
    let elsewhere = scratch.join("elsewhere");
    dirs(&[&elsewhere]);
    symlink(&elsewhere, real.join("linked-cache")).unwrap();
    refused(&real, &real.join("linked-cache"), "a symlink");
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
    // A private HOME and cache under an ancestor others may write.
    let shared = scratch.join("shared");
    let home = shared.join("home");
    dirs(&[&shared, &home, &home.join("cache")]);
    mode(&shared, 0o777);
    refused(&home, &home.join("cache"), "shared is writable by others");
    // The same HOME and cache, once nothing above them is open: a store.
    mode(&shared, 0o700);
    let store = home.join("cache/romacs/worlds");
    assert_eq!(outcome(child_of(ANCESTORS, "launch", &store).env("HOME", &home).env("XDG_CACHE_HOME", home.join("cache"))), restored());
    let _ = std::fs::remove_dir_all(&scratch);
}
