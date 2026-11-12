# golden-formation: formation with its committed digest

**Golden (TOOL-06).** The last step pins the digest; CI runs this scenario on all four
targets. If a sim or fake-content change moves it on purpose, replace the value
(the failing step prints the new one) and say why in the PR.

Tier: 1
Scene: formation Seed=3

Three frigates in a line abreast are sent to (0,0,1000). Each takes its own
slot, 60 units apart and centred on the point, and all three stop on them.

1. Scene "formation f1,f2,f3 1"
2. Scene "goto f1,f2,f3 0 0 1000"
3. Run 2 ticks
4. Wait up to 60 seconds until "f1" speed is 0
5. Run 30 ticks
6. Expect "f1" is within 0.01 of -60 0 1000
7. Expect "f2" is within 0.01 of 0 0 1000
8. Expect "f3" is within 0.01 of 60 0 1000
9. Expect "f1" speed is 0
10. Expect "f2" speed is 0
11. Expect "f3" speed is 0
12. Expect digest at tick 195 is "98e567e8551caee5"
