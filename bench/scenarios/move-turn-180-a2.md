# move-turn-180-a2: a 180-degree turn under STA2 Classic

Tier: 1
Scene: move-a2

Profile STA2 Classic, the smooth movement model, on craft with the install's
physics values: scout (stock `sscoutphys.odf`, cruise 160), battle (stock
`sbattphys.odf`, cruise 80). Each is sent to a point 3000 away, 180 degrees
off its heading. It asks for full speed at once (no slowing to turn) and turns
at the rate its speed allows, the last of the turn eased by
`turnControlAngle`. Pins heading, position and speed along the turn, and that
it is on its new course (within 1.5 degrees: the eased tail stops short of the
last degree) at the end.

1. Scene "goto scout 0 0 -3000"
2. Scene "goto battle 5000 0 -3000"
3. Run until tick 60
4. Expect "scout" heading at tick 10 is 328.245 within 0.01
5. Expect "scout" heading at tick 20 is 217.949 within 0.01
6. Expect "scout" heading at tick 30 is 180.354 within 0.01
7. Expect "scout" heading at tick 45 is 180.354 within 0.01
8. Expect "scout" heading at tick 60 is 180.354 within 0.01
9. Expect "scout" x at tick 20 is -15.87 within 0.01
10. Expect "scout" z at tick 20 is 7.208 within 0.01
11. Expect "scout" speed at tick 20 is 69.333 within 0.01
12. Expect "scout" x at tick 45 is -21.07 within 0.01
13. Expect "scout" z at tick 45 is -85.626 within 0.01
14. Expect "scout" speed at tick 45 is 156.0 within 0.01
15. Expect "scout" heading is between 178.5 and 181.5
16. Expect "battle" heading at tick 10 is 348.207 within 0.01
17. Expect "battle" heading at tick 20 is 258.912 within 0.01
18. Expect "battle" heading at tick 30 is 180.426 within 0.01
19. Expect "battle" heading at tick 45 is 179.558 within 0.01
20. Expect "battle" heading at tick 60 is 179.558 within 0.01
21. Expect "battle" x at tick 20 is 4983.863 within 0.01
22. Expect "battle" z at tick 20 is 24.323 within 0.01
23. Expect "battle" speed at tick 20 is 80.0 within 0.01
24. Expect "battle" x at tick 45 is 4971.029 within 0.01
25. Expect "battle" z at tick 45 is -36.073 within 0.01
26. Expect "battle" speed at tick 45 is 80.0 within 0.01
27. Expect "battle" heading is between 178.5 and 181.5
