#!/usr/bin/env python3
"""Mechanical inventory: every public item of a rustdoc-JSON crate whose
signature mentions a target type (default romacs_vm::Value).

Usage: hpb-inventory.py DOC.json [TypeName]   -> prints TSV + summary
Classes:  RET   = Value appears in return type (can outlive the call)
          FIELD = public field / variant payload (outlives any call)
          ROOTS = only as &mut [Value] (explicit root slice, rewritten in place)
          ARG   = only in argument position (consumed during the call)
"""
import json, sys, collections

doc = json.load(open(sys.argv[1]))
tname = sys.argv[2] if len(sys.argv) > 2 else "Value"
crate = doc["root"]
idx = doc["index"]
paths = doc["paths"]
target = {k for k, p in paths.items() if p["crate_id"] == 0 and p["path"][-1] == tname and p["kind"] == "struct"}
assert target, "type not found"

SELF_IS_TARGET = [False]

def mentions(t):
    if t is None:
        return False
    if isinstance(t, dict):
        if t.get("generic") == "Self" and SELF_IS_TARGET[0]:
            return True
        if "resolved_path" in t:
            rp = t["resolved_path"]
            if str(rp.get("id")) in target:
                return True
        return any(mentions(v) for v in t.values())
    if isinstance(t, list):
        return any(mentions(v) for v in t)
    return False

def is_root_slice(t):
    # &mut [Value]
    br = t.get("borrowed_ref") if isinstance(t, dict) else None
    if not br or not br.get("is_mutable"):
        return False
    sl = br["type"].get("slice") if isinstance(br["type"], dict) else None
    return bool(sl) and mentions(sl)

# owner of each impl method
owner = {}
for iid, it in idx.items():
    inner = it.get("inner", {})
    if "impl" in inner:
        im = inner["impl"]
        if im.get("trait") is not None and im["trait"]["path"] not in ():
            tr = im["trait"]["path"]
        else:
            tr = None
        for m in im["items"]:
            owner[str(m)] = (im["for"], tr, im.get("is_synthetic") or im.get("blanket_impl") is not None)

def tyname(t):
    if isinstance(t, dict) and "resolved_path" in t:
        return t["resolved_path"]["path"]
    return "?"

rows = []
for iid, it in idx.items():
    if it.get("crate_id") != 0:
        continue
    vis = it.get("visibility")
    kind0 = next(iter(it.get("inner", {})), "?")
    # enum-variant fields carry visibility "default" and are as public as the enum
    if vis != "public" and iid not in owner and not (vis == "default" and kind0 == "struct_field"):
        continue
    inner = it.get("inner", {})
    name = it.get("name")
    kind = next(iter(inner)) if inner else "?"
    where = None
    if iid in owner:
        f, tr, synth = owner[iid]
        if synth:
            continue
        where = tyname(f) + (f" as {tr}" if tr else "")
    else:
        p = paths.get(iid)
        where = "::".join(p["path"][:-1]) if p else "?"
    cls = None
    SELF_IS_TARGET[0] = iid in owner and str(owner[iid][0].get("resolved_path", {}).get("id")) in target
    if iid in owner and owner[iid][1] is not None and SELF_IS_TARGET[0]:
        # trait impls on the target itself (Clone/Debug/PartialEq): derived plumbing
        if owner[iid][1] not in ("PartialEq", "Clone"):
            continue
    if kind == "function":
        sig = inner["function"]["sig"]
        out = sig.get("output")
        ins = [t for _, t in sig["inputs"]]
        if mentions(out):
            cls = "RET"
        elif any(mentions(t) for t in ins):
            cls = "ROOTS" if all(is_root_slice(t) or not mentions(t) for t in ins) else "ARG"
    elif kind == "struct_field":
        if mentions(inner["struct_field"]):
            cls = "FIELD"
    elif kind == "variant":
        if mentions(inner["variant"]):
            cls = "FIELD"
    elif kind in ("type_alias", "constant", "assoc_const", "static"):
        if mentions(inner[kind]):
            cls = "FIELD" if kind == "type_alias" else "CONST"
    if cls:
        rows.append((cls, where, kind, name))

rows.sort()
for r in rows:
    print("\t".join(str(x) for x in r))
c = collections.Counter(r[0] for r in rows)
print("SUMMARY", dict(c), "total", len(rows))
