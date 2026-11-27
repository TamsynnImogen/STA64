# Agents

Rules for every agent session working on STA64 (Claude Code or otherwise),
and for the people who run them. Several sessions work the repository at once;
these rules keep them from doing the same work twice or breaking each other's
builds. The plan they work from is [docs/WORKPLAN.md](docs/WORKPLAN.md); its
"Ownership and shared files" and "Integration and merge protocol" sections
still apply in full.

GitHub issues are the only shared state between sessions. A session that is
not visible in an issue does not exist to the others.

## Before starting anything

1. `git fetch origin` and read `docs/WORKPLAN.md` on `origin/main`, not on a
   stale local `main`.
2. Check that the package is free. All three must come back empty:

   ```sh
   gh issue list --state open --search '"[SIM-04]" in:title'
   gh pr list --state open --search '"[SIM-04]" in:title'
   git ls-remote --heads origin 'wp/SIM-04-*'
   ```

3. Check that its *needs* are merged, and that no open freeze covers the
   paths it owns (see "Freezes").
4. Claim it (next section). Only then branch and start work.

Work that is not a package (a fix to `main`, a docs change) needs no claim,
but still checks open freezes and open PRs touching the same files.

## Claiming a package

Open one issue per package:

- **Title:** `[<ID>] <package name from WORKPLAN>`, e.g. `[SIM-04] Weapons`.
- **Assignee:** the GitHub account of the person whose session does the work.
- **Label:** `lane:<lane>` in lower case (`lane:con`, `lane:sim`, `lane:r0` for
  round 0, `lane:int` for integration). Create it if missing:
  `gh label create lane:sim`.
- **Body**, filled from the package's WORKPLAN row:

  ```markdown
  Package SIM-04 from docs/WORKPLAN.md (Simulation).

  **Contents:** …
  **Owns:** `crates/sta_sim/src/combat/weapons.rs`
  **Needs:** SIM-03 (merged, #NN)
  **Against:** `fake::db()`
  **Unblocks:** SIM-05, RND-09, AUD-02
  **Done when:** …
  **Branch:** `wp/SIM-04-weapons` · Size L

  Claimed by <tool> (<person>'s session).
  ```

The last line names who to ask; keep it verbatim in shape
(`Claimed by Claude Code (Immy's session).`).

**Races.** Right after opening the issue, check again without `--search`
(GitHub's search index lags new issues by a minute or more, so step 2's
search can miss a claim made seconds ago):

```sh
gh issue list --state open --json number,title \
  --jq '.[] | select(.title | startswith("[SIM-04]")) | .number'
```

If an older open issue claims the same ID, the older one wins: close yours
as a duplicate with a link and pick something else.

**One claim at a time per agent.** A session may run several agents, each
with its own claim; an agent does not hold claims it is not working on.

## While working

