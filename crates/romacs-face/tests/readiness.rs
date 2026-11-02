mod common;

use common::*;
use romacs_face::{Attribute as A, *};
use std::sync::Arc;

fn base(height: i64) -> Base {
    base_from(&d(&format!(
        r#"(:family "Mono" :foundry "x" :width normal :height {height} :weight normal :slant normal :underline t :overline nil :extend nil :strike-through nil :box nil :inverse-video nil :foreground "white" :background "black" :stipple nil)"#
    )))
}
fn registry(entries: &[(&str, &str)]) -> Deffaces {
    let mut df = Deffaces::default();
    for (name, spec) in entries {
        df.defface(name, &d(spec)).unwrap();
    }
    df
}
fn theme(entries: &[(&str, &str)]) -> Theme {
    Theme::build(
        Arc::new(registry(entries)),
        None,
        Background::Dark,
        base(113),
    )
    .unwrap()
}

#[test]
fn quoted_reader_is_bounded() {
    assert!(
        matches!(
            Datum::parse(&format!("{}x", "'".repeat(200))),
            Err(DatumError::TooDeep)
        ),
        "quote depth must be bounded"
    );
}
#[test]
fn checked_constructor_refuses_raw_depth_before_cloning() {
    let mut raw = Datum::Nil;
    for _ in 0..40 {
        raw = Datum::List(vec![raw]);
    }
    assert!(
        matches!(BoundedDatum::new(&raw), Err(DatumError::TooDeep)),
        "raw depth must not cross boundary"
    );
}
#[test]
fn shared_atoms_have_an_expanded_byte_budget() {
    let mut h = Heap::default();
    let atom = h.atom(Datum::string(&"x".repeat(1 << 20)));
    let root = h.list(&[atom; 32]);
    assert!(
        Datum::from_graph(&h, root).is_err(),
        "expanded atom bytes must be capped"
    );
}
#[test]
fn printed_bytes_are_bounded() {
    assert!(
        Datum::parse(&format!("\"{}\"", "x".repeat((16 << 20) + 1))).is_err(),
        "printed byte budget must be capped"
    );
}
#[test]
fn remapping_onto_inheriting_face_is_valid() {
    let t = theme(&[
        ("default", "((t :height 120 :foreground \"white\"))"),
        ("solaire", "((t :inherit default :background \"#111111\"))"),
        ("mode-line", "((t :weight bold))"),
        (
            "mode-line-inactive",
            "((t :inherit mode-line :slant italic))",
        ),
    ]);
    let remap =
        Remap::from_alist(&d("((default solaire) (mode-line . mode-line-inactive))")).unwrap();
    let result = View::bind(&t, &remap);
    assert!(result.is_ok(), "GNU-valid remapping must bind: {result:?}");
    let v = result.unwrap();
    assert_eq!(
        attrs_get(&v.realize("default").unwrap(), A::Background),
        d("\"#111111\"")
    );
    assert_eq!(
        attrs_get(&v.realize("mode-line").unwrap(), A::Weight),
        d("bold")
    );
    assert_eq!(
        attrs_get(&v.realize("mode-line").unwrap(), A::Slant),
        d("italic")
    );
}
#[test]
fn aliases_resolve_in_inheritance_and_lookup() {
    let mut df = registry(&[
        ("real", "((t :foreground \"green\"))"),
        ("kid", "((t :inherit old))"),
    ]);
    df.alias("old", "older").unwrap();
    df.alias("older", "real").unwrap();
    let t = Theme::build(Arc::new(df), None, Background::Dark, base(113)).unwrap();
    assert_eq!(
        &*t.definition("kid").unwrap().inherit().unwrap()[0],
        "real",
        "inheritance must resolve aliases before topology"
    );
    assert_eq!(
        attrs_get(&t.realize("kid").unwrap(), A::Foreground),
        d("\"green\""),
        "inheritance must resolve aliases"
    );
    assert!(t.realize("old").is_some(), "lookup must resolve aliases");
    let remap = Remap::from_alist(&d("((old (:weight bold) old))")).unwrap();
    let v = View::bind(&t, &remap).unwrap();
    assert_eq!(
        attrs_get(&v.realize("real").unwrap(), A::Weight),
        d("bold"),
        "remap key must resolve aliases"
    );
    assert_eq!(
        attrs_get(
            &v.effective_ref(&romacs_face::remap::parse_face_ref(&d("old")).unwrap()),
            A::Weight
        ),
        d("bold"),
        "face reference must resolve aliases"
    );
}
#[test]
fn alias_cycles_fail_before_publishing() {
    let mut df = Deffaces::default();
    df.alias("a", "b").unwrap();
    assert!(df.alias("b", "a").is_err(), "alias cycles must be rejected");
}
#[test]
fn c_height_merge_truncates_each_inheritance_step() {
    let t = theme(&[
        ("absolute", "((t :height 105))"),
        ("relative", "((t :height 1.5))"),
        ("kid", "((t :height 1.5 :inherit (relative absolute)))"),
        ("chain", "((t :height 1.5 :inherit relative))"),
    ]);
    assert_eq!(
        t.face_attribute("kid", A::Height, Inherit::Yes)
            .unwrap()
            .to_lisp(),
        "236"
    );
    assert_eq!(
        attrs_get(&t.realize("kid").unwrap(), A::Height),
        d("235"),
        "redisplay must right-fold parents before own height"
    );
    assert_eq!(
        attrs_get(&t.realize("chain").unwrap(), A::Height),
        d("253"),
        "redisplay must not pre-multiply scales over default 113"
    );
    let v = View::bind(&t, &Remap::empty()).unwrap();
    assert_eq!(attrs_get(&v.effective("kid").unwrap(), A::Height), d("235"));
    assert_eq!(attrs_get(&v.realize("chain").unwrap(), A::Height), d("253"));
}
#[test]
fn user_override_composes_and_survives_theme_replacement() {
    let df = Arc::new(registry(&[("default", "((t :height 100))")]));
    let mut spec = ThemeSpec::new("modus-vivendi");
    spec.set_face(
        "default",
        &d("((t :foreground \"white\" :background \"black\"))"),
    )
    .unwrap();
    let mut user = UserOverrides::default();
    user.set_face(
        "default",
        &BoundedDatum::parse("(:height 120 :family \"Iosevka\")").unwrap(),
    )
    .unwrap();
    let t = Theme::build_with_overrides(
        df,
        Some(Arc::new(spec)),
        Arc::new(user),
        Background::Dark,
        base(113),
    )
    .unwrap();
    assert_eq!(
        attrs_get(&t.realize("default").unwrap(), A::Height),
        d("120"),
        "user preferences must compose over theme"
    );
    assert_eq!(
        attrs_get(&t.realize("default").unwrap(), A::Family),
        d("\"Iosevka\"")
    );
    assert_eq!(
        attrs_get(&t.realize("default").unwrap(), A::Background),
        d("\"black\"")
    );
    let next = t.with_theme_spec(None, Background::Light).unwrap();
    assert_eq!(
        attrs_get(&next.realize("default").unwrap(), A::Height),
        d("120")
    );
}
#[test]
fn default_underline_reset_is_declared_not_accidental_base() {
    let t = theme(&[("default", "((t))")]);
    assert_eq!(
        t.face_attribute("default", A::Underline, Inherit::No)
            .unwrap()
            .to_lisp(),
        "nil",
        "default underline must reset before base merge"
    );
    assert_eq!(
        attrs_get(&t.realize("default").unwrap(), A::Underline),
        d("nil")
    );
}
#[test]
fn missing_parent_does_not_hide_later_parent() {
    let t = theme(&[
        ("blue", "((t :foreground \"blue\"))"),
        ("kid", "((t :inherit (missing blue)))"),
    ]);
    assert_eq!(
        attrs_get(&t.realize("kid").unwrap(), A::Foreground),
        d("\"blue\""),
        "undefined parent must be skipped"
    );
}
#[test]
fn removed_font_mechanisms_have_explicit_tests() {
    let mut df = Deffaces::default();
    let error = df
        .defface("function-height", &d("((t :height (lambda (h) (* h 2))))"))
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        error.contains("function heights are not data"),
        "function heights must be rejected"
    );
    df.defface("font", &d("((t :font \"ignored-20\" :family \"Foo-Bar\"))"))
        .unwrap();
    let t = Theme::build(Arc::new(df), None, Background::Dark, base(113)).unwrap();
    assert_eq!(
        t.face_attribute("font", A::Family, Inherit::No)
            .unwrap()
            .to_lisp(),
        "\"Foo-Bar\""
    );
    assert_eq!(
        t.face_attribute("font", A::Foundry, Inherit::No)
            .unwrap()
            .to_lisp(),
        "unspecified"
    );
    assert_eq!(attrs_get(&t.realize("font").unwrap(), A::Height), d("113"));
}
#[test]
fn c_oracle_nonunit_defaults() {
    let fixtures = Datum::parse(include_str!("gnu/redisplay.eld")).unwrap();
    let entries = [
        ("s102-absolute", "((t :height 105))"),
        ("s102-relative", "((t :height 1.5))"),
        (
            "s102-kid",
            "((t :height 1.5 :inherit (s102-relative s102-absolute)))",
        ),
        ("s102-chain", "((t :height 1.5 :inherit s102-relative))"),
        ("s102-fraction", "((t :height 1.15 :inherit s102-chain))"),
    ];
    let mut count = 0;
    for row in fixtures.as_list().unwrap() {
        let row = row.as_list().unwrap();
        let Datum::Int(height) = row[0] else {
            panic!("height")
        };
        let face = symbol(&row[1]);
        let t = Theme::build(
            Arc::new(registry(&entries)),
            None,
            Background::Dark,
            base(height),
        )
        .unwrap();
        let v = View::bind(&t, &Remap::empty()).unwrap();
        let reference = romacs_face::remap::parse_face_ref(&d(face)).unwrap();
        assert_eq!(
            attrs_get(&v.realize_ref(&reference), A::Height),
            row[3],
            "C realized reference {face}/{height}"
        );
        assert_eq!(
            attrs_get(&v.effective(face).unwrap(), A::Height),
            row[2],
            "C effective {face}/{height}"
        );
        assert_eq!(
            attrs_get(&v.realize(face).unwrap(), A::Height),
            row[3],
            "C realized {face}/{height}"
        );
        assert_eq!(
            attrs_get(&t.realize(face).unwrap(), A::Height),
            row[3],
            "Theme C realized {face}/{height}"
        );
        assert_eq!(
            slot_datum(&t.face_attribute(face, A::Height, Inherit::Default).unwrap()),
            row[4],
            "Lisp query {face}/{height}"
        );
        count += 1;
    }
    assert_eq!(count, 16);
}

