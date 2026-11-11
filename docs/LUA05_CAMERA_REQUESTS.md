# LUA-05 exact camera request bridge

This adapter-only slice follows the workplan's presentation-event contract:
requests enter the shared command/replay stream before their client consumers
exist. There is currently no STA64 client consumer for these events. Capturing
and replaying a request is not camera playback or full cinematic compatibility.
No gameplay, rendering, content, protocol or dependency change is proposed here.

## Pre-code contract

Add a dedicated `api/a2/native_camera.rs`, registered against the actual A2 table.
Each successful call queues exactly one SCRIPT `Edit::Present` containing a
`Presentation::Call` named `A2.<method>` and the typed arguments below, in order.
Use checked command submission; validate the entire call before queuing it.

| Exact native shape | Retained arguments |
|---|---|
| `void CenterCamera(int)` | Entity |
| `void SetCameraZoom(float)` | Num |
| `void CineractiveConstantSpeed(bool)` | Bool |
| `void CineractiveCameraOffset(float,float,float,bool)` | Num, Num, Num, Bool |
| `void CineractiveCameraOffset(char*,float,bool)` | Text, Num, Bool |
| `void CineractiveShot(int,int)` | Entity, Entity |
| `void CineractiveShot(char*,int,int)` | Text, Int, Entity |
| `void CineractiveShot(char*,float,int)` | Text, Num, Entity |

The pinned stock-A2 wrapper/helper trace supports argument order, path-point
versus floating path forms, and entity roles. Retain raw numeric values and
flags; do not reinterpret a zoom as distance, a path float as seconds, or an
offset flag as a guessed coordinate-space convention. Original image/map and
static captures remain private; no original code or addresses enter the repo.

Lua 5.3 retains integer versus floating numeric categories. For the ambiguous
three-argument path shot, an Integer selects the point-index overload and a
Number selects the floating overload. Thus `0` is an index and `0.0` retains a
floating parameter. Document and test this explicit bridge selection rule;
never guess an overload by whether a float happens to have a fractional part.
Other float arguments accept integers or numbers, as existing camera adapters do.

Use exact arity. Strings must be UTF-8 without embedded NUL; retain spelling,
case and emptiness. Do not resolve paths or load files in the mission bridge.
Path indices must be nonnegative signed-32-bit integers. Handles use the existing
strict unsigned-u32 helper and require a current non-DYING view entity. Native
missing/DYING/type-guard/sentinel behavior is not generally reproduced by this
strict bridge convention. Floats must be finite and fit existing signed 32.32
presentation storage; accept zero/negative values without invented clamps.
Booleans must be Lua booleans. Invalid calls queue nothing.

Leave the existing MMM Cineractive/Path source cache and fields unchanged.
Do not add a guessed Camera.center alias or report these requests as played.
Script-pointer start/fade operations remain a separate lifecycle prerequisite.
No opaque presentation event is used to substitute for gameplay/AI operations.

## Ownership, integration and checks

Own new native-camera module/tests, one narrow `api/a2/mod.rs` registration,
owning documentation and append-only TODO. Exact inventory/tool/report changes
wait for #328 to merge, so its unmerged perception changes are not consumed or
overwritten. This camera branch starts independently from merged main fbaa8d6.
No existing mission source or game input is modified.

Test every positive signature/payload including integer-versus-float ambiguity;
both Boolean values and numeric zero/negative values; strict arity/types/ranges,
NUL/non-UTF8 strings and missing/DYING entities; no queued partial requests;
SCRIPT/sequence ordering; real Sim event delivery and digest equivalence with
a control run; command/replay payload round trips and actual Recorder playback.
Preserve existing MMM camera tests. Run relevant shared tests, strict Clippy,
workspace and ARM64 checks, plus the existing converted first-objective probe.
Inventory credit means an exact queued presentation request, as the workplan
permits; it does not mean that a renderer has implemented that request.

The original LUA-05 zero-missing gate remains unchanged. The parent stays open
after this partial bridge even when all eight request signatures are tested.

## Integrated inventory follow-up

Camera source #329 merged as `8df38e1`, followed by perception #328 as
`0d979bf`. The deferred exact mappings/report and owning documentation are
therefore completed from that actual integrated main in a separate follow-up.
No unmerged shield or other prerequisite code is consumed. Request capture,
replay and exact-shape inventory proof remain separate from client playback.
