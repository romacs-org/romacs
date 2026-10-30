//! World build mode's read trace (DESIGN-M1 §6, §13 step 2): every host
//! query of a recipe is recorded with its site and classified, a per-process
//! fact read outside startup refuses, and an effect refuses and fails.
use romacs_vm::Vm;
use romacs_vm::host::{self, Class, Fact, Pinned, Site};
use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

const PINNED: Pinned = Pinned { clock: UNIX_EPOCH, pid: 4242, seed: [1, 2, 3, 4] };

/// A fresh directory for test NAME.
fn scratch(name: &str) -> PathBuf {
    // libtest runs a binary's tests as threads of one process: the pid
    // alone would let two tests share, and remove, one directory.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("romacs-world-trace-{}-{unique}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    path.to_owned()
}

fn lisp(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

/// Run FILES in a fresh VM under build mode: the trace.
fn build(files: &[PathBuf]) -> (Vm, Json) {
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    for file in files {
        vm.load_file(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    }
    (vm, host::leave_build().expect("a build was recording"))
}

fn items<'a>(trace: &'a Json, list: &str, kind: &str) -> Vec<&'a Json> {
    trace[list].as_array().unwrap().iter().filter(|r| r["kind"] == kind).collect()
}

fn refusals(trace: &Json) -> Vec<(String, usize)> {
    trace["refusals"].as_array().unwrap().iter()
        .map(|r| (r["what"].as_str().or(r["kind"].as_str()).unwrap().to_owned(), r["site"]["line"].as_u64().unwrap_or(0) as usize))
        .collect()
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap()
}

#[test]
fn class_is_the_closed_p_set() {
    for name in host::P_ENV {
        assert_eq!(host::class(Fact::Env(name.as_bytes()), None), Class::P, "{name}");
    }
    for name in ["HOME", "SHELL", "USER", "LOGNAME", "LANG", "LC_ALL", "TZ", "XDG_CONFIG_HOME", "ROMACS_RUNTIME_ROOT", "UNLISTED"] {
        assert_eq!(host::class(Fact::Env(name.as_bytes()), None), Class::K, "{name}");
    }
    for fact in [Fact::Environment, Fact::CurrentDir, Fact::Pid, Fact::Clock, Fact::Monotonic, Fact::Seed] {
        assert_eq!(host::class(fact, None), Class::P, "{fact:?}");
    }
    assert_eq!(host::class(Fact::Other, None), Class::K);
    assert_eq!(host::class(Fact::Effect, None), Class::F);
    let site = |file: &str, head: &str| Site { file: file.into(), line: 79, head: head.into() };
    let baked = site("/x/gnu-lisp/version.el", "(defconst emacs-build-time (if emacs-build-system");
    assert_eq!(host::class(Fact::Clock, Some(&baked)), Class::B);
    assert_eq!(host::class(Fact::Pid, Some(&baked)), Class::P);
    assert_eq!(host::class(Fact::Clock, Some(&site("/x/gnu-lisp/version.el", "(defconst emacs-build-number 1"))), Class::P);
    assert_eq!(host::class(Fact::Clock, Some(&site("/x/other.el", "(defconst emacs-build-time"))), Class::P);
}

#[test]
fn p_reads_outside_startup_refuse_with_their_site() {
    let dir = scratch("p-reads");
    let file = write(&dir.join("preload.el"), "(defvar x1 (getenv-internal \"TMPDIR\"))\n(defvar x2 (getenv-internal \"PWD\" initial-environment))\n(defvar x3 (emacs-pid))\n(defvar x4 (current-time))\n(defvar x5 (getenv-internal \"HOME\"))\n");
    let (mut vm, trace) = build(&[file.clone()]);
    let refused = refusals(&trace);
    for (what, line) in [("env TMPDIR (process)", 1), ("env PWD (explicit)", 2), ("pid", 3), ("clock", 4)] {
        assert!(refused.contains(&(what.into(), line)), "{what} at line {line}: {refused:?}");
    }
    let site = &trace["refusals"][0]["site"];
    assert_eq!(site["file"], json!(file.to_string_lossy()));
    assert!(site["form"].as_str().unwrap().starts_with("(defvar x1"), "{site}");
    assert!(!refused.iter().any(|(what, _)| what.contains("HOME")), "HOME is keyed: {refused:?}");
    // The answers are the pinned ones.
    assert_eq!(eval(&mut vm, "x3"), "4242");
    assert_eq!(eval(&mut vm, "(time-convert x4 'integer)"), "0");
}

