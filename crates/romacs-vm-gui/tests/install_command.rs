use romacs_vm_gui::{command, presentation, startup};
use romacs_vm::{CommandEvent, Vm};

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}
fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
    vm.set_collection_floor(1);
    startup::initialize_files(&mut vm, &[]).unwrap();
    vm
}

#[test]
fn install_command_real_global_map_dispatch() {
    let mut vm = vm();
    assert_eq!(printed(&mut vm, "(fboundp 'romacs-gui-install-command)"), "t", "INSTALL_SEAM_ENTRY_POINT");
    printed(&mut vm, "(setq global-map (make-sparse-keymap))");
    printed(&mut vm, "(romacs-gui-install-command [24 57345] '(defun romacs-gui-install-probe () (interactive) (insert \"installed\")))");
    assert_eq!(printed(&mut vm, "(lookup-key global-map [24 57345])"), "1");
    assert_eq!(printed(&mut vm, "(lookup-key (current-global-map) [24 57345])"), "romacs-gui-install-probe", "INSTALL_SEAM_REAL_MAP");
    if std::env::var_os("ROMACS_INSTALL_SEAM_MUTANT").is_some() {
        printed(&mut vm, "(define-key (current-global-map) [24 57345] nil)");
    }
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    for code in [24, 57345] {
        if let Some(outcome) = presentation.command(&mut vm, &mut commands, CommandEvent::Character(code)).unwrap() {
            assert!(outcome.signal.is_none(), "{outcome:?}");
        }
    }
    assert_eq!(printed(&mut vm, "(buffer-substring romacs-gui-scratch-start (point-max))"), "\"installed\"", "INSTALL_SEAM_PRODUCTION_DISPATCH");
    vm.collect_garbage();
    assert_eq!(printed(&mut vm, "(key-binding [24 57345])"), "romacs-gui-install-probe");
}

#[test]
fn install_command_after_gnu_loadup() {
    std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        let started = startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        startup::finish_loadup(&mut vm, &started).unwrap();
        assert_eq!(printed(&mut vm, "(lookup-key (current-global-map) [24 6])"), "find-file");
        assert!(vm.eval_source("(romacs-gui-install-command [24 6] '(defun romacs-gui-must-not-replace () (interactive) nil))").is_err());
        assert_eq!(printed(&mut vm, "(lookup-key (current-global-map) [24 6])"), "find-file");
        assert_eq!(printed(&mut vm, "(fboundp 'romacs-gui-must-not-replace)"), "nil");
        assert_eq!(printed(&mut vm, "(lookup-key (current-global-map) [24 57344])"), "nil");
        printed(&mut vm, "(romacs-gui-install-command [24 57344] '(defun romacs-gui-loadup-probe () (interactive) (insert \"loadup\")))");
        assert_eq!(printed(&mut vm, "(get 'romacs-gui-loadup-probe (intern \"#:interactive\"))"), "(nil)");
        // The startup screen is read-only; type into *scratch*.
        printed(&mut vm, "(progn (switch-to-buffer \"*scratch*\") (goto-char (point-max)) (setq romacs-test-start (point)))");
        let mut commands = command::Commands::default();
        let mut presentation = presentation::Presentation::new();
        for code in [24, 57344] {
            if let Some(outcome) = presentation.command(&mut vm, &mut commands, CommandEvent::Character(code)).unwrap() {
                assert!(outcome.signal.is_none(), "{outcome:?}");
            }
        }
        assert_eq!(printed(&mut vm, "(buffer-substring romacs-test-start (point-max))"), "\"loadup\"", "INSTALL_SEAM_LOADUP_DISPATCH");
    }).unwrap().join().unwrap();
}

#[test]
fn install_command_module_activates_during_initialize() {
    if std::env::var_os("ROMACS_INSTALL_SEAM_CHILD").is_some() {
        let mut vm = vm();
        assert_eq!(printed(&mut vm, "(key-binding [24 57345])"), "romacs-gui-module-probe", "INSTALL_SEAM_INITIALIZE_MODULE");
        let mut commands = command::Commands::default();
        let mut presentation = presentation::Presentation::new();
        for code in [24, 57345] {
            if let Some(outcome) = presentation.command(&mut vm, &mut commands, CommandEvent::Character(code)).unwrap() {
                assert!(outcome.signal.is_none(), "{outcome:?}");
            }
        }
        assert_eq!(printed(&mut vm, "(buffer-substring romacs-gui-scratch-start (point-max))"), "\"module\"", "INSTALL_SEAM_MODULE_DISPATCH");
        return;
    }
    let root = std::env::temp_dir().join(format!("romacs-install-seam-{}", std::process::id()));
    let modules = root.join("rlisp/romacs-gui-extensions");
    std::fs::create_dir_all(root.join("gnu-lisp")).unwrap();
    std::fs::create_dir_all(&modules).unwrap();
    std::fs::write(modules.join("10-setup.el"), "(setq romacs-gui-module-text \"module\")").unwrap();
    std::fs::write(modules.join("20-probe.el"), "(romacs-gui-install-command [24 57345] '(defun romacs-gui-module-probe () (interactive) (insert romacs-gui-module-text)))").unwrap();
    std::fs::write(modules.join("ignored.txt"), "(error \"must not load\")").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "install_command_module_activates_during_initialize", "--nocapture"])
        .env("ROMACS_INSTALL_SEAM_CHILD", "1")
        .env("ROMACS_RUNTIME_ROOT", &root)
        .env_remove("ROMACS_LISP_DIR")
        .output().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(output.status.success(), "INSTALL_SEAM_MODULE_CHILD: {}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn install_command_refuses_unsupported_and_existing_bindings() {
    let mut vm = vm();
    for source in [
        "(romacs-gui-install-command [24 57345] '(defun find-file () (interactive) nil))",
        "(romacs-gui-install-command [24 57345] '(defun romacs-gui-bad () nil))",
        "(romacs-gui-install-command [24 57345] '(defun romacs-gui-bad (file) (interactive) nil))",
        "(romacs-gui-install-command [24 57345] '(defun romacs-gui-bad () (interactive \"fFile: \") nil))",
        "(romacs-gui-install-command [] '(defun romacs-gui-bad () (interactive) nil))",
        "(romacs-gui-install-command [24 nope] '(defun romacs-gui-bad () (interactive) nil))",
        "(romacs-gui-install-command [14 6] '(defun romacs-gui-bad () (interactive) nil))",
        "(romacs-gui-install-command [24 57345] '(defun romacs-gui-bad () (interactive) nil) t)",
    ] {
        assert!(vm.eval_source(source).is_err(), "INSTALL_SEAM_REFUSAL: {source}");
        assert_eq!(printed(&mut vm, "(fboundp 'romacs-gui-bad)"), "nil");
    }
    printed(&mut vm, "(defun find-file () (interactive) 'gnu) (global-set-key [24 57345] 'find-file)");
    assert!(vm.eval_source("(romacs-gui-install-command [24 57345] '(defun romacs-gui-bad () (interactive) nil))").is_err());
    assert_eq!(printed(&mut vm, "(key-binding [24 57345])"), "find-file", "INSTALL_SEAM_PRESERVES_GNU");
    assert_eq!(printed(&mut vm, "(fboundp 'romacs-gui-bad)"), "nil");
}
