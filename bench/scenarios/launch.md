# launch: a built ship leaves its yard from the launch hardpoint

Tier: 1
Scene: launch Seed=3

The yard (`fx_yard_launch`) faces +x, with its launch hardpoint 40 units ahead
of its centre and a clear circle of 60. A frigate takes 450 ticks to build. It
appears at the hardpoint, not the centre, facing out, then drives out of the
circle (to 80 from the centre, the circle plus a margin) and stops. Real
builders get the same fields from `buildHardpoint` and the model's node under
every profile: `crates/sta_content/tests/launch_install.rs` checks STA2
Classic, FO4, FO, DW, A1 and A2.

1. Scene "cargo yard dilithium 1000"
2. Run 1 ticks
3. Scene "build yard fx_frigate 0 0 0"
4. Run until tick 452
5. Expect mark "built obj=yard" at tick 451
6. Expect "#2" is within 3 of 40 0 0
7. Run until tick 600
8. Expect "#2" speed is 0
9. Expect "#2" is within 2 of 80 0 0
10. Expect "#2" is within 85 of 0 0 0