#[test]
fn startup_reads_p_facts_and_only_the_bootstrap_switches() {
    let dir = scratch("startup");
    let bootstrap = write(&dir.join("rlisp/romacs-loadup-bootstrap.el"), "(unwind-protect\n    (progn (romacs--startup-phase t) (setq s1 (getenv-internal \"TMPDIR\")) (setq s2 (emacs-pid)))\n  (romacs--startup-phase nil))\n(setq s3 (getenv-internal \"TERM\"))\n(unwind-protect (romacs--startup-phase t) (romacs--startup-phase nil))\n");
    let other = write(&dir.join("other.el"), "(romacs--startup-phase t)\n(setq s4 (getenv-internal \"PATH\"))\n");
    let (_, trace) = build(&[bootstrap, other]);
    let allowed: Vec<_> = items(&trace, "records", "p-read").iter().map(|r| (r["what"].as_str().unwrap(), r["allowed"].as_str().unwrap())).collect();
    assert!(allowed.contains(&("env TMPDIR (process)", "startup")) && allowed.contains(&("pid", "startup")), "{allowed:?}");
    let refused = refusals(&trace);
    assert!(refused.contains(&("env TERM (process)".into(), 4)), "{refused:?}");
    // A second run, and a switch from anywhere else, refuse; neither enters startup.
    assert!(refused.contains(&("startup".into(), 5)) && refused.contains(&("startup".into(), 1)), "{refused:?}");
    assert!(refused.contains(&("env PATH (process)".into(), 2)), "{refused:?}");
    assert_eq!(items(&trace, "records", "startup").len(), 2, "{trace}");
}

/// A re-seed form may read its own P fact or fenced variable, and only that;
/// its variable is fenced once the form ends (root 4).
#[test]
fn a_reseed_form_reads_its_fact_and_fences_its_variable() {
    let dir = scratch("reseed");
    let extra = write(&dir.join("emacs-lisp/cl-extra.el"),
                      "(defvar cl--random-state (current-time))\n(defvar cl-other (current-time))\n");
    let project = write(&dir.join("progmodes/project.el"),
                        "(defcustom project-list-file (expand-file-name \"p\") \"\" :type 'file)\n");
    let after = write(&dir.join("after.el"), "(defvar seen (car cl--random-state))\n(defvar seen2 project-list-file)\n");
    let (_, trace) = build(&[extra, project, after]);
    let reseeds: Vec<_> = items(&trace, "records", "reseed").iter().map(|r| r["reads"].as_str().unwrap()).collect();
    assert_eq!(reseeds, ["default-directory"], "{trace}");
    let allowed: Vec<_> = items(&trace, "records", "p-read").iter().filter_map(|r| r["allowed"].as_str()).filter(|a| *a != "init").collect();
    assert_eq!(allowed, ["reseed"], "{trace}");
    let fenced: Vec<_> = items(&trace, "refusals", "fence").iter()
        .map(|r| (r["root"].as_u64().unwrap(), r["symbol"].as_str().unwrap(), r["site"]["line"].as_u64().unwrap())).collect();
    assert_eq!(fenced, [(4, "cl--random-state", 1), (4, "project-list-file", 2)], "{trace}");
    assert_eq!(refusals(&trace).iter().filter(|(what, _)| what == "clock").count(), 1, "the second clock read refuses: {trace}");
}

