//! GNU's live Lisp face layout, distinct from the resolved immutable theme.
//!
//! The evaluator owns and traces the actual values. Keeping the layout here
//! avoids converting live Lisp identity (including functions and inheritance
//! lists) into the deliberately restricted immutable `Attrs` representation.

/// `LFACE_VECTOR_SIZE` in pinned GNU `src/dispextern.h`.
pub const LFACE_VECTOR_SIZE: usize = 20;
/// Slot zero is the `face` marker; the remaining slots start unspecified.
pub const LFACE_ATTRIBUTES: [&str; LFACE_VECTOR_SIZE - 1] = [
    ":family", ":foundry", ":width", ":height", ":weight", ":slant",
    ":underline", ":inverse-video", ":foreground", ":background", ":stipple",
    ":overline", ":strike-through", ":box", ":font", ":inherit", ":fontset",
    ":distant-foreground", ":extend",
];

/// Allocate storage without prescribing a Lisp value representation.
pub fn new_lface<T: Clone>(marker: T, unspecified: T) -> Vec<T> {
    let mut slots = vec![unspecified; LFACE_VECTOR_SIZE];
    slots[0] = marker;
    slots
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_is_the_gnu_live_vector_not_the_theme_record() {
        let vector = new_lface("face", "unspecified");
        assert_eq!(vector.len(), 20);
        assert_eq!(vector[0], "face");
        assert!(vector[1..].iter().all(|s| *s == "unspecified"));
        assert_eq!(LFACE_ATTRIBUTES[14], ":font");
        assert_eq!(LFACE_ATTRIBUTES[15], ":inherit");
        assert_eq!(LFACE_ATTRIBUTES[18], ":extend");
    }
}
