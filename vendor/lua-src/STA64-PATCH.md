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

- `lobject.h`, `ltable.h`, `ltable.c`, `lvm.c` (`luaV_finishset`),
  `lstate.h`/`lstate.c`, `lapi.c`: **insertion order**. The hash part of a
  table keeps a doubly linked list of its nodes in insertion order; `next`
  walks the array part in index order and then that list.
  - A node is never reused before a rebuild: a key set to nil keeps its node
    and stays findable; assigned again it moves to the end
    (`luaH_sta64touch`). So every new key takes exactly one free node, and
    the stock trigger (no free node left) comes after a number of new keys
    that depends only on the operations, never on where keys hashed.
  - A rebuild (`sta64_rebuild`) normally rebuilds the hash part alone at a
    size that leaves a quarter free; the full rehash (stock sizing of both
    parts, which walks the array part) runs only once the new keys since the
    last one reach half the size of both parts. Both are amortised constant
    per new key. A rebuild re-inserts the hash part in its order, then the
    vanishing slice of the array part.
  - The key just past the array part doubles the array part while that part
    is more than half used (a failed check is remembered for the array size,
    `sta64_noappend`), so a list kept by list operations always lives in the
    array part, and queues or ever-growing id registries stay bounded.
  - **Rule: `luaH_resize` never runs the STA64 insert rules.** It re-inserts
    through `sta64_reinsert`, which only places a key in the array or a free
    node and never grows, rebuilds or allocates; a nested resize, or a
    `luaH_newkey` during a resize, aborts (`sta64_resizing`). Stock Lua has
    the same invariant implicitly; the patch's rules would break it.
  - `lua_sta64_tablework` counts the slots rebuilds visit (for the cost test).
  - Cost: 8 bytes per hash node (40 instead of 32 on 64-bit targets), 24 per
    table (80 instead of 56).
- `lgc.c`: **no weak tables** (`__mode` is ignored, every table is traversed
  as strong, whoever set the metatable), and the key of an entry set to nil
  stays alive until its table is rebuilt instead of becoming a dead key, so
  whether that key is still found never depends on when the collector ran.
- `ltablib.c`: **`table.sort` is a stable merge sort** (insertion-sorted runs
  of 8, then merges through two buffer tables). The quicksort, which took a
  pivot from `clock()` and `time()`, is gone. The table is written only after
  sorting, so a raising comparator leaves it unchanged; an inconsistent
  comparator gives some permutation, never "invalid order function".
- `lobject.h`, `lstate.h`, `lstate.c`, `lfunc.c`, `lstring.c`, `ltable.c`,
  `lapi.c`, `lua.h`, `lauxlib.c`: **reference ids instead of addresses**.
  `luaL_tolstring` (behind `tostring`, `print`, `%s`) prints `table: #12`
  where it printed an address. `lua_sta64_refid` hands out ids per state in
  first-printed order; tables, closures, threads and full userdata keep theirs
  in a field (4 bytes, in existing padding except for the thread), light C
  functions and light userdata in the registry table `sta64.refids`.
  `lua_sta64_refid` is also the link-time marker the tests check.
- `lvm.c` (`l_strcmp`): **string comparison by bytes** (`strcmp`) instead of
  `strcoll`, so `<` on strings does not depend on the process locale.
- `lstrlib.c`, `lobject.c`: **no locale**: C-locale character classes and
  case (`string.lower/upper`, pattern classes); numbers print with a dot and
  a string with the locale's radix character is not a number.
- `lobject.c`, `lstrlib.c`, `luaconf.h`, `llimits.h`, `lstate.c`, `lua.h`:
  **the same float text everywhere**: "nan", "inf", "-inf" for `tostring`
  and `%e %f %g`; `%a` is always Lua's own formatter (no flags, width or
  precision); `^` calls the power function the host sets with
  `lua_sta64_setpow` (STA64's sandbox sets `libm::pow`).

## Internal checks

`sta64_check` (in `ltable.c`) guards the patch's own invariants: no resize
inside a resize, no new key during a resize, always a free node when a resize
re-inserts. A failing check calls `abort()` and ends the process at once, on
purpose: going on would corrupt memory. It can only mean a bug in this patch,
never something a script did; no script, however hostile, may be able to
reach it. If it fires, report it as an issue with the label `lane:lua`, the
script or a reduced snippet that triggers it, and the STA64 version, and do
not work around it in Lua.

## Constraints for STA64 code

- Never give a thread its own hook (`Thread::set_hook`): mlua keeps per-thread
  hooks in a weak-keyed table, and with `__mode` ignored those threads would
  never be collected. STA64 uses only global hooks.
- Host code running in a mission state must not turn references into text
  (`tostring` of tables, `Value::to_string` on them): that hands out the next
  reference id and shifts the script's later ids. Host chunks that read
  mission state use `txt` from `sta_mission::sandbox::HOST_TEXT` instead.

## Updating Lua or lua-src

1. Copy the new upstream crate's `src/lib.rs`, `LICENSE` and the Lua 5.3
   source directory over this one, unchanged, and commit that alone (so the
   next commit is the patch as a diff). Keep `Cargo.toml` as it is apart from
   the version, which must match what `mlua-sys` asks for.
2. Re-apply every `STA64` hunk: `git diff <the previous pristine commit>
   <the previous patched commit> -- vendor/lua-src` is the patch to port.
3. Run the guards: `tools/agent-cargo test -p sta_mission --test lua_semantics
   --test lua_tables --test lua_repro --test lua_numbers --test lua_locale`
   (semantics; table internals with a differential test against a model; the
   authored hazard scripts in several states and in child processes; number
   text; locale independence) and then `test -p sta_mission` in full. Add the
   `full-ci` label to the PR (four operating systems build this C).
4. Run the same tests once with the C under AddressSanitizer (Linux, gcc's
   `libasan`; a separate target directory), adding to the command:
   `--target-dir target/asan --config "env.CFLAGS.value='-fsanitize=address -fno-omit-frame-pointer -g'"
   --config "env.CFLAGS.force=true" --config "env.ASAN_OPTIONS='detect_leaks=0:abort_on_error=1:verify_asan_link_order=0'"
   --config "target.x86_64-unknown-linux-gnu.rustflags=['-Clink-arg=-fsanitize=address','-Clink-arg=-lasan']"`,
   and once with Lua's internal assertions: `--target-dir target/luadebug
   --config "env.CFLAGS.value='-DLUA_DEBUG'" --config "env.CFLAGS.force=true"`.

Keep the changes small and local, and record each one here.
