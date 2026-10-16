#!/usr/bin/env python3
"""Show the markdown and full canonical diff for named cases."""
import sys, json, os, difflib
sys.path.insert(0, os.path.dirname(__file__))
from compare import package_dump
out = sys.argv[1]; names = sys.argv[2:]
md = {}
for f, prefix in (("commonmark.json", "cm"), ("gfm.json", "gfm")):
    for ex in json.load(open(os.path.join(os.path.dirname(__file__), "corpus", f))):
        md[f"{prefix}-{ex['example']:03d}"] = ex["markdown"]
idx = {e["name"]: e for e in json.load(open(os.path.join(out, "index.json")))}
for name in names:
    e = idx[name]
    print(f"===== {name}: {md.get(name, '')!r}")
    if not (e["ts"]["ok"] and e["rs"]["ok"]):
        print("ts:", e["ts"], "\nrs:", e["rs"]); continue
    a = package_dump(os.path.join(out, f"{name}.ts.docx")); b = package_dump(os.path.join(out, f"{name}.rs.docx"))
    for part in sorted(set(a) | set(b)):
        if a.get(part) != b.get(part):
            print(f"--- {part}")
            print("\n".join(difflib.unified_diff(a.get(part, []), b.get(part, []), "ts", "rs", lineterm="", n=2)))
