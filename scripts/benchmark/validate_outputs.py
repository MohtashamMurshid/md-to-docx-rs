"""Inspect saved benchmark DOCX files outside the timed/memory processes."""
import hashlib
import io
import json
import re
import sys
import xml.etree.ElementTree as ET
import zipfile
from collections import Counter
from pathlib import Path

from PIL import Image

OUT = Path(sys.argv[1]).resolve()
W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def inspect(path):
    with zipfile.ZipFile(path) as archive:
        assert archive.testzip() is None, path
        for name in archive.namelist():
            if name.endswith(".xml") or name.endswith(".rels"):
                ET.fromstring(archive.read(name))
        raw = archive.read("word/document.xml")
        document = ET.fromstring(raw)
        text = "".join(n.text or "" for n in document.iter(W + "t"))
        paragraphs = list(document.iter(W + "p"))
        colors = Counter(n.get(W + "val") for n in document.iter(W + "color"))
        relationships = ET.fromstring(archive.read("word/_rels/document.xml.rels"))
        targets = [n.get("Target") for n in relationships if n.get("Type", "").endswith("/hyperlink")]
        numbering = ET.fromstring(archive.read("word/numbering.xml"))
        stats = {
            "docx_bytes": path.stat().st_size,
            "document_xml_bytes": len(raw),
            "document_xml_sha256": hashlib.sha256(raw).hexdigest(),
            "all_visible_text_sha256": hashlib.sha256(text.encode()).hexdigest(),
            "paragraphs": len(paragraphs), "runs": len(list(document.iter(W + "r"))),
            "color_elements": sum(colors.values()), "distinct_colors": sorted(colors),
            "hyperlink_relationships": len(targets), "distinct_hyperlink_targets": len(set(targets)),
            "abstract_numbering_definitions": len(list(numbering.iter(W + "abstractNum"))),
            "numbering_levels": len(list(numbering.iter(W + "lvl"))),
            "format_elements": {key: len(list(document.iter(W + key))) for key in ["b", "i", "u", "strike", "hyperlink"]},
            "uncompressed_total_bytes": sum(p.file_size for p in archive.infolist()),
            "parts_uncompressed_bytes": {p.filename: p.file_size for p in archive.infolist()},
        }
        media = [name for name in archive.namelist() if name.startswith("word/media/") and not name.endswith("/")]
        stats["media_count"] = len(media)
        if path.name.startswith("chart-") and not path.name.endswith("-memory.docx"):
            assert len(media) == 1
            data = archive.read(media[0])
            assert data[:8] == b"\x89PNG\r\n\x1a\n"
            dimensions = [int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")]
            assert dimensions == [1200, 800], dimensions
            stats["chart_dimensions"] = dimensions
            stats["chart_png_bytes"] = len(data)
            stats["chart_png_sha256"] = hashlib.sha256(data).hexdigest()
            pixels = Image.open(io.BytesIO(data)).convert("RGBA").tobytes()
            stats["chart_rgba_sha256"] = hashlib.sha256(pixels).hexdigest()
            stats["chart_nonwhite_pixels"] = sum(pixels[i:i+4] != b"\xff\xff\xff\xff" for i in range(0, len(pixels), 4))
            assert stats["chart_nonwhite_pixels"] > 10000, "chart is missing plotted content"
            (OUT / (path.stem + ".png")).write_bytes(data)
    return stats, text


def main():
    results = json.loads((OUT / "results.json").read_text())
    checks = []
    texts, summaries = {}, {}
    for case in results["results"]:
        name = case["case"]
        for engine in case["engines"]:
            key = f"{name}-{engine}"
            stats, text = inspect(OUT / "docx" / (key + ".docx"))
            texts[key], summaries[key] = text, stats
            case["engines"][engine]["output"] = stats
            checks.append(f"{key}: ZIP integrity and all XML parts parse")
            if name == "prose":
                assert text.count("Paragraph ") == 3200
                assert stats["distinct_hyperlink_targets"] == 25
            if name == "lists":
                for fragment in ["first item", "Second item", "Nested ordered item", "Third item"]:
                    assert text.count(fragment) == 1200
            if name in ["plain-code", "highlighted-ts"]:
                assert len(re.findall(r"export function compute\d+_\d+", text.replace("\u00a0", " "))) == 640
            if name == "highlighted-ts":
                plain = summaries[f"plain-code-{engine}"]
                assert stats["color_elements"] > plain["color_elements"] * 2
                assert len(stats["distinct_colors"]) > len(plain["distinct_colors"])
                normalized = text.replace("\u00a0", " ")
                assert normalized == texts[f"plain-code-{engine}"].replace("\u00a0", " "), "highlighting changed code text beyond NBSP substitution"
                stats["nonbreaking_spaces"] = text.count("\u00a0")
                checks.append(f"{key}: actual token colors; all 640 functions preserved; text matches plain output after NBSP-to-space normalization")
        ts = texts[f"{name}-typescript"]
        for engine in ["rust-node", "rust-core"]:
            key = f"{name}-{engine}"
            if key not in texts:
                continue
            if name != "chart":
                exact = texts[key] == ts
                assert texts[key].replace("\u00a0", " ") == ts.replace("\u00a0", " "), f"visible text differs: {key}"
                summaries[key]["text_matches_typescript_exactly"] = exact
                checks.append(f"{key}: concatenated Word text matches TypeScript" + (" exactly" if exact else " after NBSP-to-space normalization"))
        if name not in ["plugin-fallback", "highlighted-ts"]:
            assert summaries[f"{name}-rust-node"]["document_xml_sha256"] == summaries[f"{name}-rust-core"]["document_xml_sha256"]
            checks.append(f"{name}: native Node and core document XML match exactly")
        if name == "highlighted-ts":
            assert texts[f"{name}-rust-node"] == ts
            assert summaries[f"{name}-rust-node"]["runs"] == summaries[f"{name}-typescript"]["runs"]
            assert summaries[f"{name}-rust-node"]["distinct_colors"] == summaries[f"{name}-typescript"]["distinct_colors"]
            checks.append("Node highlighting: text, token-run count and color set match lowlight")
        if name == "prose":
            for engine in ["rust-node", "rust-core"]:
                assert summaries[f"{name}-{engine}"]["format_elements"] == summaries[f"{name}-typescript"]["format_elements"]
        if name == "chart":
            for engine in ["rust-node", "rust-core"]:
                assert summaries[f"{name}-{engine}"]["chart_rgba_sha256"] == summaries[f"{name}-typescript"]["chart_rgba_sha256"]
            checks.append("chart: decoded RGBA pixels match TypeScript, Rust Node and Rust core exactly; plotted content is present")
    results["verification"] = {
        "passed_checks": checks,
        "limitations": [
            "Concatenated Word text checks do not establish whitespace layout, numbering restart behavior, pagination or styling equivalence.",
            "Syntect and lowlight use different grammars and token/theme mappings.",
            "Rust core uses Syntect; Node uses lowlight callbacks. Core highlighting text comparison normalizes NBSP to space.",
            "Raw HTML underline is checked as literal text in both Node implementations.",
            "This chart's decoded pixels match exactly; PNG encoding differs. Other chart definitions were not checked for pixel equality.",
            "Word/LibreOffice rendering and browser DOM execution were not tested.",
        ],
    }
    probe = OUT / "compatibility-probe"
    observed = {}
    for engine in ["typescript", "rust-node"]:
        stats, text = inspect(probe / f"{engine}.docx")
        observed[engine] = {"word_text": text, "underline_elements": stats["format_elements"]["u"]}
    assert observed["typescript"] == {"word_text": "HTML underline: <u>word</u>", "underline_elements": 0}
    assert observed["rust-node"] == observed["typescript"]
    results["verification"]["html_underline_probe"] = observed
    (probe / "observed-difference.json").write_text(json.dumps(observed, indent=2) + "\n")
    (OUT / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    (OUT / "validation.json").write_text(json.dumps(results["verification"], indent=2) + "\n")
    print(f"Passed {len(checks)} output checks")


if __name__ == "__main__":
    main()
