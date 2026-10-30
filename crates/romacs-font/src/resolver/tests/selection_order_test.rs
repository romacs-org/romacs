use super::*;
use FontSelectionAttribute::{Height, Slant, Weight, Width};
use romacs_display_protocol::font::FontSlantKind;

fn state(order: [FontSelectionAttribute; 4]) -> FontSelectionState {
    let mut state = FontSelectionState::default();
    state.set_order(order);
    state
}

const WEIGHT_FIRST: [FontSelectionAttribute; 4] = [Weight, Width, Height, Slant];

fn candidate(weight: u16, width: u16) -> ListedFace {
    ListedFace {
        family: "selection-order-fixture".into(),
        file: romacs_test_fonts::noto_sans_mono_variable()
            .to_string_lossy()
            .into_owned(),
        selector: 0,
        postscript_name: None,
        weight,
        slant: FontSlantKind::Normal,
        width,
        spacing: None,
        foundry: None,
        fc_style: (80, 0, 100),
    }
}

#[test]
fn selection_order_weight_can_outrank_width() {
    let faces = [candidate(700, 5), candidate(400, 7)];
    let request = FontRequest::new("selection-order-fixture", 16.0);
    assert_eq!(
        best_face(&faces, &request, state(DEFAULT_SELECTION_ORDER), |_| 0)
            .unwrap()
            .weight,
        700
    );
    let selected = best_face(&faces, &request, state(WEIGHT_FIRST), |_| 0).expect("candidate");
    assert_eq!(
        selected.weight, 400,
        "weight-first selects exact weight over exact width"
    );
}

#[test]
fn selection_order_slant_can_outrank_weight() {
    let mut italic = candidate(400, 5);
    italic.slant = FontSlantKind::Italic;
    let faces = [italic, candidate(700, 5)];
    let request = FontRequest::new("selection-order-fixture", 16.0);
    let selected = best_face(&faces, &request, state([Slant, Height, Width, Weight]), |_| 0).unwrap();
    assert_eq!(
        selected.slant,
        FontSlantKind::Normal,
        "slant-first selects upright over exact weight"
    );
}

#[test]
fn selection_order_last_duplicate_overwrites_first_priority() {
    // GNU font.c:2342-2354 assigns each occurrence; the last width wins.
    let faces = [candidate(700, 5), candidate(400, 7)];
    let request = FontRequest::new("selection-order-fixture", 16.0);
    let selected = best_face(&faces, &request, state([Width, Weight, Height, Width]), |_| 0).unwrap();
    assert_eq!(selected.weight, 400, "LAST_DUPLICATE_OVERWRITES_FIRST_PRIORITY");
}

#[test]
fn selection_order_duplicates_retain_omitted_attributes_and_height_is_zero() {
    let faces = [candidate(700, 7), candidate(400, 5)];
    let request = FontRequest::new("selection-order-fixture", 16.0);
    for order in [[Height; 4], [Slant; 4]] {
        let selected = best_face(&faces, &request, state(order), |_| 0).unwrap();
        assert_eq!(
            selected.weight, 400,
            "GNU font.c:2338-2354 retains omitted width/weight shifts; old omission expectation was wrong"
        );
    }
    let selected = best_face(&faces, &request, state([Height; 4]), |face| {
        u32::from(face.weight == 700)
    })
    .unwrap();
    assert_eq!(
        selected.weight, 400,
        "compatibility still precedes ordered style"
    );
}

#[test]
fn selection_order_omitted_priorities_retain_history() {
    let faces = [candidate(700, 5), candidate(400, 7)];
    let request = FontRequest::new("selection-order-fixture", 16.0);
    let mut history = FontSelectionState::default();
    history.set_order([Height; 4]);
    assert_eq!(best_face(&faces, &request, history, |_| 0).unwrap().weight, 700);
    history.set_order(WEIGHT_FIRST);
    history.set_order([Height; 4]);
    assert_eq!(history.shifts(), [16, 2, 23, 2]);
    assert_eq!(best_face(&faces, &request, history, |_| 0).unwrap().weight, 400,
        "OMITTED_WEIGHT_PRIORITY_RETAINS_INTERVENING_ORDER");
}

