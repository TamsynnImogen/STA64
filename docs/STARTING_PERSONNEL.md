# Starting crew and officer-limit prerequisite

Pre-code design for UI-18 #371, following the live resource-bar report.
Implements modding decisions D1/D2/D5/D9/D10/D11. Original data stays read-only.

## Observed gaps and bounded scope

Normal/lots faction resource values are distinct; current code reads only four
normal stocks. GUI resource ID 0 is crew, omitted because no player crew state
exists. The officer view is hardcoded 0/0. The current header reader cannot
resolve the MP officer array or arithmetic expressions.

Introduce typed starting-resource modes and officer choices from the mounted
RTS_CFG.h and faction ODFs. Default to the native normal choices; do not use
editor STARTING_CREW for Instant Action. Preserve zero as authored and keep
crew separate from the four mined resources and from crew capacity/planet
contributions. Include resolved numeric rules in the content hash, excluding
cosmetic labels. Existing source-only fake content stays unconfigured.

Shared prerequisite: carry Normal/Lots and officer-choice index through
GameOptions and replay recording; give initialized players an optional crew
stock and officer maximum. Expose crew in PlayerView and use existing officer
fields for the live-entity officer-cost census and configured maximum. Crew is
an actual saved/hashed player value, not a HUD copy of the ODF. Do not pretend
that the existing engine already implements crew regeneration, officer queue
reservations/admission, casualties or per-entity current crew. Those are
separate gameplay work; this prerequisite establishes starting values and
honest current state for the requested resource bar.

The added serialized view/setup surface requires a versioned seam transition
(revision 3), replay format 2 and snapshot format 12. Keep the existing snapshot
Body unchanged, appending a mandatory personnel suffix. Read historical 8--11
snapshots only through explicit version/seam rules, preserving unknown crew
and officer maximum as absent rather than inventing initial values. Reject
old replay/view logs explicitly. Fakes and all current callers must compile.
No dependency upgrade, engine fork or new native binary execution.

After the prerequisite merges, wire the setup choices and resource ID 0 to the
shared values, respecting noCrew/noOfficers and the existing CFG geometry and
colours. Show unsupported Unlimited honestly; its ongoing native semantics are
not equivalent to Lots. New production/crew regeneration policies are outside
this bounded starting-value correction.

## Gates

Synthetic header array/expression/override tests, exact installed officer table
values, distinct normal/lots faction stocks including crew, zero/missing
handling, hashes ignoring cosmetic label edits, deterministic save/replay and
historical reader tests; scoped strict Clippy/workspace/ARM64 checks. A later
optimized HUD candidate needs live selection/resource-mode/FPS acceptance.
December desktop alpha scope and original/accepted artifacts stay preserved.

## Historical identity and replay boundary

For old snapshot formats 8--11 only, admit the previous content identity with
just the newly introduced starting-rules hash omitted; all prior unit/weapon/
tech rules must still match. Restore no personnel for those historical records,
then write current format/hash on the next save. A genuine unchanged v9 fixture
is tested against this migration and unrelated unit changes still reject.
Current-format snapshots and network identities never use the legacy hash.

Replay format 2 persists the selected options. This does not implement recording
a live Instant Action match: the existing Recorder starts a bare map through
Sim::from_setup, whereas explicit faction starts use from_setup_with_starts.
Do not claim that persisting these options resolves that separate pre-existing
starting-force recording limitation. Current shared save/restore does preserve
the actual initialized crew and officer maximum.

## Dependency and ownership note

Only a test-only dependency on the existing workspace sta_vfs crate is added to
sta_sim, so the actual mounted content-to-simulation start path can be checked
headlessly. No new external dependency/version is introduced. Mechanical client
and AI fake literals use crew=None until their source exposes actual state.
The subsequent HUD/setup consumer must start from the merged prerequisite.

## Prerequisite verification

581 shared asset/content/simulation tests and 302 bench/AI/mission tests pass;
440 client tests pass with the mechanical seam updates. Existing ignored tests
remain ignored. Strict seven-crate all-target Clippy, workspace/all-target and
NDK29/API26 ARM64 client-library checks pass. This is not APK/device evidence.

Explicit installed runs resolve Classic officer choices to 600/300/800/999 and
FO4's authored choices to 99999. Classic's actual shared initial view has crew
1200 (Normal) or 30000 (Lots), live officer census 16 and maximum 600. Both
resource modes survive current snapshot round trips. Existing unrelated content
warnings are retained; this is not complete native/mod compatibility acceptance.

Private logs are under `STA64-artifacts/probes/ui17/personnel-*`. This shared
prerequisite alone does not draw crew or expose setup selectors; the UI consumer
follows after merge. No crew spending/regeneration, officer admission/reserved
queue accounting, or per-unit current crew is claimed.
