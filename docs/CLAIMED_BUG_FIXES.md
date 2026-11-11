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

Implementation and verification are in progress. Each issue records its own
source and acceptance evidence; none is closed merely because it was claimed.
