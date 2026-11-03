//! The tree-sitter grammars this executable ships (crates/romacs-grammars,
//! DIVERGENCES.md VM2-TREESIT-BUNDLED), registered for every VM it runs.

/// Register the bundled grammars with the VM's grammar loader, once per
/// process, before the first VM loads a language.
pub fn register() {
    let grammars = romacs_grammars::BUNDLED.iter().map(|g| romacs_vm::BundledGrammar {
        name: g.name, language: g.language.into_raw(), url: g.url, commit: g.commit,
    }).collect();
    // SAFETY: each is a grammar crate's `LANGUAGE', its `tree_sitter_LANG'
    // linked into this executable, returning its static TSLanguage.
    unsafe { romacs_vm::register_bundled_grammars(grammars) };
}
