# Claimed bug fixes

This batch follows the seven existing bug reports, without a HUD redesign,
cinematic window, dependency upgrade or modification of original assets.
Source/headless verification and manual visual/audio acceptance are separate.
The December desktop alpha and shared Android/desktop game remain unchanged.

## Contracts before implementation

- #339: use the already merged authoritative playable-race list at both menu
  and launch. Refresh its regression evidence; do not invent a second picker.
- #340: newly fired weapons require the target to be visible to the shooter's
  real team through the existing sensors calculation, in addition to range,
  arcs and cloak checks. Apply this to ordered and automatic fire, recomputing
  after movement each tick. An ally's sensors suffice; previously explored fog
  does not. Losing sight suspends firing without deleting the attack order.
  Already launched projectiles continue. The existing seatless legacy/bench
  API retains its omniscient fallback; an unknown seat in a configured game
  does not gain vision. No new persisted state, format or sensor rule.
- #341: preserve stable HUD value nodes/glyphs for resource/status updates and
  ensure new text reaches UI/visibility preparation in the same frame. A
  selection/layout change can rebuild structure; changing a number should not
  rebuild the entire panel. Verify frame continuity rather than infer a fix
  solely from entity counts.
- #342: verify the existing per-bee Welding phase gate through waiting,
  travelling, welding and return, including textured child visibility where
  practical. Do not globally disable repair or legacy effects.
- #343: at most one active Voice-bus line. Drop conflicting requests immediately,
  never queue them. Actual playback completion admits the next request; missing
  or failed audio must not hold admission. Other buses are independent.
- #344: hide Attack when the resolved ODF explicitly disables `canAttack` or
  the actor has no supported ordinary weapon (point defence/special-only mounts
  do not grant it). This reads presentation capability from resolved content,
  not a raw ODF rule inside the sim; existing weapon mechanics are unchanged.
  Keep the current fallback when the UI flag is absent. Rally belongs to
  stationary producers, not mobile builders. Put supported formations on their
  own page behind a Formations button, retaining the existing shared orders.
  This corrects reported capabilities/menu nesting, not all planned stock UI.
- #356: make the placement preview readable as a translucent green/red
  blueprint, independent of a hull texture's darkness/alpha. Change only
  ghost-owned material copies, including custom hull-material descendants;
  completed models and shared source materials must remain untouched.

Existing SPR/ODF inputs remain read-only (modding D1/D3). Original native
executables/hooks are never loaded or executed (D2). #340 changes a shared
simulation outcome intentionally and requires deterministic/replay/snapshot
regressions and full OS CI. The other fixes do not change save/network formats.

## Evidence

Verified locally on 2026-11-11:

- #339 is closed against the existing merged #348/#347 fix. Menu/launch and
  content eligibility regressions were refreshed, not reimplemented.
- 379 client library tests passed (one existing ignored); 320 simulation tests
  including integrations passed; 174 content tests passed (two install-gated
  ignored); 299 AI/bench/mission consumer tests passed (ten existing ignored).
- The new artillery tests cover ordered/automatic fire, allied spotters, sight
  loss without order loss, range/cloak, unknown seats and seatless fallback.
  An in-flight projectile continues after sight loss, with identical events
  and digests after snapshot resume. Existing replay/save regressions pass.
- HUD tests preserve resource/info descendants while numbers change, preserve
  glyphs through resizing, and verify first-frame inherited visibility. Command
  tests cover authored/inherited Attack flags, utility-only mounts, producer
  rally and formation-page clicks/selection changes.
- Voice tests cover long-clip conflicts, same-frame alerts, no delayed replay,
  missing audio, independent other buses and stop release. Real Kira admission
  follows handle completion, including asynchronous stop fades.
- Ghost tests verify separate material copies, source preservation, both tints
  and conversion from custom hull materials. A real windowless Vulkan test of
  Classic `fyard` on the GTX 1660 Ti captured **8,532 dominant green pixels and
  8,532 dominant red pixels with ambient lighting zero**. It first exercises
  real HullMaterial conversion, then removes that path on ghost descendants;
  original materials remain unchanged. This is actual GPU proof, not a manual
  in-game acceptance result.
- Strict client/simulation all-target Clippy, workspace all-target check and
  NDK29/API26 Android ARM64 client-library compilation pass. No new APK/device
  or original-game execution is claimed. Dependencies and save/wire layouts
  are unchanged. Independent source review found no remaining blocker.

The optional GPU check is `installed_ghost_is_readable_without_lights`, gated
by `STA64_GPU_TESTS=1 STA64_GHOST_TESTS=1 STA64_TEST_INSTALL=...`, optional
`STA64_TEST_MOD`, `STA64_TEST_MODEL` (default `fyard`) and `STA64_GPU_OUT`.
It creates no window or input-device events. Proprietary inputs and captured
images stay outside the repository. Logs are under
`STA64-artifacts/probes/claimed-bugs/`.

The independently merged documentation PR #363 contained only the contract.
The implementation is published separately from current main; earlier branches
are preserved. An optimized desktop candidate and its receipt accompany the
implementation PR. Manual HUD, voice and construction checks are added to
TODO without altering earlier entries. No bug is closed merely by assignment,
and full stock UI/campaign/native runtime parity is not claimed by this batch.
