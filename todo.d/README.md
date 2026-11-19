# todo.d: check lines of PRs not yet folded into TODO.md

Every PR that leaves something to check or to do adds **one new file here**
instead of appending to [`TODO.md`](../TODO.md): two PRs never edit the same
lines, so a merge to `main` no longer puts every other open PR in conflict.

- **Name:** the branch name with `/` turned into `-`, plus `.md`
  (`fix/mining-sound` -> `todo.d/fix-mining-sound.md`). Known before the PR
  exists and unique per branch.
- **Content:** exactly `TODO.md`'s format. Headings are copied from `TODO.md`
  (`## Later`, `### Follow-ups from ...`); lines before the first heading
  belong to `## Needs your check`. Each section starts with a check line.

  ```markdown
  - [?] Mining sound (fix/mining-sound): ... what to check, its grade.

  ## Later
  - [ ] ... what stays open.
  ```

- **Later edits:** tick, change or delete a line where it lives now: in the
  fragment until it is folded, in `TODO.md` after. A fragment left empty is
  deleted.
- **Reading all open lines:** `tools/todo.py` (or `tools/todo.py list PATTERN`)
  prints the open lines of `TODO.md` and every fragment as `file:line: text`.
- **Folding:** a housekeeping PR runs `tools/todo.py fold`, which appends each
  fragment's lines at the end of the matching section of `TODO.md` and
  deletes the fragments (`tools/todo.py check` first; `fold --dry-run` shows
  what moves). Self-test: `python3 tools/test_todo.py`.

This file is not a fragment and is never folded.
