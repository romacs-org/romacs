#!/usr/bin/env python3
"""Render the GNU-suite floor restatement for review.

    python3 tools/lib/restatement_report.py OLD.json NEW.json [--command FILE] > REPORT.md

OLD is the ratchet in force (parity-ratchet.json), NEW what
`parity_ratchet.py record --out NEW.json` measured. The report shows the
arithmetic from the recorded figure to the honest ones, and lists EVERY
member whose standing changes with its class and evidence. It decides
nothing: the restatement happens only when a maintainer runs `record` with the
`--lower` arguments it prints.
"""
from __future__ import annotations

import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import parity_ratchet as pr  # noqa: E402


def keys(passing: dict) -> set[str]:
    return {f'{f}::{n}' for f, names in passing.items() for n in names}


def render(old: dict, new: dict, evidence: dict, command: str | None) -> str:
    og, ng = old['gnu_suite'], new['gnu_suite']
    old_keys, new_keys = keys(og['passing']), keys(ng['passing'])
    classes = ng['classes']
    budget = keys(ng['vacuous_budget'])
    protected_new = new_keys - budget
    gone = sorted(old_keys - new_keys)
    to_budget = sorted(old_keys & budget)
    gained = sorted(new_keys - old_keys)
    h = ng['honest']
    real = ng['real_ert']
    lines = [
        '# GNU-suite floor restatement: proposal',
        '',
        'Nothing here is applied. The ratchet in force stays as it is until a maintainer runs the',
        '`record --lower` command at the end. Every number below was measured by',
        '`tools/lib/parity_ratchet.py record --out` on the tree named in its provenance.',
        '',
        '## The figures, each labelled',
        '',
        '| Figure | Harness | Value |',
        '|---|---|---|',
        f"| recorded floor today (`gnu_suite.passed`, schema {old.get('schema')}) | {pr.REDUCED_LABEL} | {og['passed']} |",
        f"| reduced-harness passes now (`gnu_suite.passed`) | {pr.REDUCED_LABEL} | {ng['passed']} |",
        f"| protected floor (passes outside the budget; a loss FAILS) | same | {len(protected_new)} |",
        f"| GENUINE (probe-proven; upper bound) | same | {h['genuine']} |",
        f"| **counted**: GENUINE and its file loaded with no failed top-level form | same | **{h['counted']}** |",
        f"| UNPROBEABLE (protected, never counted) | same | {h['unprobeable']} |",
        f"| vacuous budget (may only decrease; a loss is PROGRESS) | same | {h['vacuous_budget']} |",
        f"| **real ERT** (`gnu_suite.real_ert`) | {pr.REAL_LABEL} | **{real['passed']} of {real['of']}** |",
        '',
        '## Arithmetic',
        '',
        f"    recorded floor                      {og['passed']:>4}",
        f"  - no longer pass (vacuous-pass rule, honest failures, other) {len(gone):>4}",
        f"  + new reduced-harness passes        {len(gained):>4}",
        f"  = reduced-harness passes            {ng['passed']:>4}",
        f"  - vacuous budget                    {h['vacuous_budget']:>4}   by class "
        + ', '.join(f'{c} {h["by_class"][c]}' for c in pr.BUDGET_CLASSES),
        f"  = protected floor                   {len(protected_new):>4}   GENUINE {h['genuine']} + UNPROBEABLE "
        f"{h['unprobeable']}",
        f"    of which counted (file loaded)    {h['counted']:>4}",
        f"    real ERT                          {real['passed']:>4} of {real['of']}",
        '',
    ]
    if og['passed'] - len(gone) + len(gained) != ng['passed']:
        lines.append('**ARITHMETIC DOES NOT CLOSE -- do not apply.**')

    def row(key, why_from_new=True):
        f, t = key.split('::', 1)
        c = classes.get(key)
        e = evidence.get(key) or {}
        cls = c['class'] if c else f"(not passing now; probe said {e.get('class', 'nothing')})"
        why = (c or {}).get('why') or e.get('why') or ''
        loaded = '' if not c else ('loaded' if c['file_loaded'] else f"file failed forms {c['failed_forms'][:4]}")
        return f'| `{f}` | `{t}` | {cls} | {loaded} | {why[:220].replace("|", "/")} |'

    for title, members, note in (
            ('Members leaving the floor because they no longer pass', gone,
             'Measured failing now. Under the vacuous-pass rule a should-error satisfied by absent machinery is not '
             'a pass; the "now" verdict is in the measurement log. Each was a floor member: lowering is needed.'),
            ('Members moving from the protected floor into the vacuous budget', to_budget,
             'Still pass under the reduced harness, but the probe shows the pass is not evidence.'),
            ('New reduced-harness passes (enter the protected floor; counted only if GENUINE and loaded)', gained, ''),
            ('Counted members', sorted(k for k, c in classes.items() if c['counted']), ''),
            ('Protected but NOT counted: GENUINE in a file that failed a top-level form',
             sorted(k for k, c in classes.items() if c['class'] == 'GENUINE' and not c['counted']), ''),
            ('UNPROBEABLE (no probe evidence, or evidence this measurement contradicts)',
             sorted(k for k, c in classes.items() if c['class'] == 'UNPROBEABLE'), '')):
        lines += ['', f'## {title} ({len(members)})', '']
        if note:
            lines += [note, '']
        if members:
            lines += ['| file | test | class | file load | evidence |', '|---|---|---|---|---|']
            lines += [row(k) for k in members]
    lines += ['', '## The single act that applies it', '']
    if command:
        lines += ['```', command.strip(), '```']
    else:
        lines += ['(run `record` without `--lower`: its refusal prints the exact command)']
    return '\n'.join(lines) + '\n'


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 4):
        sys.exit(__doc__)
    old, new = (json.loads(pathlib.Path(a).read_text()) for a in argv[:2])
    command = pathlib.Path(argv[3]).read_text() if len(argv) == 4 else None
    problems = pr.consistency(new) or pr.regeneration_problems(new, pr.load_evidence(pr.DEFAULT_ROOT))
    if problems:
        sys.exit('the proposed ratchet is inconsistent:\n  ' + '\n  '.join(problems))
    sys.stdout.write(render(old, new, pr.load_evidence(pr.DEFAULT_ROOT), command))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
