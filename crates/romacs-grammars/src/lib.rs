//! The tree-sitter grammars Romacs ships (DIVERGENCES VM2-TREESIT-BUNDLED).
//! Each is the commit its GNU ts mode pins in `treesit-language-source-alist',
//! linked into the executable; `romacs-vm-gui' registers [`BUNDLED`] with the
//! VM's grammar loader, which finds them after the user's grammar directories
//! and before the system's.
use tree_sitter_language::LanguageFn;

/// One shipped grammar: GNU's language symbol, its grammar function, and the
/// repository and commit its sources are.
pub struct Bundled {
    pub name: &'static str,
    pub language: LanguageFn,
    pub url: &'static str,
    pub commit: &'static str,
}

macro_rules! bundled {
    ($($name:literal $language:expr, $url:literal $commit:literal;)*) => {
        &[$(Bundled { name: $name, language: $language, url: $url, commit: $commit }),*]
    };
}

/// The grammars, by GNU's language name.
pub const BUNDLED: &[Bundled] = bundled! {
    "bash" tree_sitter_bash::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-bash" "487734f87fd87118028a65a4599352fa99c9cde8";
    "c" tree_sitter_c::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-c" "3aa2995549d5d8b26928e8d3fa2770fd4327414e";
    "cpp" tree_sitter_cpp::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-cpp" "f41b4f66a42100be405f96bdc4ebc4a61095d3e8";
    "css" tree_sitter_css::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-css" "6a442a3cf461b0ce275339e5afa178693484c927";
    "go" tree_sitter_go::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-go" "12fe553fdaaa7449f764bc876fd777704d4fb752";
    "html" tree_sitter_html::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-html" "d9219ada6e1a2c8f0ab0304a8bd9ca4285ae0468";
    "javascript" tree_sitter_javascript::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-javascript" "108b2d4d17a04356a340aea809e4dd5b801eb40d";
    "jsdoc" tree_sitter_jsdoc::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-jsdoc" "b253abf68a73217b7a52c0ec254f4b6a7bb86665";
    "json" tree_sitter_json::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-json" "4d770d31f732d50d3ec373865822fbe659e47c75";
    "markdown" tree_sitter_md::LANGUAGE, "https://github.com/tree-sitter-grammars/tree-sitter-markdown" "413285231ce8fa8b11e7074bbe265b48aa7277f9";
    "markdown-inline" tree_sitter_md::INLINE_LANGUAGE, "https://github.com/tree-sitter-grammars/tree-sitter-markdown" "413285231ce8fa8b11e7074bbe265b48aa7277f9";
    "python" tree_sitter_python::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-python" "bffb65a8cfe4e46290331dfef0dbf0ef3679de11";
    "rust" tree_sitter_rust::LANGUAGE, "https://github.com/tree-sitter/tree-sitter-rust" "18b0515fca567f5a10aee9978c6d2640e878671a";
    "toml" tree_sitter_toml_ng::LANGUAGE, "https://github.com/tree-sitter-grammars/tree-sitter-toml" "64b56832c2cffe41758f28e05c756a3a98d16f41";
    "tsx" tree_sitter_typescript::LANGUAGE_TSX, "https://github.com/tree-sitter/tree-sitter-typescript" "8e13e1db35b941fc57f2bd2dd4628180448c17d5";
    "typescript" tree_sitter_typescript::LANGUAGE_TYPESCRIPT, "https://github.com/tree-sitter/tree-sitter-typescript" "8e13e1db35b941fc57f2bd2dd4628180448c17d5";
    "yaml" tree_sitter_yaml::LANGUAGE, "https://github.com/tree-sitter-grammars/tree-sitter-yaml" "b733d3f5f5005890f324333dd57e1f0badec5c87";
};
