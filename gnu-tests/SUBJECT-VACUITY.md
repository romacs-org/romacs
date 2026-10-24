# Subject-only vacuity classification

This is a bounded classifier for reviewed recipes, not proof of arbitrary test
adequacy. `--subject-only` preserves every assertion. It changes `(/ 1 0)` to
`nil` in should-error-argument and the format call to `nil` in format-c-float.
An original must pass and every subject mutant must reach an assertion failure
under romacs mini-ERT, pinned GNU real ERT, and GNU mini-ERT to be labelled
ASSERTION-SENSITIVE. Errors, missing results and registration failures are not
kills. A mixed outcome is UNCLASSIFIED.

The positive control is format-c-float; its retained should-error assertion
must detect the no-error subject. The negative control calls a function without
an assertion; changing only the function definition from t to nil must survive.
Both controls must qualify before any verdict is imported.

body-runs has no inner assertion: the flag assignment is its entire subject,
and its oracle belongs to the enclosing ert-self-test wrapper (pristine source
lines 40-52). Disabling that assignment tests precisely this member boundary,
not general whole-body deletion. It is classified CANNOT-FAIL if it survives;
this does not say that the enclosing self-test is vacuous. lexical-binding has
no supported subject-fault recipe: the former local-let probe changed assertion
context and failed registration. It stays UNCLASSIFIED. No existing exclusion
or historical budget entry for those two is moved by this implementation.

## Evidence lifecycle

1. Commit the classifier on a clean tree, then run:
   `python3 gnu-tests/tools/probe-vacuity-focus.py --subject-only --output AUDIT`.
2. `python3 gnu-tests/tools/import-vacuity-evidence.py AUDIT --format subject
   --output RUN_DIR/VACUITY-VERDICT.json` verifies the original full manifest,
   replays raw outputs, checks exact recipe inventory and probe text, then writes
   the portable receipt and `RUN_DIR/MEMBER-EVIDENCE.json`. RUN_DIR must be outside
   the measured source. `scripts/gate` performs these steps for its built adapter.
3. Do not commit either generated file. Set `ROMACS_VACUITY_RECEIPT` to the run's
   receipt when measuring outside the gate; the member path is its sibling, or
   can be set explicitly with `ROMACS_MEMBER_EVIDENCE`. Both must share a directory.
   The portable receipt retains the original manifest and every sealed
   probe/stdout/stderr, and checks hashes and recomputed verdicts on every load.
   It binds every tracked path, mode and byte except `parity-ratchet.json`.
   `member-evidence-policy.json` preserves the historical classifications as an
   explicit source input, not fresh measurements. Only the disputed subject is
   generated from the fresh receipt. Every source change invalidates the receipt.
   The original revision/tree must remain an ancestor; altered source bindings
   and altered raw results are refused.
4. The GNU measurement additionally requires the current adapter's exact hash
   to equal the sealed release binary. This is a local import attestation,
   not cryptographic proof against a malicious importer. The original full
   manifest stays strict: it is not made to accept a successor source tree.

No pristine GNU source, floor, expected count, test, or budget is edited.
Only should-error-argument may be promoted from a fresh sealed verdict. Its
classification means bounded assertion sensitivity, not full ERT semantics or
clean whole-file execution; the existing file-load condition still controls
whether it is counted. Use a fresh probe after any source change.

Repeated `record` calls still execute all measurements. If the complete payload
and source/binary bindings are identical, retain the first record's provenance
instead of introducing timestamp/load/timing churn. The second call's observed
results remain in its run log; the unchanged JSON continues to describe the
first actual run. Changed payloads or bindings always get fresh provenance.