#[test]
fn rejected_remap_has_a_paintable_buffer_fallback() {
    let t = theme(&[
        ("a", "((t :foreground \"red\"))"),
        ("b", "((t :foreground \"blue\"))"),
    ]);
    let bad = Remap::from_alist(&d("((a b) (b a))")).unwrap();
    let (view, error) = View::bind_or_unremapped(&t, &bad);
    assert!(
        error.is_some(),
        "remap rejection must return its diagnostic"
    );
    assert_eq!(
        view.realize("a"),
        t.realize("a"),
        "rejected remap must paint the original theme"
    );
    assert!(view.remap().is_empty());
}

#[test]
fn graph_atoms_are_borrowed_until_budget_check() {
    // A reader can return existing atom storage without allocating any copies.
    struct Atom(String);
    impl LispGraph for Atom {
        type Node = ();
        fn view(&self, _: ()) -> NodeView<'_, ()> {
            NodeView::Str(&self.0)
        }
    }
    let atom = Atom("x".repeat(romacs_face::datum::MAX_BYTES + 1));
    assert!(matches!(
        BoundedDatum::from_graph(&atom, ()),
        Err(DatumError::TooManyBytes)
    ));
}
#[test]
fn owner_preferences_compose_over_bundled_modus() {
    let dump = load_dump("modus-vivendi");
    let (df, spec) = inputs(&dump, &deffaces_table());
    let user = UserOverrides::from_alist(
        &BoundedDatum::parse(include_str!("../examples/user-overrides.eld")).unwrap(),
    )
    .unwrap();
    let t = Theme::build_with_overrides(df, spec, Arc::new(user), dump.mode, base_from(&dump.base))
        .unwrap();
    assert_eq!(
        attrs_get(&t.realize("default").unwrap(), A::Height),
        d("120"),
        "owner preferences default height"
    );
    assert_eq!(
        attrs_get(&t.realize("default").unwrap(), A::Family),
        d("\"Iosevka\"")
    );
    assert_eq!(
        attrs_get(&t.realize("italic").unwrap(), A::Slant),
        d("italic")
    );
    assert_eq!(
        attrs_get(&t.realize("org-modern-symbol").unwrap(), A::Family),
        d("\"Iosevka Aile\"")
    );
    assert_eq!(
        attrs_get(&t.realize("variable-pitch").unwrap(), A::Family),
        d("\"Iosevka\"")
    );
    for face in [
        "fringe",
        "window-divider",
        "window-divider-first-pixel",
        "window-divider-last-pixel",
    ] {
        for attr in [A::Foreground, A::Background] {
            assert_eq!(
                attrs_get(&t.realize(face).unwrap(), attr),
                d("\"#000000\""),
                "owner {face}/{}",
                attr.keyword()
            );
        }
    }
    assert_eq!(
        attrs_get(&t.realize("default").unwrap(), A::Background),
        *plist_get(&dump.default_row, ":background").unwrap()
    );
}
#[test]
fn alias_names_at_theme_and_user_boundaries() {
    let mut df = registry(&[("real", "((t :foreground \"green\"))")]);
    df.alias("old", "real").unwrap();
    let mut spec = ThemeSpec::new("alias-spec");
    spec.set_face("old", &d("((t :foreground \"blue\"))"))
        .unwrap();
    let t = Theme::build(
        Arc::new(df.clone()),
        Some(Arc::new(spec)),
        Background::Dark,
        base(113),
    )
    .unwrap();
    assert_eq!(
        attrs_get(&t.realize("real").unwrap(), A::Foreground),
        d("\"blue\""),
        "theme alias key must resolve"
    );
    let mut user = UserOverrides::default();
    user.set_face("old", &d("(:foreground \"red\")")).unwrap();
    let t = Theme::build_with_overrides(
        Arc::new(df),
        None,
        Arc::new(user),
        Background::Dark,
        base(113),
    )
    .unwrap();
    assert_eq!(
        attrs_get(&t.realize("real").unwrap(), A::Foreground),
        d("\"red\""),
        "user alias key must resolve"
    );
    let t = t.with_face_attribute("old", ":weight", &d("bold")).unwrap();
    assert_eq!(
        attrs_get(&t.realize("real").unwrap(), A::Weight),
        d("bold"),
        "set-face alias key must resolve"
    );
}
// Subprocess isolation makes an abort a failed parent assertion rather than a
// dead test runner. These are the review's million-deep reproductions.
#[test]
fn million_quotes_do_not_abort() {
    if std::env::var_os("S102_QUOTE_CHILD").is_some() {
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(|| {
                assert!(matches!(
                    Datum::parse(&format!("{}x", "'".repeat(1_000_000))),
                    Err(DatumError::TooDeep)
                ));
            })
            .unwrap()
            .join()
            .unwrap();
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "million_quotes_do_not_abort", "--nocapture"])
        .env("S102_QUOTE_CHILD", "1")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "million quotes must return TooDeep without abort: {status}"
    );
}

