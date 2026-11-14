//! The tree-sitter grammars this executable ships (crates/romacs-grammars,
//! DIVERGENCES.md VM2-TREESIT-BUNDLED), registered for every VM it runs.

/// Register the bundled grammars with the VM's grammar loader, once per
/// process, before the first VM loads a language.
pub fn register() {
    let grammars = romacs_grammars::BUNDLED.iter().map(|g| romacs_vm::BundledGrammar {
        name: g.name, language: g.language, url: g.url, commit: g.commit,
    }).collect();
    romacs_vm::register_bundled_grammars(grammars);
}
