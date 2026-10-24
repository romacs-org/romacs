#![cfg(unix)]
//! Ownership regression for an independent Git project fixture.

use romacs_tui_tests::TuiTempDirectory;
use std::fs;
use std::process::Command;

fn make_git_project_fixture(label: &str) -> TuiTempDirectory {
    let root = TuiTempDirectory::new(&format!("romacs-project-root-{label}-"));
    let src = root.join("src");
    fs::create_dir_all(&src).expect("create project fixture source directory");
    fs::write(root.join("README.md"), "# Neo project probe\n").expect("write project readme");
    fs::write(src.join("alpha.el"), "(defun romacs-project-alpha () 1)\n")
        .expect("write alpha source");
    fs::write(src.join("beta.el"), "(defun romacs-project-beta () 2)\n")
        .expect("write beta source");

    let status = Command::new("git")
        .arg("init")
        .arg("-q")
        .arg(root.path())
        .status()
        .expect("run git init for project fixture");
    assert!(
        status.success(),
        "git init should succeed for project fixture"
    );

    let status = Command::new("git")
        .arg("-C")
        .arg(root.path())
        .arg("add")
        .arg("README.md")
        .arg("src/alpha.el")
        .arg("src/beta.el")
        .status()
        .expect("run git add for project fixture");
    assert!(
        status.success(),
        "git add should succeed for project fixture"
    );

    root
}

#[test]
fn git_project_fixture_removes_its_tree_when_its_owner_drops() {
    let fixture = make_git_project_fixture("drop-contract");
    let path = fixture.path().to_path_buf();

    drop(fixture);

    assert!(
        !path.exists(),
        "temporary project fixture survived its owning value: {}",
        path.display()
    );
}
