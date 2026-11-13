# Persistent shield visibility

RND-32 adds the optional A2FOExtensions object ODF command
`alwaysShowShields = 1`. The default is off. The renderer reads the effective
ODF after ordinary includes and layer resolution, so an explicit zero in a
child disables its parent's setting.

A configured ship or station has one continuous shield outline while its
current shields are positive. At zero the outline disappears; recharge
restores it. Hit and collapse effects remain independent. The outline follows
the rendered unit and is suppressed while cloaked, outside the local player's
perception, while under construction, or without a live render root. Removing
a unit cleans up its outline. Leaving a match releases the renderer's shared
outline mesh and material.

The shader participates in the existing developer-only shader shadowing path
(D12), including restoration of the embedded source when a shadow is removed.

This is native client presentation (D1, D2). It uses existing world views and
resolved ODF metadata, without adding rules, simulation state, content digest
fields or save/network formats (D9, D10). No original executable, hook or DLL
is loaded. The source rule is read from A2FOAlwaysShowShields' implementation
and documentation (static); this does not establish matching original-game
pixels.

The shell uses the rendered model's scaled bounds, including their centre
offset, with 12 percent padding on each half-axis. Without model bounds it
uses the selection radius or a safe definition-radius fallback. Its quiet
blue rim is an original shader, not Armada's Clairvoyance-link effect. These
are explicit presentation approximations; native silhouette, colour and
effect parity remain unmeasured. No game assets are bundled.

The renderer caches policy per content definition and shares one mesh and
material across outlines. Shield depletion, recharge and hits do not allocate
another material. Child transforms follow the interpolated unit root.

Run the lifecycle and shader checks through the agent build wrapper:

```sh
tools/agent-cargo test --locked -p sta64 --lib render::fx::persistent_shields
```

The ignored synthetic framebuffer test needs a windowless Vulkan adapter:

```sh
tools/agent-cargo test --locked -p sta64 --lib \
  render::fx::persistent_shields::gpu_tests -- --ignored --nocapture
```

It compares positive, depleted and recharged shields around a synthetic hull.
`STA64_GPU_OUT` optionally saves those frames outside the checkout. A software
Vulkan run proves the offscreen rendering path, not hardware performance or
original-game visual parity. Manual comparison remains a separate gate.
