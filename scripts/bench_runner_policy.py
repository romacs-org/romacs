"""Runner admission is separate from optional measurement calibration.

Empty eligible_rows refuses every request, including implicit suites. Calibration
is an additional constraint, never an override of this reviewed runner policy.
"""

# No machine is implicitly approved by the distributed source. Deployment-local
# callers must supply their own reviewed policy before permitting host tuning.
RUNNER_ELIGIBILITY = {}


def check_runner(host, core_args, *, allow_unknown=False):
    policy = RUNNER_ELIGIBILITY.get(host)
    if policy is None:
        if allow_unknown:
            return
        raise ValueError(f'unknown benchmark runner: {host}')
    if not policy['eligible_rows']:
        raise ValueError(f"runner {host} has no eligible rows: {policy['reason']}")
