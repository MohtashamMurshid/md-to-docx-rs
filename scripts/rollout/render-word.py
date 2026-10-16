#!/usr/bin/env python3
"""Export paired DOCX files through installed Word and compare every PDF page."""
import argparse
import fcntl
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from PIL import Image, ImageChops, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/parity"))
from compare import compare_case

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("directory", type=Path)
parser.add_argument("--reuse-pdf", action="store_true", help="Reuse existing exports only for unchanged DOCX files")
args = parser.parse_args()
directory = args.directory.resolve()
# Word's AppleScript document references can follow the active window. Keep
# exports serial even when two comparison processes are started independently.
(ROOT / "output").mkdir(exist_ok=True)
lock = (ROOT / "output/.word-export.lock").open("w")
fcntl.flock(lock, fcntl.LOCK_EX)
previous_path = directory / "validation.json"
previous = {entry["name"]: entry for entry in json.loads(previous_path.read_text())} if previous_path.exists() else {}
entries = json.loads((directory / "index.json").read_text())
word = subprocess.check_output(["osascript", "-e", 'tell application "Microsoft Word" to get version'], text=True).strip()
all_pages = []
for entry in entries:
    entry["package_differences"] = compare_case(directory, entry)
    renders = {}
    for engine in ("ts", "rs"):
        document = directory / f'{entry["name"]}.{engine}.docx'
        pdf = document.with_suffix(".pdf")
        document_hash = hashlib.sha256(document.read_bytes()).hexdigest()
        cached_hash = previous.get(entry["name"], {}).get(engine, {}).get("docx_sha256")
        if not args.reuse_pdf or not pdf.exists() or document_hash != cached_hash:
            subprocess.run(["osascript", str(ROOT / "scripts/rollout/export-word.applescript"), str(document), str(pdf)], check=True, timeout=90)
        target = directory / "rendered" / f'{entry["name"]}.{engine}'
        target.mkdir(parents=True, exist_ok=True)
        for stale in target.glob("page-*.png"):
            stale.unlink()
        subprocess.run(["pdftoppm", "-scale-to", "1600", "-png", str(pdf), str(target / "page")], check=True, stderr=subprocess.DEVNULL)
        subprocess.run(["pdftotext", "-layout", str(pdf), str(pdf.with_suffix(".txt"))], check=True)
        pages = sorted(target.glob("page-*.png"))
        entry[engine].update(pages=len(pages), docx_sha256=document_hash)
        renders[engine] = pages
    a, b = renders["ts"], renders["rs"]
    entry["rendered_pages_equal"] = len(a) == len(b) and all(
        Image.open(x).size == Image.open(y).size
        and ImageChops.difference(Image.open(x).convert("RGB"), Image.open(y).convert("RGB")).getbbox() is None
        for x, y in zip(a, b))
    entry["rendered_text_equal"] = (directory / f'{entry["name"]}.ts.txt').read_text() == (directory / f'{entry["name"]}.rs.txt').read_text()
    all_pages.extend(b)
    print(entry["name"], "pages", len(a), len(b), "pixels equal", entry["rendered_pages_equal"], "package differences", len(entry["package_differences"]), flush=True)

for offset in range(0, len(all_pages), 6):
    sheet = Image.new("RGB", (1290, 1140), "white")
    for index, page in enumerate(all_pages[offset:offset + 6]):
        thumbnail = Image.open(page).convert("RGB")
        thumbnail.thumbnail((400, 530))
        cell = Image.new("RGB", (430, 570), "#dedede")
        cell.paste(thumbnail, ((430 - thumbnail.width) // 2, 28))
        ImageDraw.Draw(cell).text((10, 8), f"{page.parent.name} {page.name}", fill="black")
        sheet.paste(cell, (index % 3 * 430, index // 3 * 570))
    sheet.save(directory / f"contact-{offset // 6 + 1}.png")
(directory / "validation.json").write_text(json.dumps(entries, indent=2) + "\n")
(directory / "reader.json").write_text(json.dumps({"reader": "Microsoft Word", "version": word, "cases": len(entries), "pages_per_engine": len(all_pages), "raster_longest_side_pixels": 1600}, indent=2) + "\n")
sys.exit(any(e["package_differences"] or not e["rendered_pages_equal"] or not e["rendered_text_equal"] for e in entries))
