# move-turn-in-place-fo: turning at rest under FO4

Tier: 1
Scene: move-fo

Profile FO4, the smooth movement model, on craft with the install's physics
values: escort (FO 4.0 `smooth_physics_150`, cruise 180), cruiser
(`smooth_physics_100`, cruise 120), battle (`smooth_physics_60`, cruise 72).
Each is sent to a point 300 to its right. `turnOmegaFractionAtRest` and
`turnAlphaFractionAtRest` decide whether a craft can turn before it moves
(stock: 0, a car; FO: 1). Pins the heading after 1, 15 and 30 ticks and how
far it has run forward by then.

1. Scene "goto escort 300 0 0"
2. Scene "goto cruiser 5300 0 0"
3. Scene "goto battle 10300 0 0"
4. Run 60 ticks
5. Expect "escort" heading at tick 1 is 0.1 within 0.01
6. Expect "escort" heading at tick 15 is 22.431 within 0.01
7. Expect "escort" heading at tick 30 is 75.552 within 0.01
8. Expect "escort" z at tick 30 is 16.345 within 0.01
9. Expect "cruiser" heading at tick 1 is 0.066 within 0.01
10. Expect "cruiser" heading at tick 15 is 14.954 within 0.01
11. Expect "cruiser" heading at tick 30 is 58.045 within 0.01
12. Expect "cruiser" z at tick 30 is 13.139 within 0.01
13. Expect "battle" heading at tick 1 is 0.04 within 0.01
14. Expect "battle" heading at tick 15 is 8.973 within 0.01
15. Expect "battle" heading at tick 30 is 35.89 within 0.01
16. Expect "battle" z at tick 30 is 8.806 within 0.01