/// Each `romacs--world-reseed' form restates its pinned GNU source.
#[test]
fn reseed_forms_restate_their_gnu_sources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bootstrap = std::fs::read_to_string(root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    for (file, source, restated) in [
        ("emacs-lisp/cl-extra.el", "(defvar cl--random-state (cl--make-random-state (cl--random-time)))",
         "(setq cl--random-state (cl--make-random-state (cl--random-time)))"),
        ("progmodes/project.el", "(defcustom project-list-file", "(custom-reevaluate-setting 'project-list-file)"),
        ("subr.el", "(defvar user-emacs-directory\n  ;; The value does not matter since Emacs sets this at startup.\n  nil", "(let ((user-emacs-directory nil))"),
        ("emacs-lisp/find-func.el",
         "(defvar find-function-C-source-directory\n  (let ((dir (expand-file-name \"src\" source-directory)))\n    (if (file-accessible-directory-p dir) dir))",
         "(let ((dir (expand-file-name \"src\" source-directory)))\n             (if (file-accessible-directory-p dir) dir))"),
    ] {
        let gnu = std::fs::read_to_string(root.join("gnu-lisp").join(file)).unwrap();
        assert!(gnu.contains(source), "{file} no longer has {source}");
        assert!(bootstrap.contains(&format!("\"{file}\"")) && bootstrap.contains(restated), "the re-seed of {file}");
    }
}

#[test]
fn the_build_time_is_baked_at_its_one_site() {
    let dir = scratch("baked");
    let version = write(&dir.join("version.el"), "(defconst emacs-build-time (current-time))\n(defconst emacs-other-time (current-time))\n");
    let (mut vm, trace) = build(&[version]);
    let reads: Vec<_> = items(&trace, "records", "p-read").iter().map(|r| r["allowed"].as_str().unwrap()).filter(|a| *a != "init").collect();
    assert_eq!(reads, ["baked"], "{trace}");
    assert_eq!(refusals(&trace), [("clock".into(), 2)]);
    assert_eq!(eval(&mut vm, "(time-convert emacs-build-time 'integer)"), "0");
}

#[test]
fn keyed_environment_reads_record_form_and_value() {
    let dir = scratch("env");
    let file = write(&dir.join("env.el"), "(getenv-internal \"HOME\")\n(getenv-internal \"ROMACS_WORLD_TRACE_ABSENT\")\n(getenv-internal \"HOME\" '(\"HOME=/elsewhere\"))\n(getenv-internal \"HOME\" initial-environment)\n(let ((default-directory \"/\")) (expand-file-name \"~\"))\n(user-login-name)\n");
    let (_, trace) = build(&[file]);
    let home = std::env::var("HOME").unwrap();
    let env: Vec<_> = items(&trace, "records", "env").iter()
        .map(|r| (r["form"].as_str().unwrap().to_owned(), r["name"].as_str().unwrap().to_owned(), r["value"].clone(), r["site"]["line"].as_u64()))
        .collect();
    for (form, name, value, line) in [
        ("process", "HOME", json!(home), Some(1)),
        ("process", "ROMACS_WORLD_TRACE_ABSENT", Json::Null, Some(2)),
        // An explicit list records the launch value, not the list's.
        ("explicit", "HOME", json!(home), Some(3)),
        ("explicit", "HOME", json!(home), Some(4)),
        ("process", "HOME", json!(home), Some(5)),
    ] {
        assert!(env.contains(&(form.into(), name.into(), value.clone(), line)), "{form} {name} {line:?}: {env:?}");
    }
    assert!(env.iter().any(|(form, name, _, _)| form == "initial" && name == "LOGNAME"), "UserIdentity: {env:?}");
    // Only evaluating the fenced `initial-environment' as an argument refuses.
    let refused: Vec<_> = trace["refusals"].as_array().unwrap().iter().map(|r| (r["kind"].as_str().unwrap(), r["symbol"].as_str(), r["site"]["line"].as_u64())).collect();
    assert_eq!(refused, [("fence", Some("initial-environment"), Some(4))], "{trace}");
}

