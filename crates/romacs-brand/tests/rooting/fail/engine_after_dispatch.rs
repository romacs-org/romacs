// A raw handler using its engine after the dispatch that consumed it.
use romacs_brand::*;
#[path = "../standin.rs"]
mod standin;
use standin::*;
fn dispatch(_e: Eng<'_, Engine>) -> u64 { 0 }
fn bad(e: Eng<'_, Engine>) -> u64 {
    let held = e.stack[0].0;
    let out = dispatch(e);
    e.stack.len() as u64 + held + out
}
fn main() {}
