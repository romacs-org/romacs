//! R221: `lisp/` is GNU's Lisp byte for byte at the oracle pin.
//!
//! The guard is `tools/lib/lisp_oracle.py check`; `tools/check-derived` runs it
//! before a landing, and this test runs it inside the gate. The fixture tests
//! below prove each refusal fires on a synthetic GNU repository, so the guard
//! is known to be able to fail (a guard that has never failed is not a guard).
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn guard(root: &Path, extra: &[&str]) -> Output {
    Command::new("python3")
        .arg(repo().join("tools/lib/lisp_oracle.py"))
        .arg("check")
        .arg("--root")
        .arg(root)
        .args(extra)
        .output()
        .expect("python3 runs tools/lib/lisp_oracle.py")
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn lisp_is_the_gnu_pin_byte_for_byte() {
    let out = guard(&repo(), &[]);
    assert!(out.status.success(), "lisp/ differs from the GNU pin (R221):\n{}", text(&out));
    assert!(text(&out).contains("every shared lisp/gnu/ path is the pin blob"), "{}", text(&out));
}

/// A throwaway GNU repository holding two files, and a root that copies them.
struct Fixture {
    dir: PathBuf,
    pin: String,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("lisp-oracle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let gnu = dir.join("gnu");
        std::fs::create_dir_all(gnu.join("lisp/sub")).unwrap();
        std::fs::write(gnu.join("lisp/a.el"), ";;; a.el -*- lexical-binding: t -*-\n(provide 'a)\n").unwrap();
        std::fs::write(gnu.join("lisp/sub/b.el"), ";;; b.el -*- lexical-binding: t -*-\n(provide 'b)\n").unwrap();
        let git = |args: &[&str]| {
            let s = Command::new("git").arg("-C").arg(&gnu)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .args(args).output().unwrap();
            assert!(s.status.success(), "{}", text(&s));
            String::from_utf8(s.stdout).unwrap()
        };
        git(&["init", "-q"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "pin"]);
        let pin = git(&["rev-parse", "HEAD"]).trim().to_owned();
        let root = dir.join("root");
        std::fs::create_dir_all(root.join("lisp/gnu/sub")).unwrap();
        std::fs::copy(gnu.join("lisp/a.el"), root.join("lisp/gnu/a.el")).unwrap();
        std::fs::copy(gnu.join("lisp/sub/b.el"), root.join("lisp/gnu/sub/b.el")).unwrap();
        Fixture { dir, pin }
    }
    fn root(&self) -> PathBuf {
        self.dir.join("root")
    }
    fn check(&self) -> Output {
        let gnu = self.dir.join("gnu");
        guard(&self.root(), &["--gnu", gnu.to_str().unwrap(), "--pin", &self.pin])
    }
    fn blob(&self, path: &str) -> String {
        let s = Command::new("git").arg("-C").arg(self.dir.join("gnu"))
            .args(["rev-parse", &format!("HEAD:{path}")]).output().unwrap();
        String::from_utf8(s.stdout).unwrap().trim().to_owned()
    }
    fn overlay(&self, rel: &str, body: &str, manifest_blob: Option<&str>) {
        let o = self.root().join("lisp-overlay");
        std::fs::create_dir_all(o.join(rel).parent().unwrap()).unwrap();
        std::fs::write(o.join(rel), body).unwrap();
        if let Some(blob) = manifest_blob {
            std::fs::write(o.join("MANIFEST.tsv"), format!("{rel}\t{blob}\ta test reason\n")).unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn refused(out: &Output, needle: &str) {
    assert!(!out.status.success(), "the guard ACCEPTED a tree it must refuse ({needle}):\n{}", text(out));
    assert!(text(out).contains(needle), "refused, but not for {needle}:\n{}", text(out));
}

#[test]
fn pristine_fixture_passes() {
    let f = Fixture::new("clean");
    let out = f.check();
    assert!(out.status.success(), "{}", text(&out));
}

#[test]
fn one_changed_byte_is_refused() {
    let f = Fixture::new("byte");
    let p = f.root().join("lisp/gnu/sub/b.el");
    let mut bytes = std::fs::read(&p).unwrap();
    let before = bytes.clone();
    let i = bytes.len() - 3;
    bytes[i] ^= 0x01;
    std::fs::write(&p, &bytes).unwrap();
    assert_ne!(std::fs::read(&p).unwrap(), before, "the mutant must actually change the file");
    refused(&f.check(), "MODIFIED lisp/gnu/sub/b.el");
}

#[test]
fn an_absent_gnu_path_is_refused() {
    let f = Fixture::new("absent");
    std::fs::remove_file(f.root().join("lisp/gnu/a.el")).unwrap();
    refused(&f.check(), "ABSENT lisp/gnu/a.el");
}

/// With a repository, undeclared Lisp is refused even if ignored; ignored
/// compiled products are not inputs. ledger/lisp-pin.json must name GNU's pin/tree.
#[test]
fn with_a_repository_extras_and_a_wrong_pin_record_are_refused() {
    let f = Fixture::new("extra");
    let root = f.root();
    let git = |args: &[&str]| {
        let s = Command::new("git").arg("-C").arg(&root).args(args).output().unwrap();
        assert!(s.status.success(), "{}", text(&s));
        String::from_utf8(s.stdout).unwrap()
    };
    git(&["init", "-q"]);
    let tree = Command::new("git").arg("-C").arg(f.dir.join("gnu")).args(["rev-parse", "HEAD:lisp"]).output().unwrap();
    let tree = String::from_utf8(tree.stdout).unwrap().trim().to_owned();
    std::fs::create_dir_all(root.join("ledger")).unwrap();
    let record = |commit: &str, tree: &str| std::fs::write(root.join("ledger/lisp-pin.json"),
        format!("{{\"gnu_commit\": \"{commit}\", \"lisp_tree\": \"{tree}\"}}\n")).unwrap();
    record(&f.pin, &tree);
    std::fs::write(root.join(".gitignore"), "lisp/gnu/generated.elc\nlisp/gnu/generated.el\n").unwrap();
    std::fs::write(root.join("lisp/gnu/generated.elc"), "; compiled build product\n").unwrap();
    let out = f.check();
    assert!(out.status.success(), "an ignored build product must not count: {}", text(&out));

    std::fs::write(root.join("lisp/gnu/generated.el"), "; undeclared runtime input\n").unwrap();
    refused(&f.check(), "EXTRA lisp/gnu/generated.el");
    std::fs::remove_file(root.join("lisp/gnu/generated.el")).unwrap();

    std::fs::write(root.join("lisp/gnu/ours.el"), "(provide 'ours)\n").unwrap();
    refused(&f.check(), "EXTRA lisp/gnu/ours.el");
    std::fs::remove_file(root.join("lisp/gnu/ours.el")).unwrap();

    record(&f.pin, "0000000000000000000000000000000000000000");
    refused(&f.check(), "ledger/lisp-pin.json: lisp_tree");
    record("1111111111111111111111111111111111111111", &tree);
    refused(&f.check(), "ledger/lisp-pin.json: gnu_commit");
    std::fs::remove_file(root.join("ledger/lisp-pin.json")).unwrap();
    refused(&f.check(), "ledger/lisp-pin.json is missing");
}

#[test]
fn an_overlay_file_without_a_manifest_row_is_refused() {
    let f = Fixture::new("undeclared");
    f.overlay("a.el", ";;; a.el -*- lexical-binding: t -*-\n;; romacs-overlay: x\n(provide 'a)\n", None);
    refused(&f.check(), "UNDECLARED lisp-overlay/a.el");
}

#[test]
fn a_declared_overlay_is_accepted_and_each_defect_refused() {
    let f = Fixture::new("overlay");
    let blob = f.blob("lisp/a.el");
    let good = ";;; a.el -*- lexical-binding: t -*-\n;; romacs-overlay: a test reason\n(provide 'a)\n";
    f.overlay("a.el", good, Some(&blob));
    let out = f.check();
    assert!(out.status.success(), "{}", text(&out));

    f.overlay("a.el", good, Some("0000000000000000000000000000000000000000"));
    refused(&f.check(), "STALE lisp-overlay/a.el");

    f.overlay("a.el", ";;; a.el -*- lexical-binding: t -*-\n(provide 'a)\n;; changed\n", Some(&blob));
    refused(&f.check(), "romacs-overlay: <reason>");

    f.overlay("nogn.el", ";; romacs-overlay: x\n", None);
    std::fs::write(
        f.root().join("lisp-overlay/MANIFEST.tsv"),
        format!("a.el\t{blob}\tr\nnogn.el\t{blob}\tr\n"),
    )
    .unwrap();
    std::fs::write(f.root().join("lisp-overlay/a.el"), good).unwrap();
    refused(&f.check(), "varies no GNU file");
}