#[test]
fn selection_order_packs_overlapping_fields_with_or_and_caps_distances() {
    let score = style_score(0, 100, FontSlantKind::Normal, 1,
        900, FontSlantKind::Italic, 9);
    // All three style fields end at shift 2 after this history. Distances
    // all exceed 127: OR yields 127 << 2, not their sum or a tuple.
    let mut history = FontSelectionState::default();
    history.set_order([Weight; 4]);
    history.set_order([Width; 4]);
    assert_eq!(score.ordered(history), (0, 127 << 2));
    let uncapped = style_score(0, 400, FontSlantKind::Normal, 5,
        500, FontSlantKind::Normal, 6);
    assert_eq!(uncapped.ordered(history), (0, (20 | 13) << 2),
        "OVERLAPPING_FIELDS_USE_OR_NOT_SUM_OR_MAX");
}

#[cfg(target_os = "linux")]
#[test]
fn selection_order_reorders_character_candidates() {
    let mut resolver = FontResolver::new().unwrap();
    resolver.set_selection_order(WEIGHT_FIRST);
    let faces = [candidate(700, 5), candidate(400, 7)];
    let request = FontRequest::new("selection-order-fixture", 16.0);
    let selected = resolver
        .first_with_glyph(&faces, &request, 'A', |_| 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        selected.font.width, 7,
        "character ranking uses resolver priorities"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn selection_order_change_invalidates_primary_and_character_answers() {
    assert!(crate::register_font_file(
        &romacs_test_fonts::noto_sans_mono_variable()
    ));
    let mut resolver = FontResolver::new().unwrap();
    let request = FontRequest::new("Noto Sans Mono", 16.0);
    let policy = FontsetPolicy::default();
    let primary = resolver.resolve_primary(&request, &policy).unwrap();
    let character = resolver.resolve_char(&request, 'A', &policy).unwrap();
    assert_eq!(resolver.primary.len(), 1, "primary answer is warm");
    assert_eq!(resolver.chars.len(), 1, "character answer is warm");
    assert_eq!(resolver.selection_order(), DEFAULT_SELECTION_ORDER);
    resolver.set_selection_order(DEFAULT_SELECTION_ORDER);
    assert_eq!(
        resolver.primary.len(),
        1,
        "unchanged order retains primary answer"
    );
    assert_eq!(
        resolver.chars.len(),
        1,
        "unchanged order retains character answer"
    );
    resolver.set_selection_order(WEIGHT_FIRST);
    assert_eq!(resolver.selection_order(), WEIGHT_FIRST);
    assert!(
        resolver.primary.is_empty(),
        "changed order invalidates cached primary answer"
    );
    assert!(
        resolver.chars.is_empty(),
        "changed order invalidates cached character answer"
    );
    assert_eq!(
        resolver.resolve_primary(&request, &policy).unwrap(),
        primary
    );
    assert_eq!(
        resolver.resolve_char(&request, 'A', &policy).unwrap(),
        character
    );
    assert_eq!(resolver.primary.len(), 1, "primary answer is recomputed");
    assert_eq!(resolver.chars.len(), 1, "character answer is recomputed");

    // A producer can change A -> B -> A between layouts. The final raw
    // order AND shifts equal the warm state; revision must still evict.
    let warm = resolver.selection_state();
    let mut producer = warm;
    producer.set_order(DEFAULT_SELECTION_ORDER);
    producer.set_order(WEIGHT_FIRST);
    assert_eq!(producer.order(), warm.order());
    assert_eq!(producer.shifts(), warm.shifts());
    assert_ne!(producer.revision(), warm.revision());
    resolver.set_selection_state(producer);
    assert!(resolver.primary.is_empty(), "REVISION_EVICTS_PRIMARY_ON_SKIPPED_ROUND_TRIP");
    assert!(resolver.chars.is_empty(), "REVISION_EVICTS_CHARACTER_ON_SKIPPED_ROUND_TRIP");
}
