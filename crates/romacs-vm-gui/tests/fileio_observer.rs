//! Pure file-I/O observer controls: no live GUI, compositor, or Pillow.
#[test]
fn fileio_document_observer_controls() {
    let harness = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("harness");
    let output = std::process::Command::new("python3")
        .args(["-m", "unittest", "-v", "test_fileio_live"])
        .current_dir(harness)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("python3 is required for maintained file-I/O observer controls");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stderr.contains("Ran 10 tests") && !stderr.contains("skipped"),
        "file-I/O observer controls failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
