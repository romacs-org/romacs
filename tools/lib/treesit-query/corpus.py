"""Assemble queries from the editor's modes and pinned grammar packages."""
import json
from itertools import product
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from treesit_grammars import crate_dirs, table

DATA = Path(__file__).resolve().parent
# Distinct individual, combined and indent queries observed from each mode.
MODE_MINIMUM = dict(bash=20, c=24, cpp=23, css=17, go=21, html=9, java=17,
                    javascript=26, jsdoc=9, json=12, python=21, rust=22,
                    toml=11, tsx=22, typescript=21, yaml=14)
PROBES = [
    "(_) @named", "_ @any", "(ERROR) @error", "(MISSING) @missing",
    "(not_a_query_node)", "(_ @bad", '((_) @n (#match? @n "a\\\\b"))',
    '((_) @n (#eq? @unknown "a"))', "(_ . (_) @first)", "(_ (_) @last .)",
    '(_) @n (#eq? @n "a")', '(#set! "property" "value")',
    '((#is? "property"))',
]
JSON_PROBES = [
    "(pair !key (number) @number) @parent",
    "[(number) (string)] @value",
    "(array (number)* @head . (true)? @tail)",
    '((string) @key . ":" (number) @val)',
    "(pair value: (number) @v) @p",
    "(document (number))",
]
RUST_PROBES = [
    "(expression) @expr", "(expression/identifier) @name", "(type/_type) @t",
    '(("fn") @keyword (#eq? @keyword "fn"))',
    '((identifier) @µ (#eq? @µ "λ"))',
    '((identifier) @name (#eq? @name "λ"))',
    "(identifier) @a.b",
    "(identifier) @a-b",
    "(identifier)\v@name",
]


def repetitions():
    """Quantified children on both sides of anchors, with both branch orders."""
    for first, second, left, right, anchor in product(
        ("(number)", "[(false) (number)]", "[(number) (false)]"),
        ("(string)", "[(null) (string)]", "[(string) (null)]"),
        ("", "?", "*", "+"),
        ("", "?", "+"),
        (". ", ""),
    ):
        yield f"(array {first}{left} @first {anchor}{second}{right} @second) @array"


def build(root, work):
    samples = json.loads((DATA / "samples.json").read_text())
    modes = json.loads((work / "mode-queries.json").read_text())
    dirs = crate_dirs(root)
    cases = []
    pinned = 0
    for name, crate, _, _ in table(root):
        directory = dirs[crate] / "queries"
        files = sorted(path for pattern in ("highlights*.scm", "indents*.scm", "indent*.scm")
                       for path in directory.rglob(pattern))
        files = list(dict.fromkeys(files))
        pinned += len(files)
        queries = [path.read_text() for path in files]
        origins = [f"{crate}/queries/{path.relative_to(directory)}" for path in files]
        shipped = dict(modes[name])
        if len(shipped) < MODE_MINIMUM[name]:
            raise ValueError(f"{name} only extracted {len(shipped)} mode queries")
        queries.extend(shipped)
        origins.extend(shipped.values())
        focused = list(PROBES)
        if name == "json":
            focused.extend(JSON_PROBES)
            focused.extend(repetitions())
        if name == "rust":
            focused.extend(RUST_PROBES)
        queries.extend(focused)
        origins.extend(["hand-written probe"] * len(focused))
        cases.append(dict(language=name, text=samples[name], queries=queries,
                          origins=origins, shipped=len(shipped) + len(files)))
    if pinned < 18 or set(samples) != {case["language"] for case in cases}:
        raise ValueError(f"incomplete grammar inventory ({pinned} pinned query files)")
    return cases
