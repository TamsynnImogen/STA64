# Shared shield edits: LUA-05 prerequisite

The stock-A2 setters write per-instance shield fields: SetShieldPercent writes
current shields as maximum times the supplied fraction, and SetMaxShields writes
both maximum and current shields to the supplied value. They do not edit the
ODF/class definition or emit a damage event. Private own-image wrapper captures
support these field operations, including zero and unclamped signed values.

STA64 already stores, hashes and serializes these fields. Add two privileged
edits rather than a new shield system or duplicate mission-side state.

## Contract before implementation

- Append `Edit::ShieldFraction(EntityId, Fixed)` and
  `Edit::MaxShields(EntityId, Fixed)` as tags 22/23 after PerceivedTeam. Preserve
  all existing enum tags and payloads. These are additive commands under the
  current seam-2 evolution policy; readers must understand new variants to
  decode a replay containing them. Do not claim older binaries accept them.
- Only SCRIPT/BENCH may apply either edit. Require an existing non-DYING entity.
  Generic Core admits its represented shield storage without inferring native
  ancestry from actor kind, hull or progress. The later A2 adapter must handle
  the narrower native Craft/construction-proxy guard before calling Core.
- ShieldFraction computes `maximum * fraction` using checked signed 32.32
  multiplication with the same shift/truncation as Fixed multiplication. Reject
  a result outside Fixed storage atomically; never wrap or saturate it. One
  means full. Preserve zero, negative fractions and overcharge above one,
  matching the raw native field write rather than inventing a 0..100 conversion
  or clamp. Do not scale the previous current value.
- MaxShields sets per-instance maximum and fills current shields to that value.
  Preserve representable signed/zero values. Do not change the immutable
  definition, another actor, recharge rate, hull, flags or directionality.
- An active pool is index 0 for a single shield and all four indices for the
  existing directional model. Write each active pool; leave inactive storage
  untouched. Core's directional maximum is already per-facing; keep that rule.
  Legacy profiles default to one pool. No equivalence with A2FOExtensions'
  aggregate-to-asymmetric-facing sidecar modes is claimed or introduced.
- Emit no Hit/ShieldsDown event just for a setter. Normal subsequent damage,
  recharge and Heal consume the mutated instance state through their existing
  rules. Sorted SCRIPT sequences determine multiple edits' application order.

No new gameplay default, resource limit, profile/ODF/CFG/AIP change, mutable table
or view field is needed. Existing entity shield hashing and snapshot-11 records
already include current, maximum and directionality. Keep save formats, Body,
SEAM_REV, content hashes and default Native golden digests unchanged. The existing
narrow historical snapshot paths and revision-1 replay rejection remain intact.
Fixed storage limits and checked-overflow rejection are explicit STA64 numeric
policies, not native floating-point/invalid-input parity claims. Wider native
damage edge cases and extension-specific shield behavior remain separate work.

## Scope and checks

Own a new private shield-edit helper/test module and narrow lib/command/spawn
integration, this contract, a WORKPLAN seam note and append-only TODO. No Lua
adapter consumes this separate prerequisite until merged. No mission/client/
content/network/dependency source or original input changes are part of it.

Required proof: SCRIPT/BENCH versus player rejection; absent/DYING atomicity;
absolute maximum/fill and fraction-of-maximum order; single/directional and
inactive pools; zero/negative/overcharge; checked product overflow boundaries;
other actors/definitions/flags/hull preserved; real damage/recharge/Heal using
new state; all old/new wire tags; command replay, snapshot continuation, current
view and digest changes; unchanged default Native and genuine v8 fixture checks.
Run shared tests, independent review, strict Clippy, workspace/ARM64 checks and
full CI. No APK, device/native-game or full shield-model parity claim follows
from headless tests alone. Desktop/Android use the same implementation and the
December 2026 alpha target is unchanged.
