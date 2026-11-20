#!/usr/bin/env python3
"""Per-key A/B: perkey_live's sessions of a base and a candidate binary, interleaved.

A pair is one perkey_live session of the base binary, then one of the
candidate: each a fresh private headless labwc and a fresh binary, typing
the same keys into the same text, with the same options (any perkey_live
option not named here is passed to both).  Pairs run back to back, so a
load that drifts during the run drifts both arms alike.  Both arms read
the same environment: set ROMACS_RUNTIME_ROOT to one runtime tree both
binaries are compatible with, to compare the binaries alone.  Left unset,
each binary finds its runtime tree as it normally does, and a delta can
come from either tree.

A phase is one of perkey_live's key phases (self-insert, C-n, C-f; region
under --populate).  The verdict rests on its idle-subtracted evaluator
instructions per key (counted sessions: the evaluator thread's perf
counter), which a loaded host barely moves.  Per phase the report gives
each arm's median and spread ((max - min) / median over its sessions) and
the median of the pairs' deltas, (candidate - base) / base, with their
range, and how many pairs counted.  A pair does not count for a phase when
either session was rejected, refused the phase, or read a count that is
not positive once idle is subtracted (a ratio to it means nothing).

NOISY: the arms' instruction ranges overlap, some base session reading as
high as some candidate session or the reverse.  The delta is then within
the sessions' own scatter, and only its median says which way it leans.

The verdict of a phase: a regression when its median instruction delta is
past --bound (percent) and it is not NOISY; inconclusive when it is past
the bound but NOISY, or fewer than two pairs counted; otherwise flat or
better.

Information only, never in the verdict: wall time per key, from
--latency-pairs pairs of perkey_live's latency sessions (key enqueue to
render observed, p50 and p95 per phase per arm), printed with the host's
one-minute load average at the start and end of the run.  Wall time moves
with load; the alternation keeps the two arms' figures comparable.

Exit: 0 when every phase is flat or better, 1 on a regression, 2 when a
phase is inconclusive, so a check can call it: 2 asks for more pairs, it
does not fault the change.

usage: perkey_ab.py --base PATH --binary PATH --output DIR [--pairs 3]
                    [--latency-pairs 1] [--bound 2.0] [perkey_live options]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics

import perkey_live as pk

ARMS = ('base', 'candidate')


# Pure parts: the verdict of two arms' counts, and the report's text.

def compare(base, candidate, bound):
    """Per phase, the verdict of BASE against CANDIDATE: each maps a phase
    to its per-key counts in pair order (None where that session did not
    count it).  BOUND is the regression bound, in percent."""
    report = {}
    for name in dict.fromkeys([*base, *candidate]):
        pairs = [(b, c) for b, c in zip(base.get(name, []), candidate.get(name, []))
                 if b is not None and c is not None and b > 0 and c > 0]
        if len(pairs) < 2:
            report[name] = {'pairs': len(pairs), 'verdict': 'inconclusive', 'noisy': None}
            continue
        bs, cs = [b for b, _ in pairs], [c for _, c in pairs]
        deltas = [100 * (c - b) / b for b, c in pairs]
        delta = statistics.median(deltas)
        noisy = min(cs) <= max(bs) and min(bs) <= max(cs)
        verdict = ('inconclusive' if noisy else 'regression') if delta > bound else 'flat or better'
        report[name] = {'pairs': len(pairs), 'base': pk.spread(bs), 'candidate': pk.spread(cs),
                        'delta_pct': delta, 'delta_range_pct': [min(deltas), max(deltas)],
                        'noisy': noisy, 'verdict': verdict}
    return report


def exit_code(report):
    """1 on a regression, else 2 if any phase is inconclusive, else 0."""
    verdicts = {phase['verdict'] for phase in report.values()}
    return 1 if 'regression' in verdicts else 2 if 'inconclusive' in verdicts or not report else 0


def table(report, bound, pairs):
    """The report as text, a line a phase, of PAIRS pairs run."""
    lines = [f"perkey A/B: base then candidate, {pairs} pairs run; instructions per key, median [spread]; "
             f"delta median [range]; regression past +{bound}%"]
    for name, p in report.items():
        if 'delta_pct' not in p:
            lines.append(f"  {name:12} {p['pairs']} pairs counted: inconclusive")
            continue
        b, c = p['base'], p['candidate']
        lines.append(f"  {name:12} {b['median'] / 1e6:7.3f}M [{100 * b['spread']:.1f}%]  {c['median'] / 1e6:7.3f}M "
                     f"[{100 * c['spread']:.1f}%]  {p['delta_pct']:+.2f}% [{p['delta_range_pct'][0]:+.2f} .. "
                     f"{p['delta_range_pct'][1]:+.2f}]  {p['pairs']} pairs  {p['verdict']}{' NOISY' if p['noisy'] else ''}")
    return '\n'.join(lines)


def wall_table(walls, load):
    """Wall time per key, per phase and arm, with the LOAD it ran under:
    information beside the verdict."""
    lines = [f"wall time per key, key enqueue to render observed, p50 / p95 ms (information only; "
             f"load {load['start']:.1f} -> {load['end']:.1f}):"]
    for name in dict.fromkeys(n for arm in ARMS for n in walls.get(arm, {})):
        cells = [f"{arm} {w['p50_ms']:.2f} / {w['p95_ms']:.2f}" if (w := walls.get(arm, {}).get(name)) else f'{arm} -'
                 for arm in ARMS]
        lines.append(f"  {name:12} " + '   '.join(cells))
    return '\n'.join(lines)


# The run: interleaved perkey_live sessions.

def arguments(argv=None):
    """Our options, and each arm's perkey_live arguments."""
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--base', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True, help='the candidate')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--pairs', type=int, default=3)
    parser.add_argument('--latency-pairs', type=int, default=1, help='latency session pairs (wall time, information only)')
    parser.add_argument('--bound', type=float, default=2.0, help='regression bound, percent')
    args, rest = parser.parse_known_args(argv)
    arms = {arm: pk.arguments(rest + ['--binary', str(binary.resolve()), '--output', str((args.output / arm).resolve())])
            for arm, binary in zip(ARMS, (args.base, args.binary))}
    # Two probes with different sentinels first: a --sessions or
    # --latency-sessions given in any spelling, with any value, replaces both
    # alike, so the probes agree on it only when it was given.
    probes = [pk.arguments(['--sessions', s, '--latency-sessions', s] + rest + ['--binary', '/', '--output', '/'])
              for s in ('1', '2')]
    if probes[0].sessions == probes[1].sessions or probes[0].latency_sessions == probes[1].latency_sessions \
            or probes[0].record is not None:
        parser.error('perkey_ab runs its own sessions (--pairs, --latency-pairs) and does not profile: '
                     'no --sessions, --latency-sessions or --record')
    return args, arms


