# move-turn-180-fo: a 180-degree turn under FO4

Tier: 1
Scene: move-fo

Profile FO4, the smooth movement model, on craft with the install's physics
values: escort (FO 4.0 `smooth_physics_150`, cruise 180), cruiser
(`smooth_physics_100`, cruise 120), battle (`smooth_physics_60`, cruise 72).
Each is sent to a point 3000 away, 180 degrees off its heading. It asks for
full speed at once (no slowing to turn) and turns at the rate its speed
allows, the last of the turn eased by `turnControlAngle`. Pins heading,
position and speed along the turn, and that it is on its new course (within
1.5 degrees: the eased tail stops short of the last degree) at the end.

1. Scene "goto escort 0 0 -3000"
2. Scene "goto cruiser 5000 0 -3000"
3. Scene "goto battle 10000 0 -3000"
4. Run until tick 120
5. Expect "escort" heading at tick 15 is 337.569 within 0.01
6. Expect "escort" heading at tick 30 is 270.275 within 0.01
7. Expect "escort" heading at tick 60 is 179.294 within 0.01
8. Expect "escort" heading at tick 90 is 179.289 within 0.01
9. Expect "escort" heading at tick 120 is 179.289 within 0.01
10. Expect "escort" x at tick 30 is -14.423 within 0.01
11. Expect "escort" z at tick 30 is 15.543 within 0.01
12. Expect "escort" speed at tick 30 is 46.8 within 0.01
13. Expect "escort" x at tick 90 is -40.003 within 0.01
14. Expect "escort" z at tick 90 is -158.527 within 0.01
15. Expect "escort" speed at tick 90 is 140.4 within 0.01
16. Expect "escort" heading is between 178.5 and 181.5
17. Expect "cruiser" heading at tick 15 is 345.046 within 0.01
18. Expect "cruiser" heading at tick 30 is 300.183 within 0.01
19. Expect "cruiser" heading at tick 60 is 191.92 within 0.01
20. Expect "cruiser" heading at tick 90 is 179.309 within 0.01
21. Expect "cruiser" heading at tick 120 is 179.309 within 0.01
22. Expect "cruiser" x at tick 30 is 4992.854 within 0.01
23. Expect "cruiser" z at tick 30 is 13.119 within 0.01
24. Expect "cruiser" speed at tick 30 is 31.2 within 0.01
25. Expect "cruiser" x at tick 90 is 4959.23 within 0.01
26. Expect "cruiser" z at tick 90 is -88.624 within 0.01
27. Expect "cruiser" speed at tick 90 is 93.6 within 0.01
28. Expect "cruiser" heading is between 178.5 and 181.5
29. Expect "battle" heading at tick 15 is 351.027 within 0.01
30. Expect "battle" heading at tick 30 is 324.11 within 0.01
31. Expect "battle" heading at tick 60 is 228.789 within 0.01
32. Expect "battle" heading at tick 90 is 180.435 within 0.01
33. Expect "battle" heading at tick 120 is 179.348 within 0.01
34. Expect "battle" x at tick 30 is 9997.283 within 0.01
35. Expect "battle" z at tick 30 is 8.806 within 0.01
36. Expect "battle" speed at tick 30 is 18.72 within 0.01
37. Expect "battle" x at tick 90 is 9958.758 within 0.01
38. Expect "battle" z at tick 90 is -32.76 within 0.01
39. Expect "battle" speed at tick 90 is 56.16 within 0.01
40. Expect "battle" heading is between 178.5 and 181.5
