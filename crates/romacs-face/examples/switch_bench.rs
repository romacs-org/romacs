//! Theme switching cost: `cargo run --release -p romacs-face --example switch_bench`.
//!
//! Builds tango-dark and wombat from the GNU oracle fixtures (the same inputs
//! the parity test uses), then times (a) building each theme once, which is
//! where spec collapse and inheritance resolution happen, and (b) switching
//! the active theme back and forth, which is all a theme change costs after
//! that. Compare with GNU's `enable-theme`, which replays every face spec.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use romacs_face::attr::validate;
use romacs_face::{
    ActiveTheme, Attribute, AttrsBuilder, Background, Base, Datum, Deffaces, Theme, ThemeSpec,
};

fn plist_get<'a>(plist: &'a Datum, key: &str) -> Option<&'a Datum> {
    plist
        .as_list()?
        .chunks(2)
        .find(|kv| kv[0].as_symbol() == Some(key))
        .and_then(|kv| kv.get(1))
}

fn load(
    dir: &Path,
    name: &str,
    table: &[(String, Datum)],
) -> (Arc<Deffaces>, Arc<ThemeSpec>, Background, Base) {
    let dump =
        Datum::parse(&std::fs::read_to_string(dir.join(format!("{name}.eld"))).unwrap()).unwrap();
    let faces = plist_get(&dump, ":faces").unwrap().as_list().unwrap();
    let mut df = Deffaces::default();
    let mut spec = ThemeSpec::new(name);
    for e in faces {
        let v = e.as_list().unwrap();
        let face = v[0].as_symbol().unwrap();
        let s = table
            .iter()
            .find(|(n, _)| n == face)
            .map(|(_, s)| s.clone())
            .unwrap_or(Datum::Nil);
        df.defface(face, &romacs_face::BoundedDatum::new(&s).unwrap())
            .unwrap();
        if let Some(ts) = v[1].as_list().and_then(|l| l.first()) {
            spec.set_face(face, &romacs_face::BoundedDatum::new(ts).unwrap())
                .unwrap();
        }
    }
    let mode = if plist_get(&dump, ":mode").unwrap().as_symbol() == Some("dark") {
        Background::Dark
    } else {
        Background::Light
    };
    let base_plist = plist_get(&dump, ":base").unwrap();
    let mut b = AttrsBuilder::default();
    for a in Attribute::ALL
        .into_iter()
        .filter(|a| *a != Attribute::DistantForeground)
    {
        b.set(
            a,
            validate(
                "base",
                a,
                &romacs_face::BoundedDatum::new(plist_get(base_plist, a.keyword()).unwrap())
                    .unwrap(),
            )
            .unwrap(),
        );
    }
    (
        Arc::new(df),
        Arc::new(spec),
        mode,
        Base::new(b.build()).unwrap(),
    )
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/gnu/fixtures");
    let table: Vec<(String, Datum)> = std::fs::read_to_string(dir.join("deffaces.eld"))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let d = Datum::parse(l).unwrap();
            let v = d.as_list().unwrap();
            (v[0].as_symbol().unwrap().to_owned(), v[1].clone())
        })
        .collect();
    let (df, a_spec, a_mode, base) = load(&dir, "tango-dark", &table);
    let (_, b_spec, b_mode, _) = load(&dir, "wombat", &table);
    let builds = 50;
    let t0 = Instant::now();
    let mut a = None;
    let mut b = None;
    for _ in 0..builds {
        a = Some(Theme::build(df.clone(), Some(a_spec.clone()), a_mode, base.clone()).unwrap());
        b = Some(Theme::build(df.clone(), Some(b_spec.clone()), b_mode, base.clone()).unwrap());
    }
    let build_ms = t0.elapsed().as_secs_f64() * 1000.0 / (2 * builds) as f64;
    let (a, b) = (a.unwrap(), b.unwrap());
    let mut active = ActiveTheme::new(a.clone());
    let switches = 1_000_000u32;
    let t0 = Instant::now();
    for i in 0..switches {
        let next = if i % 2 == 0 { b.clone() } else { a.clone() };
        std::hint::black_box(active.switch(next));
    }
    let switch_ns = t0.elapsed().as_secs_f64() * 1e9 / switches as f64;
    println!(
        "faces={} build-once-ms={build_ms:.3} switch-ns={switch_ns:.1}",
        a.len()
    );
}
