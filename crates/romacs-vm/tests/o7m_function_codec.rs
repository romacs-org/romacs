//! O7m C1. The source and the object-bearing artifact are generated here.
#[macro_use]
#[path = "support/o7m_codec.rs"]
mod support;

use self::support::{configure_store, eval, gnu_lisp};
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, fs, path::Path};

const SOURCE: &str = ";; -*- lexical-binding: t; -*-\n\
(defun o7m-call (x) (if x (o7m-target '(7)) x))\n";

fn artifact(root: &Path) -> (std::path::PathBuf, std::path::PathBuf, Vec<u8>) {
    let source = root.join("codec.el");
    fs::write(&source, SOURCE).unwrap();
    let mut compiler = gnu_lisp(&[]);
    eval(&mut compiler, "(require 'romacs-compile)");
    configure_store(&root.join("store"));
    let (path, held) = compiler.compile_lisp_file(&source).unwrap();
    assert!(!held);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(&bytes[..5], b"RLC6\n");
    (source, path, bytes)
}

// The RLC6 outer envelope has path and host observations. Start the comparison
// at its name table, so moving the checkout or changing compiler ID is harmless.
fn payload(bytes: &[u8]) -> &[u8] {
    let mut body = &bytes[21..];
    assert_eq!(body[0], 0, "source-only artifact");
    body = &body[1..];
    field(&mut body); // empty status detail
    field(&mut body); // source
    for _ in 0..natural(&mut body) {
        field(&mut body);
    }
    body
}

fn natural(bytes: &mut &[u8]) -> usize {
    let mut value = 0;
    for shift in (0..usize::BITS).step_by(7) {
        let (byte, rest) = bytes.split_first().unwrap();
        *bytes = rest;
        value |= usize::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return value;
        }
    }
    panic!("invalid LEB128");
}

fn field<'a>(bytes: &mut &'a [u8]) -> &'a [u8] {
    let len = natural(bytes);
    let (value, rest) = bytes.split_at(len);
    *bytes = rest;
    value
}

fn load_and_probe(source: &Path, probe: &str, expected: &str) {
    let mut receiver = gnu_lisp(&[]);
    assert_eq!(
        eval(
            &mut receiver,
            "(defun o7m-unrelated (x) (list (o7m-other x) x))"
        ),
        "o7m-unrelated"
    );
    eval(&mut receiver, "(defun o7m-target (x) x)");
    let taken = "(plist-get (romacs-compiled-lisp-stats) :taken)";
    let before = eval(&mut receiver, taken).parse::<usize>().unwrap();
    let name = serde_json::to_string(&source.with_extension("")).unwrap();
    eval(&mut receiver, &format!("(load {name} nil t)"));
    assert_eq!(
        eval(&mut receiver, taken),
        (before + 1).to_string(),
        "receiver missed artifact"
    );
    receiver.collect_compacting();
    assert_eq!(eval(&mut receiver, probe), expected);
    eval(
        &mut receiver,
        "(fset 'o7m-target (lambda (x) (cons 'new-target x)))",
    );
    assert_eq!(eval(&mut receiver, "(o7m-call t)"), "(new-target 7)");
}

// S7. Protect the existing artifact payload, then drive its installed cache
// through first call and a symbol-cell change in a receiver with different IDs.
#[test]
fn artifact_payload_unchanged() {
    in_private_store!(artifact_payload_unchanged, |root| {
        let (source, _, bytes) = artifact(root);
        let mut digest = String::with_capacity(64);
        for byte in Sha256::digest(payload(&bytes)) {
            write!(digest, "{byte:02x}").unwrap();
        }
        assert_eq!(
            digest,
            "8621e51f2b7e90f14829862d6106dcda034cf39837a1dee46dd72ba6f330ee97"
        );
        load_and_probe(&source, "(o7m-call t)", "(7)");
    });
}

// R6. Replace the fixture's one parameter form with Form::Object -> node 1,
// the same cons referenced by its literal. Compiled ops still bind the
// original argument register. The reader must resolve both to one private cons.
#[test]
fn form_object_header_resolved() {
    in_private_store!(form_object_header_resolved, |root| {
        let (source, path, mut bytes) = artifact(root);
        patch_parameter(&mut bytes);
        fs::write(path, bytes).unwrap();
        load_and_probe(
            &source,
            "(let ((param (car (aref (symbol-function 'o7m-call) 0)))) \
              (list (consp param) (eq param (o7m-call t))))",
            "(t t)",
        );
    });
}

fn patch_parameter(bytes: &mut Vec<u8>) {
    let mut units = payload(bytes);
    let x = (0..natural(&mut units))
        .find(|_| field(&mut units) == b"x")
        .unwrap();
    // Rewind to skip the whole name table, including the names after x.
    units = payload(bytes);
    for _ in 0..natural(&mut units) {
        field(&mut units);
    }
    assert_eq!(natural(&mut units), 1);
    for _ in 0..4 {
        natural(&mut units);
    }
    let length_at = bytes.len() - units.len();
    let unit = field(&mut units);
    let unit_start = bytes.len() - units.len() - unit.len();
    let header = [0, 1, 1, 1, 2, u8::try_from(x).unwrap()];
    let positions: Vec<_> = unit
        .windows(header.len())
        .enumerate()
        .filter_map(|(i, window)| (window == header).then_some(i + 4))
        .collect();
    assert_eq!(
        positions.len(),
        1,
        "ambiguous parameter header: {unit:02x?}"
    );
    let offset = unit_start + positions[0];
    assert_ne!(bytes[length_at] & 0x7f, 0x7f);
    bytes[length_at] += 1;
    bytes.splice(offset..offset + 2, [9, 5, 1]);
    let checksum = twox_hash::XxHash3_128::oneshot(&bytes[21..]).to_le_bytes();
    bytes[5..21].copy_from_slice(&checksum);
}
