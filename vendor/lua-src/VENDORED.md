# Vendored: lua-src (Lua 5.3.6)

- Upstream: crates.io `lua-src` 551.0.2 (<https://github.com/mlua-rs/lua-src-rs>),
  the crate mlua's `vendored` feature builds Lua from. Lua 5.3.6 itself is
  <https://www.lua.org/>.
- Licence: MIT. `LICENSE` (lua-src) and the copyright notice at the end of
  `lua-5.3.6/lua.h` (Lua) must stay with this code.
- Used through `[patch.crates-io]` in the workspace `Cargo.toml`; not a
  workspace member. Every Lua state in STA64 (mission scripts, the scene
  scripts of the client) runs on it.
- Copied: `src/lib.rs` unchanged, `lua-5.3.6/` (the commit "Vendor lua-src
  551.0.2 ... unchanged" holds the pristine copy, so `git diff <that commit> --
  vendor/lua-src` shows every local change). Not copied: the other Lua versions
  and the crate's tests; `Cargo.toml` is rewritten (no test target).

## Local changes (re-apply after every update)

All are marked `STA64` in the source. Their purpose: a mission script given
the same inputs does the same thing in every Lua state, in every process, on
every machine (issue #1300, `docs/MISSIONS.md` "Determinism").

- `lobject.h`, `ltable.h`, `ltable.c`, `lvm.c` (`luaV_finishset`):
  **insertion order**. The hash part of a table keeps a doubly linked list of
  its nodes in insertion order; `next` walks the array part in index order and
  then that list. A key assigned again after it was set to nil moves to the
  end (`luaH_sta64touch`). The hash part is rebuilt after as many insertions
  as it has nodes, instead of when the free-position search fails, so the
  moment (and so which integer keys live in the array part) depends only on
  the script's operations, not on where keys hashed. Rehash re-inserts the
  hash part in its order, then the vanishing slice of the array part. Cost: 8
  bytes per hash node (40 instead of 32 on 64-bit targets), 16 per table (72
  instead of 56).
- `lgc.c`: **no weak tables** (`__mode` is ignored, every table is traversed
  as strong), and the key of an entry set to nil stays alive until its table
  is rebuilt instead of becoming a dead key, so whether that key is still found
  never depends on when the collector ran.
- `ltablib.c`: **`table.sort` is a stable merge sort** (insertion-sorted runs
  of 8, then merges through two buffer tables). The quicksort, which took a
  pivot from `clock()` and `time()`, is gone. The table is written only after
  sorting, so a raising comparator leaves it unchanged; an inconsistent
  comparator gives some permutation, never "invalid order function".
- `lvm.c` (`l_strcmp`): string comparison uses `strcmp` (byte order) instead
  of `strcoll`, so `<` on strings does not depend on the process locale.

Prefer keeping these changes small and local; record each one here.
