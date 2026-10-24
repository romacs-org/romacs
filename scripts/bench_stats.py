"""Deterministic paired-ratio percentile bootstrap for core-bench."""
import math
import random
import statistics

BOOTSTRAP_RESAMPLES = 10000
BOOTSTRAP_SEED = 20261015


def percentile(ordered, fraction):
    position = (len(ordered) - 1) * fraction
    low = math.floor(position)
    high = math.ceil(position)
    return ordered[low] + (ordered[high] - ordered[low]) * (position - low)


def paired_stats(candidate, baseline, threshold=1.20):
    """Bootstrap repeat pairs, never independently resample the two engines."""
    if not candidate or len(candidate) != len(baseline):
        raise ValueError('expected nonempty equally sized repeat pairs')
    if any(isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v) or v <= 0
           for v in [*candidate, *baseline]):
        raise ValueError('paired times must be finite and positive')
    ratios = [cand / base for cand, base in zip(candidate, baseline)]
    rng = random.Random(BOOTSTRAP_SEED)
    boot = sorted(statistics.median(rng.choices(ratios, k=len(ratios)))
                  for _ in range(BOOTSTRAP_RESAMPLES))
    low, high = percentile(boot, .025), percentile(boot, .975)
    verdict = 'PASS' if high <= threshold else 'FAIL' if low > threshold else 'INCONCLUSIVE'
    return {'median': statistics.median(ratios), 'ci_lo': low, 'ci_hi': high,
            'n': len(ratios), 'verdict': verdict, 'threshold': threshold,
            'bootstrap_resamples': BOOTSTRAP_RESAMPLES, 'bootstrap_seed': BOOTSTRAP_SEED}


def next_repeat_target(current, verdicts, explicit=False):
    """Explicit -n is exact; automatic inconclusive runs grow by four to 15."""
    if explicit or current >= 15 or 'INCONCLUSIVE' not in verdicts:
        return current
    return min(current + 4, 15)
