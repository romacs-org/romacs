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
const CONCURRENT: &str = "concurrent_first_launches_share_one_builder_and_say_they_wait";
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
            let flag = romacs_vm::host::read_to_string(input.join("flag")).unwrap_or_default();
            if flag.trim() == "fail" {
                let error = world.abandon_build("the input says fail".into());
                println!("WORLD-IMAGE {}", json!({"mode": "failed", "error": error}));
                std::process::exit(1);
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
            let len = match world.finish_build(vm, 42u64, json!({})) {
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
    // The whole store goes with `all`, and only the store's own files.
    std::fs::write(store.join("unrelated"), "kept").unwrap();
    assert!(romacs_vm::world::store::reset(&store, true).unwrap() > 0);
    assert_eq!(std::fs::read_dir(&store).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(), ["unrelated"]);
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
