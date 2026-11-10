# rally: yard launches follow a point; clearing leaves the next ship idle

Tier: 1
Scene: rally Seed=3

The yard produces two frigates with a point rally at (0,0,1000). Both fly
there. Clearing the rally before a third build leaves that frigate idle.
The fake frigate costs 250 dilithium and takes 450 ticks to build.

1. Scene "cargo yard dilithium 1000"
2. Run 1 ticks
3. Scene "rally yard 0 0 1000"
4. Scene "build yard fx_frigate 0 0 0"
5. Scene "build yard fx_frigate 0 0 0"
6. Run 1 ticks
7. Expect player 1 resource dilithium is 500
8. Run until tick 600
9. Expect mark "built obj=yard" at tick 451
10. Expect "#2" speed is above 0
11. Run until tick 930
12. Expect mark "built obj=yard" at tick 901
13. Expect 2 ships on team 1
14. Expect "#3" speed is above 0
15. Run until tick 1200
16. Expect "#2" is within 5 of 0 0 1000
17. Expect "#3" is within 5 of 0 0 1000
18. Scene "rally yard off"
19. Scene "build yard fx_frigate 0 0 0"
20. Run until tick 1700
21. Expect 3 ships on team 1
22. Expect "#4" speed is 0
23. Expect "#4" is within 100 of 0 0 0
