use std::io::Write;
#[test]
fn mode_initialization_does_not_reallocate_active_register_storage() {
    let mut child=std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
        .arg("--source").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"(emacs-lisp-mode)").unwrap();
    let output=child.wait_with_output().unwrap();
    assert!(output.status.success(),"fresh mode process failed: {:?}, stderr={}",output.status,String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(),"nil\n");
}
