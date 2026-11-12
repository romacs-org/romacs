//! Tests for the parity-reference pin.
//!
//! The fixtures are synthetic on purpose.  A guard whose tests need the real
//! pinned mirror can only be exercised on one machine, and --- worse --- could
//! not plant a MISMATCH without rebuilding the very mirror this crate exists to
//! stop anyone rebuilding.  Synthetic dumps carry a real `struct dump_header`
//! prefix, so they travel the same code path the real one does.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::*;

/// A synthetic editor: a script that reports a release version, plus the dump
/// GNU would load beside it.
///
/// The version is what the running editor answers for `emacs-version`, the one
/// identity an installed GNU can prove: the repository revision is computed
/// from a source checkout at runtime (`lisp/version.el`) and `make install`
/// leaves none, while the binary fingerprint is a build identity that changes
/// with the toolchain while the source stays put.
struct Fixture {
    _dir: tempfile::TempDir,
    executable: PathBuf,
    pdmp: PathBuf,
}

impl Fixture {
    /// `version` is what the script reports; `fingerprint` the 32 bytes the
    /// dump header carries at offset 16.
    fn new(version: &str, fingerprint: [u8; FINGERPRINT_LEN]) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let executable = dir.path().join("emacs");
        let pdmp = dir.path().join("emacs.pdmp");
        fs::write(&executable, format!("#!/bin/sh\nprintf '%s' '{version}'\n"))
            .expect("write executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("make the fixture executable");
        fs::write(&pdmp, dump_bytes(fingerprint)).expect("write dump");
        Self {
            _dir: dir,
            executable,
            pdmp,
        }
    }

    fn manifest(&self) -> ReferenceManifest {
        manifest_of(&self.executable)
    }
}

/// A `struct dump_header` prefix: the magic, the 32-byte build fingerprint,
/// then filler.
fn dump_bytes(fingerprint: [u8; FINGERPRINT_LEN]) -> Vec<u8> {
    let mut dump = Vec::new();
    dump.extend_from_slice(DUMP_MAGIC);
    dump.extend_from_slice(&fingerprint);
    dump.extend_from_slice(&[0x5a; 128]);
    dump
}

/// The manifest a synthetic editor would be pinned by.  `emacs_version` is the
/// release the default fixture reports, so a fixture made with another version
/// is the planted mismatch.
fn manifest_of(executable: &Path) -> ReferenceManifest {
    let observed = observe(executable).expect("observe the fixture");
    ReferenceManifest {
        schema: "1".to_string(),
        emacs_version: "31.0.90".to_string(),
        mirror_commit: "0".repeat(40),
        build_time: "2026-06-10T02:39:56-04:00".to_string(),
        fingerprint: observed.fingerprint,
        executable_sha256: observed.executable_sha256,
        executable_size: observed.executable_size,
        pdmp_sha256: observed.pdmp_sha256,
        pdmp_size: observed.pdmp_size,
    }
}

/// A synthetic `make install`ed GNU: the executable under `bin/`, the dump
/// where `Makefile.in:628-630` actually puts it -- in libexec, named for the
/// build fingerprint, nowhere near the executable.
struct InstalledFixture {
    _dir: tempfile::TempDir,
    executable: PathBuf,
    pdmp: PathBuf,
}

impl InstalledFixture {
    fn new(version: &str, fingerprint: [u8; FINGERPRINT_LEN]) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).expect("bin dir");
        let versioned = bin.join("emacs-31.1.1");
        fs::write(&versioned, format!("#!/bin/sh\nprintf '%s' '{version}'\n"))
            .expect("write executable");
        fs::set_permissions(&versioned, fs::Permissions::from_mode(0o755))
            .expect("make the fixture executable");
        // `make install` links the unversioned name to the versioned one.
        fs::copy(&versioned, bin.join("emacs")).expect("install symlink stand-in");
        let libexec = dir.path().join("libexec/emacs/31.1.1/x86_64-pc-linux-gnu");
        fs::create_dir_all(&libexec).expect("libexec dir");
        let pdmp = libexec.join(format!("emacs-{}.pdmp", hex(&fingerprint)));
        fs::write(&pdmp, dump_bytes(fingerprint)).expect("write dump");
        Self {
            _dir: dir,
            executable: bin.join("emacs"),
            pdmp,
        }
    }

    fn manifest(&self) -> ReferenceManifest {
        manifest_of(&self.executable)
    }
}