#[test]
fn machine_and_user_facts_are_host_records() {
    let dir = scratch("host");
    let file = write(&dir.join("host.el"), "(system-name)\n(user-uid)\n(user-real-uid)\n(group-gid)\n(group-real-gid)\n(num-processors)\ninvocation-name\n");
    let (_, trace) = build(&[file]);
    let facts: Vec<_> = items(&trace, "records", "host").iter().map(|r| r["fact"].as_str().unwrap().to_owned()).collect();
    for fact in ["system-name", "euid", "uid", "egid", "gid", "exe"] {
        assert!(facts.iter().any(|f| f == fact), "{fact}: {facts:?}");
    }
    assert!(facts.iter().any(|f| f.starts_with("processors")), "{facts:?}");
}

#[test]
fn file_reads_probes_and_canonical_names_are_recorded() {
    let dir = scratch("files");
    let data = write(&dir.join("data.txt"), "world\n");
    std::os::unix::fs::symlink(&data, dir.join("link")).unwrap();
    let file = write(&dir.join("files.el"), &format!(
        "(with-temp-buffer (insert-file-contents {data}))\n(condition-case nil (with-temp-buffer (insert-file-contents {missing})) (file-missing nil))\n(file-exists-p {missing})\n(file-symlink-p {link})\n",
        data = lisp(&data), missing = lisp(&dir.join("missing")), link = lisp(&dir.join("link"))));
    let (_, trace) = build(&[file]);
    let reads = items(&trace, "records", "file-read");
    let read = |path: &Path| reads.iter().find(|r| r["path"] == json!(path.to_string_lossy())).map(|r| r["result"].clone());
    let digest = "e258d248fda94c63753607f7c4494ee0fcbe92f1a76bfdac795c9d84101eb317";
    assert_eq!(read(&data), Some(json!({"len": 6, "sha256": digest})), "{reads:?}");
    assert_eq!(read(&dir.join("missing")), Some(json!({"errno": libc::ENOENT})), "{reads:?}");
    let probes = items(&trace, "records", "probe");
    assert!(probes.iter().any(|p| p["path"] == json!(dir.join("missing").to_string_lossy()) && p["result"]["errno"] == json!(libc::ENOENT)), "{probes:?}");
    let canon = items(&trace, "records", "canon");
    let link = probes.iter().chain(&canon).any(|r| r["path"] == json!(dir.join("link").to_string_lossy())
        && (r["result"] == json!(data.to_string_lossy())));
    assert!(link, "the symlink's target is recorded: {probes:?} {canon:?}");
}

#[test]
fn a_charset_map_records_the_missing_map_and_the_text() {
    let dir = scratch("charset");
    write(&dir.join("TESTMAP.txt"), "0x00-0x7F 0x0000\n");
    let file = write(&dir.join("charset.el"), &format!(
        "(progn (setq charset-map-path (list {dir})) (define-charset-internal 'world-trace-test nil [0 127 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil \"TESTMAP\" nil nil nil nil) (decode-char 'world-trace-test 65))\n",
        dir = lisp(&dir)));
    let (_, trace) = build(&[file]);
    let reads: Vec<_> = items(&trace, "records", "file-read").iter().map(|r| (r["path"].as_str().unwrap().to_owned(), r["result"].clone())).collect();
    let map = reads.iter().position(|(p, r)| p.ends_with("TESTMAP.map") && r["errno"] == json!(libc::ENOENT));
    let text = reads.iter().position(|(p, r)| p.ends_with("TESTMAP.txt") && r["len"].is_u64());
    assert!(map.is_some() && map < text, "{reads:?}");
}

#[test]
fn listings_record_names_in_order_and_how_they_ended() {
    let dir = scratch("listing");
    for name in ["a", "b", "c", "d"] {
        write(&dir.join("d").join(name), "");
    }
    let listed = dir.join("d");
    let order: Vec<String> = std::fs::read_dir(&listed).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    let file = write(&dir.join("list.el"), &format!("(directory-files {d} nil nil t)\n(directory-files {d} nil nil t 2)\n", d = lisp(&listed)));
    let (_, trace) = build(&[file]);
    let lists: Vec<_> = items(&trace, "records", "dir-list").iter().map(|r| (r["site"]["line"].as_u64().unwrap(), r["names"].clone(), r["terminal"].clone())).collect();
    assert!(lists.contains(&(1, json!(order), json!("end"))), "{lists:?} {order:?}");
    let (_, names, terminal) = lists.iter().find(|(line, ..)| *line == 2).expect("COUNT listing");
    let names: Vec<String> = serde_json::from_value(names.clone()).unwrap();
    assert!(order.starts_with(&names) && (terminal == "stopped" || terminal == "end"), "{lists:?}");
}

