//! A resolution cache keyed by value identity.
//!
//! Realising a face is a pure function of `(theme, remapping, face)`. Both
//! values carry a [`Generation`] minted when they were built and never
//! changed after, so `(theme generation, remap generation, face)` names one
//! result forever: an entry can never be stale, and there is no counter for
//! anyone to forget to bump. Invalidation is only ever a memory question,
//! answered by [`FaceCache::retain`].

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::attr::Attrs;
use crate::remap::{Remap, View};
use crate::spec::FaceError;
use crate::theme::{Generation, Theme};

type FaceKey = (Generation, Generation, Arc<str>);

#[derive(Default)]
pub struct FaceCache {
    views: FxHashMap<(Generation, Generation), Arc<View>>,
    faces: FxHashMap<FaceKey, Option<Arc<Attrs>>>,
    hits: u64,
    misses: u64,
}

impl FaceCache {
    pub fn new() -> FaceCache {
        FaceCache::default()
    }

    /// The bound view for THEME under REMAP, built once per pair.
    pub fn view(&mut self, theme: &Theme, remap: &Arc<Remap>) -> Result<Arc<View>, FaceError> {
        let key = (theme.generation(), remap.generation());
        if let Some(v) = self.views.get(&key) {
            return Ok(v.clone());
        }
        let v = Arc::new(View::bind(theme, remap)?);
        self.views.insert(key, v.clone());
        Ok(v)
    }

    /// FACE realised under THEME and REMAP; `Ok(None)` for an unknown face.
    pub fn realize(
        &mut self,
        theme: &Theme,
        remap: &Arc<Remap>,
        face: &str,
    ) -> Result<Option<Arc<Attrs>>, FaceError> {
        let key: FaceKey = (theme.generation(), remap.generation(), face.into());
        if let Some(hit) = self.faces.get(&key) {
            self.hits += 1;
            return Ok(hit.clone());
        }
        self.misses += 1;
        let value = if remap.is_empty() {
            theme.realize(face).map(Arc::new)
        } else {
            self.view(theme, remap)?.realize(face).map(Arc::new)
        };
        self.faces.insert(key, value.clone());
        Ok(value)
    }

    /// Drop every entry whose theme is not in LIVE.
    pub fn retain(&mut self, live: &[Generation]) {
        self.views.retain(|(t, _), _| live.contains(t));
        self.faces.retain(|(t, _, _), _| live.contains(t));
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }
}
