# golden-pd-torpedoes: pd-torpedoes with its committed digest

**Golden (TOOL-06).** The last step pins the digest; CI runs this scenario on all four
targets. If a sim or fake-content change moves it on purpose, replace the value
(the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: combat-pd Seed=7

West, a lone cruiser fires a torpedo every 90 ticks (0, 90, 180) at a station
800 units off. The torpedo (13.33 u/tick) is inside the PD's 250 range after
42 moves and the PD takes it on the next tick: PD shots at 43 and 133, no
hit on the station. East, three cruisers fire together; the PD cycles every
10 ticks and the shots need 18-19 ticks to cross its range, so it takes two
(43 and 53) and the third hits at 61. The next volley (fired at 90) repeats
that 90 ticks later: PD at 133 and 143, a hit at 151.

1. Run 200 ticks
2. Expect mark "fire.start obj=lone hp=1" at tick 0
3. Expect mark "fire.start obj=lone hp=1" at tick 90
4. Expect mark "fire.start obj=lone hp=1" at tick 180
5. Expect mark "fire.start obj=st_a hp=1" at tick 43
6. Expect mark "fire.start obj=st_a hp=1" at tick 133
7. Expect no mark "fire.start obj=st_a hp=1" between ticks 44 and 132
8. Expect no mark "hit obj=st_a" by tick 200
9. Expect "st_a" hull at tick 200 is 3000
10. Expect mark "fire.start obj=trio_1 hp=1" at tick 0
11. Expect mark "fire.start obj=trio_3 hp=1" at tick 0
12. Expect mark "fire.start obj=st_b hp=1" at tick 43
13. Expect mark "fire.start obj=st_b hp=1" at tick 53
14. Expect no mark "hit obj=st_b" between ticks 0 and 60
15. Expect mark "hit obj=st_b" at tick 61
16. Expect no mark "hit obj=st_b" between ticks 62 and 150
17. Expect mark "fire.start obj=st_b hp=1" at tick 133
18. Expect mark "fire.start obj=st_b hp=1" at tick 143
19. Expect mark "hit obj=st_b" at tick 151
20. Expect no mark "fire.start obj=st_a hp=0" by tick 200
21. Expect digest at tick 200 is "785de4a8348de86f"
