#!/usr/bin/env python3
"""Group differing cases by the first lines of their document.xml diff."""
import sys, json, os, re, collections, difflib
sys.path.insert(0, os.path.dirname(__file__))
from compare import package_dump
out = sys.argv[1]
part = sys.argv[2] if len(sys.argv) > 2 else "word/document.xml"
idx = json.load(open(os.path.join(out, "index.json")))
clusters = collections.defaultdict(list)
for e in idx:
    if not (e["ts"]["ok"] and e["rs"]["ok"]): continue
    a = package_dump(os.path.join(out, f"{e['name']}.ts.docx")); b = package_dump(os.path.join(out, f"{e['name']}.rs.docx"))
    if a.get(part) == b.get(part): continue
    diff = [l for l in difflib.unified_diff(a.get(part, []), b.get(part, []), lineterm="", n=0) if l[:1] in "+-" and l[:3] not in ("+++", "---")]
    sig = "\n".join(re.sub(r"'.*'", "'…'", l.strip()) for l in diff[:4])
    clusters[sig].append(e["name"])
for sig, names in sorted(clusters.items(), key=lambda kv: -len(kv[1])):
    print(f"### {len(names)}: {', '.join(names[:10])}\n{sig}\n")
