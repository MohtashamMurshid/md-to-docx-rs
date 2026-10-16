#!/usr/bin/env python3
"""Compare TypeScript and Rust outputs produced by generate.mjs.

Both packages are canonicalized: XML attributes are sorted, property
containers have their children sorted, relationship ids are replaced with
their targets, media names are replaced with content hashes, and volatile
timestamps are blanked. Equal canonical dumps establish package-structure parity under these
normalizations; they do not establish matching reader layout or pagination.
"""
import sys, os, json, zipfile, hashlib, re, difflib, collections
from xml.etree import ElementTree as ET

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
R = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}"
PKG = "{http://schemas.openxmlformats.org/package/2006/relationships}"
SORT_CHILDREN = {"rPr", "pPr", "tcPr", "tblPr", "trPr", "sectPr", "tblBorders", "tcBorders", "pBdr", "tblCellMar", "tcMar", "numPr", "lvl", "docDefaults", "settings", "coreProperties"}
SKIP_PARTS = set()

def qualified(name):
    # Prefix spelling is irrelevant; namespace identity is not.
    namespaces = {
        W[1:-1]: "w", R[1:-1]: "r",
        "http://schemas.openxmlformats.org/officeDocument/2006/math": "m",
        "http://schemas.openxmlformats.org/drawingml/2006/main": "a",
        "http://schemas.openxmlformats.org/drawingml/2006/picture": "pic",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing": "wp",
        "http://www.w3.org/XML/1998/namespace": "xml",
    }
    if name.startswith("{"):
        namespace, local = name[1:].split("}", 1)
        return f"{namespaces.get(namespace, namespace)}:{local}"
    return name

def media_hash(z, name):
    data = z.read(name)
    if os.environ.get("PARITY_MEDIA_MODE") == "pixels" and data.startswith(b"\x89PNG\r\n\x1a\n"):
        from PIL import Image
        import io
        with Image.open(io.BytesIO(data)) as image:
            data = repr(image.size).encode() + image.convert("RGBA").tobytes()
    return "media:" + hashlib.sha1(data).hexdigest()[:12]

def load_rels(z, part):
    d, f = os.path.split(part)
    rel = f"{d}/_rels/{f}.rels" if d else f"_rels/{f}.rels"
    out = {}
    if rel in z.namelist():
        root = ET.fromstring(z.read(rel))
        for r in root:
            t = r.get("Target") or ""
            typ = (r.get("Type") or "").rsplit("/", 1)[-1]
            target = t
            if typ == "image":
                p = os.path.normpath(os.path.join(d, t)).replace("\\", "/")
                if p in z.namelist():
                    target = media_hash(z, p)
            out[r.get("Id")] = f"{typ}:{target}" + (":ext" if r.get("TargetMode") == "External" else "")
    return out

def canon(el, rels, depth=0, lines=None):
    if lines is None: lines = []
    tag = el.tag.split("}")[-1]
    attrs = []
    for k, v in el.attrib.items():
        kn = qualified(k)
        if k.startswith(R) and v in rels: v = rels[v]
        attrs.append(f"{kn}={v!r}")
    attrs.sort()
    text = el.text or ""
    if not text.strip() and tag not in {"t", "instrText", "delText", "lpwstr", "lpstr"}: text = ""
    lines.append("  " * depth + f"<{qualified(el.tag)} {' '.join(attrs)}>" + (repr(text) if text else ""))
    kids = list(el)
    if tag in SORT_CHILDREN:
        kids = sorted(kids, key=lambda k: (k.tag, sorted(k.attrib.items())))
    for k in kids:
        canon(k, rels, depth + 1, lines)
    return lines

def rels_dump(z, part):
    root = ET.fromstring(z.read(part))
    d = os.path.dirname(part).replace("_rels", "").rstrip("/")
    items = []
    for r in root:
        t = r.get("Target") or ""; typ = (r.get("Type") or "").rsplit("/", 1)[-1]
        if typ == "image":
            p = os.path.normpath(os.path.join(d, t)).replace("\\", "/")
            if p in z.namelist(): t = media_hash(z, p)
        items.append(f"{typ} {t} {r.get('TargetMode') or ''}")
    return sorted(items)

def content_types_dump(z):
    root = ET.fromstring(z.read("[Content_Types].xml"))
    return sorted(f"{c.tag.split('}')[-1]} {c.get('Extension') or c.get('PartName')} {c.get('ContentType')}" for c in root)

