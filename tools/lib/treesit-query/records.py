"""Compare the complete ordered record stream for a frozen query corpus.

A differential cannot detect a capture removed from both streams, which is acceptable because the runner records the C arm from the C binary.
"""
from contextlib import ExitStack
import json


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


class Records:
    def __init__(self, stream):
        self.lines = enumerate(stream, 1)
        self.name = stream.name

    def take(self, key, context):
        entry = next(self.lines, None)
        expected = canonical(key)
        if entry is None:
            raise ValueError(f"{self.name}: missing {expected} at {context}")
        line, text = entry
        row = json.loads(text)
        if (not isinstance(row, list) or len(row) != len(key) + 1
                or canonical(row[:-1]) != expected):
            raise ValueError(f"{self.name}:{line}: expected {expected} at {context}, "
                             f"got {text[:180].rstrip()}")
        return row[-1]

    def finish(self):
        if entry := next(self.lines, None):
            raise ValueError(f"{self.name}:{entry[0]}: unexpected extra record")


def require_equal(old, new, context):
    if canonical(old) != canonical(new):
        raise ValueError(f"{context}\nC {str(old)[:400]}\ncandidate {str(new)[:400]}")


def query_context(case, index):
    return f"{case['language']} query {index} ({case['origins'][index]})\n{case['queries'][index]}"


def read_compiles(arms, case):
    compiled = []
    for index in range(len(case["queries"])):
        context = "compile " + query_context(case, index)
        old, new = (arm.take(["compile", index], context) for arm in arms)
        require_equal(old, new, context)
        if isinstance(old, list) and len(old) == 2 and old[0] == "ok":
            compiled.append(index)
        elif not (isinstance(old, list) and len(old) == 3 and old[0] == "error"):
            raise ValueError(f"invalid compile outcome at {context}")
    return compiled


def compare_state(arms, case, state, compiled):
    label = f"seed {state[0]} edit {state[1]}"
    key = ["state", *state[:3]]
    old, new = (arm.take(key, f"{case['language']} {label}") for arm in arms)
    require_equal(old, new, f"{case['language']} {label} tree")
    for index in compiled:
        for slot, bounds in enumerate(state[3]):
            yield index, compare_run(arms, case, label, index, slot, bounds)


def compare_run(arms, case, label, index, slot, bounds):
    context = f"{label} range {slot} {bounds} " + query_context(case, index)
    old, new = (arm.take(["run", index, slot, bounds], context) for arm in arms)
    require_equal(old, new, context)
    if not isinstance(old, list):
        raise ValueError(f"invalid match list at {context}")
    return any(items for _, items in old)


def compare_case(arms, case):
    name = case["language"]
    count = len(case["queries"])
    old, new = (arm.take(["grammar", name, count], name) for arm in arms)
    require_equal(old, new, f"{name} ABI version")
    compiled = read_compiles(arms, case)
    hits = set()
    executions = 0
    for state in case["states"]:
        for index, captured in compare_state(arms, case, state, compiled):
            executions += 1
            if captured:
                hits.add(index)
    if len(compiled) < 8 or len(hits) < 3 or not any(i < case["shipped"] for i in hits):
        raise ValueError(f"insufficient coverage for {name}: {len(compiled)} compiles, "
                         f"{len(hits)} matching queries")
    print(f"{name:12} {count:3} sources  {len(compiled):3} accepted  "
          f"{len(hits):3} with captures  {executions:5} executions")
    return executions


def capture(index, node, symbol, start, end):
    """Expected fact for a named JSON fixture node on the first line."""
    return [index, node, symbol, 1, [start, end], [[0, start], [0, end]]]


def probe_expectations():
    first = [[0, [capture(0, 3, 10, 1, 2)]]]
    second = [[0, [capture(0, 5, 10, 4, 5)]]]
    number = [capture(index, 9, 10, 6, 7) for index in range(4)]
    pair = capture(1, 3, 18, 1, 7)
    return [
        ("subtree-and-bounds", [(first + second, first + second), (second, second),
                                (first + second, first + second), (first, first)]),
        ("wildcard-negated-field", [([], []), ([[0, [pair, number[0]]]], [])]),
        ("fourth-capture", [([[0, number[:3]]], [[0, number]])]),
        ("wide-step-index", [([], [[32768, [capture(1, 1, 11, 0, 4)]]])]),
    ]


def verify_probes(arms, rust):
    for name, probes in probe_expectations():
        for index, (old, new) in enumerate(probes):
            key = ["probe", name, index]
            context = f"probe {name} case {index}"
            for arm, expected in zip(arms, (old, new if rust else old), strict=True):
                actual = arm.take(key, context)
                if canonical(actual) != canonical(expected):
                    raise ValueError(f"{arm.name}: {context}: expected {canonical(expected)}, "
                                     f"got {canonical(actual)}")
            print(f"{context}: C {canonical(old)} → candidate {canonical(new if rust else old)}")


def compare(reference_records, candidate_records, cases, rust):
    with ExitStack() as files:
        arms = [Records(files.enter_context(path.open())) for path in
                (reference_records, candidate_records)]
        executions = sum(compare_case(arms, case) for case in cases)
        verify_probes(arms, rust)
        for arm in arms:
            arm.finish()
    sources = sum(len(case["queries"]) for case in cases)
    shipped = sum(case["shipped"] for case in cases)
    edits = sum(state[1] != 0 for case in cases for state in case["states"])
    print(f"{len(cases)} grammars, {sources} sources, {edits} seeded edits, "
          f"{executions} ordered executions")
    print(f"All {shipped} shipped-query sources compare equal.")