#[test]
fn million_raw_nodes_do_not_enter_face_apis() {
    if std::env::var_os("S102_RAW_CHILD").is_some() {
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(|| {
                let mut raw = Datum::Nil;
                for _ in 0..1_000_000 {
                    raw = Datum::List(vec![raw]);
                }
                let result = BoundedDatum::new(&raw);
                assert!(
                    matches!(result, Err(DatumError::TooDeep)),
                    "reject raw before clone"
                );
                // Raw construction is deliberately untrusted; its owner must not
                // recursively drop it. No raw tree enters a face or remap API.
                while let Datum::List(mut items) = raw {
                    raw = items.pop().unwrap();
                }
            })
            .unwrap()
            .join()
            .unwrap();
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "million_raw_nodes_do_not_enter_face_apis",
            "--nocapture",
        ])
        .env("S102_RAW_CHILD", "1")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "raw million-deep tree must be rejected without cloning: {status}"
    );
}
#[test]
fn romacs_matches_graphical_specs() {
    for selector in ["graphic", "romacs", "x w32 ns haiku pgtk android", "gtk"] {
        let spec = d(&format!("((((type {selector})) :weight bold))"));
        let c = Collapsed::collapse("graphical", &spec).unwrap();
        assert_eq!(
            c.get(Background::Dark)
                .map(|f| f.attrs().get(A::Weight).to_lisp()),
            Some("bold".into()),
            "graphical selector {selector} must match"
        );
    }
}
#[test]
fn duplicate_preference_and_alias_keys_are_explicit_errors() {
    assert!(
        UserOverrides::from_alist(&d("((same :weight bold) (same :weight light))")).is_err(),
        "duplicate user preference keys must be rejected"
    );
    let mut df = registry(&[("real", "((t))")]);
    df.alias("old", "real").unwrap();
    let mut spec = ThemeSpec::new("ambiguous");
    spec.set_face("old", &d("((t :weight bold))")).unwrap();
    spec.set_face("real", &d("((t :weight light))")).unwrap();
    assert!(
        Theme::build(
            Arc::new(df),
            Some(Arc::new(spec)),
            Background::Dark,
            base(113)
        )
        .is_err(),
        "duplicate canonical theme keys must be rejected"
    );
}

/// Repeated `set-face-attribute' edits keep only a keyword's latest value,
/// and the slot `:bold' and `:weight' share still ends as the last edit to
/// either set it, whatever order they came in.
#[test]
fn repeated_attribute_edits_keep_the_last_word() {
    let t = theme(&[("f", "((t))")]);
    let weight = |t: &Theme| attrs_get(&t.realize("f").unwrap(), A::Weight);
    let edits = |pairs: &[(&str, &str)]| {
        pairs.iter().fold(t.clone(), |t, (k, v)| t.with_face_attribute("f", k, &d(v)).unwrap())
    };
    assert_eq!(weight(&edits(&[(":weight", "light"), (":bold", "t"), (":weight", "heavy")])), d("heavy"));
    assert_eq!(weight(&edits(&[(":bold", "t"), (":weight", "light"), (":bold", "nil")])), d("normal"));
    assert_eq!(weight(&edits(&[(":weight", "light"), (":bold", "t")])), d("bold"));
    let many = (0..2000).fold(t.clone(), |t, i| {
        t.with_face_attribute("f", ":weight", &d(if i % 2 == 0 { "light" } else { "heavy" })).unwrap()
    });
    assert_eq!(weight(&many), d("heavy"));
}
