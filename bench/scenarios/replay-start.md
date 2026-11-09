# replay-start: replay the recorded run

Tier: 1
Scene: replay-start

The second half: `goto-mix.replay` (from `replay-save`) is a map, a seed and
the command log. The runner feeds the log to a fresh sim, checks the digests
recorded in the file as it goes (a mismatch is logged as `replay desync`),
and the run ends in the same state as the recorded one.

1. Run until tick 700
2. Expect mark "spawn obj=#1" at tick 0
3. Expect "bench.log" does not contain "desync"
4. Expect "#1" is within 0.01 of 500 0 -200
5. Expect "#1" speed is 0
6. Expect "#2" speed is 0
7. Expect digest at tick 700 to match run "replay-save"
8. Expect digest at tick 301 to match run "replay-save"