#[test]
fn effects_refuse_and_fail() {
    let dir = scratch("effects");
    let out = dir.join("out");
    let file = write(&dir.join("effects.el"), &format!(
        "(setq w (condition-case e (write-region \"x\" nil {out}) (file-error (car e))))\n(setq p (condition-case e (process-lines \"true\") (error e)))\n(setq j (condition-case e (romacs--process-start '(\"true\") \"\" \"/\" 5) (error e)))\n", out = lisp(&out)));
    let (mut vm, trace) = build(&[file]);
    assert!(!out.exists());
    // An asynchronous job refuses as it is submitted, not on its worker.
    assert_eq!(eval(&mut vm, "(list (and w t) (and p t) (consp j))"), "(t t t)");
    let refused: Vec<_> = items(&trace, "refusals", "effect").iter().map(|r| (r["what"].as_str().unwrap().to_owned(), r["site"]["line"].as_u64().unwrap())).collect();
    assert!(refused.iter().any(|(what, line)| what.starts_with("write ") && *line == 1), "{refused:?}");
    assert!(refused.iter().any(|(what, line)| what == "run true" && *line == 2), "{refused:?}");
    assert!(refused.iter().any(|(what, line)| what == "run true" && *line == 3), "{refused:?}");
}

#[test]
fn off_mode_answers_the_host() {
    assert_eq!(host::mode(), host::Mode::Off);
    let mut vm = Vm::new();
    vm.world_begin();
    assert_eq!(eval(&mut vm, "(emacs-pid)"), std::process::id().to_string());
    assert_eq!(eval(&mut vm, "(romacs--startup-phase t)"), "nil");
    let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let lisp_now: u64 = eval(&mut vm, "(time-convert nil 'integer)").parse().unwrap();
    assert!(Duration::from_secs(lisp_now) + Duration::from_secs(5) >= now);
    assert_eq!(host::leave_build(), None);
}

#[test]
fn confinement_fails_and_lists_a_read_outside_the_roots() {
    let dir = scratch("confined");
    let outside = write(&dir.join("outside/data.txt"), "x");
    let file = write(&dir.join("roots/read.el"), &format!(
        "(setq r (condition-case nil (with-temp-buffer (insert-file-contents {out}) 'read) (file-error 'failed)))\n", out = lisp(&outside)));
    // The pinned table's directories, empty and read-only, as the runner
    // makes them, and a directory outside the roots: a case query measures
    // each one's filesystem without reading beneath it.
    let store = dir.join("out.json.d");
    for name in ["cwd", "tmp", "bin"] {
        std::fs::create_dir_all(store.join("p").join(name)).unwrap();
    }
    let probe = write(&dir.join("roots/probe.el"), &format!("(setq c (list (file-name-case-insensitive-p {}) (file-name-case-insensitive-p {})))\n",
                                                             lisp(&store.join("p/cwd/x")), lisp(&dir.join("outside/x"))));
    let roots = romacs_vm::world::sandbox::build_roots(vec![dir.join("roots")], &store);
    romacs_vm::world::sandbox::confine(&roots, &[]).expect("Landlock");
    assert_eq!(std::fs::read(&outside).map_err(|e| e.raw_os_error()), Err(Some(libc::EACCES)));
    assert_eq!(std::fs::read_dir(dir.join("outside")).map(drop).map_err(|e| e.raw_os_error()), Err(Some(libc::EACCES)));
    // Its flags still answer, read outside the thread's domain.
    assert_ne!(host::directory_flags(&dir.join("outside")).map_err(|e| e.raw_os_error()), Err(Some(libc::EACCES)));
    let (mut vm, trace) = build(&[file, probe]);
    assert_eq!(eval(&mut vm, "(list r c)"), "(failed (nil nil))");
    let confined: Vec<_> = items(&trace, "refusals", "confined").iter().map(|r| r["path"].as_str().unwrap().to_owned()).collect();
    assert_eq!(confined, [outside.to_string_lossy()], "{trace}");
}

