//! `nreverse` of an image list is refused by VM2's image contract, not by a
//! GNU type check: Lisp must never see an invented `wrong-type-argument` for
//! a valid list, and the image cons must be untouched (review of vm2/bulk1).
use romacs_vm::{ImageRegion, Vm};

#[test]
fn image_nreverse_refusal_is_not_a_type_error() {
    // The image root is the list (7 42) (same encoding as tests/gc.rs).
    let words: [u64; 4] = [(7 << 3) | 1, (1 << 63) | (1 << 3) | 3, (42 << 3) | 1, 0];
    let image = ImageRegion::from_bytes(words.into_iter().flat_map(u64::to_le_bytes).collect()).unwrap();
    let mut vm = Vm::new();
    vm.attach_image(image.clone()).unwrap();
    vm.declare_special("world", image.reference(0).unwrap());
    let outcome = vm
        .eval_source("(condition-case e (nreverse world) (wrong-type-argument (list 'wrong-type e)))")
        .map(|v| v.display().unwrap());
    let error = outcome.expect_err("an image cons was reversed or the refusal was caught as a type error");
    assert!(error.contains("Attempt to modify read-only object"), "{error}");
    let after = vm.eval_source("world").unwrap();
    assert_eq!(after.display().unwrap(), "(7 42)");
}
