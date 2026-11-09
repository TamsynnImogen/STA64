# Scenario fixtures

These scenarios come from Armada II Refit (https://github.com/cedrichaase/armada2-refit),
`testbench/scenarios/`, at commit 749332ed081ae6e8656a0b6a1fb30363e963a759, copied
byte for byte: `cloak` and `shields-hits` (Scene commands and answers), `port-firing`
and `bench-determinism` (a `Setup:` header, paused frames), `qol-big-selection` and
`specials` (selection, orders, specials), `ui-menus-single` (mostly dropped input
steps) and `_enter-instant-action` (a fragment).

MIT License, Copyright (c) 2026 Cedric Haase. See `../../../a2fmt/LICENSE` for the
licence text, which applies to these files as well.

`REFIT_ROOT=<checkout> cargo test -p sta_bench` sweeps every scene and scenario of
a Refit checkout instead.

## parity/

`parity/refit/` stands in for a recorded Refit run (`Scene.marks`,
`Scene.objects.jsonl`): the checkout had no recorded run to copy (a run needs
the game), so these are written to the format in Refit's
`testbench/scene/README.md` ("Marks", "Objects"), starting from the example
lines there (the `epoch` line, `ready`, `cmd`, `select`, the `fgalaxy` object
line) at commit 4177bd948cd31f2fbcf00ebc822021ed1a192172, and extended with
`fire`, `cloak` and `decloak` marks and a header for the `cloak` scene. The
numbers are made up to sit near ours. Replace with a real take when one
exists. `parity/ours/` is the unedited output of `sta-bench run cloak`
(`marks`, `objects.jsonl`).

Same MIT licence, Copyright (c) 2026 Cedric Haase.
