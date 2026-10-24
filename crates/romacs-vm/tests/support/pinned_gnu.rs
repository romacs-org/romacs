fn pinned_gnu() -> std::ffi::OsString {
    let binary = std::env::var_os("ROMACS_GNU_ORACLE")
        .or_else(|| std::env::var_os("ROMACS_FORCE_ORACLE_PATH"))
        .expect(
            "set ROMACS_GNU_ORACLE to the pinned GNU executable; see scripts/build-gnu-oracle.sh",
        );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let output = std::process::Command::new("python3")
        .arg(root.join("scripts/gnu_oracle.py"))
        .arg("path")
        .env("ROMACS_GNU_ORACLE", binary)
        .output()
        .expect("python3 is required to validate the GNU oracle");
    assert!(
        output.status.success(),
        "GNU oracle validation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("GNU path must be UTF-8")
        .trim()
        .into()
}
