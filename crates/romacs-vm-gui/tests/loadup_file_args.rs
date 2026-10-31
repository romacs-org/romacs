//! --loadup's file arguments are visited as startup.el's
//! `process-file-arg' visits them, through GNU's `find-file-noselect':
//! uniquify names two notes.org by their directories and each file joins
//! `file-name-history', as pinned GNU does for `emacs a/notes.org
//! b/notes.org'.
#[path = "../src/loadup.rs"] mod loadup;
#[path = "../src/eval_last_sexp.rs"] mod eval_last_sexp;
#[path = "../src/startup.rs"] mod startup;
#[path = "../src/command.rs"] mod command;
#[path = "../src/presentation.rs"] mod presentation;
#[path = "../src/native.rs"] mod native;
#[path = "../src/echo.rs"] mod echo;
use romacs_vm::Vm;

#[allow(dead_code)]
fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    vm.frame_snapshot(width, height)
}

#[test]
fn loadup_file_arguments_visit_through_gnus_find_file() {
    let root = std::env::temp_dir().join(format!("romacs-loadup-args-{}", std::process::id()));
    let files: Vec<std::path::PathBuf> = ["a", "b"].iter().map(|dir| {
        std::fs::create_dir_all(root.join(dir)).unwrap();
        let file = root.join(dir).join("notes.org");
        std::fs::write(&file, format!("* {dir}\n")).unwrap();
        file
    }).collect();
    let quoted: Vec<String> = files.iter().map(|f| format!("{:?}", f.to_str().unwrap())).collect();
    // The visited buffers' names, and `file-name-history''s head.
    let (a, b) = (&quoted[0], &quoted[1]);
    let report = format!("(list (buffer-name (get-file-buffer {a})) (buffer-name (get-file-buffer {b}))
      (equal (mapcar (quote expand-file-name) (list (car file-name-history) (cadr file-name-history))) (list {b} {a})))");
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch"]).args(&files).args(["--eval", &format!("(princ (prin1-to-string {report}))")])
        .stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert_eq!(gnu, r#"("notes.org<a>" "notes.org<b>" t)"#);
    let mine = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        let mut started = startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        started.files = files;
        startup::finish_loadup(&mut vm, &started).unwrap();
        vm.eval_source(&report).unwrap().display().unwrap()
    }).unwrap().join().unwrap();
    assert_eq!(mine, gnu);
}
