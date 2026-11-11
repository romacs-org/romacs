#!/bin/bash
set -e

cd "$(dirname "$0")"

SELF="$(basename "$0")"

# Collect all run-*.sh scripts except this one
TESTS=()
for script in run-*.sh; do
    if [ "$script" != "$SELF" ]; then
        TESTS+=("$script")
    fi
done

if [ ${#TESTS[@]} -eq 0 ]; then
    echo "No test scripts found."
    exit 0
fi

# Track results
declare -a RESULTS
PASSED=0
FAILED=0

# Disable exit-on-error so we can handle failures gracefully
set +e

for test in "${TESTS[@]}"; do
    echo ""
    echo "=== Running test: $test ==="
    bash "./$test"
    EXIT_CODE=$?
    if [ $EXIT_CODE -eq 0 ]; then
        echo "=== PASS: $test ==="
        RESULTS+=("PASS  $test")
        PASSED=$((PASSED + 1))
    else
        echo "=== FAIL: $test ==="
        RESULTS+=("FAIL  $test")
        FAILED=$((FAILED + 1))
    fi
done

TOTAL=${#TESTS[@]}

echo ""
echo "==========================="
echo "ROMACS TEST SUITE SUMMARY"
echo "==========================="
for result in "${RESULTS[@]}"; do
    echo "$result"
done
echo "==========================="
echo "Total: $TOTAL tests, $PASSED passed, $FAILED failed"

if [ $FAILED -ne 0 ]; then
    exit 1
fi
exit 0
