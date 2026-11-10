# move-waypoints-a2: a waypoint chain under STA2 Classic

Tier: 1
Scene: move-a2

Profile STA2 Classic, the smooth movement model, on craft with the install's
physics values: scout (stock `sscoutphys.odf`, cruise 160), battle (stock
`sbattphys.odf`, cruise 80). The first two are sent along (0,1500),
(1500,1500), (1500,3000) from their start, the last two legs queued. With
`pathLeadDistance` they move on to the next leg before reaching a waypoint and
take both corners near cruise speed, stopping only at the end. Pins cruise
speed through both corners, the tick each comes to rest and where.

1. Scene "goto scout 0 0 1500"
2. Scene "goto scout 1500 0 1500 queue"
3. Scene "goto scout 1500 0 3000 queue"
4. Scene "goto battle 5000 0 1500"
5. Scene "goto battle 6500 0 1500 queue"
6. Scene "goto battle 6500 0 3000 queue"
7. Run 2 ticks
8. Wait up to 60 seconds until "scout" speed is 0
9. Expect "scout" speed at tick 62 is above 144.0
10. Expect "scout" speed at tick 122 is above 144.0
11. Expect "scout" speed at tick 182 is above 144.0
12. Expect "scout" speed at tick 242 is above 144.0
13. Expect "scout" speed at tick 302 is above 144.0
14. Expect "scout" speed at tick 362 is above 144.0
15. Expect "scout" speed at tick 422 is above 144.0
16. Expect "scout" speed at tick 482 is above 144.0
17. Expect "scout" speed at tick 542 is above 144.0
18. Expect "scout" speed at tick 602 is above 144.0
19. Expect "scout" speed at tick 662 is above 144.0
20. Expect "scout" speed at tick 847 is above 0
21. Expect "scout" speed at tick 848 is 0
22. Expect "scout" x at tick 848 is 1500.823 within 0.01
23. Expect "scout" z at tick 848 is 2999.963 within 0.01
24. Wait up to 60 seconds until "battle" speed is 0
25. Expect "battle" speed at tick 31 is above 72.0
26. Expect "battle" speed at tick 91 is above 72.0
27. Expect "battle" speed at tick 151 is above 72.0
28. Expect "battle" speed at tick 211 is above 72.0
29. Expect "battle" speed at tick 271 is above 72.0
30. Expect "battle" speed at tick 331 is above 72.0
31. Expect "battle" speed at tick 391 is above 72.0
32. Expect "battle" speed at tick 451 is above 72.0
33. Expect "battle" speed at tick 511 is above 72.0
34. Expect "battle" speed at tick 571 is above 72.0
35. Expect "battle" speed at tick 631 is above 72.0
36. Expect "battle" speed at tick 691 is above 72.0
37. Expect "battle" speed at tick 751 is above 72.0
38. Expect "battle" speed at tick 811 is above 72.0
39. Expect "battle" speed at tick 871 is above 72.0
40. Expect "battle" speed at tick 931 is above 72.0
41. Expect "battle" speed at tick 991 is above 72.0
42. Expect "battle" speed at tick 1051 is above 72.0
43. Expect "battle" speed at tick 1111 is above 72.0
44. Expect "battle" speed at tick 1171 is above 72.0
45. Expect "battle" speed at tick 1231 is above 72.0
46. Expect "battle" speed at tick 1291 is above 72.0
47. Expect "battle" speed at tick 1351 is above 72.0
48. Expect "battle" speed at tick 1411 is above 72.0
49. Expect "battle" speed at tick 1471 is above 72.0
50. Expect "battle" speed at tick 1612 is above 0
51. Expect "battle" speed at tick 1613 is 0
52. Expect "battle" x at tick 1613 is 6500.526 within 0.01
53. Expect "battle" z at tick 1613 is 2988.151 within 0.01
