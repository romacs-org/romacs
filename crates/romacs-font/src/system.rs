//! Reuse discovery, not mutable shaping state, across a native catalog epoch.
use cosmic_text::FontSystem;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
};

static EPOCH: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT: OnceLock<Mutex<CatalogSnapshot>> = OnceLock::new();

#[derive(Default)]
struct CatalogSnapshot {
    value: Option<(u64, String, fontdb::Database)>,
}

impl CatalogSnapshot {
    fn system(&mut self, epoch: u64, load: impl FnOnce() -> FontSystem) -> FontSystem {
        if self
            .value
            .as_ref()
            .is_none_or(|(observed, _, _)| *observed != epoch)
        {
            let (locale, db) = load().into_locale_and_db();
            self.value = Some((epoch, locale, db));
        }
        let (_, locale, db) = self.value.as_ref().expect("catalog was just initialized");
        FontSystem::new_with_locale_and_db(locale.clone(), db.clone())
    }
}

/// Invalidate discovery on the native catalog owner's published change edge.
/// A change racing discovery leaves an old epoch on that snapshot, so the next
/// constructor rebuilds it; existing services also retain their native cursor.
pub fn invalidate() {
    EPOCH.fetch_add(1, Ordering::Release);
}

/// Force discovery for a consumer that observed a catalog replacement. This
/// also covers injected/custom backends that do not publish the native counter.
pub fn reload_font_system() -> FontSystem {
    invalidate();
    new_font_system()
}

/// Each caller owns its own fontdb and cosmic shaping caches. Only the initial
/// unmodified database is cloned; pinning a file in one service cannot leak
/// synthetic family names, loaded fonts, or generation-local ids into another.
pub fn new_font_system() -> FontSystem {
    let epoch = EPOCH.load(Ordering::Acquire);
    SNAPSHOT
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .system(epoch, FontSystem::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn empty_system() -> FontSystem {
        let mut db = fontdb::Database::new();
        db.set_monospace_family("Original");
        FontSystem::new_with_locale_and_db("en-US".into(), db)
    }

    #[test]
    fn repeated_services_discover_once_per_catalog_epoch() {
        let scans = Cell::new(0);
        let mut snapshot = CatalogSnapshot::default();
        let make = || {
            scans.set(scans.get() + 1);
            empty_system()
        };
        let _first = snapshot.system(0, make);
        let _second = snapshot.system(0, make);
        assert_eq!(
            scans.get(),
            1,
            "one native catalog must not be rescanned for every service"
        );
        let _changed = snapshot.system(1, make);
        assert_eq!(scans.get(), 2, "catalog changes must rebuild discovery");
    }

    #[test]
    fn service_database_mutation_is_private() {
        let mut snapshot = CatalogSnapshot::default();
        let mut first = snapshot.system(0, empty_system);
        first.db_mut().set_monospace_family("Private");
        let second = snapshot.system(0, empty_system);
        assert_eq!(
            second.db().family_name(&fontdb::Family::Monospace),
            "Original"
        );
        assert_eq!(
            first.db().family_name(&fontdb::Family::Monospace),
            "Private"
        );
    }
}
