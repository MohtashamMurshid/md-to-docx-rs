"""Checks that the differential comparator retains meaningful XML differences."""
import io
import unittest
import zipfile
from xml.etree import ElementTree as ET

from compare import canon, part_dump, compare_case


class CanonicalizationTests(unittest.TestCase):
    def test_typed_error_contexts_are_compared(self):
        expected = dict(ok=False, name="MarkdownConversionError", message="too long", context={"length": 3})
        actual = dict(expected, context={"length": "3"})
        self.assertEqual([kind for kind, _ in compare_case("unused", {"name": "error", "ts": expected, "rs": actual})], ["error-context"])

    def test_error_classes_are_compared(self):
        expected = dict(ok=False, name="MarkdownConversionError", message="failed", context=None)
        actual = dict(expected, name="TypeError")
        self.assertEqual([kind for kind, _ in compare_case("unused", {"name": "error", "ts": expected, "rs": actual})], ["error-name"])

    def test_text_spaces_are_not_discarded(self):
        self.assertNotEqual(canon(ET.fromstring('<t> </t>'), {}), canon(ET.fromstring('<t/>'), {}))

    def test_namespace_identity_is_preserved(self):
        self.assertNotEqual(canon(ET.fromstring('<r xmlns="urn:one"/>'), {}), canon(ET.fromstring('<r xmlns="urn:two"/>'), {}))

    def test_namespace_prefix_spelling_is_irrelevant(self):
        self.assertEqual(canon(ET.fromstring('<a:r xmlns:a="urn:test"/>'), {}), canon(ET.fromstring('<b:r xmlns:b="urn:test"/>'), {}))

    def test_explicit_timestamps_are_compared(self):
        data = io.BytesIO()
        with zipfile.ZipFile(data, 'w') as package:
            package.writestr('docProps/core.xml', '<cp:coreProperties xmlns:cp="urn:core" xmlns:dcterms="urn:date"><dcterms:created>2020-01-02</dcterms:created></cp:coreProperties>')
        data.seek(0)
        with zipfile.ZipFile(data) as package:
            self.assertNotEqual(part_dump(package, 'docProps/core.xml'), part_dump(package, 'docProps/core.xml', ['created']))


if __name__ == '__main__':
    unittest.main()
