//! A theme cannot change without becoming a new value with a new generation,
//! so a cache keyed by generation cannot serve data it does not describe.
//!
//! The case this exists for: romacs-core's `sync_runtime_faces_for_frame`
//! rewrites a frame's default face without bumping `face_change_count`, so a
//! width measured before the sync can be served after it. Here "rewrite the
//! default face" is `with_face_attribute("default", ...)`, which can only
//! return a new theme.

mod common;

use std::sync::Arc;

use common::*;
use romacs_face::{
    ActiveTheme, Attribute, Background, Deffaces, FaceCache, Remap, Theme, ThemeSpec,
};

fn theme() -> Theme {
    let mut df = Deffaces::default();
    df.defface("default", &d("((t nil))")).unwrap();
    df.defface("bold", &d("((t :weight bold))")).unwrap();
    df.defface("s86-big", &d("((t :inherit bold :height 1.5))"))
        .unwrap();
    let base = base_from(&d(
        r#"(:family "Mono" :foundry "x" :width normal :height 100 :weight normal
        :slant normal :underline nil :overline nil :extend nil :strike-through nil :box nil
        :inverse-video nil :foreground "black" :background "white" :stipple nil)"#,
    ));
    Theme::build(Arc::new(df), None, Background::Light, base).unwrap()
}

#[test]
fn every_change_is_a_new_value_with_a_new_generation() {
    let t0 = theme();
    let t1 = t0
        .with_face_attribute("default", ":height", &d("120"))
        .unwrap();
    let t2 = t1
        .with_defface("s86-new", &d("((t :inherit s86-big))"))
        .unwrap();
    let mut spec = ThemeSpec::new("s86");
    spec.set_face("bold", &d("((t :foreground \"red\"))"))
        .unwrap();
    let t3 = t2
        .with_theme_spec(Some(Arc::new(spec)), Background::Dark)
        .unwrap();
    let gens = [
        t0.generation(),
        t1.generation(),
        t2.generation(),
        t3.generation(),
    ];
    for (i, a) in gens.iter().enumerate() {
        for b in &gens[i + 1..] {
            assert_ne!(a, b, "two distinct face tables share a generation");
        }
    }
    // The older values are untouched by the newer ones.
    assert_eq!(
        attrs_get(&t0.realize("s86-big").unwrap(), Attribute::Height),
        d("150")
    );
    assert_eq!(
        attrs_get(&t1.realize("s86-big").unwrap(), Attribute::Height),
        d("180")
    );
    assert!(!t1.contains("s86-new") && t2.contains("s86-new"));
    assert_eq!(
        attrs_get(&t3.realize("s86-new").unwrap(), Attribute::Foreground),
        d("\"red\"")
    );
    // A clone is the same value: same generation, same data.
    assert_eq!(t0.clone().generation(), t0.generation());
}

#[test]
fn a_cached_measurement_cannot_outlive_a_default_face_rewrite() {
    let mut cache = FaceCache::new();
    let none = Remap::empty();
    let mut active = ActiveTheme::new(theme());
    let before = cache
        .realize(active.get(), &none, "s86-big")
        .unwrap()
        .unwrap();
    assert_eq!(attrs_get(&before, Attribute::Height), d("150"));
    // The sync: rewrite the frame's default face.
    let synced = active
        .get()
        .with_face_attribute("default", ":height", &d("200"))
        .unwrap();
    active.switch(synced);
    let after = cache
        .realize(active.get(), &none, "s86-big")
        .unwrap()
        .unwrap();
    assert_eq!(
        attrs_get(&after, Attribute::Height),
        d("300"),
        "stale face served after the default face changed"
    );
    assert_eq!((cache.hits(), cache.misses()), (0, 2));
    // Asking again under the same value is a hit.
    cache.realize(active.get(), &none, "s86-big").unwrap();
    assert_eq!(cache.hits(), 1);
}

#[test]
fn switching_back_to_an_earlier_theme_reuses_its_entries() {
    let mut cache = FaceCache::new();
    let none = Remap::empty();
    let light = theme();
    let dark = light
        .with_face_attribute("default", ":background", &d("\"black\""))
        .unwrap();
    let mut active = ActiveTheme::new(light.clone());
    let a = cache.realize(active.get(), &none, "bold").unwrap().unwrap();
    let previous = active.switch(dark);
    let b = cache.realize(active.get(), &none, "bold").unwrap().unwrap();
    active.switch(previous);
    let c = cache.realize(active.get(), &none, "bold").unwrap().unwrap();
    assert_eq!(attrs_get(&a, Attribute::Background), d("\"white\""));
    assert_eq!(attrs_get(&b, Attribute::Background), d("\"black\""));
    assert!(
        Arc::ptr_eq(&a, &c),
        "the earlier theme's entry should be reused"
    );
    cache.retain(&[light.generation()]);
    assert_eq!(cache.len(), 1);
}

#[test]
fn remapping_is_part_of_the_key() {
    let mut cache = FaceCache::new();
    let t = theme();
    let r1 = Remap::from_alist(&d("((bold :height 2.0 bold))")).unwrap();
    let r2 = Remap::from_alist(&d("((bold :height 2.0 bold))")).unwrap();
    assert_ne!(r1.generation(), r2.generation());
    let plain = cache
        .realize(&t, &Remap::empty(), "s86-big")
        .unwrap()
        .unwrap();
    let remapped = cache.realize(&t, &r1, "s86-big").unwrap().unwrap();
    assert_eq!(attrs_get(&plain, Attribute::Height), d("150"));
    assert_eq!(attrs_get(&remapped, Attribute::Height), d("300"));
}

#[test]
fn theme_internals_are_not_reachable_from_outside() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
