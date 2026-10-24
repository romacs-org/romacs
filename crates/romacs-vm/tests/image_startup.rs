use romacs_vm::{ImageBuilder, Fixed};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn run(store: &std::path::Path, key: &str, root: &str, form: &str) -> std::process::Output {
    run_after_spawn(store, key, root, form, |_| {})
}

fn run_after_spawn(
    store: &std::path::Path,
    key: &str,
    root: &str,
    form: &str,
    after_spawn: impl FnOnce(&mut std::process::Child),
) -> std::process::Output {
    // Invalid image arguments may be rejected before stdin is read. Preload
    // the tiny request while the parent still owns the reader, so neither
    // scheduler order nor early child exit can race a parent-side write.
    // 512 bytes is POSIX's minimum PIPE_BUF: these fixtures cannot fill it.
    assert!(
        form.len() <= 512,
        "inline image fixture exceeds preloaded pipe bound"
    );
    let (input, mut writer) = std::io::pipe().unwrap();
    writer.write_all(form.as_bytes()).unwrap();
    drop(writer);
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
        .args(["--image", store.to_str().unwrap(), key, root])
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    after_spawn(&mut child);
    child.wait_with_output().unwrap()
}

#[test]
fn early_argument_rejection_can_finish_before_the_parent_continues() {
    let result = run_after_spawn(&std::env::temp_dir(), "../escape", "0", "42", |child| {
        assert!(!child.wait().unwrap().success());
    });
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("image key must be 64 lowercase hexadecimal digits")
    );
}
#[test]
fn lazy_cli_corruption_fails_without_answer_or_source_fallback() {
    use std::os::unix::fs::{FileExt, PermissionsExt};
    let store = std::env::temp_dir().join(format!("vm2-cli-corrupt-{}", std::process::id()));
    let mut b = ImageBuilder::default();
    let text = b.string(&"x".repeat(16384));
    b.cons(text, Fixed::NIL);
    let key = b.publish(&store, [4; 32]).unwrap();
    let path = store.join(&key);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.write_all_at(&[255], 8192).unwrap();
    let result = run(&store, &key, "0", "[\"length\",[\"car\",\"world\"]]");
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("image page 2 hash mismatch"));
    std::fs::remove_dir_all(store).unwrap();
}
#[test]
fn mapped_cli_proves_image_use_and_never_source_falls_back() {
    let store = std::env::temp_dir().join(format!("vm2-cli-store-{}", std::process::id()));
    let mut b = ImageBuilder::default();
    let text = b.string("Ελλάδα");
    b.cons(text, Fixed::NIL);
    let key = b.publish(&store, [9; 32]).unwrap();
    let result = run(&store, &key, "0", "[\"length\",[\"car\",\"world\"]]");
    assert!(result.status.success(), "{:?}", result);
    assert_eq!(result.stdout, b"6\n");
    let stats: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(stats["mapped_image"], key);
    assert_eq!(stats["startup_collections"], 0);
    assert_eq!(stats["startup_private_objects"], 0);
    assert_eq!(stats["startup_source_loads"], 0);
    let display = run(&store, &key, "0", "[\"car\",\"world\"]");
    assert_eq!(display.stdout, "\"Ελλάδα\"\n".as_bytes());
    let mutation = run(&store, &key, "0", "[\"nreverse\",\"world\"]");
    assert!(!mutation.status.success());
    assert!(String::from_utf8_lossy(&mutation.stderr).contains("immutable image"));
    for (bad_key, root) in [
        ("../escape".to_string(), "0"),
        ("0".repeat(64), "0"),
        (key.clone(), "1"),
    ] {
        let result = run(&store, &bad_key, root, "42");
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("mapped_image"));
    }
    std::fs::remove_dir_all(store).unwrap();
}
