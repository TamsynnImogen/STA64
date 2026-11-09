# ui-hud-layout: the HUD over a bench scene

Tier: 2
Scene: hud Seed=3
Size: 1280x720

The HUD (UI-04) drawn over the `hud` scene: a yard, a flagship and two escorts
of player 1, a rival base and a raider of player 2. Tier 1 checks the scene
(what the HUD is fed from); tier 2 runs it in `sta64 --headless` with
`STA64_UI_HUD=1` (the runner passes the environment on) and requires the same
digest as the in-process run and frames that repeat byte for byte:

    STA64_UI_HUD=1 STA64_UI_HUD_SELECT=yard sta-bench run ui-hud-layout --tier 2 --repeat 2

What the frame holds (the layout, the grid's buttons, the info panel's three
heights, the resources strip, the radar and notices in their places) is checked
by `crates/sta64/tests/hud_frame.rs` (`STA64_GPU_TESTS=1 cargo test -p sta64
--test hud_frame`), which reads the `hud layout:` line the run prints and looks
for the theme's colours in each rectangle.

1. Run 1 ticks
2. Expect 3 ships on team 1
3. Expect 1 stations on team 1
4. Expect 1 ships on team 2
