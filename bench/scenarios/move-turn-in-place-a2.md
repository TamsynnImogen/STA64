# move-turn-in-place-a2: turning at rest under STA2 Classic

Tier: 1
Scene: move-a2

Profile STA2 Classic, the smooth movement model, on craft with the install's
physics values: scout (stock `sscoutphys.odf`, cruise 160), battle (stock
`sbattphys.odf`, cruise 80). Each is sent to a point 300 to its right.
`turnOmegaFractionAtRest` and `turnAlphaFractionAtRest` decide whether a craft
can turn before it moves (stock: 0, a car; FO: 1). Pins the heading after 1,
15 and 30 ticks and how far it has run forward by then.

1. Scene "goto scout 300 0 0"
2. Scene "goto battle 5300 0 0"
3. Run 60 ticks
4. Expect "scout" heading at tick 1 is 0.255 within 0.01
5. Expect "scout" heading at tick 15 is 72.146 within 0.01
6. Expect "scout" heading at tick 30 is 91.414 within 0.01
7. Expect "scout" z at tick 30 is 10.865 within 0.01
8. Expect "battle" heading at tick 1 is 0.0 within 0.01
9. Expect "battle" heading at tick 15 is 42.001 within 0.01
10. Expect "battle" heading at tick 30 is 94.35 within 0.01
11. Expect "battle" z at tick 30 is 24.004 within 0.01
