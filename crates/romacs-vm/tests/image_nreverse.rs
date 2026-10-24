//! `nreverse` of an image list is refused by VM2's image contract, not by a
//! GNU type check: Lisp must never see an invented `wrong-type-argument` for
//! a valid list, and the image cons must be untouched (review of vm2/bulk1).
use romacs_vm::{ImageRegion, Vm};
use std::{fs::OpenOptions, io::Write};

#[test]
fn image_nreverse_refusal_is_not_a_type_error() {
    let path = std::env::temp_dir().join(format!("vm2-image-nreverse-{}", std::process::id()));
    let mut file = OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap();
    // The image root is the list (7 42) (same encoding as tests/gc.rs).
    let words: [u64; 4] = [(7 << 3) | 1, (1 << 63) | (1 << 3) | 3, (42 << 3) | 1, 0];
    for w in words {
        file.write_all(&w.to_le_bytes()).unwrap();
    }
    file.flush().unwrap();
    let image = unsafe { ImageRegion::map(&file) }.unwrap();
    std::fs::remove_file(path).unwrap();
    let mut vm = Vm::new();
    vm.attach_image(image.clone()).unwrap();
    vm.declare_special("world", image.reference(0).unwrap());
    let outcome = vm
        .eval_source("(condition-case e (nreverse world) (wrong-type-argument (list 'wrong-type e)))")
        .map(|v| v.display().unwrap());
    let error = outcome.expect_err("an image cons was reversed or the refusal was caught as a type error");
    assert!(error.contains("immutable image"), "{error}");
    let after = vm.eval_source("world").unwrap();
    assert_eq!(after.display().unwrap(), "(7 42)");
}
