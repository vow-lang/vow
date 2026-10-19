"""Behavior tests for scripts/spec_assemble.py."""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import spec_assemble  # noqa: E402

PARTS = {
    "010-intro.md": "# Title\n\nIntro line.\n",
    "020-body.md": "\n## Body\n\nOne.\nTwo.\nThree.\n",
    "030-tail.md": "\n## Tail\n\nEnd.\n",
}


class SpecAssembleTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.spec = Path(self.tmp.name)
        folder = self.spec / "src" / "doc"
        folder.mkdir(parents=True)
        for name, text in PARTS.items():
            (folder / name).write_text(text)
        self.folder = folder
        self.target = self.spec / "doc.md"

    def assembled(self) -> str:
        return "".join(PARTS.values())

    def test_assemble_concatenates_parts_in_name_order(self):
        self.assertEqual(spec_assemble.assemble("doc", self.spec), self.assembled())

    def test_write_all_creates_then_reports_nothing_when_fresh(self):
        self.assertEqual(spec_assemble.write_all(self.spec), ["doc"])
        self.assertEqual(self.target.read_text(), self.assembled())
        self.assertEqual(spec_assemble.write_all(self.spec), [])

    def test_check_flags_missing_and_stale_files(self):
        self.assertEqual(len(spec_assemble.check(self.spec)), 1)
        spec_assemble.write_all(self.spec)
        self.assertEqual(spec_assemble.check(self.spec), [])
        self.target.write_text("edited\n")
        (message,) = spec_assemble.check(self.spec)
        self.assertIn("docs/spec/doc.md is stale", message)
        self.assertIn("--import", message)

    def test_check_flags_oversized_part(self):
        spec_assemble.write_all(self.spec)
        big = self.folder / "020-body.md"
        big.write_text("x\n" * 10_001)
        spec_assemble.write_all(self.spec)
        messages = spec_assemble.check(self.spec)
        self.assertEqual(len(messages), 1)
        self.assertIn("--split", messages[0])

    def test_import_ports_an_edit_inside_a_part(self):
        spec_assemble.write_all(self.spec)
        self.target.write_text(self.assembled().replace("Two.", "Two, edited."))
        self.assertTrue(spec_assemble.import_doc("doc", self.spec))
        self.assertIn("Two, edited.", (self.folder / "020-body.md").read_text())
        self.assertEqual(spec_assemble.check(self.spec), [])

    def test_import_ports_insert_at_boundary_and_ends(self):
        spec_assemble.write_all(self.spec)
        text = self.assembled()
        text = text.replace("\n## Tail", "Added at boundary.\n\n## Tail")
        text = "Before title.\n" + text + "\nAfter end.\n"
        self.target.write_text(text)
        self.assertTrue(spec_assemble.import_doc("doc", self.spec))
        self.assertEqual(spec_assemble.assemble("doc", self.spec), text)

    def test_import_ports_a_deletion_across_parts(self):
        spec_assemble.write_all(self.spec)
        text = self.assembled().replace("Three.\n\n## Tail\n\nEnd.\n", "Gone.\n")
        self.target.write_text(text)
        self.assertTrue(spec_assemble.import_doc("doc", self.spec))
        self.assertEqual(spec_assemble.assemble("doc", self.spec), text)

    def test_import_reports_no_change_for_a_fresh_file(self):
        spec_assemble.write_all(self.spec)
        self.assertFalse(spec_assemble.import_doc("doc", self.spec))

    def test_init_splits_large_document_into_byte_exact_parts(self):
        sections = [f"## Section {i}\n\n" + "line of text\n" * 400 for i in range(8)]
        text = "# Big\n\n" + "\n".join(sections)
        (self.spec / "big.md").write_text(text)
        written = spec_assemble.init_doc("big", self.spec)
        self.assertGreater(len(written), 3)
        for path in written:
            self.assertLessEqual(path.stat().st_size, spec_assemble.MAX_PART_BYTES)
        self.assertEqual(spec_assemble.assemble("big", self.spec), text)
        self.assertTrue(written[1].read_text().startswith("\n## Section"))

    def test_init_cuts_inside_a_block_with_no_blank_lines(self):
        text = "# Dense\n\n" + "- item line that never breaks\n" * 2000
        (self.spec / "dense.md").write_text(text)
        written = spec_assemble.init_doc("dense", self.spec)
        self.assertGreater(len(written), 3)
        self.assertEqual(spec_assemble.assemble("dense", self.spec), text)
        self.assertEqual(len({p.name for p in written}), len(written))

    def test_init_refuses_to_overwrite_existing_parts(self):
        (self.spec / "doc.md").write_text(self.assembled())
        with self.assertRaises(ValueError):
            spec_assemble.init_doc("doc", self.spec)

    def test_split_part_numbers_new_part_between_neighbours(self):
        body = "".join(f"## S{i}\n\n" + "words\n" * 50 + "\n" for i in range(6))
        (self.folder / "020-body.md").write_text(body)
        before = spec_assemble.assemble("doc", self.spec)
        first, second = spec_assemble.split_part(self.folder / "020-body.md")
        self.assertEqual(first.name, "020-body.md")
        self.assertTrue(second.name.startswith("025-"))
        self.assertEqual(spec_assemble.assemble("doc", self.spec), before)

    def test_renumber_restores_a_gap(self):
        (self.folder / "030-tail.md").rename(self.folder / "021-tail.md")
        spec_assemble.renumber("doc", self.spec)
        names = sorted(p.name for p in self.folder.glob("*.md"))
        self.assertEqual(names, ["010-intro.md", "020-body.md", "030-tail.md"])

    def test_repository_assembled_files_are_fresh(self):
        self.assertEqual(spec_assemble.check(), [])


if __name__ == "__main__":
    unittest.main()
