"""The query differential must reject new errors and narrow records."""
import importlib.machinery
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "treesit-query-diff"
LOADER = importlib.machinery.SourceFileLoader("treesit_query_diff", str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
DIFF = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(DIFF)


def capture(index):
    return [index, 3, 7, 1, [4, 5], [[0, 4], [0, 5]]]


def rows(*ids, pattern=0):
    return [[pattern, [capture(index) for index in ids]]]


class QueryDiffTest(unittest.TestCase):
    def test_only_named_reproductions_may_differ(self):
        common = [rows(0) * n for n in (2, 1, 2, 1)]
        old = {"subtree-and-bounds": common, "wide-step-index": [[]],
               "wildcard-negated-field": [[], rows(1, 0)], "fourth-capture": [rows(0, 1, 2)]}
        new = {"subtree-and-bounds": common, "wide-step-index": [rows(1, pattern=32768)],
               "wildcard-negated-field": [[], []], "fourth-capture": [rows(0, 1, 2, 3)]}
        DIFF.verify_probes(old, new, True)
        new["fourth-capture"][0][0][1][3][4] = [5, 6]
        with self.assertRaisesRegex(ValueError, "changed the captured node"):
            DIFF.verify_probes(old, new, True)

    def test_equal_node_identity_does_not_hide_wrong_context(self):
        case = dict(language="json", queries=["(_) @n"], origins=["shipped.el"], shipped=1)
        with tempfile.TemporaryDirectory() as directory:
            before, after = (Path(directory) / name for name in ("before", "after"))
            header = "grammar json 14 1\nseed 1 edit 0\n"
            old, new = rows(0), rows(0)
            new[0][1][0][2] = 8
            before.write_text(header + f"0 0 0 {json.dumps(old)}\n")
            after.write_text(header + f"0 0 0 {json.dumps(new)}\n")
            with self.assertRaisesRegex(ValueError, "shipped.el"):
                DIFF.compare(before, after, [case], True)
            after.write_text(before.read_text())
            with self.assertRaisesRegex(ValueError, "insufficient coverage"):
                DIFF.compare(before, after, [case], True)


if __name__ == "__main__":
    unittest.main()