def counts(arm_args, index):
    """One counted session's per-key counts per phase, or None if it was rejected."""
    result = pk.session(arm_args, index, False)
    return result, (None if pk.invalid(result) else pk.session_per_key(result, arm_args.keys))


def main(argv=None):
    args, arms = arguments(argv)
    for arm in ARMS:
        arms[arm].output.mkdir(parents=True)
    load = {'start': os.getloadavg()[0]}
    found = {arm: [] for arm in ARMS}
    rejected = {}
    for index in range(args.pairs):
        for arm in ARMS:
            result, per_phase = counts(arms[arm], index)
            found[arm].append(per_phase or {})
            if per_phase is None:
                rejected[f'{arm}-{index}'] = pk.invalid(result)
    timed = {arm: [] for arm in ARMS}
    for index in range(args.latency_pairs):
        for arm in ARMS:
            result = pk.session(arms[arm], index, True)
            if why := pk.invalid(result):
                rejected[f'{arm}-latency-{index}'] = why
            else:
                timed[arm].append(result)
    load['end'] = os.getloadavg()[0]
    # Every phase the sessions run has a row, also one no session counted.
    by_phase = {arm: {name: [session.get(name) for session in found[arm]] for name in pk.phases(arms['base'])}
                for arm in ARMS}
    report = compare(by_phase['base'], by_phase['candidate'], args.bound)
    walls = {arm: pk.latencies(timed[arm]) for arm in ARMS}
    summary = {arm: {'binary': str(arms[arm].binary), 'sha256': hashlib.sha256(arms[arm].binary.read_bytes()).hexdigest()}
               for arm in ARMS}
    summary |= {'pairs': args.pairs, 'latency_pairs': args.latency_pairs, 'bound_pct': args.bound, 'load': load,
                'keys_per_phase': arms['base'].keys, 'phases': report, 'wall_ms_information_only': walls,
                'rejected': rejected, 'counts': by_phase, 'exit': exit_code(report)}
    (args.output / 'report.json').write_text(json.dumps(summary, indent=1) + '\n')
    print(table(report, args.bound, args.pairs))
    print(wall_table(walls, load))
    print(f"rejected sessions: {rejected or 'none'}")
    return summary['exit']


if __name__ == '__main__':
    raise SystemExit(main())
