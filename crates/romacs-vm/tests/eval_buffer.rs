use romacs_vm::{Fixed, Vm};

fn expect(vm: &mut Vm, form: &str, printed: &str) {
    let result = vm.eval_source(form).and_then(|v| v.display());
    assert_eq!(result, Ok(printed.to_owned()), "{form}");
}

#[test]
fn get_buffer_lookup_identity_and_absence() {
    let mut vm = Vm::new();
    expect(&mut vm, "(get-buffer \"absent-s124\")", "nil");
    expect(
        &mut vm,
        "(eq (get-buffer (current-buffer)) (current-buffer))",
        "t",
    );
    expect(
        &mut vm,
        "(eq (get-buffer \"*scratch*\") (current-buffer))",
        "t",
    );
    assert!(vm.eval_source("(get-buffer 42)").is_err());
}

#[test]
fn switch_to_buffer_creates_and_preserves_named_buffers() {
    let mut vm = Vm::new();
    expect(
        &mut vm,
        "(eq (switch-to-buffer \"s124-a\") (current-buffer))",
        "t",
    );
    expect(
        &mut vm,
        "(progn (insert \"alpha\") (switch-to-buffer \"s124-b\") (insert \"beta\") (switch-to-buffer \"s124-a\" t t) (buffer-string))",
        "\"alpha\"",
    );
    expect(
        &mut vm,
        "(eq (switch-to-buffer (get-buffer \"s124-b\")) (current-buffer))",
        "t",
    );
    assert!(vm.eval_source("(switch-to-buffer 42)").is_err());
}

#[test]
fn erase_buffer_widens_and_moves_markers() {
    let mut vm = Vm::new();
    expect(
        &mut vm,
        "(progn (insert \"abcδεf\") (narrow-to-region 2 4) (erase-buffer))",
        "nil",
    );
    expect(
        &mut vm,
        "(list (point) (point-min) (point-max) (buffer-size) (buffer-string))",
        "(1 1 1 0 \"\")",
    );
}

#[test]
fn forward_line_unicode_boundaries_and_shortages() {
    let mut vm = Vm::new();
    expect(
        &mut vm,
        "(progn (insert \"αβ\ncd\nend\") (goto-char 2) (forward-line))",
        "0",
    );
    expect(&mut vm, "(point)", "4");
    expect(&mut vm, "(forward-line 9)", "7");
    expect(&mut vm, "(point)", "10");
    expect(&mut vm, "(forward-line -9)", "-7");
    expect(&mut vm, "(point)", "1");
    expect(
        &mut vm,
        "(progn (goto-char 5) (forward-line 0) (point))",
        "4",
    );
    expect(
        &mut vm,
        "(progn (narrow-to-region 2 6) (goto-char 2) (forward-line nil) (point))",
        "4",
    );
}

#[test]
fn modes_and_update_data_model() {
    let mut vm = Vm::new();
    expect(
        &mut vm,
        "(progn (emacs-lisp-mode) major-mode)",
        "emacs-lisp-mode",
    );
    expect(&mut vm, "(progn (menu-bar-mode 1) menu-bar-mode)", "t");
    expect(&mut vm, "(progn (menu-bar-mode -1) menu-bar-mode)", "nil");
    expect(&mut vm, "(progn (menu-bar-mode) menu-bar-mode)", "t");
    expect(&mut vm, "(force-mode-line-update)", "nil");
    expect(&mut vm, "(force-mode-line-update 'all)", "all");
}

#[test]
fn daemon_truth_and_initializer_refusal() {
    let mut vm = Vm::new();
    expect(&mut vm, "(daemonp)", "nil");
    let error = vm.eval_source("(daemon-initialized)").unwrap_err();
    assert!(error.contains("daemon"), "{error}");
}

#[test]
fn locate_library_searches_without_loading() {
    let mut vm = Vm::new();
    expect(&mut vm, "(locate-library \"s124-does-not-exist\")", "nil");
    let dir = std::env::var_os("TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("s124-buffer-library-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("fixture.el"), "(setq s124-loaded t)").unwrap();
    vm.set_load_path(&[dir.clone()]).unwrap();
    let expected = format!("{:?}", dir.join("fixture.el").to_string_lossy());
    expect(&mut vm, "(locate-library \"fixture\")", &expected);
    expect(&mut vm, "(boundp 's124-loaded)", "nil");
    expect(&mut vm, "(locate-library \"fixture\" t)", "nil");
    expect(&mut vm, "(locate-library \"fixture.el\" t)", &expected);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn named_buffer_roots_survive_counted_collection() {
    let mut vm = Vm::new();
    expect(
        &mut vm,
        "(progn (switch-to-buffer \"s124-root\") (insert \"kept\"))",
        "nil",
    );
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(vm.heap_stats().collections > before);
    expect(
        &mut vm,
        "(eq (get-buffer \"s124-root\") (current-buffer))",
        "t",
    );
    expect(
        &mut vm,
        "(progn (switch-to-buffer \"s124-other\") (switch-to-buffer \"s124-root\") (buffer-string))",
        "\"kept\"",
    );
    let _ = Fixed::NIL;
}
