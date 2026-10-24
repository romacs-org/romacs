"""Paired statistics for the two Constantine measurement tiers (pure functions).

Every observation is a log ratio log(candidate / control) taken from one
INTERLEAVED pair: both arms ran back to back on the same host, alternating which
arm went first. Host load inflates the variance of these log ratios; it cannot
bias them unless it is asymmetric between the two arms of a pair, which the
alternation turns into variance as well. Resource starvation is the exception
(see romacs_accept_constantine.assess) and is screened by telemetry, never by
looking at the timings.

DEV tier: a two-look t-interval on paired log ratios of counts (instructions,
allocations), alpha split evenly across the looks (Bonferroni) so peeking adds
no false claims. The guarantee is MODEL-DEPENDENT: with 3-5 pairs a t-interval
is calibrated only for roughly normal pair noise. Measured instruction noise is
~0.01-0.02% and continuous; a standard-deviation floor (instrument resolution,
DEV_SD_FLOORS) keeps a degenerate zero-spread sample from proving anything, but
bimodal noise at the materiality floor's scale (0.25%) would defeat it. Wall
time is reported with the same interval but is informational in this tier.

ACCEPTANCE tier: an internal pilot sets a fixed confirmation sample size from
measured variance, declared and frozen BEFORE the first confirmation sample;
the confirmation sample alone is analysed once with an exact Wilcoxon/Walsh
interval. Because n depends only on the (discarded) pilot, the confirmation
interval keeps its nominal coverage for any n, including a capped one.
"""
import math
from statistics import NormalDist, fmean, median, stdev

VERDICTS = ('BETTER', 'WORSE', 'INDISTINGUISHABLE')

# ---------------------------------------------------------------- Student t


def _beta_fraction(a, b, x):
    """Continued fraction for the regularized incomplete beta (modified Lentz)."""
    tiny = 1e-300
    qab, qap, qam = a + b, a + 1.0, a - 1.0
    c, d = 1.0, 1.0 - qab * x / qap
    d = 1.0 / (d if abs(d) > tiny else tiny)
    h = d
    for m in range(1, 500):
        m2 = 2 * m
        for numerator in (m * (b - m) * x / ((qam + m2) * (a + m2)),
                          -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2))):
            d = 1.0 + numerator * d
            d = 1.0 / (d if abs(d) > tiny else tiny)
            c = 1.0 + numerator / c
            c = c if abs(c) > tiny else tiny
            step = c * d
            h *= step
        if abs(step - 1.0) < 1e-15:
            return h
    raise ValueError('incomplete beta did not converge')


def _beta_regularized(a, b, x):
    if x <= 0:
        return 0.0
    if x >= 1:
        return 1.0
    front = math.exp(math.lgamma(a + b) - math.lgamma(a) - math.lgamma(b)
                     + a * math.log(x) + b * math.log1p(-x))
    if x < (a + 1) / (a + b + 2):
        return front * _beta_fraction(a, b, x) / a
    return 1.0 - front * _beta_fraction(b, a, 1.0 - x) / b


def t_cdf(t, df):
    tail = 0.5 * _beta_regularized(df / 2.0, 0.5, df / (df + t * t))
    return 1.0 - tail if t > 0 else tail


def t_quantile(p, df):
    """Upper quantile of Student's t by bisection (p in (0.5, 1))."""
    if not 0.5 < p < 1 or df < 1:
        raise ValueError('t_quantile needs 0.5 < p < 1 and df >= 1')
    low, high = 0.0, 1.0
    while t_cdf(high, df) < p:
        high *= 2
    for _ in range(200):
        mid = (low + high) / 2
        low, high = (mid, high) if t_cdf(mid, df) < p else (low, mid)
    return (low + high) / 2

# ---------------------------------------------------------------- DEV tier


DEV_LOOKS = (3, 5)          # pairs at which the dev verdict is evaluated
DEV_ALPHA = 0.05            # overall two-sided false-claim rate per metric
# Materiality floors: a difference the interval proves but that is smaller
# than this is reported as INDISTINGUISHABLE (with the proof noted), so that
# a 0.01% wobble is never sold as a win. Counts are near-deterministic; wall
# time under load is not, and is informational in this tier.
DEV_FLOORS = {'instructions': math.log(1.0025), 'allocations': math.log(1.0025),
              'wall_ms': math.log(1.02)}
# Instrument-resolution floors on the pair standard deviation (log units),
# 2.5x above the largest instruction spread measured on loaded Constantine.
DEV_SD_FLOORS = {'instructions': 0.0005, 'allocations': 0.0005, 'wall_ms': 0.005}
DEV_PRIMARY = 'instructions'


def look_alpha(looks=DEV_LOOKS, alpha=DEV_ALPHA):
    return alpha / len(looks)


def paired_logs(candidate, control, smoothing=0):
    """log(c/b) per pair; smoothing=1 tolerates zero allocation counts."""
    if not candidate or len(candidate) != len(control):
        raise ValueError('expected nonempty equally sized pairs')
    values = []
    for c, b in zip(candidate, control):
        if (isinstance(c, bool) or isinstance(b, bool)
                or not all(isinstance(v, (int, float)) and math.isfinite(v) for v in (c, b))):
            raise ValueError('paired values must be finite numbers')
        if c + smoothing <= 0 or b + smoothing <= 0:
            raise ValueError('paired values must be positive')
        values.append(math.log((c + smoothing) / (b + smoothing)))
    return values