def part_dump(z, name, fixed_timestamps=()):
    if name == "[Content_Types].xml": return content_types_dump(z)
    if name.endswith(".rels"): return rels_dump(z, name)
    if name.startswith("word/media/"): return [media_hash(z, name)]
    data = z.read(name)
    if name == "docProps/core.xml":
        for tag in ("created", "modified"):
            if tag not in fixed_timestamps:
                pattern = rb"<dcterms:" + tag.encode() + rb"[^>]*>[^<]*</dcterms:" + tag.encode() + rb">"
                data = re.sub(pattern, b"<dcterms:" + tag.encode() + b"/>", data)
    try:
        root = ET.fromstring(data)
    except ET.ParseError:
        # Raw control characters make the XML invalid in both engines; compare
        # the rest of the document with them replaced.
        cleaned = re.sub(rb"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]|\xef\xbf[\xbe\xbf]", b"\xef\xbf\xbd", data)
        try:
            root = ET.fromstring(cleaned)
        except ET.ParseError as e:
            return [f"<<unparseable {e}>>"]
        return ["<<contains invalid XML characters>>"] + canon(root, load_rels(z, name))
    return canon(root, load_rels(z, name))

def package_dump(path, fixed_timestamps=()):
    z = zipfile.ZipFile(path)
    names = sorted(n for n in z.namelist() if not n.endswith("/"))
    out = {}
    for n in names:
        key = n
        if n.startswith("word/media/"):
            key = "word/media/" + media_hash(z, n)
        out[key] = part_dump(z, n, fixed_timestamps)
    return out

def compare_case(out_dir, entry):
    name = entry["name"]
    ts, rs = entry.get("ts"), entry.get("rs")
    if not ts["ok"] or not rs["ok"]:
        if ts["ok"] != rs["ok"]:
            return [("error-parity", f"ts={'ok' if ts['ok'] else ts['message'][:100]} rs={'ok' if rs['ok'] else rs['message'][:100]}")]
        problems = []
        for key, category in [("name", "error-name"), ("message", "error-message"), ("context", "error-context")]:
            if ts.get(key) != rs.get(key):
                problems.append((category, f"ts={str(ts.get(key))[:300]!r} rs={str(rs.get(key))[:300]!r}"))
        return problems
    a = package_dump(os.path.join(out_dir, f"{name}.ts.docx"), entry.get("fixedTimestamps", ()))
    b = package_dump(os.path.join(out_dir, f"{name}.rs.docx"), entry.get("fixedTimestamps", ()))
    problems = []
    for part in sorted(set(a) | set(b)):
        if part in SKIP_PARTS: continue
        if part not in a: problems.append((f"extra-part {part}", "")); continue
        if part not in b: problems.append((f"missing-part {part}", "")); continue
        if a[part] != b[part]:
            diff = [l for l in difflib.unified_diff(a[part], b[part], "ts", "rs", lineterm="", n=0) if l[:1] in "+-" and l[:3] not in ("+++", "---")]
            problems.append((f"diff {part}", "\n".join(diff[:60])))
    return problems

def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "..", "output", "parity")
    show = sys.argv[2] if len(sys.argv) > 2 else None
    index = json.load(open(os.path.join(out_dir, "index.json")))
    total = 0; failing = 0
    by_class = collections.Counter(); examples = {}
    for entry in index:
        if show and not re.search(show, entry["name"]): continue
        total += 1
        problems = compare_case(out_dir, entry)
        if problems:
            failing += 1
            for cls, detail in problems:
                sig = cls
                by_class[sig] += 1
                examples.setdefault(sig, []).append((entry["name"], detail))
            if show:
                print(f"=== {entry['name']}")
                for cls, detail in problems:
                    print(f"--- {cls}\n{detail}")
    print(f"{total - failing}/{total} cases identical")
    for sig, count in by_class.most_common():
        names = [n for n, _ in examples[sig]]
        print(f"{count:5d}  {sig}   e.g. {', '.join(names[:6])}")
    if not show:
        print("\nFirst example per class:")
        for sig, _ in by_class.most_common():
            n, d = examples[sig][0]
            print(f"--- {sig} [{n}]\n{d[:1200]}")
    return 1 if failing else 0

if __name__ == "__main__":
    sys.exit(main())
