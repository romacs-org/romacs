//! Placeholders filled in one pass over a template, so a value filled in
//! is never searched for a later placeholder. Chained `str::replace` did
//! that: a temporary directory whose random name held `PT` became
//! `(point-max)` inside a path, and the pinned GNU oracle failed to open it.

/// TEMPLATE with each occurrence of a placeholder of PAIRS, (placeholder,
/// value), replaced by its value; the longest placeholder wins where
/// several start at one place.
pub fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    assert!(pairs.iter().all(|(key, _)| !key.is_empty()), "an empty placeholder matches everywhere");
    let mut keys: Vec<&(&str, &str)> = pairs.iter().collect();
    keys.sort_by_key(|(key, _)| std::cmp::Reverse(key.len()));
    let mut filled = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        if let Some((key, value)) = keys.iter().find(|(key, _)| rest.starts_with(key)) {
            filled.push_str(value);
            rest = &rest[key.len()..];
        } else {
            filled.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::fill;

    #[test]
    fn a_value_is_not_searched_for_a_later_placeholder() {
        let filled = fill("(goto-char PT) (find-file FILE)", &[("FILE", "\"/tmp/rct-xWUPTy/f\""), ("PT", "(point-max)")]);
        assert_eq!(filled, "(goto-char (point-max)) (find-file \"/tmp/rct-xWUPTy/f\")");
    }

    #[test]
    #[should_panic(expected = "an empty placeholder")]
    fn an_empty_placeholder_is_refused() {
        fill("x", &[("", "")]);
    }

    #[test]
    fn the_longest_placeholder_wins() {
        assert_eq!(fill("F1 F12", &[("F1", "a"), ("F12", "b")]), "a b");
    }
}
