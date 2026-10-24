pub fn patterns() -> Vec<Vec<u8>> {
    (0..10)
        .flat_map(|i| {
            [
                format!(r"^\*+ TODO Task {i}\>"),
                format!(r"^#\+PROPERTY: header-args:lang{i} .*"),
                format!(r"^[ \t]*:CUSTOM_ID: section-{i}$"),
                format!(r"\[\[file:chapter-{i}\.org\]\[.*\]\]"),
                format!(r"^#\+begin_src language{i}\>"),
                format!(r"<2026-10-{:02} .*>", i + 1),
            ]
            .map(String::into_bytes)
        })
        .collect()
}
