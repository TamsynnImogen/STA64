# freighters: moon to station and back

Tier: 1
Scene: freighters Seed=3

The moon is 1500 units from the owner's station. The freighter mines 150
(300 ticks), flies to within 150 of the station, unloads (300 ticks) and
flies back, at no more than 110 units a second and with its own turning and
braking, so a trip takes more than the 600 + 2 * 327 = 1254 ticks of
straight flight. On this movement model the first delivery lands on tick
1039 and the cycle then repeats every 1860 ticks; the windows below hold the
steady income (150 a trip) without pinning single ticks, so the movement
re-fit of SIM-15 needs no edit unless the cycle moves out of them. The
rival's station on the moon is not a destination: the rival earns nothing and
the owner's delivery goes to the distant station.

1. Scene "mine miner moon"
2. Run 9000 ticks
3. Expect mark "delivered player=1 resource=0 amount=150" between ticks 1000 and 1100
4. Expect mark "delivered player=1 resource=0 amount=150" between ticks 2850 and 2950
5. Expect mark "delivered player=1 resource=0 amount=150" between ticks 4700 and 4800
6. Expect mark "delivered player=1 resource=0 amount=150" between ticks 6550 and 6700
7. Expect mark "delivered player=1 resource=0 amount=150" between ticks 8400 and 8550
8. Expect no mark "delivered player=1" between ticks 0 and 999
9. Expect player 1 resource dilithium at tick 9000 is between 750 and 900
10. Expect player 2 resource dilithium at tick 9000 is 0
11. Expect no mark "delivered player=2" by tick 9000
12. Expect "miner" is within 1550 of "moon" at tick 9000