fn pinned_fingerprint() -> [u8; FINGERPRINT_LEN] {
    let mut bytes = [0u8; FINGERPRINT_LEN];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = index as u8;
    }
    bytes
}

// ---------------------------------------------------------------------------
// The happy path
// ---------------------------------------------------------------------------

#[test]
fn a_matching_editor_attests_at_both_depths() {
    let fixture = Fixture::new("31.0.90", pinned_fingerprint());
    let manifest = fixture.manifest();
    for depth in [AttestationDepth::Fingerprint, AttestationDepth::Exhaustive] {
        let attested = attest_against(&fixture.executable, depth, &manifest)
            .unwrap_or_else(|error| panic!("{depth} attestation should pass: {error}"));
        assert_eq!(attested.depth(), depth);
        assert_eq!(
            attested.executable(),
            fs::canonicalize(&fixture.executable)
                .expect("canonicalize")
                .as_path()
        );
        assert_eq!(
            attested.pdmp(),
            fs::canonicalize(&fixture.pdmp)
                .expect("canonicalize")
                .as_path()
        );
    }
}

#[test]
fn the_stamp_carries_the_reference_and_the_depth() {
    let fixture = Fixture::new("31.0.90", pinned_fingerprint());
    let manifest = fixture.manifest();
    for depth in [AttestationDepth::Fingerprint, AttestationDepth::Exhaustive] {
        let attested =
            attest_against(&fixture.executable, depth, &manifest).expect("attestation should pass");
        assert_eq!(
            ReferenceUse::Attested(attested).stamp(),
            format!("gnu=31.0.90 fingerprint=000102030405 mirror=00000000000 attest={depth}")
        );
    }

    let unpinned = ReferenceUse::Unpinned {
        executable: fixture.executable.clone(),
    };
    assert_eq!(
        unpinned.stamp(),
        "gnu=UNATTESTED (ROMACS_PARITY_REFERENCE=none) attest=none"
    );
    assert!(!unpinned.is_attested());
}

// ---------------------------------------------------------------------------
// Sensitivity: plant a mismatch, confirm the guard fires, and check the message
// ---------------------------------------------------------------------------

#[test]
fn a_different_release_is_refused_by_its_version() {
    // The incident this crate exists for: same path, another GNU.  An
    // installed build cannot prove its source commit, so the release version
    // is the identity, and the cheap depth's whole job is to check it.
    let manifest = Fixture::new("31.0.90", pinned_fingerprint()).manifest();
    let rebuilt = Fixture::new("30.2", pinned_fingerprint());

    let error = attest_against(
        &rebuilt.executable,
        AttestationDepth::Fingerprint,
        &manifest,
    )
    .expect_err("a different release must be refused");
    assert!(
        attest_against(&rebuilt.executable, AttestationDepth::Exhaustive, &manifest).is_err(),
        "a different release must be refused at the exhaustive depth too"
    );
    match &error {
        AttestationError::Mismatch {
            field,
            expected,
            actual,
            ..
        } => {
            assert_eq!(*field, "emacs version");
            assert_eq!(expected, "31.0.90");
            assert_eq!(actual, "30.2");
        }
        other => panic!("expected a version mismatch, got {other:?}"),
    }
    let rendered = error.to_string();
    assert!(
        rendered.contains("parity reference MISMATCH on emacs version"),
        "message must name the refusal: {rendered}"
    );
    assert!(
        rendered.contains("pin-reference"),
        "message must name the deliberate re-baselining path: {rendered}"
    );
    assert!(
        rendered.contains("ROMACS_PARITY_REFERENCE=none"),
        "message must name the opt-out: {rendered}"
    );
}

