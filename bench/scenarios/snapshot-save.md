# snapshot-save: freighters mid-trip, saved and continued

Tier: 1
Scene: freighters Seed=3

The first half of the snapshot check. A freighter has delivered once and is
flying back to the moon when the world is saved on tick 1200; the run goes
on to tick 3000 (two deliveries). `snapshot-start` resumes from the file and
must reach the same digests. This scenario writes `freighters-mid.snap` under
the bench data directory, so it sorts and runs before `snapshot-start`.

1. Scene "mine miner moon"
2. Run until tick 1200
3. Expect mark "delivered player=1" at tick 1039
4. Save snapshot as "freighters-mid.snap"
5. Run until tick 3000
6. Expect mark "delivered player=1" between ticks 2850 and 2950
7. Expect player 1 resource dilithium at tick 3000 is between 300 and 330
