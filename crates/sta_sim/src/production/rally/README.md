# SIM-17 rally points

`Order::Rally { builders, at }` sets destinations for completed mobile units.
Builders need a nonempty content build list, must be finished and alive, and
must belong to the sender (SCRIPT/BENCH may address any player's builders).
Eligible members of a mixed list take the order; an entirely ineligible list
is rejected. Setting or clearing a rally does not cancel production or move
the builder.

`Target::Point` stores a position. `Target::Entity` stores an id and samples
its position when each new mobile unit completes; already launched units
keep their original movement destination. This does not issue follow,
attack or mine orders. `Target::None` clears the rally. Destroyed targets or
builders clear their saved rows; an ownership change invalidates the old
owner's rally on the next production step. Capture gameplay itself remains
outside this package.

Rallies are sorted by builder id, enter the deterministic digest, and persist
in snapshot format 7. Format 6 saves are rejected, as required by the existing
snapshot version policy. Runs with no rally state keep their previous digest.
The appended order preserves the old command tags. Snapshot and replay tests
check continued events and digests.

The headless bench accepts `rally yard 0 0 1000`, `rally yard targetUnit`,
and `rally yard off`; comma-separated builder lists are supported. Run:

```sh
cargo run -p sta_bench --bin sta-bench -- run rally
```

This package implements the simulation and bench. UI-14's right-click wiring,
UI-13's capability-based grid, rally markers, SIM-16's launch hardpoints and
clearance, and measured A1/A2/FO behavior remain separate work. No native
runtime or visual acceptance is claimed here.
