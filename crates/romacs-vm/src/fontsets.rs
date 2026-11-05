//! The fontset registry (GNU's Vfontset_table): every base fontset by id,
//! the default one 0, with an index of names. What painting needs of it is
//! published as a `romacs_layout_input::FontsetState`; the id table and the
//! index stay here, so neither is copied per publication.
use romacs_layout_input::{Fontset, FontsetState};
use std::collections::HashMap;
use std::sync::Arc;

/// `fontset-default''s full name.
pub(crate) const DEFAULT_NAME: &str = "-*-*-*-*-*-*-*-*-*-*-*-*-fontset-default";

pub(crate) struct Registry {
    sets: Vec<Arc<Fontset>>,
    /// Lower-case full names and aliases, for literal lookups.
    names: HashMap<String, usize>,
    /// The frame's fontset.
    pub frame: usize,
    /// Bumped by every edit, and by a change of the charset order.
    revision: u64,
    charset_tick: u64,
    /// The `symbol' script's ranges, when `use-default-font-for-symbols' is
    /// on: GNU's check of `char-script-table', read once.
    symbols: Arc<Vec<(u32, u32)>>,
    /// `char-script-table''s ranges by script, as of its `edits' count.
    scripts: HashMap<u64, Arc<Vec<(u32, u32)>>>,
    script_edits: u64,
    /// The default rules, made while the world was built, for the display
    /// that opens: a batch session has none, as GNU's.
    staged: Option<(Vec<Arc<Fontset>>, HashMap<String, usize>)>,
    published: Arc<FontsetState>,
}
romacs_snapshot::snapshot_struct!(Registry { sets, names, frame, revision, charset_tick, symbols, scripts, script_edits, staged, published });

impl Default for Registry {
    fn default() -> Self {
        let default = Arc::new(Fontset::new(DEFAULT_NAME));
        let names = [(DEFAULT_NAME.to_string(), 0), ("fontset-default".to_string(), 0)].into();
        Self { sets: vec![default], names, frame: 0, revision: 0, charset_tick: 0, symbols: Arc::default(), scripts: HashMap::new(), script_edits: 0, staged: None, published: Arc::default() }
    }
}

impl Registry {
    pub(crate) fn get(&self, id: usize) -> &Fontset {
        &self.sets[id]
    }

    /// Fontset ID to change: what painting holds keeps its own copy of the
    /// nodes the edit touches.
    pub(crate) fn edit(&mut self, id: usize) -> &mut Fontset {
        self.revision += 1;
        Arc::make_mut(&mut self.sets[id])
    }

    /// A new fontset named NAME.
    pub(crate) fn make(&mut self, name: &str) -> usize {
        self.sets.push(Arc::new(Fontset::new(name)));
        let id = self.sets.len() - 1;
        self.names.insert(name.to_lowercase(), id);
        id
    }

    pub(crate) fn alias(&mut self, alias: &str, id: usize) {
        self.names.insert(alias.to_lowercase(), id);
    }

    pub(crate) fn named(&self, name: &str) -> Option<usize> {
        self.names.get(&name.to_lowercase()).copied()
    }

    /// Every fontset, oldest first.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (usize, &Arc<Fontset>)> {
        self.sets.iter().enumerate()
    }

    /// SCRIPT's ranges as of the table's EDITS, if known.
    pub(crate) fn script(&mut self, script: u64, edits: u64) -> Option<Arc<Vec<(u32, u32)>>> {
        if self.script_edits != edits {
            self.scripts.clear();
            self.script_edits = edits;
        }
        self.scripts.get(&script).cloned()
    }

    /// Every script's ranges, as of the table's edits count last given.
    pub(crate) fn set_scripts(&mut self, scripts: HashMap<u64, Arc<Vec<(u32, u32)>>>) {
        self.scripts = scripts;
    }

    /// Put the fontsets written so far aside for `unstage', leaving only an
    /// empty default fontset.
    pub(crate) fn stage(&mut self) {
        let fresh = Self::default();
        self.staged = Some((std::mem::replace(&mut self.sets, fresh.sets), std::mem::replace(&mut self.names, fresh.names)));
        self.revision += 1;
    }

    /// Install the staged fontsets; false when there are none.
    pub(crate) fn unstage(&mut self) -> bool {
        let Some((sets, names)) = self.staged.clone() else { return false };
        (self.sets, self.names) = (sets, names);
        self.revision += 1;
        true
    }

    /// Whether the symbol ranges are SYMBOLS, as the option now says.
    pub(crate) fn symbols_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    pub(crate) fn set_symbols(&mut self, symbols: Vec<(u32, u32)>) {
        self.symbols = Arc::new(symbols);
        self.revision += 1;
    }

    /// What painting reads, rebuilt when an edit or CHARSET_TICK moved: a few
    /// Arc clones and ORDER, the charset order, which only runs then.
    pub(crate) fn state(&mut self, charset_tick: u64, order: impl FnOnce() -> (HashMap<u32, u32>, u32)) -> (Arc<FontsetState>, bool) {
        if charset_tick != self.charset_tick {
            self.charset_tick = charset_tick;
            self.revision += 1;
        }
        if self.published.revision == self.revision {
            return (Arc::clone(&self.published), false);
        }
        let (rank, preferred) = order();
        let mut state = FontsetState::new(Arc::clone(&self.sets[0]), Arc::clone(&self.sets[self.frame]), Arc::new(rank), preferred, self.revision);
        state.default_font_symbols = Arc::clone(&self.symbols);
        self.published = Arc::new(state);
        (Arc::clone(&self.published), true)
    }

    /// The state last published.
    pub(crate) fn published(&self) -> &Arc<FontsetState> {
        &self.published
    }
}
