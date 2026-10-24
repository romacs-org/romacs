#!/usr/bin/env python3
"""GNU fixnum-tagging differential cases; same fresh-process compiler harness."""
import bignum_oracle as oracle


def cases():
    rows = []

    def add(family, source):
        rows.append(dict(family=family, source=source))

    for n in (-(2**61), -(2**61)+1, -(2**60)-1, -(2**60),
              -(2**60)+1, -1, 0, 1, 2**60-1, 2**60, 2**60+1,
              2**61-2, 2**61-1):
        add("literal", str(n))
        add("predicate", f"(list (integerp {n}) (fixnump {n}) (bignump {n}))")
        add("identity", f"(let ((f (lambda (x) (+ {n} x)))) (eq (funcall f 0) (funcall f 0)))")
        for test in ("eq", "eql", "equal"):
            add("hash", f"(let ((h (make-hash-table :test '{test}))) (puthash (+ {n} 0) 42 h) (list (gethash (+ {n} 0) h) (hash-table-count h)))")
    for source in ("most-positive-fixnum", "most-negative-fixnum",
                   "(1+ 1152921504606846975)", "(1- -1152921504606846976)",
                   "(- -2305843009213693952 0)",
                   "(* 1152921504606846976 1)"):
        add("arithmetic", source)
    assert len({row['source'] for row in rows}) == len(rows)
    return rows


if __name__ == "__main__":
    oracle.cases = cases
    oracle.__file__ = __file__
    oracle.PROTOCOL = "fixnums-compiled-source-v1"
    oracle.main()
