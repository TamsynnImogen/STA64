# Wireframes and selection gauges

User reference correction after #383. Implements D1/D2/D5/D11. This is client
presentation only; no simulation, save, network or content-digest change.

Wireframes currently enlarge a GUI texture with a forced nearest sampler.
Use an explicit linear-filter opt-in for stock wireframe layers, retaining
authored CFG rectangles and SPR crops. Sampler policy must participate in both
page and resolved-art cache keys so shared texture pages cannot change other
buttons or sprite fonts. Do not mutate original art or scale the whole HUD.

World gauges currently scale thickness with the model's selection sphere and
sit near its origin, which puts wide station bars across the hull. Pinned
Armada2.exe SHA256 8d3ba6ec826198daf3e0c7b3435e9ce8195293e56da7c87e0af2634eaa4693b3
instead projects eight model-bounds corners (0x5081b0/0x5082c0, called with
scale1 at0x50971d), then uses the resulting screen rectangle. Gauge height is
the integer2 at0x70dcd8; shield top is rectangle top minus height minus2
(0x5089ad..5089c1), hull top is rectangle top minus height plus2
(0x508af7..508b0b). These are static original-A2 findings, not original-game
execution or general FO parity proof.

Retain the existing overlay lifecycle and shared view values. Supply converted,
scaled local model bounds on model attachment; project those bounds for gauge
width/anchor, with a selection-radius fallback only for missing bounds. Use
screen-pixel thickness and spacing, not radius-scaled thickness. Hide invalid,
behind-camera or offscreen projections safely. Height and gap defaults can be
overridden by optional STA64-only selectionGaugeHeight/selectionGaugeGap in
the resolved race GUI CFG. Existing HUD bar rectangles remain authoritative.

Following the user's stock reference, numeric hull/shield/energy status values
show rounded percentages as bare integers, without a percent glyph. Current
crew stays a count. Keep existing faction health thresholds and CFG/A2FO colour
overrides, and test that changes in actual hull/shield/crew pools change tint.
No subsystem values are fabricated for backends not yet exposed by the view.

Verify sampler isolation, authored crops/rectangles, fractional scaling, camera
zoom/rotation and model translation/scale, thin separated gauges above the
bounds, invalid projection handling, live values/colours and overlay cleanup.
Run client regressions, mounted offscreen rendering, strict/workspace/ARM64
checks and stage an optimized manual candidate. Original files and prior builds
stay intact; December desktop alpha remains the priority.
