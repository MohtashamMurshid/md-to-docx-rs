#!/usr/bin/env python3
"""Fail on any difference beyond the reviewed exceptions, checking PNG pixels too."""
import ast
import json
import re
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from compare import compare_case, package_dump


def chart_diagnostics_match(directory, entry):
    prefix = "Invalid chart block: expected chart JSON ("
    if not entry['ts']['ok'] or not entry['rs']['ok']:
        return (not entry['ts']['ok'] and not entry['rs']['ok']
                and all(entry[engine]['message'].startswith(prefix)
                        and entry[engine]['message'].endswith(')')
                        for engine in ('ts', 'rs')))
    packages = []
    for engine in ('ts', 'rs'):
        package = package_dump(Path(directory, f"{entry['name']}.{engine}.docx"), entry.get('fixedTimestamps', ()))
        replacements = 0
        for lines in package.values():
            for index, line in enumerate(lines):
                if '[Chart could not be rendered: expected chart JSON (' not in line:
                    continue
                head, separator, literal = line.partition('>')
                value = ast.literal_eval(literal)
                if value.startswith('[Chart could not be rendered: expected chart JSON (') and value.endswith(')]'):
                    lines[index] = head + separator + repr('[Chart could not be rendered: expected chart JSON (<parser diagnostic>)]')
                    replacements += 1
        if not replacements:
            return False
        packages.append(package)
    return packages[0] == packages[1]


def main():
    directory = sys.argv[1] if len(sys.argv) > 1 else 'output/parity-final'
    entries = json.loads(Path(directory, 'index.json').read_text())
    known = {entry['name']: entry for entry in json.loads(Path(__file__).with_name('known-differences.json').read_text())}
    strict_count = pixel_count = 0
    failures = []
    for entry in entries:
        os.environ.pop('PARITY_MEDIA_MODE', None)
        strict = compare_case(directory, entry)
        os.environ['PARITY_MEDIA_MODE'] = 'pixels'
        pixels = compare_case(directory, entry)
        strict_count += not strict
        pixel_count += not pixels
        if not strict:
            continue
        exception = known.get(entry['name'])
        valid = False
        if exception:
            kind = exception['kind']
            classes = {category for category, _ in pixels}
            if kind == 'png-encoding':
                valid = not pixels
            elif kind == 'chart-json-diagnostic':
                valid = classes <= {'error-message', 'diff word/document.xml'} and chart_diagnostics_match(directory, entry)
            elif kind == 'upstream-process-failure':
                valid = entry['ts'].get('name') in {'ProcessError', 'TimeoutError'} and entry['rs']['ok'] and classes == {'error-parity'}
            elif kind == 'upstream-heading-footnote':
                allowed = {
                    "-<bookmarkUniqueNumericId >'() => ++currentCount'",
                    "+<w:bookmarkStart w:id='1' w:name='mdfootnote_1'>",
                    "+<w:r >", "+<w:rPr >",
                    "+<w:rStyle w:val='FootnoteReference'>",
                    "+<w:footnoteReference w:id='1'>", "+<w:bookmarkEnd w:id='1'>",
                }
                valid = classes == {'diff word/document.xml'} and all({line[0] + line[1:].strip() for line in detail.splitlines()} == allowed for _, detail in pixels)
            elif kind == 'upstream-empty-fragment':
                valid = classes == {'diff word/document.xml'} and all(
                    len(detail.splitlines()) == 2
                    and re.fullmatch(r"-\s*<w:hyperlink r:id='rId[^']+' w:history='1'>", detail.splitlines()[0])
                    and detail.splitlines()[1][1:].strip() == "<w:hyperlink w:anchor='' w:history='1'>"
                    for _, detail in pixels)
        if not valid:
            failures.append((entry['name'], [category for category, _ in strict]))
    os.environ.pop('PARITY_MEDIA_MODE', None)
    print(f'{strict_count}/{len(entries)} match with PNG bytes; {pixel_count}/{len(entries)} match with decoded PNG pixels')
    for name, categories in failures:
        print(f'UNEXPECTED {name}: {categories}')
    print(f'{len(failures)} unexpected differences')
    return bool(failures)


if __name__ == '__main__':
    sys.exit(main())
