# nav-nebula-repeat: nav-nebula again, digests compared

Tier: 1
Scene: nav-nebula Seed=3

The routed run twice in one process: the planner's grid, field and cache
leave the same digest at every compared tick (the golden value itself is
pinned in `sta_sim`'s `nav` tests, which CI runs on all four targets).

1. Scene "goto ship 0 0 3000"
2. Run 600 ticks
3. Expect digest at tick 300 to match run "nav-nebula"
4. Expect digest at tick 600 to match run "nav-nebula"
