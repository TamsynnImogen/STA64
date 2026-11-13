# shots-planet: the same planet scene drawn at two ticks

Tier: 1
Scene: planet Seed=7

Nothing in the scene moves, so the frame at tick 0 and the frame at tick 100
are the same picture, up to a rounding. At tier 1 the screenshot and
comparison steps are recorded and skipped (and a comparison of a name never
taken fails); `sta-bench run shots-planet --tier 2` draws the frames in a
`sta64 --headless` process (frames are skipped without a GPU) and settles the
comparisons.

1. Take a screenshot called "first"
2. Run 100 ticks
3. Take a screenshot called "later"
4. Expect shots "first" and "later" to match within 1
5. Take a screenshot called "again"
6. Expect shots "later" and "again" to match
