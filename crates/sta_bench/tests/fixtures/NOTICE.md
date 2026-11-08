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
