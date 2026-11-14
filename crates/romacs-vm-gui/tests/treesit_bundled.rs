//! The grammars the executable ships (crates/romacs-grammars, DIVERGENCES
//! VM2-TREESIT-BUNDLED): each is the commit its GNU ts mode pins in
//! `treesit-language-source-alist', and the loader finds it after the
//! user's grammar directories and before the system's, never for a
//! language `treesit-load-name-override-list' names.
use romacs_vm_gui::grammars;
use romacs_vm::Vm;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A VM on Romacs's loadup chain (tests/advised_commands.rs's), with the
/// bundled grammars and a `user-emacs-directory' of its own.
fn bootstrapped(user: &Path) -> Vm {
    grammars::register();
    let root = root();
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    let lisp = root.join("lisp/gnu");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('='))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    paths.push(root.join("lisp/romacs"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("lisp/romacs/romacs-loadup-bootstrap.el")).unwrap();
    vm.eval_source(&format!("(setq user-emacs-directory {:?} treesit-extra-load-path nil)", format!("{}/", user.display()))).unwrap();
    vm
}

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(64 << 20).spawn(f).unwrap().join().unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("romacs-treesit-bundled-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// GNU's override list substitutes the library and symbol for a language,
/// so an override never falls back to the bundled grammar; the bundled
/// grammar shadows the system's (markdown, which distributions ship);
/// every bundled grammar loads, parses, and reports the executable as its
/// location. In this order: a loaded grammar is cached for the process, as
/// GNU keeps its libraries open.
#[test]
fn bundled_grammars_load_unless_overridden_and_before_the_systems() {
    let user = scratch("load");
    let report = on_big_stack(move || {
        let mut vm = bootstrapped(&user);
        let exe = std::env::current_exe().unwrap().display().to_string();
        vec![
            printed(&mut vm, "(let ((treesit-load-name-override-list '((rust \"libtree-sitter-nosuch\" \"tree_sitter_rust\"))))
                (list (treesit-language-available-p 'rust) (car (cdr (treesit-language-available-p 'rust t)))))"),
            printed(&mut vm, &format!("(equal (treesit-grammar-location 'markdown) {exe:?})")),
            printed(&mut vm, "(mapcar (lambda (g) (plist-get g :name)) (romacs-treesit-bundled-grammars))"),
            printed(&mut vm, &format!("(delq nil (mapcar (lambda (g)
                (let ((name (plist-get g :name)))
                  (unless (and (treesit-language-available-p name) (equal (treesit-grammar-location name) {exe:?})
                               (memq (plist-get g :abi) '(14 15))
                               (with-temp-buffer (insert \"x\") (treesit-node-p (treesit-parser-root-node (treesit-parser-create name)))))
                    name)))
                (romacs-treesit-bundled-grammars)))")),
            printed(&mut vm, "(with-temp-buffer (insert \"fn f() {}\")
                (string= (treesit-node-type (treesit-node-child (treesit-parser-root-node (treesit-parser-create 'rust)) 0))
                         \"function_item\"))"),
        ]
    });
    assert_eq!(report, [
        "(nil not-found)",
        "t",
        "(bash c cpp css go html javascript jsdoc json markdown markdown-inline python rust toml tsx typescript yaml)",
        "nil",
        "t",
    ]);
}

/// A library in the user's grammar directory is tried before the bundled
/// grammar: here this process's own C library under json's name, which
/// opens but has no `tree_sitter_json', so loading json reports GNU's
/// symbol-error instead of finding the bundled json. In a batch session of
/// the executable, whose grammar cache is its own.
#[test]
fn a_library_in_the_users_grammar_directory_comes_first() {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    let libc = maps.lines().filter_map(|line| line.split_whitespace().nth(5))
        .find(|path| path.ends_with("/libc.so.6")).expect("this process maps libc.so.6");
    let user = scratch("shadow");
    std::fs::create_dir_all(user.join("tree-sitter")).unwrap();
    std::os::unix::fs::symlink(libc, user.join("tree-sitter/libtree-sitter-json.so")).unwrap();
    let form = format!("(progn (setq user-emacs-directory {:?})
        (princ (list (car (cdr (treesit-language-available-p 'json t))) (treesit-language-available-p 'python))))",
        format!("{}/", user.display()));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["-Q", "--batch", "--eval", &form]).env("HOME", &user).env("ROMACS_WORLD", "off").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "(symbol-error t)");
}

/// Each bundled grammar is the repository and commit its GNU ts mode
/// names in `treesit-language-source-alist' (read from lisp/gnu/, its
/// entries evaluated as the files would).
#[test]
fn bundled_grammars_are_the_commits_gnu_pins() {
    let user = scratch("pins");
    let report = on_big_stack(move || {
        let mut vm = bootstrapped(&user);
        let files: Vec<String> = ["progmodes", "textmodes"].iter().flat_map(|dir| {
            std::fs::read_dir(root().join("lisp/gnu").join(dir)).unwrap().filter_map(|e| e.ok()).map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "el")
                    && std::fs::read_to_string(p).is_ok_and(|t| t.contains("'treesit-language-source-alist")))
                .map(|p| format!("{:?}", p.display().to_string()))
                .collect::<Vec<_>>()
        }).collect();
        printed(&mut vm, &format!("(let (recipes)
            (dolist (file (list {}))
              (with-temp-buffer
                (insert-file-contents file)
                (condition-case nil
                    (while t
                      (let ((form (read (current-buffer))))
                        (when (and (eq (car-safe form) 'add-to-list)
                                   (equal (nth 1 form) ''treesit-language-source-alist))
                          (push (eval (nth 2 form) t) recipes))))
                  (end-of-file nil))))
            (delq nil (mapcar (lambda (g)
                (let ((recipe (assq (plist-get g :name) recipes)))
                  (unless (and recipe (equal (nth 1 recipe) (plist-get g :url))
                               (equal (plist-get (nthcdr 2 recipe) :commit) (plist-get g :commit)))
                    (list (plist-get g :name) recipe))))
              (romacs-treesit-bundled-grammars))))", files.join(" ")))
    });
    assert_eq!(report, "nil");
}

/// lisp/romacs/romacs-treesit.el around `after-find-file': a visit's parse reaches
/// the mode's parser; a visit that cannot use one (unibyte, NOMODES, a file
/// whose local mode overrules the guess) neither fails nor leaves one behind.
#[test]
fn a_visit_parses_early_and_leaves_no_parse_unused() {
    let directory = scratch("visit");
    on_big_stack(move || {
        let mut vm = bootstrapped(&directory);
        let body = "int a;\n".repeat(1000);
        let write = |name: &str, text: String| {
            let path = directory.join(name);
            std::fs::write(&path, text).unwrap();
            path.display().to_string()
        };
        let (plain, literal, nomodes, local) = (write("plain.c", body.clone()), write("literal.c", body.clone()),
                                                write("nomodes.c", body.clone()), write("local.c", format!("/* -*- fundamental -*- */\n{body}")));
        let source = format!(
            "(progn (require 'treesit) (require 'c-ts-mode) (load \"romacs-treesit\") \
               (add-to-list 'major-mode-remap-alist '(c-mode . c-ts-mode)) \
               (let ((idle (lambda () (prog1 (romacs--treesit-parse-early 'c) (romacs--treesit-parse-early nil))))) \
                 (list (with-current-buffer (find-file-noselect {plain:?}) \
                         (list major-mode (treesit-node-child-count (treesit-buffer-root-node)))) \
                       (with-temp-buffer (set-buffer-multibyte nil) (insert-file-contents-literally {literal:?} t) (after-find-file nil nil nil nil t) \
                         (list enable-multibyte-characters major-mode)) \
                       (with-temp-buffer (insert-file-contents {nomodes:?} t) (after-find-file nil nil nil nil t) \
                         (list major-mode (funcall idle))) \
                       (with-current-buffer (find-file-noselect {local:?}) \
                         (list major-mode (funcall idle))))))");
        assert_eq!(printed(&mut vm, &source), "((c-ts-mode 1000) (nil fundamental-mode) (fundamental-mode t) (fundamental-mode t))");
    });
}
