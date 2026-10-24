use super::*;

fn fam(name: &str) -> FontsetEntry {
    FontsetEntry::Font(FontsetFontSpec::family(name))
}

fn families(policy: &FontsetPolicy, ch: char) -> Vec<Option<String>> {
    policy
        .entries_for(ch)
        .map(|entry| match entry {
            FontsetEntry::Font(spec) => spec.family.clone(),
            FontsetEntry::ExplicitNone => Some("<none>".into()),
        })
        .collect()
}

#[test]
fn empty_policy_has_no_entries() {
    assert!(FontsetPolicy::new().entries_for('Ω').next().is_none());
}

#[test]
fn range_entries_precede_fallback_and_splits_keep_neighbours() {
    let mut policy = FontsetPolicy::new();
    policy.set_font(
        FontsetTarget::Fallback,
        fam("Fallback"),
        FontsetMode::Append,
    );
    policy.set_font(
        FontsetTarget::Range(0x370, 0x3FF),
        fam("Greek"),
        FontsetMode::Overwrite,
    );
    // A narrower prepend splits the Greek range in three.
    policy.set_font(
        FontsetTarget::Range(0x3A0, 0x3AF),
        fam("Capitals"),
        FontsetMode::Prepend,
    );
    assert_eq!(
        families(&policy, '\u{3B1}'),
        [Some("Greek".into()), Some("Fallback".into())]
    );
    assert_eq!(
        families(&policy, '\u{3A9}'),
        [
            Some("Capitals".into()),
            Some("Greek".into()),
            Some("Fallback".into())
        ]
    );
    assert_eq!(
        families(&policy, '\u{371}'),
        [Some("Greek".into()), Some("Fallback".into())]
    );
    assert_eq!(families(&policy, 'Ж'), [Some("Fallback".into())]);
}

#[test]
fn append_and_overwrite_follow_the_inherited_registry() {
    let mut policy = FontsetPolicy::new();
    policy.set_font(
        FontsetTarget::Range(0x1F600, 0x1F64F),
        fam("A"),
        FontsetMode::Append,
    );
    policy.set_font(
        FontsetTarget::Range(0x1F600, 0x1F64F),
        fam("B"),
        FontsetMode::Append,
    );
    assert_eq!(
        families(&policy, '😀'),
        [Some("A".into()), Some("B".into())]
    );
    policy.set_font(
        FontsetTarget::Range(0x1F600, 0x1F600),
        FontsetEntry::ExplicitNone,
        FontsetMode::Overwrite,
    );
    assert_eq!(families(&policy, '😀'), [Some("<none>".into())]);
    assert_eq!(
        families(&policy, '😁'),
        [Some("A".into()), Some("B".into())]
    );
}

#[test]
fn a_range_spanning_gaps_fills_them() {
    let mut policy = FontsetPolicy::new();
    policy.set_font(
        FontsetTarget::Range(20, 29),
        fam("Mid"),
        FontsetMode::Overwrite,
    );
    policy.set_font(
        FontsetTarget::Range(10, 40),
        fam("Wide"),
        FontsetMode::Append,
    );
    for code in [10, 19, 30, 40] {
        assert_eq!(
            families(&policy, char::from_u32(code).unwrap()),
            [Some("Wide".into())],
            "{code}"
        );
    }
    assert_eq!(
        families(&policy, char::from_u32(25).unwrap()),
        [Some("Mid".into()), Some("Wide".into())]
    );
    assert!(families(&policy, char::from_u32(41).unwrap()).is_empty());
}

#[test]
fn every_edit_bumps_the_generation() {
    let mut policy = FontsetPolicy::new();
    let start = policy.generation();
    policy.set_font(FontsetTarget::Fallback, fam("X"), FontsetMode::Append);
    policy.set_family_alternatives("Monospace", vec!["courier".into()]);
    assert_eq!(policy.generation(), start + 2);
    assert_eq!(policy.families_for("monospace"), ["monospace", "courier"]);
}
