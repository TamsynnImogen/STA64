# firing: a Galaxy shoots a Borg cube

Tier: 1
Scene: combat-firing Seed=7

The shooter's Attack= order lands on tick 0. On the fake content the beam
(hardpoint 0, 20 damage) fires every 45 ticks and each burst ends 15 ticks
after its shot; the torpedo (hardpoint 1, 60 damage) fires every 90 ticks and
reaches the target 27 ticks later. The target is immortal, healed every tick
and has its weapons off, so the run is the same however long it goes.

1. Run 200 ticks
2. Expect mark "fire.start obj=shooter hp=0" at tick 0
3. Expect mark "fire.end obj=shooter hp=0" at tick 15
4. Expect mark "fire.start obj=shooter hp=0" at tick 45
5. Expect mark "fire.end obj=shooter hp=0" at tick 60
6. Expect mark "fire.start obj=shooter hp=0" at tick 90
7. Expect mark "fire.start obj=shooter hp=1" at tick 0
8. Expect mark "fire.start obj=shooter hp=1" at tick 90
9. Expect mark "fire.start obj=shooter hp=1" at tick 180
10. Expect no mark "fire.start obj=shooter hp=1" between ticks 1 and 89
11. Expect mark "hit obj=target by=shooter" at tick 0
12. Expect mark "hit obj=target by=shooter" at tick 27
13. Expect no mark "fire.start obj=target" by tick 200
14. Expect no mark "rejected" by tick 200
15. Expect "shooter" speed is 0
16. Expect "target" exists at tick 200
17. Expect "target" hull at tick 200 is 300
