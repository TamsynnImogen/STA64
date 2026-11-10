# move-turn-90-a2: a 90-degree turn under STA2 Classic

Tier: 1
Scene: move-a2

Profile STA2 Classic, the smooth movement model, on craft with the install's
physics values: scout (stock `sscoutphys.odf`, cruise 160), battle (stock
`sbattphys.odf`, cruise 80). Each is sent to a point 3000 away, 90 degrees off
its heading. It asks for full speed at once (no slowing to turn) and turns at
the rate its speed allows, the last of the turn eased by `turnControlAngle`.
Pins heading, position and speed along the turn, and that it is on its new
course (within 1.5 degrees: the eased tail stops short of the last degree) at
the end.

1. Scene "goto scout 3000 0 0"
2. Scene "goto battle 8000 0 0"
3. Run until tick 60
4. Expect "scout" heading at tick 10 is 31.755 within 0.01
5. Expect "scout" heading at tick 20 is 88.665 within 0.01
6. Expect "scout" heading at tick 30 is 89.187 within 0.01
7. Expect "scout" heading at tick 45 is 89.244 within 0.01
8. Expect "scout" heading at tick 60 is 89.244 within 0.01
9. Expect "scout" x at tick 20 is 16.602 within 0.01
10. Expect "scout" z at tick 20 is 11.672 within 0.01
11. Expect "scout" speed at tick 20 is 69.333 within 0.01
12. Expect "scout" x at tick 45 is 110.48 within 0.01
13. Expect "scout" z at tick 45 is 13.077 within 0.01
14. Expect "scout" speed at tick 45 is 156.0 within 0.01
15. Expect "scout" heading is between 88.5 and 91.5
16. Expect "battle" heading at tick 10 is 11.793 within 0.01
17. Expect "battle" heading at tick 20 is 81.027 within 0.01
18. Expect "battle" heading at tick 30 is 90.359 within 0.01
19. Expect "battle" heading at tick 45 is 90.359 within 0.01
20. Expect "battle" heading at tick 60 is 90.359 within 0.01
21. Expect "battle" x at tick 20 is 5015.953 within 0.01
22. Expect "battle" z at tick 20 is 25.114 within 0.01
23. Expect "battle" speed at tick 20 is 80.0 within 0.01
24. Expect "battle" x at tick 45 is 5082.576 within 0.01
25. Expect "battle" z at tick 45 is 25.483 within 0.01
26. Expect "battle" speed at tick 45 is 80.0 within 0.01
27. Expect "battle" heading is between 88.5 and 91.5