def dev_interval(logs, alpha, floor, sd_floor=0.0):
    """Three-way verdict from a paired t-interval with a materiality floor."""
    n = len(logs)
    if n < 2:
        raise ValueError('a paired interval needs at least two pairs')
    estimate = fmean(logs)
    spread = max(stdev(logs), sd_floor)
    half = t_quantile(1 - alpha / 2, n - 1) * spread / math.sqrt(n)
    low, high = estimate - half, estimate + half
    proven = low > 0 or high < 0
    if low > 0 and estimate >= floor:
        verdict = 'WORSE'
    elif high < 0 and estimate <= -floor:
        verdict = 'BETTER'
    else:
        verdict = 'INDISTINGUISHABLE'
    return {'verdict': verdict, 'ratio': math.exp(estimate),
            'ci_lo': math.exp(low), 'ci_hi': math.exp(high), 'confidence': 1 - alpha,
            'n': n, 'floor_ratio': math.exp(floor), 'sd_log': spread,
            # A difference proven but below the floor stays INDISTINGUISHABLE.
            'proven_below_floor': proven and verdict == 'INDISTINGUISHABLE',
            # Resolved: a direction, or the whole interval inside +/- floor.
            'resolved': verdict != 'INDISTINGUISHABLE' or (low >= -floor and high <= floor)}


# ---------------------------------------------------------------- ACCEPTANCE


ACCEPT_PROTOCOL = 'constantine-pilot-fixed-walsh-v2'
PILOT_BLOCKS = 8            # 16 pairs; excluded from inference
MIN_BLOCKS = 10
MAX_BLOCKS = 60
POWER = 0.8
WILCOXON_EFFICIENCY = 0.955   # asymptotic relative efficiency vs t under normality
# Distance from the true ratio to the bar at which a candidate should PASS with
# probability POWER: a power-planning margin, NOT a promised CI half-width (the
# achieved interval is reported instead).
POWER_MARGIN = {'base': math.log(1.02), 'gnu': math.log(1.05)}
THRESHOLD = {'base': 1.02, 'gnu': 1.20}


def signed_rank_counts(n):
    counts = [1]
    for rank in range(1, n + 1):
        updated = counts + [0] * rank
        for total, count in enumerate(counts):
            updated[total + rank] += count
        counts = updated
    return counts


def interval_index(n, alpha):
    """Largest k with exact P(W <= k-1) <= alpha/2; Walsh CI = (w[k-1], w[-k])."""
    total, best = 0, None
    for index, count in enumerate(signed_rank_counts(n)):
        total += count
        if total / 2**n > alpha / 2:
            break
        best = (index + 1, total / 2**n)
    if best is None:
        raise ValueError('too few blocks for a finite exact interval')
    return best


def plan(pilot, alpha, margin, cap=MAX_BLOCKS):
    """Freeze the confirmation size from pilot variance only (never location)."""
    if len(pilot) != PILOT_BLOCKS or not 0 < alpha < 1 or margin <= 0 or cap < MIN_BLOCKS:
        raise ValueError('invalid pilot/planning contract')
    if not all(math.isfinite(x) for x in pilot):
        raise ValueError('nonfinite pilot')
    sigma = stdev(pilot)
    z = NormalDist().inv_cdf(1 - alpha / 2) + NormalDist().inv_cdf(POWER)
    requested = max(MIN_BLOCKS, math.ceil((z * sigma / margin) ** 2 / WILCOXON_EFFICIENCY))
    return {'protocol': ACCEPT_PROTOCOL, 'pilot_blocks': PILOT_BLOCKS, 'pilot_excluded': True,
            'pilot_stdev_log': sigma, 'alpha': alpha, 'power_target': POWER,
            'power_margin_log': margin, 'requested_blocks': requested,
            'confirmation_blocks': min(requested, cap), 'cap_blocks': cap,
            'capped': requested > cap,
            'stopping': 'fixed confirmation n frozen from pilot variance; analysed once',
            'assumptions': 'independent, symmetric block log ratios about a common location'}


def block_value(pairs, control, metric='wall_ms'):
    """Mean log ratio of an AB/BA block: cancels linear drift and order effects."""
    if len(pairs) != 2:
        raise ValueError('incomplete balanced block')
    logs = []
    for pair in pairs:
        for arm in ('cand', control):
            value = pair[arm].get(metric)
            if isinstance(value, bool) or not isinstance(value, (int, float)) \
                    or not math.isfinite(value) or value <= 0:
                raise ValueError('invalid sample; no ratio')
        logs.append(math.log(pair['cand'][metric] / pair[control][metric]))
    return sum(logs) / 2


def analyze(values, planned, threshold):
    if len(values) != planned['confirmation_blocks'] or not values:
        raise ValueError('incomplete fixed confirmation; no ratio')
    if not all(math.isfinite(x) for x in values) or threshold <= 0:
        raise ValueError('invalid confirmation')
    k, tail = interval_index(len(values), planned['alpha'])
    walsh = sorted((a + b) / 2 for i, a in enumerate(values) for b in values[i:])
    low, high = walsh[k - 1], walsh[-k]
    bar = math.log(threshold)
    verdict = 'FAIL' if low > bar else 'PASS' if high <= bar else 'INCONCLUSIVE'
    direction = 'WORSE' if low > 0 else 'BETTER' if high < 0 else 'INDISTINGUISHABLE'
    return {'verdict': verdict, 'direction': direction, 'median': math.exp(median(walsh)),
            'ci_lo': math.exp(low), 'ci_hi': math.exp(high), 'coverage': 1 - 2 * tail,
            'alpha': planned['alpha'], 'blocks': len(values), 'pairs': 2 * len(values),
            'threshold': threshold, 'achieved_half_width_log': (high - low) / 2,
            'planned_power_margin_log': planned['power_margin_log'], 'capped': planned['capped']}


def pair_order(control, index):
    """Alternate which arm starts: even pairs control first, odd pairs candidate first."""
    return [control, 'cand'] if index % 2 == 0 else ['cand', control]
