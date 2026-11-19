import contextlib
import importlib.util
import io
import pathlib
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).with_name("todo.py")
SPEC = importlib.util.spec_from_file_location("todo", SCRIPT)
todo = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(todo)

TODO = """# T

## Needs your check
- [?] old check

### Dated
- [?] dated check

## Later
- [ ] later item
- [x] done item
"""


class TodoTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        (self.root / "TODO.md").write_text(TODO, encoding="utf-8")
        (self.root / "todo.d").mkdir()
        (self.root / "todo.d" / "README.md").write_text("- [ ] not a fragment\n", encoding="utf-8")

    def tearDown(self):
        self.tmp.cleanup()

    def frag(self, name, text):
        (self.root / "todo.d" / name).write_text(text, encoding="utf-8")

    def run_main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = todo.main(["--root", str(self.root), *argv])
        return code, out.getvalue(), err.getvalue()

    def test_list_combines_open_lines_and_skips_readme_and_done(self):
        self.frag("fix-a.md", "- [?] new check\n\n## Later\n- [ ] new later\n  - [ ] sub item\n")
        code, out, _ = self.run_main("list")
        self.assertEqual(code, 0)
        self.assertEqual(out.splitlines(), [
            "TODO.md:4: - [?] old check",
            "TODO.md:7: - [?] dated check",
            "TODO.md:10: - [ ] later item",
            "todo.d/fix-a.md:1: - [?] new check",
            "todo.d/fix-a.md:4: - [ ] new later",
            "todo.d/fix-a.md:5: - [ ] sub item",
        ])
        _, out, _ = self.run_main("list", "--all", "DONE|sub")
        self.assertEqual(out.splitlines(), ["TODO.md:11: - [x] done item", "todo.d/fix-a.md:5: - [ ] sub item"])

    def test_fold_appends_at_section_end_and_deletes_fragments(self):
        self.frag("fix-a.md", "- [?] a check\n  continued\n\n## Later\n- [ ] a later\n")
        self.frag("wp-b.md", "## Needs your check\n- [?] b check\n")
        code, _, _ = self.run_main("fold")
        self.assertEqual(code, 0)
        self.assertEqual((self.root / "TODO.md").read_text(encoding="utf-8"), """# T

## Needs your check
- [?] old check
- [?] a check
  continued
- [?] b check

### Dated
- [?] dated check

## Later
- [ ] later item
- [x] done item
- [ ] a later
""")
        self.assertEqual(sorted(p.name for p in (self.root / "todo.d").iterdir()), ["README.md"])

    def test_unknown_heading_blocks_fold_and_changes_nothing(self):
        self.frag("fix-a.md", "- [?] fine\n")
        self.frag("fix-b.md", "## No such section\n- [ ] x\n")
        code, _, err = self.run_main("check")
        self.assertEqual(code, 1)
        self.assertIn("heading not in TODO.md", err)
        code, _, _ = self.run_main("fold")
        self.assertEqual(code, 1)
        self.assertEqual((self.root / "TODO.md").read_text(encoding="utf-8"), TODO)
        self.assertTrue((self.root / "todo.d" / "fix-a.md").exists())

    def test_check_rejects_prose_before_the_first_check_line(self):
        self.frag("fix-a.md", "Some prose\n- [ ] x\n")
        code, _, err = self.run_main("check")
        self.assertEqual(code, 1)
        self.assertIn("not a check line", err)

    def test_dry_run_writes_nothing(self):
        self.frag("fix-a.md", "- [?] a\n")
        code, out, _ = self.run_main("fold", "--dry-run")
        self.assertEqual(code, 0)
        self.assertIn("1 line(s) -> ## Needs your check", out)
        self.assertEqual((self.root / "TODO.md").read_text(encoding="utf-8"), TODO)
        self.assertTrue((self.root / "todo.d" / "fix-a.md").exists())

    def test_crlf_no_trailing_newline_and_empty_fragment(self):
        (self.root / "todo.d" / "fix-a.md").write_bytes(b"## Later\r\n- [ ] crlf item")
        self.frag("fix-b.md", "\n\n")
        self.assertEqual(self.run_main("fold")[0], 0)
        text = (self.root / "TODO.md").read_text(encoding="utf-8")
        self.assertTrue(text.endswith("- [x] done item\n- [ ] crlf item\n"))
        self.assertNotIn("\r", text)
        self.assertEqual(sorted(p.name for p in (self.root / "todo.d").iterdir()), ["README.md"])

    def test_second_fold_changes_nothing(self):
        self.frag("fix-a.md", "- [?] a\n")
        self.assertEqual(self.run_main("fold")[0], 0)
        once = (self.root / "TODO.md").read_text(encoding="utf-8")
        self.assertEqual(self.run_main("fold")[0], 0)
        self.assertEqual((self.root / "TODO.md").read_text(encoding="utf-8"), once)

    def test_check_flags_files_without_md_extension(self):
        self.frag("fix-a.txt", "- [ ] x\n")
        code, _, err = self.run_main("check")
        self.assertEqual(code, 1)
        self.assertIn("todo.d/fix-a.txt: not a .md file", err)

    def test_non_utf8_fragment_is_a_message_not_a_traceback(self):
        (self.root / "todo.d" / "fix-a.md").write_bytes(b"- [ ] \xff\n")
        with self.assertRaises(SystemExit) as ctx:
            self.run_main("check")
        self.assertIn("not UTF-8", str(ctx.exception.code))
        self.assertEqual((self.root / "TODO.md").read_text(encoding="utf-8"), TODO)


if __name__ == "__main__":
    unittest.main()
