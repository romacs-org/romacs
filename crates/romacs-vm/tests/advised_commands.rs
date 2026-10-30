//! An advised command stays a command: `interactive-form' and `commandp' ask
//! `oclosure-interactive-form' for an OClosure such as nadvice's advice
//! (data.c:1186, eval.c:2301), as pinned GNU 32.0.50 answers on its loadup
//! chain.  org-fold advises org-self-insert-command, every Org buffer's
//! self-insertion.
use romacs_vm::Vm;
use std::path::Path;

fn bootstrapped(root: &Path) -> Vm {
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    let lisp = root.join("gnu-lisp");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('=')
            && !name.ends_with('~') && !name.ends_with(".orig") && !name.ends_with(".rej"))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    if names.iter().any(|n| n == "obsolete") { paths.push(lisp.join("obsolete")); }
    paths.push(root.join("rlisp"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

const CASE: &str = "(progn
  (defun adv-cmd (n) (interactive \"p\") n)
  (defun adv-plain (n) n)
  (advice-add 'adv-cmd :before #'ignore)
  (advice-add 'adv-plain :before #'ignore)
  (list (commandp 'adv-cmd) (interactive-form 'adv-cmd)
        (commandp 'adv-plain) (interactive-form 'adv-plain)))";

#[test]
fn advised_commands_match_pinned_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {CASE}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let root2 = root.clone();
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root2);
        vm.eval_source(&format!("(prin1-to-string {CASE})")).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
    }).unwrap().join().unwrap();
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
}
