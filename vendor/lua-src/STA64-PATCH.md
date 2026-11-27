# Vendored and patched: lua-src (Lua 5.3.6)

- Upstream: crates.io `lua-src` 551.0.2 (<https://github.com/mlua-rs/lua-src-rs>),
  the crate mlua's `vendored` feature builds Lua from. Lua 5.3.6 itself is
  <https://www.lua.org/>.
- Licence: MIT. `LICENSE` (lua-src) and the copyright notice at the end of
  `lua-5.3.6/lua.h` (Lua) must stay with this code.
- Used through `[patch.crates-io]` in the workspace `Cargo.toml`; not a
  workspace member. Every Lua state in STA64 (mission scripts, the client's
  scene scripts) runs on it. `Cargo.lock` lists `lua-src` without a registry
  source; if the patch stopped applying, `--locked` builds would fail and the
  test `the_bundled_lua_is_the_patched_copy` (`crates/sta_mission/tests/lua_semantics.rs`)
  would too.
- Copied: `src/lib.rs` unchanged, `lua-5.3.6/` (the commit "Vendor lua-src
  551.0.2 ... unchanged" holds the pristine copy). Not copied: the other Lua
  versions and the crate's tests; `Cargo.toml` is rewritten (no test target)
  and `build.rs` is added: it only tells cargo to watch `lua-5.3.6/`, because
  mlua-sys compiles that C from its own build script and cargo would otherwise
  not notice an edit to it.

## Why

Mission scripts must do the same thing, given the same inputs, in every Lua
state, every process and on every machine (issue #1300): saves that re-run a
mission and any design where several machines run a script depend on it.
Stock Lua orders `pairs` by hash seeds and addresses, sorts with a pivot from
the clock, lets weak tables expose collector timing, prints addresses and
compares strings by locale. The guarantees for mission authors are in
`docs/MISSIONS.md` ("Reproducible tables, sorting and references").

## What is patched

Every change is marked `STA64` in the source.

- `lobject.h`, `ltable.h`, `ltable.c`, `lvm.c` (`luaV_finishset`):
  **insertion order**. The hash part of a table keeps a doubly linked list of
  its nodes in insertion order; `next` walks the array part in index order and
  then that list. A key assigned again after it was set to nil moves to the
  end (`luaH_sta64touch`). The hash part is rebuilt after as many insertions
  as it has nodes, instead of when the free-position search fails, so the
  moment depends only on the script's operations, not on where keys hashed.
  A rebuild re-inserts the hash part in its order, then the vanishing slice
  of the array part. The key just past the array part (`#t + 1` of a list)
  doubles the array part instead of going to the hash part, so a list kept
  without holes is always in the array part. Cost: 8 bytes per hash node (40
  instead of 32 on 64-bit targets), 16 per table (72 instead of 56).
- `lgc.c`: **no weak tables** (`__mode` is ignored, every table is traversed
  as strong, whoever set the metatable), and the key of an entry set to nil
  stays alive until its table is rebuilt instead of becoming a dead key, so
  whether that key is still found never depends on when the collector ran.
- `ltablib.c`: **`table.sort` is a stable merge sort** (insertion-sorted runs
  of 8, then merges through two buffer tables). The quicksort, which took a
  pivot from `clock()` and `time()`, is gone. The table is written only after
  sorting, so a raising comparator leaves it unchanged; an inconsistent
  comparator gives some permutation, never "invalid order function".
- `lvm.c` (`l_strcmp`): **string comparison by bytes** (`strcmp`) instead of
  `strcoll`, so `<` on strings does not depend on the process locale.

## Updating Lua or lua-src

1. Copy the new upstream crate's `src/lib.rs`, `LICENSE` and the Lua 5.3
   source directory over this one, unchanged, and commit that alone (so the
   next commit is the patch as a diff). Keep `Cargo.toml` as it is apart from
   the version, which must match what `mlua-sys` asks for.
2. Re-apply every `STA64` hunk: `git diff <the previous pristine commit>
   <the previous patched commit> -- vendor/lua-src` is the patch to port.
3. Run the guards: `tools/agent-cargo test -p sta_mission --test lua_semantics
   --test lua_repro` (semantics; the authored hazard scripts in several
   states and in child processes) and then `test -p sta_mission` in full.
   Add the `full-ci` label to the PR (four operating systems build this C).

Keep the changes small and local, and record each one here.
