use romacs_vm::Vm;
fn answer(vm:&mut Vm,source:&str)->Result<String,String>{vm.eval_source(source).and_then(|v|v.display())}
#[test]
fn menu_mode_noninteractive_arguments_enable_or_explicitly_toggle_truth() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(menu-bar-mode 1) (list (menu-bar-mode) (menu-bar-mode nil))"),Ok("(t t)".into()));
    assert_eq!(answer(&mut vm,"(setq menu-bar-mode 'enabled) (menu-bar-mode 'toggle)"),Ok("nil".into()));
    assert_eq!(answer(&mut vm,"(list (menu-bar-mode 0.5) (menu-bar-mode 'anything))"),Ok("(nil t)".into()));
}
#[test]
fn menu_and_library_search_use_current_buffer_variable_values() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(menu-bar-mode -1) (make-local-variable 'menu-bar-mode) (list (menu-bar-mode 1) menu-bar-mode (progn (set-buffer (get-buffer-create \"s124-local-menu-other\")) menu-bar-mode))"),Ok("(t t nil)".into()));
    let dir=std::env::temp_dir().join(format!("s124-local-library-{}",std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();std::fs::write(dir.join("local-only.el"),"(error \"must not load\")").unwrap();
    let path=format!("{:?}",dir.to_string_lossy());
    let expected=format!("{:?}",dir.join("local-only.el").to_string_lossy());
    assert_eq!(answer(&mut vm,&format!("(make-local-variable 'load-path) (set 'load-path (list {path})) (locate-library \"local-only\")")),Ok(expected));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn let_uses_existing_local_else_default_without_creating_local() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(let ((truncate-lines t)) (with-temp-buffer truncate-lines))"),Ok("t".into()));
    assert_eq!(answer(&mut vm,"truncate-lines"),Ok("nil".into()));
    assert_eq!(answer(&mut vm,"(defvar-local s124-bind-auto nil) (let ((s124-bind-auto 'bound)) (set 's124-bind-auto 'changed)) s124-bind-auto"),Ok("nil".into()));
    assert_eq!(answer(&mut vm,"(defvar-local s124-auto nil) (let ((s124-auto 'default-bound)) (with-temp-buffer s124-auto))"),Ok("default-bound".into()));
    assert_eq!(answer(&mut vm,"(setq truncate-lines 'local) (list (let ((truncate-lines 'bound)) (list truncate-lines (with-temp-buffer truncate-lines))) truncate-lines)"),Ok("((bound nil) local)".into()));
    assert_eq!(answer(&mut vm,"(condition-case e (let ((truncate-lines 'broken)) (error \"unwind\")) (error truncate-lines))"),Ok("local".into()));
    vm.set_collection_floor(1);
    let collections=vm.heap_stats().collections;
    assert_eq!(answer(&mut vm,"(dotimes (i 80) (copy-sequence \"dead-before-dynamic-owner\")) (set-buffer (get-buffer-create \"s124-dynamic-gc\")) (defvar-local s124-bind-gc nil) (let ((s124-bind-gc (list 'bound))) (with-temp-buffer (dotimes (i 20000) (copy-sequence \"churn\"))) (set 's124-bind-gc (list 'changed))) s124-bind-gc"),Ok("nil".into()));
    assert!(vm.heap_stats().collections>collections);
}
#[test]
fn ascii_unibyte_buffer_names_share_identity_with_text_names() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(switch-to-buffer \"foo\") (eq (get-buffer (unibyte-string 102 111 111)) (current-buffer))"),Ok("t".into()));
    assert_eq!(answer(&mut vm,"(eq (switch-to-buffer (unibyte-string 98 97 114)) (get-buffer \"bar\"))"),Ok("t".into()));
}
