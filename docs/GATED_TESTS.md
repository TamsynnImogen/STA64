# Install-gated tests

Some tests read a game the repository cannot ship (ADR 0003). They are ordinary tests that **skip** when the
install they need is not there, and CI never has one, so they run only on a machine that has the games. Run them
before a release; they rot otherwise (issue #1352).

## One variable per kind of install

A test names the kind of install it needs (`sta_testkit::Kind`, a dev-only crate: never a dependency of a
non-test target). Each kind has its own variable; a test whose variable is unset, or whose install is of another
game, prints one line and passes:

```
SKIP: set STA64_TEST_ROOTS_INSTALL (needs a mod-family (Roots) install)
```

| Kind | Variable | Also |
| --- | --- | --- |
| First game (Armada I) | `STA64_TEST_A1_INSTALL` | |
| Stock second game (Armada II, no mods) | `STA64_TEST_A2_STOCK_INSTALL` | |
| Mod family (Roots lineage) | `STA64_TEST_ROOTS_INSTALL` | `STA64_TEST_ROOTS_MOD` names the mod to activate |
| Fleet Operations 3.2.7 | `STA64_TEST_FO327_INSTALL` | |
| Dominion War | `STA64_TEST_DW_INSTALL` | |
| Evolution | `STA64_TEST_EVOLUTION_INSTALL` | |
| Any game, structure only | the first of the above that is set | |

`STA64_TEST_MAP` names a map the install has; without it, or when the install lacks it, a test that needs a
map takes one the install has (`sta_testkit::a_map`) and skips when it has none. `STA64_GPU_TESTS=1` is the
separate opt-in of the GPU tests; they stay off unless asked.

The old general variables `STA64_TEST_INSTALL` and `STA64_TEST_MOD` still work during the transition, for a kind
that has a content profile, when the install they name is of that kind (checked read-only through the mount the
game uses). They are removed when every crate's tests have moved.

## Running the whole set before a release

```sh
cp tools/gated.example tools/gated.local     # once; git-ignored; fill in the folders of this machine
tools/run-gated --list                       # which groups have a path set, and whether the folder exists
tools/run-gated                              # every group with a path set, the light crates only
tools/run-gated --client --group mod-family  # also the client's gated tests, one group
tools/run-gated --client --gpu               # the client's, with the GPU opt-in
```

By default the runner leaves the client (`sta64`) out: its tests need a heavy build and plenty of free memory,
which can bring a busy machine down. `--client` adds it, one group at a time (use `--group`), and the runner says
so before it starts. It never launches a game either way.

`tools/run-gated` runs `tools/agent-cargo test --workspace` once per group, with only that group's variables set
(cargo's `--config env.NAME=...` form), filtered to the tests whose name contains `install`, and prints per group
how many tests passed, failed and were skipped. The tests write only to temporary folders and never launch a game;
the installs are read only. `tools/gated.local` holds paths of your machine and is never committed. Before each group
the runner unsets every exported `STA64_TEST_*` and `STA64_GPU_*` variable and gives the group's own in cargo's
forced `env` form, so only that group's variables are in effect (`--dry-run` shows it). A value in the local file
must not contain a double quote or a backslash; the runner says which line if it does.

The filter finds the tests with `install` in their name; a few gated tests lack it and are not run by the runner
until they are renamed (a follow-up), and an `#[ignore]`d gated test is never run by it. A skipped test prints its
`SKIP:` line, and the runner's skipped count is the number of distinct such lines.

A test that needs an install writes `SKIP: ...` and returns when it cannot run, never `unwrap`s a missing mod or
file, and has `install` in its name so the runner finds it.
