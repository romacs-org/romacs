//! Probe for GC.md's "A Fixed image reference carries no image identity".
use romacs_vm::{Fixed, ImageBuilder, ImageRegion, Vm};

fn main() {
    let store = std::env::temp_dir().join(format!("vm2-fixed-probe-{}", std::process::id()));
    let mut a = ImageBuilder::default();
    let s = a.string("alpha");
    a.cons(s, Fixed::NIL);
    let s = a.string("alpha-second");
    a.cons(s, Fixed::NIL);
    let ka = a.publish(&store, [1; 32]).unwrap();
    let mut b = ImageBuilder::default();
    let s = b.string("beta");
    b.cons(s, Fixed::NIL);
    let kb = b.publish(&store, [2; 32]).unwrap();
    let ra = unsafe { ImageRegion::open(&store, &ka) }.unwrap();
    let rb = unsafe { ImageRegion::open(&store, &kb) }.unwrap();
    let a0 = ra.reference(0).unwrap();
    let a1 = ra.reference(1).unwrap();

    let mut own = Vm::new();
    own.attach_image(ra.clone()).unwrap();
    own.declare_special("x", a0);
    println!("A-ref idx0 in VM(A):     {:?}", own.eval_source("(car x)").map(|v| v.display()));

    let mut other = Vm::new();
    other.attach_image(rb.clone()).unwrap();
    other.declare_special("x", a0);
    println!("A-ref idx0 in VM(B):     {:?}", other.eval_source("(car x)").map(|v| v.display()));
    other.declare_special("y", a1);
    println!("A-ref idx1 in VM(B):     {:?}", other.eval_source("(car y)").map(|v| v.display()));

    let mut bare = Vm::new();
    bare.declare_special("x", a0);
    println!("A-ref idx0 in VM(no img): {:?}", bare.eval_source("(car x)").map(|v| v.display()));
    std::fs::remove_dir_all(store).unwrap();
}
