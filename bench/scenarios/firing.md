# firing: a Galaxy shoots a Borg cube

Tier: 1
Scene: firing Seed=7

The shooter's Attack= order lands on tick 0. On the fake content the beam
(hardpoint 0) fires at once, and a torpedo follows on hardpoint 1.

1. Run 200 ticks
2. Expect mark "fire.start obj=shooter hp=0" by tick 0
3. Expect mark "fire.start obj=shooter hp=1" by tick 5
4. Expect mark "hit obj=target by=shooter" by tick 10
5. Expect "shooter" speed is 0
6. Expect "target" hull at tick 200 is below 300
