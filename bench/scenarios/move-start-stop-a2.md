# move-start-stop-a2: start and stop under STA2 Classic

Tier: 1
Scene: move-a2

Profile STA2 Classic, the smooth movement model, on craft with the install's
physics values: scout (stock `sscoutphys.odf`, cruise 160), battle (stock
`sbattphys.odf`, cruise 80). Each is sent 1500 straight ahead from rest.
Acceleration is `1.3 combatSpeed forwardAccel` and braking the same; a craft
brakes to rest once inside `tooCloseToTurn` of its point and below half its
combat speed, so it may stop a little short. Pins the speed after 10 and 30
ticks, the tick it comes to rest and where.

1. Scene "goto scout 0 0 1500"
2. Scene "goto battle 5000 0 1500"
3. Run 2 ticks
4. Wait up to 60 seconds until "scout" speed is 0
5. Expect "scout" speed at tick 327 is above 0
6. Expect "scout" speed at tick 328 is 0
7. Expect "scout" z at tick 328 is 1500.0 within 0.01
8. Expect "scout" x at tick 328 is 0 within 0.01
9. Wait up to 60 seconds until "battle" speed is 0
10. Expect "battle" speed at tick 578 is above 0
11. Expect "battle" speed at tick 579 is 0
12. Expect "battle" z at tick 579 is 1487.947 within 0.01
13. Expect "battle" x at tick 579 is 5000 within 0.01
14. Expect "scout" speed at tick 10 is 34.667 within 0.01
15. Expect "scout" speed at tick 30 is 104.0 within 0.01
16. Expect "battle" speed at tick 10 is 52.0 within 0.01
17. Expect "battle" speed at tick 30 is 80.0 within 0.01