- Work on `wp/<ID>-<slug>` in your own worktree, only in the paths the
  package owns (plus the hotspot exceptions in WORKPLAN's ownership table).
- Push early and often. A pushed branch shows the claim is alive.
- Blocked, need a seam change, or found the plan wrong: comment on the issue
  first, then open the seam PR or ask. Don't work around a seam locally.
  Touching paths another claim owns follows "Idle claims and mechanical
  edits" below.
- Don't start from another package's unmerged branch; use the fakes.

## Freezes

A package that moves or rewrites files others may touch (R0-3's carve of
`sta64`, a seam PR at a milestone boundary) declares a freeze in its issue
body:

```markdown
**Freeze:** `crates/sta64/src/` until this issue closes.
```

Before claiming, check for freezes:
`gh issue list --state open --search 'Freeze in:body'`. Packages whose owned
paths fall under an open freeze wait; ones elsewhere carry on.

## Finishing

- PR title as the issue (`[SIM-04] Weapons and fire arcs`), body starting
  `Closes #NN`, then the *done when* evidence, files touched outside *owns*,
  decisions the plan did not dictate, and new `TODO.md` check lines.
- Open it as a draft until the *done when* check passes on your branch; mark
  it ready after that. The user merges; agents never merge their own PRs.
- CI on a PR runs `core crates` on `ubuntu-latest` always, and `sta64` on
  `ubuntu-latest` only when the PR touches `crates/sta64/`, `assets/`,
  `bench/`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` or the CI
  workflow. `core crates` runs the tests, clippy and the `sta-bench` report;
  `sta64` runs `cargo check --all-targets`, clippy, the sim-only bench smoke
  and the release tests (a PR that only touches `docs/` runs no CI). Linux
  only is the default for a pull request, and is enough to merge. The full
  4-OS matrix (Linux x86-64 and ARM, macOS, Windows) runs on every
  non-docs-only push to `main`, one run at a time (later pushes collapse into
  one pending run), and is where platform differences are caught. The `full-ci`
  label is optional: it runs all four OSes, including `sta64`, on the PR
  (each such run takes 30 to 60 minutes, by STA64's own observation). Add it
  only when the risk of the change is the platform itself: path handling, line
  endings, platform-specific code, linking or vendored native code, build
  scripts, the CI workflow. Not for determinism or file formats as such.
- Whoever merged a PR that turns `main` red on another OS fixes it or reverts
  it first, before other work. This binds agents and people alike.
- Because a PR that does not touch `sta64` no longer builds it, run
  `cargo check --workspace --all-targets` and the tests of every crate you
  changed locally before pushing (see "Builds"), whatever CI will run.
- Rebase on `origin/main` before asking for review. Regenerate `Cargo.lock`
  (`cargo update -w`), never hand-merge it.
- Never push to `main`, never force-push a branch someone else has pulled.
- After the PR is pushed and the worktree is clean, remove the worktree.

## Releasing a claim

- **Stopping:** comment why and what is pushed, unassign yourself, add the
  label `unclaimed`, and edit the last body line to
  `Released by <tool> (<person>'s session).` Leave the issue open and the
  branch in place so the next claimant can build on it.
- **Stale:** a claim with no push and no issue comment for 48 hours may be
  questioned with a comment on the issue. If nobody answers within 24 hours,
  a new session may take it over: comment, reassign, replace the claim line.

## Idle claims and mechanical edits

The "Stale" rule above is about taking a package over. This section is about
touching the paths of a claim without taking it over. It does not weaken the
"Stale" rule, and the 48-hour question and 24-hour takeover stay as they are.

A claim is **idle** when it is open and has had no push to its branch or
branches, no PR and no comment on its issue for seven days. A claim that is
not idle keeps the rule in "While working": comment on its issue first, then
open the seam PR or ask.

**Mechanical edits.** Any PR may make mechanical, behaviour-free edits in the
paths an idle claim owns: a field added with its default at call sites, a
rename a seam change forces, an import. Nothing that changes what that
package's code does. The PR:

- lists every such file under "files touched outside owns" and says the edits
  are mechanical,
- links the claim issue and leaves a comment there when the PR opens,
- requests the claim holder as reviewer.

The claim holder may ask on the PR for a different shape. Until they do, the
claim does not block the PR.

**Work an accepted ADR assigns elsewhere.** When an accepted ADR moves state
or code that an idle claim owns into another package, that package may
proceed after a notice on the claim issue and 24 hours without objection.
Tests that the claim's merged PRs added stay unchanged and count as the
contract; a test that would have to change is raised on the claim issue first.
The claim's still-open parts stay with it. The issue is not reassigned or
edited.

**Seam PRs.** A seam PR that adds a field or variant fixes every caller on
`main` in the same PR, including callers under open claims. Those fixes are
mechanical edits under the conditions above. For a claim that is not idle, the
holder agrees first, on the issue or on the PR.

## Packages not in the plan

Every claimed ID must be in WORKPLAN. New work gets the next free number in
its lane (`CON-15`) through a small PR that adds the row to WORKPLAN's lane
table, with *owns*, *needs* and *done when*. That PR may be the package's own
first commit, but the issue body says `Not in WORKPLAN yet: added by #NN.`
until it merges.

## Decisions: ADRs and experiments

An ADR (`docs/adr/`) records a technical decision that is hard to reverse:
one that other work, stored data, a file or wire format, mod-facing behaviour
or a stated promise will come to depend on, so that undoing it later costs far
more than making it now. Examples by topic: the engine choice, the
deterministic simulation rules, the modding rules.

A decision that is easy to reverse needs no ADR. Try it, measure it, and back
it out if it is wrong: a small PR that can be reverted on its own, the
evidence in the PR body, open points as `TODO.md` lines. Fail fast; do not
wait for a decision record. An experiment is still ordinary work: claims,
freezes, review, evidence grades, the content rules and every accepted ADR
apply to it. It skips only the record, not the process.

Is it easy to reverse? Ask: can one revert undo it; does any save, replay,
protocol, content or mod format change; has other work been built on it. If
unsure, say so in the PR and ask.

Keep ADRs true. When a change alters what an existing ADR says, amend the ADR
in the same PR or a companion docs PR, even when the change itself is easy to
revert. An out-of-date ADR is worse than none.

Quick experiments do not lower the bar for code that stays. What is merged is
meant to be maintained for years: tests that fail without the change, no
leftover switches, dead paths or measurement code from the experiment, names
and comments that explain the rule. An experiment that did not work out is
removed, not left behind.

Who approves is unchanged (see "Modding decisions" below and
[docs/adr/README.md](docs/adr/README.md)).

## Modding decisions

[ADR 0004](docs/adr/0004-modding-v1.md) (modding v1, D1 to D13; formerly
`docs/MODDING_DECISIONS.md`) is accepted and binding.
Read it before touching mod loading, the VFS mount (`sta_vfs`), profile
detection, `RTS_CFG.h`/ODF rule handling, the content digest, render recipes
or mission Lua, and cite the decision numbers your change relies on in the
issue and PR (`Implements D7`). Do not add a feature the "Not in v1" list
excludes. If a decision is wrong or blocks you, comment on your issue and
stop; changing a decision is a PR to that file approved by Tammy and Cedric.

## Research and game files

Recorded as [ADR 0003](docs/adr/0003-clean-room-content-policy.md); the rules below are the operative wording.

- Decompilation findings and measurements of the original games are not
  committed unless STA64 needs them to work (a data table the code reads, a
  test fixture you authored yourself). Keep them outside the repository, in a
  separate research directory on your machine. Code comments, WORKPLAN rows and
  PR bodies state the rule STA64 follows, the data keys it reads and the
  evidence grade (measured / read from data / read from the executable, static / not measured), in a line or two:
  no `docs/research/` write-ups, no measurement or bench run tables, no
  pseudo-code, addresses or disassembly, no bench run logs.
- Never commit game assets or anything copied or converted from them, in any
  form: no textures, models, sounds, movies, maps, ODF/cfg/sprite/tech-tree
  files or excerpts beyond a key name and a value, no frames showing game art,
  no re-encoded copies. Tests that need them are install-gated and read the
  install at run time.
- Stock mission scripts, whether converted from the mission DLLs or ported by
  hand, count as game content and are never committed or released. Conversion
  happens on the player's machine from their own install into the user data
  directory (WORKPLAN LUA-15 and the note under the LUA table).

## Builds

The repository is built on people's own machines, often by several sessions
at once. Bevy makes `sta64` heavy.

- Run cargo through the wrapper, which encodes the machine-wide policy (low
  priority, a lock per build class so heavy builds queue instead of piling up,
  `--jobs 2`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, and sccache
  when it is installed):

  ```sh
  tools/agent-cargo check -p sta_sim          # light crate
  tools/agent-cargo check -p sta64            # heavy (Bevy), inferred
  tools/agent-cargo check --workspace --all-targets   # shared check target dir
  tools/agent-cargo --dry-run test -p sta_sim # print lock, env, command; run nothing
  ```

  Heavy builds have two slots (`heavy.lock`, `heavy2.lock`): the second is
  allowed only while no original-game bench session runs. Whoever runs one
  holds `bench.lock` for its duration through the same tool
  (`tools/agent-cargo --bench ./a2test run hud`; shared, so several bench
  games may hold it); a heavy build that finds `heavy.lock` busy then waits
  on it instead of taking the second slot.

  Waiting is first come, first served, also across `LOCK-BUSY` retries: the
  wrapper keeps one ticket per worktree in `<locks>/queue/<class>/` and only
  the oldest ones (one per free slot) may try the lock. A retry from the same
  worktree keeps its place; a ticket whose worktree has not polled for 20 s
  (`STA64_QUEUE_ACTIVE`) no longer holds back a free slot, and one idle for
  30 minutes is deleted. `STA64_QUEUE=0` (or `--no-queue`) turns the queue off.

  `LOCK-BUSY` (exit 75) means another agent holds the lock for longer than
  `STA64_LOCK_WAIT` seconds (default 540, under an agent tool call's
  10-minute limit): wait and run it again. Other knobs: `STA64_LOCK_DIR`,
  `STA64_SCCACHE=0`, `SCCACHE_DIR`, and the opt-in `STA64_MOLD=1`; see the
  header of `tools/agent-cargo`. Its self-test is
  `tools/agent-cargo-selftest.sh` (no build). CI does not use the wrapper.
  Fallback without the wrapper (or where `flock` is missing):
  `renice -n 19 -p $$ >/dev/null && cargo test -p sta_sim --jobs 2`.
  (`nice`/`ionice` prefixes are refused by Claude Code's worktree guard;
  `renice` on the shell is not.) The workspace check shares one target
  directory, `<main checkout>/.claude/target-check`, across worktrees. Cargo
  does not key a workspace crate's artefacts by worktree, so the wrapper
  remembers which worktree used that directory last and, when yours differs,
  deletes the workspace crates' fingerprints first (they are rebuilt; the
  third-party dependencies stay cached; without `flock` checks are not
  serialised and this protection does not hold). Never `touch` source files or run
  the check against a stale directory to work around odd errors: if a check
  error does not match your source, report it.
- Incremental builds are opt-in: `tools/agent-cargo --incremental check -p
  sta_sim` (or `STA64_INCREMENTAL=1`) leaves `CARGO_INCREMENTAL` unset, so the
  workspace crates compile incrementally (measured 3.5 to 9 times faster edit
  cycles; dependencies stay cached in sccache). It costs 2 to 3 GB of
  `incremental/` per worktree `target/`, leaves job counts and every other
  policy as they are, and is ignored (with a notice) for the shared workspace
  check directory. Use it while iterating on one crate in one worktree, not
  for one-off checks.
- One cargo command at a time per agent, in the foreground; no background
  builds, no `pkill`.
- Scope to the packages you changed (`-p`). Prefer `cargo check` while
  iterating; run `cargo check --workspace --all-targets` once before pushing
  instead of a full workspace build. CI runs the full matrix.
- Each worktree has its own `target/`; delete worktrees you are done with.
