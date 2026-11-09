# shields: directional shields take the hits, then the hull

Tier: 1
Scene: combat-duel Seed=7

The Galaxy shoots the cube from astern, so only the aft pool and the hull
move (150 per facing, recharge 0.25 a tick, hull 300). Each tick recharges,
then takes the hits: beam 20 at 0, 45, 90..., torpedo 60 at 27, 117, 207...
Tick N is the world after N steps, so a hit at tick T shows at tick T+1.
Values are worked out by hand in `combat/damage_tests.rs` (model()).

1. Run 120 ticks
2. Expect "target" shield.aft at tick 1 is 130 within 0.01
3. Expect "target" shield.fore at tick 1 is 150
4. Expect "target" shield.port at tick 1 is 150
5. Expect "target" shield.starboard at tick 1 is 150
6. Expect "target" shield.aft at tick 28 is 76.75 within 0.01
7. Expect "target" shield.aft at tick 46 is 61.25 within 0.01
8. Expect "target" shield.aft at tick 91 is 52.5 within 0.01
9. Expect "target" hull at tick 117 is 300
10. Expect no mark "shields.down obj=target" between ticks 0 and 116
11. Expect mark "shields.down obj=target facing=1" at tick 117
12. Expect "target" shield.aft at tick 118 is 0
13. Expect "target" hull at tick 118 is 299.25 within 0.01
14. Expect "target" shield.aft at tick 120 is 0.5 within 0.01
15. Expect "target" shield.fore at tick 120 is 150
16. Expect mark "hit obj=target by=shooter facing=1" at tick 117
17. Expect no mark "hit obj=target facing=0" by tick 120
