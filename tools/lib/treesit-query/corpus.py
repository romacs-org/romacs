"""Assemble queries from the editor's modes and pinned grammar packages."""
import json
import hashlib
from itertools import product
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from treesit_grammars import crate_dirs, table

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
    samples = json.loads((work / "samples.json").read_text())
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


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def ranges(near):
    return [[0, 2**32 - 1], [max(0, near - 7), near + 8], [near, near]]


def edit_state(text, near, random):
    random ^= random >> 12
    random ^= (random << 25) & (2**64 - 1)
    random ^= random >> 27
    start = min(max(0, near - 8) + random % 17, len(text))
    delete, insert = [(1, ""), (2, "}"), (0, "x"), (0, "\n"), (0, '"'), (0, ","), (0, "(")][random % 7]
    end = min(start + delete, len(text))
    text[start:end] = insert.encode()
    return random, start + len(insert), [start, end, insert]


def states(sample):
    """Freeze every actual edit, resulting input digest and ordered range."""
    for seed in (1, 2, 3):
        text = bytearray(sample.encode())
        random, near = seed, len(text) // 2
        yield [seed, 0, None, ranges(near), sha256(text)]
        for index in range(1, 17):
            random, near, change = edit_state(text, near, random)
            yield [seed, index, change, ranges(near), sha256(text)]


def freeze_cases(cases):
    for case in cases:
        case.update(text_sha256=sha256(case["text"].encode()),
                    query_sha256=[sha256(query.encode()) for query in case["queries"]],
                    states=list(states(case["text"])))
    return cases


def check_cases(cases):
    if [case["language"] for case in cases] != sorted(MODE_MINIMUM):
        raise ValueError("incomplete or duplicate manifest grammar inventory")
    for case in cases:
        fresh = freeze_cases([dict(case)])[0]
        key = ("text_sha256", "query_sha256", "states")
        if any(case[field] != fresh[field] for field in key):
            raise ValueError(f"manifest digests or schedule differ for {case['language']}")
        count = len(case["queries"])
        if len(case["origins"]) != count or not 0 < case["shipped"] <= count:
            raise ValueError(f"invalid manifest sources for {case['language']}")
