# Bundled tree-sitter grammars

The grammars Romacs links into its executable (`crates/romacs-grammars`,
DIVERGENCES.md VM2-TREESIT-BUNDLED). Each one is the commit its GNU ts mode
pins in `treesit-language-source-alist`, so a ts mode parses with the
grammar it was written against. A grammar the user installs in
`treesit-extra-load-path` or `user-emacs-directory`'s `tree-sitter/`
directory is used in its place.

The crates are crates.io releases whose sources are that commit's, or the
commit itself as a git dependency where no release is.
`crates/romacs-grammars/sources.json` records the SHA-256 of every source
file each build compiles, as fetched from the upstream repository at the
commit; `tools/lib/treesit_grammars.py check` (run by `tools/check-derived`)
holds Cargo's sources to it. `tools/lib/treesit_grammars.py so --out DIR`
builds the same grammars as shared libraries, which GNU Emacs loads from
`treesit-extra-load-path`.

All are MIT licensed; each licence is in `LICENSES/`, as the upstream
repository has it at the commit.

| Language | Repository | Commit | Crate | Licence | Licence file |
|---|---|---|---|---|---|
| bash | https://github.com/tree-sitter/tree-sitter-bash | `487734f87fd87118028a65a4599352fa99c9cde8` | tree-sitter-bash 0.23.3 (crates.io) | MIT, Copyright (c) 2017 Max Brunsfeld | `LICENSES/tree-sitter-bash.LICENSE` |
| c | https://github.com/tree-sitter/tree-sitter-c | `3aa2995549d5d8b26928e8d3fa2770fd4327414e` | tree-sitter-c 0.23.4 (git) | MIT, Copyright (c) 2014 Max Brunsfeld | `LICENSES/tree-sitter-c.LICENSE` |
| cpp | https://github.com/tree-sitter/tree-sitter-cpp | `f41b4f66a42100be405f96bdc4ebc4a61095d3e8` | tree-sitter-cpp 0.23.4 (crates.io) | MIT, Copyright (c) 2014 Max Brunsfeld | `LICENSES/tree-sitter-cpp.LICENSE` |
| css | https://github.com/tree-sitter/tree-sitter-css | `6a442a3cf461b0ce275339e5afa178693484c927` | tree-sitter-css 0.23.1 (crates.io) | MIT, Copyright (c) 2018 Max Brunsfeld | `LICENSES/tree-sitter-css.LICENSE` |
| go | https://github.com/tree-sitter/tree-sitter-go | `12fe553fdaaa7449f764bc876fd777704d4fb752` | tree-sitter-go 0.23.4 (git) | MIT, Copyright (c) 2014 Max Brunsfeld | `LICENSES/tree-sitter-go.LICENSE` |
| html | https://github.com/tree-sitter/tree-sitter-html | `d9219ada6e1a2c8f0ab0304a8bd9ca4285ae0468` | tree-sitter-html 0.23.2 (crates.io) | MIT, Copyright (c) 2014 Max Brunsfeld | `LICENSES/tree-sitter-html.LICENSE` |
| javascript | https://github.com/tree-sitter/tree-sitter-javascript | `108b2d4d17a04356a340aea809e4dd5b801eb40d` | tree-sitter-javascript 0.23.1 (crates.io) | MIT, Copyright (c) 2014 Max Brunsfeld | `LICENSES/tree-sitter-javascript.LICENSE` |
| jsdoc | https://github.com/tree-sitter/tree-sitter-jsdoc | `b253abf68a73217b7a52c0ec254f4b6a7bb86665` | tree-sitter-jsdoc 0.23.2 (crates.io) | MIT, Copyright (c) 2018 Max Brunsfeld | `LICENSES/tree-sitter-jsdoc.LICENSE` |
| json | https://github.com/tree-sitter/tree-sitter-json | `4d770d31f732d50d3ec373865822fbe659e47c75` | tree-sitter-json 0.24.8 (crates.io) | MIT, Copyright (c) 2014 Max Brunsfeld | `LICENSES/tree-sitter-json.LICENSE` |
| markdown | https://github.com/tree-sitter-grammars/tree-sitter-markdown | `413285231ce8fa8b11e7074bbe265b48aa7277f9` | tree-sitter-md 0.4.0 (git) | MIT, Copyright (c) 2021 Matthias Deiml | `LICENSES/tree-sitter-markdown.LICENSE` |
| markdown-inline | https://github.com/tree-sitter-grammars/tree-sitter-markdown | `413285231ce8fa8b11e7074bbe265b48aa7277f9` | tree-sitter-md 0.4.0 (git) | MIT, Copyright (c) 2021 Matthias Deiml | `LICENSES/tree-sitter-markdown.LICENSE` |
| python | https://github.com/tree-sitter/tree-sitter-python | `bffb65a8cfe4e46290331dfef0dbf0ef3679de11` | tree-sitter-python 0.23.6 (crates.io) | MIT, Copyright (c) 2016 Max Brunsfeld | `LICENSES/tree-sitter-python.LICENSE` |
| rust | https://github.com/tree-sitter/tree-sitter-rust | `18b0515fca567f5a10aee9978c6d2640e878671a` | tree-sitter-rust 0.24.0 (crates.io) | MIT, Copyright (c) 2017 Maxim Sokolov | `LICENSES/tree-sitter-rust.LICENSE` |
| toml | https://github.com/tree-sitter-grammars/tree-sitter-toml | `64b56832c2cffe41758f28e05c756a3a98d16f41` | tree-sitter-toml-ng 0.7.0 (crates.io) | MIT, Copyright (c) Ika | `LICENSES/tree-sitter-toml.LICENSE` |
| tsx | https://github.com/tree-sitter/tree-sitter-typescript | `8e13e1db35b941fc57f2bd2dd4628180448c17d5` | tree-sitter-typescript 0.23.2 (crates.io) | MIT, Copyright (c) 2017 Max Brunsfeld | `LICENSES/tree-sitter-typescript.LICENSE` |
| typescript | https://github.com/tree-sitter/tree-sitter-typescript | `8e13e1db35b941fc57f2bd2dd4628180448c17d5` | tree-sitter-typescript 0.23.2 (crates.io) | MIT, Copyright (c) 2017 Max Brunsfeld | `LICENSES/tree-sitter-typescript.LICENSE` |
| yaml | https://github.com/tree-sitter-grammars/tree-sitter-yaml | `b733d3f5f5005890f324333dd57e1f0badec5c87` | tree-sitter-yaml 0.7.0 (crates.io) | MIT, Copyright (c) 2024 tree-sitter-grammars contributors | `LICENSES/tree-sitter-yaml.LICENSE` |
