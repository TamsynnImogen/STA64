# replay-save: record a run as a replay

Tier: 1
Scene: goto Seed=3

The first half of the replay check. The scene's two cruisers and their goto
orders (a queued waypoint chain, and a stop) are recorded, scene spawns
included, and the file is played back from tick 0 at save time, which fails
the step on any digest that differs. Writes `goto-mix.replay` under the bench
data directory, so it sorts and runs before `replay-start`. The orders wait
one tick: on tick 0 the sim would apply the owner's orders before the bench's
spawn edits, which a replay (it has no pre-tick-0 spawns) cannot reproduce.

1. Run 1 ticks
2. Scene "goto ship 0 0 600"
3. Scene "goto ship 500 20 600 queue"
4. Scene "goto ship 500 0 -200 queue"
5. Scene "goto runner 2000 0 3000"
6. Scene "at 60 stop runner"
7. Run until tick 700
8. Expect "ship" is within 0.01 of 500 0 -200
9. Save replay as "goto-mix.replay"