#[test]
fn an_input_serves_the_bytes_its_record_describes() {
    let dir = scratch("captured");
    let path = write(&dir.join("data.txt"), "first\n");
    host::enter_build(PINNED);
    let mut input = host::open_input(&path, 0).unwrap();
    // Rewritten in place after the open: what is read is what was recorded.
    std::fs::write(&path, "second content\n").unwrap();
    let head = input.read_range(0, 3).unwrap();
    let rest = input.read_range(3, 100).unwrap();
    let trace = host::leave_build().unwrap();
    assert_eq!([head, rest].concat(), b"first\n");
    let reads = items(&trace, "records", "file-read");
    assert_eq!(reads.len(), 1, "{trace}");
    assert_eq!(reads[0]["result"], json!({"len": 6, "sha256": "b640e840b19d378660b32fb51ae18d67dccb4a8596a29e7bd72c1b2ae5928f41"}), "{trace}");
}

#[test]
fn records_keep_names_that_are_not_utf8_distinct() {
    use std::os::unix::ffi::OsStrExt;
    let dir = scratch("bytes");
    let names = [b"n\xff".as_slice(), b"n\xfe".as_slice()];
    for name in names {
        write(&dir.join(std::ffi::OsStr::from_bytes(name)), "");
    }
    host::enter_build(PINNED);
    let listed: Vec<Vec<u8>> = host::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().as_bytes().to_vec()).collect();
    host::probe(host::Probe::Exists, dir.join(std::ffi::OsStr::from_bytes(names[0])));
    let trace = host::leave_build().unwrap();
    let list = items(&trace, "records", "dir-list");
    let recorded: Vec<Json> = serde_json::from_value(list[0]["names"].clone()).unwrap();
    assert_eq!(recorded, listed.iter().map(|name| host::encoded(name)).collect::<Vec<_>>());
    assert_ne!(recorded[0], recorded[1]);
    assert_eq!(recorded.iter().filter(|n| **n == json!({"hex": "6eff"})).count(), 1, "{recorded:?}");
    let probe = items(&trace, "records", "probe");
    let mut path = dir.as_os_str().as_bytes().to_vec();
    path.extend(b"/n\xff");
    assert_eq!(probe[0]["path"], host::encoded(&path), "{probe:?}");
}

#[test]
fn a_case_query_records_the_filesystem_facts_restore_asks_again() {
    let dir = scratch("case");
    std::fs::create_dir_all(dir.join("d")).unwrap();
    // /dev/shm is a tmpfs, whose directories carry a casefold flag.
    let probe = write(&dir.join("probe.el"), &format!("(setq c (list (file-name-case-insensitive-p {}) (file-name-case-insensitive-p \"/dev/shm/\")))\n",
                                                       lisp(&dir.join("d/x"))));
    let (mut vm, trace) = build(&[probe]);
    assert_eq!(eval(&mut vm, "c"), "(nil nil)");
    let facts: Vec<&Json> = items(&trace, "records", "probe").into_iter()
        .filter(|r| r["probe"] == "fs-type" || r["probe"] == "dir-flags").collect();
    let kinds: Vec<&str> = facts.iter().map(|r| r["probe"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["fs-type", "fs-type", "dir-flags"], "{trace}");
    // The directory changes after the build, as HOME and TMPDIR do: the
    // query keyed its type, not its times, so every record still holds.
    write(&dir.join("d/new"), "");
    let records = trace["records"].as_array().unwrap();
    for record in records.iter().filter(|r| host::keyed(r)) {
        let mut expected = record.clone();
        expected.as_object_mut().unwrap().remove("site");
        assert_eq!(host::replay(record), Some(expected));
    }
}
