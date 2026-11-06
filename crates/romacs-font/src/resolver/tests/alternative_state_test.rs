use super::*;

fn family_rows(alternative: &str) -> Vec<Vec<String>> {
    vec![vec![
        "romacs-missing-alternative-fixture".into(),
        alternative.into(),
    ]]
}

#[cfg(target_os = "linux")]
#[test]
fn alternative_family_setter_invalidates_both_warm_caches_even_unchanged_or_round_trip() {
    assert!(crate::register_font_file(
        romacs_test_fonts::noto_sans_mono_variable()
    ));
    let mut resolver = FontResolver::new().unwrap();
    let request = FontRequest::new("Noto Sans Mono", 16.0);
    let mut state = FontAlternativeState::default();
    for round_trip in [false, true] {
        resolver.resolve_primary(&request).unwrap();
        resolver.resolve_char(&request, 'A').unwrap();
        assert_eq!(resolver.primary.len(), 1);
        assert_eq!(resolver.chars.len(), 1);
        // Re-adopting the same publication is not a setter invocation.
        resolver.set_alternative_state(state.clone());
        assert_eq!(resolver.primary.len(), 1);
        assert_eq!(resolver.chars.len(), 1);
        let before = state.clone();
        if round_trip {
            state.set_family_alist(family_rows("Noto Sans Mono"));
        }
        state.set_family_alist(vec![]);
        assert_eq!(state.family_alist(), before.family_alist());
        assert_ne!(state.revision(), before.revision());
        resolver.set_alternative_state(state.clone());
        assert!(
            resolver.primary.is_empty(),
            "ALTERNATIVE_SETTER_EVICTS_PRIMARY"
        );
        assert!(
            resolver.chars.is_empty(),
            "ALTERNATIVE_SETTER_EVICTS_CHARACTERS"
        );
    }
}
