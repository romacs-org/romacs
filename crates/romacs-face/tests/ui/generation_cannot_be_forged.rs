// Generations are minted by constructors only; no one can make or bump one.
fn main() {
    let _ = romacs_face::Generation(std::num::NonZeroU64::new(7).unwrap());
}