#[test]
fn byte_provenance_does_not_gate_the_release() {
    // The manifest still RECORDS the executable and dump bytes (observe and
    // pin-reference write them), but the identity is the release, so a
    // same-version build with different bytes -- another toolchain's rebuild,
    // or a shipped file edited afterwards -- attests at both depths.  The
    // fingerprint's own job is pairing a binary with its dump, which GNU
    // enforces itself when the editor runs.
    let manifest = Fixture::new("31.0.90", pinned_fingerprint()).manifest();
    let changed = Fixture::new("31.0.90", pinned_fingerprint());

    let mut script = fs::read(&changed.executable).expect("read executable");
    script.extend_from_slice(b"# a post-build edit that keeps the release\n");
    fs::write(&changed.executable, &script).expect("rewrite executable");
    let mut dump = fs::read(&changed.pdmp).expect("read dump");
    let last = dump.len() - 1;
    dump[last] ^= 0xff;
    fs::write(&changed.pdmp, &dump).expect("rewrite dump");

    for depth in [AttestationDepth::Fingerprint, AttestationDepth::Exhaustive] {
        attest_against(&changed.executable, depth, &manifest)
            .unwrap_or_else(|error| panic!("{depth} must not gate on bytes: {error}"));
    }
}

#[test]
fn a_missing_dump_is_refused_and_named_as_the_dump() {
    let fixture = Fixture::new("31.0.90", pinned_fingerprint());
    let manifest = fixture.manifest();
    fs::remove_file(&fixture.pdmp).expect("remove dump");
    let error = attest_against(
        &fixture.executable,
        AttestationDepth::Fingerprint,
        &manifest,
    )
    .expect_err("a missing dump must be refused");
    assert!(
        matches!(&error, AttestationError::DumpMissing { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_file_that_is_not_a_dump_is_named_as_such_rather_than_misread() {
    let fixture = Fixture::new("31.0.90", pinned_fingerprint());
    let manifest = fixture.manifest();
    // Long enough to reach the magic check: a shorter file is refused earlier,
    // as a header that could not be READ, which is a different fact.
    fs::write(
        &fixture.pdmp,
        b"this is not a dump file at all, not one single bit of one",
    )
    .expect("overwrite");
    let error = attest_against(
        &fixture.executable,
        AttestationDepth::Fingerprint,
        &manifest,
    )
    .expect_err("a non-dump must be refused");
    match &error {
        AttestationError::DumpMagic { found, .. } => {
            assert!(found.starts_with("this is not a du"), "found: {found}");
        }
        other => panic!("expected a magic refusal, got {other:?}"),
    }

    // A file too short to hold a header is refused as a header that could not
    // be read, not as a bad magic: the two are different facts about the dump.
    fs::write(&fixture.pdmp, b"short").expect("truncate");
    let error = attest_against(
        &fixture.executable,
        AttestationDepth::Fingerprint,
        &manifest,
    )
    .expect_err("a truncated dump must be refused");
    assert!(
        matches!(&error, AttestationError::DumpMissing { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_missing_editor_is_refused_as_the_editor() {
    let error = attest_against(
        Path::new("/nonexistent/definitely-not-an-editor"),
        AttestationDepth::Fingerprint,
        &Fixture::new("31.0.90", pinned_fingerprint()).manifest(),
    )
    .expect_err("a missing editor must be refused");
    assert!(
        matches!(&error, AttestationError::ExecutableUnresolved { .. }),
        "got {error:?}"
    );
}

// ---------------------------------------------------------------------------
// The opt-out
// ---------------------------------------------------------------------------

#[test]
fn the_opt_out_is_exact_and_absence_means_attest() {
    assert!(!opt_out_requested(None).expect("absent means attest"));
    assert!(opt_out_requested(Some("none")).expect("none means unpinned"));
    for typo in ["None", "NONE", "1", "true", "yes", "", "none "] {
        let error =
            opt_out_requested(Some(typo)).expect_err("only the exact value may disable the guard");
        assert!(
            matches!(&error, AttestationError::UnknownOptOut { .. }),
            "{typo:?} gave {error:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The manifest format
// ---------------------------------------------------------------------------

/// The manifest as this repository checks it in.
fn checked_in_manifest_text() -> String {
    fs::read_to_string(manifest_path()).expect("the checked-in manifest must be readable")
}

#[test]
fn the_checked_in_manifest_parses_and_round_trips() {
    let manifest =
        parse_manifest(&checked_in_manifest_text()).expect("the checked-in pin must parse");
    assert_eq!(manifest.schema, "1");
    let rendered = render_manifest_keys(&manifest);
    assert_eq!(
        parse_manifest(&rendered).expect("a rendered manifest must parse"),
        manifest,
        "pin-reference writes what this crate reads"
    );
}

#[test]
fn the_parser_refuses_rather_than_skips() {
    let base = render_manifest_keys(&ReferenceManifest {
        schema: "1".to_string(),
        emacs_version: "31.0.90".to_string(),
        mirror_commit: "a".repeat(40),
        build_time: "t".to_string(),
        fingerprint: "b".repeat(64),
        executable_sha256: "c".repeat(64),
        executable_size: 1,
        pdmp_sha256: "d".repeat(64),
        pdmp_size: 2,
    });
    parse_manifest(&base).expect("the baseline must parse");

    // Each of these is a way a pin could silently stop meaning what it says.
    let corruptions: [(&str, String); 9] = [
        (
            "a line the format does not define",
            format!("{base}[table]\n"),
        ),
        ("a duplicate key", format!("{base}pdmp_size = \"3\"\n")),
        ("an unknown key", format!("{base}pdmp_sha512 = \"e\"\n")),
        (
            "an unquoted value",
            base.replace("pdmp_size = \"2\"", "pdmp_size = 2"),
        ),
        (
            "a non-hex digest",
            base.replace(&"d".repeat(64), &"z".repeat(64)),
        ),
        (
            "a truncated digest",
            base.replace(&"d".repeat(64), &"d".repeat(63)),
        ),
        (
            "a non-hex fingerprint",
            base.replace(&"b".repeat(64), &"z".repeat(64)),
        ),
        (
            "a truncated fingerprint",
            base.replace(&"b".repeat(64), &"b".repeat(63)),
        ),
        (
            "an unrecognised schema",
            base.replace("schema = \"1\"", "schema = \"2\""),
        ),
    ];
    for (what, text) in corruptions {
        assert!(
            parse_manifest(&text).is_err(),
            "the parser must refuse {what}"
        );
    }

    // ...and must still accept the things the format DOES define.
    let decorated = format!("# a comment\n\n{base}\n   \n# trailing note\n");
    parse_manifest(&decorated).expect("comments and blank lines are part of the format");
}

#[test]
fn a_missing_key_is_named() {
    let text = "schema = \"1\"\n";
    let error = parse_manifest(text).expect_err("an incomplete pin must be refused");
    assert!(error.contains("missing key"), "{error}");
}

#[test]
fn an_installed_layout_attests_its_libexec_dump() {
    let fixture = InstalledFixture::new("31.0.90", pinned_fingerprint());
    let manifest = fixture.manifest();
    for depth in [AttestationDepth::Fingerprint, AttestationDepth::Exhaustive] {
        let attested = attest_against(&fixture.executable, depth, &manifest)
            .unwrap_or_else(|error| panic!("{depth} attestation should pass: {error}"));
        assert_eq!(
            attested.pdmp(),
            fs::canonicalize(&fixture.pdmp)
                .expect("canonicalize")
                .as_path(),
            "the attested dump must be the installed libexec one"
        );
    }
}
