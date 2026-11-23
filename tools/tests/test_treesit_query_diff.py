"""Failure cases for the complete tree-sitter query record comparison."""
from contextlib import redirect_stdout
from copy import deepcopy
import importlib.machinery
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "treesit-query-diff"
LOADER = importlib.machinery.SourceFileLoader("treesit_query_diff", str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
DIFF = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(DIFF)
from records import capture, probe_expectations


def fixture():
    case = dict(language="json", queries=["(_) @n"] * 8, origins=["shipped.el"] * 8,
                shipped=8, states=[[1, 0, None, [[0, 10]], "hash"],
                                    [1, 1, [0, 0, "x"], [[0, 10]], "hash2"]])
    rows = [["grammar", "json", 8, 14]]
    rows += [["compile", i, ["ok", {"patterns": 1, "captures": [[110]], "predicates": [[]]}]]
             for i in range(8)]
    matches = [[0, [capture(0, 1, 10, 0, 1), capture(0, 2, 10, 1, 2)]]]
    for seed, index, change, ranges, digest in case["states"]:
        rows.append(["state", seed, index, change, digest])
        rows += [["run", i, 0, ranges[0], deepcopy(matches)] for i in range(8)]
    return case, rows


def probes(rust):
    return [["probe", name, index, new if rust else old]
            for name, rows in probe_expectations() for index, (old, new) in enumerate(rows)]


class QueryDiffTest(unittest.TestCase):
    def compare(self, case, old, new):
        with tempfile.TemporaryDirectory() as directory:
            before, after = (Path(directory) / name for name in ("before", "after"))
            for path, rows in ((before, old), (after, new)):
                path.write_text("".join(json.dumps(row) + "\n" for row in rows))
            with redirect_stdout(io.StringIO()):
                DIFF.compare(before, after, [case], True)

    def test_each_manifest_query_requires_one_compile_and_one_run_per_edit(self):
        case, records = fixture()
        self.compare(case, records + probes(False), records + probes(True))
        for label, cut, duplicate in (
            ("missing compile", lambda row: row[:2] != ["compile", 2], None),
            ("missing query and runs", lambda row: row[:2] not in (["compile", 2], ["run", 2]), None),
            ("duplicate compile", lambda row: True, 2),
            ("missing run", lambda row: row is not records[-2], None),
            ("duplicate run", lambda row: True, len(records) - 1),
            ("missing edit", lambda row: row is not records[18], None),
            ("out of manifest", lambda row: True, None),
        ):
            with self.subTest(label=label):
                altered = [row for row in records if cut(row)]
                if duplicate is not None:
                    altered.insert(duplicate, records[duplicate])
                if label == "out of manifest":
                    altered.insert(5, ["compile", 9, ["error", 0, "Syntax error at"]])
                with self.assertRaisesRegex(ValueError, "expected"):
                    self.compare(case, altered + probes(False), altered + probes(True))

    def test_dropped_extra_reordered_and_wrong_capture(self):
        case, old = fixture()
        for action in ("drop", "extra", "swap", "wrong kind"):
            with self.subTest(action=action):
                new = deepcopy(old)
                captures = new[-1][-1][0][1]
                if action == "drop":
                    captures.pop()
                elif action == "extra":
                    captures.append(captures[0])
                elif action == "swap":
                    captures.reverse()
                else:
                    captures[0][2] = 8
                with self.assertRaisesRegex(ValueError, "seed 1 edit 1 range 0.*shipped.el"):
                    self.compare(case, old + probes(False), new + probes(True))

    def test_known_exceptions_require_exact_nodes(self):
        case, records = fixture()
        for label, arm in (("wildcard-negated-field", False),
                           ("fourth-capture", True), ("wide-step-index", True)):
            with self.subTest(label=label):
                c = probes(False)
                rust = probes(True)
                row = next(row for row in (rust if arm else c) if row[1] == label and row[-1])
                row[-1][0][1][0][4] = [999, 1001]
                with self.assertRaisesRegex(ValueError, f"probe {label} case"):
                    self.compare(case, records + c, records + rust)

    def test_extra_record_after_last_probe_fails(self):
        case, records = fixture()
        with self.assertRaisesRegex(ValueError, "unexpected extra record"):
            self.compare(case, records + probes(False), records + probes(True) + [["other"]])


if __name__ == "__main__":
    unittest.main()
