# A1 start locations (CON-28)

This contract precedes implementation. Add pure format-side helpers to
`sta_bzn::a1`; content conversion and client integration follow separately.
The OpenSourced-Space-RTS audit exposed the A1 map gap, and the existing
A2FOExtensions MDF reader supplied an additional format lead. Check gameplay
placement against the original A1 executable rather than inverting preview pixels.

## Native evidence and limits

The pinned original GOG `Armada.exe` (SHA-256
`1c37ee8e4a24380c950a91ce5a7f9f96c4fcf3d8ffc0cc7e58faec3835350be6`)
has separate MDF preview and new-game placement paths:

- MDF reader `0x4c0a50` reads `StartLocations`, accepting 2..8. Its missing or
  out-of-range count falls back to eight. MDF writer `0x4c0cc0` projects the
  first `Camera_i` point to a 118-unit preview grid. Those integer coordinates
  lose precision; the A2FO host adapter's inverse-117 policy is not an original
  A1 world-position rule.
- New-game caller `0x440cc6` enters `0x442fd0`. For a team needing starting
  forces, `0x4430b5..0x44312e` constructs its exact `Camera_i` label, calls
  `0x4348d0`, and copies the first point's XYZ. Later placement constructs a
  transform and calls the object factory at `0x443284`. Starting units can
  receive additional native placement offsets; this helper resolves the base
  point only, not that loadout algorithm.
- Lookup `0x4348d0` compares complete labels case-sensitively and returns the
  first matching path in retained order. A missing path uses a native fallback
  with centre XZ and mean `SHIP_COMBAT_FLOOR`/`SHIP_COMBAT_CEILING` Y. These are
  configuration values, not the map's vertical bounds. Do not substitute the
  map Y midpoint or silently invent this fallback in the format crate.

Private instruction captures and source fingerprints remain outside the repo.
This is static source/binary evidence, not execution of the original game.

## API and supported subset

Add `mdf_start_count(&[u8]) -> Result<u8, StartError>` and
`player_start_positions(&MapData, u8) -> Result<Vec<[f32; 3]>, StartError>`.
No new dependencies, format fields, defaults, content hashes or serialization.

The count helper recognizes exactly `StartLocations = <unsigned decimal>`
with ASCII whitespace around the key/value, LF/CRLF/CR lines and an optional
final newline. It ignores unrelated lines, including preview `Start1` etc.
Require one complete canonical assignment, reject duplicate assignments,
invalid/overflowing decimal values and counts outside 2..8. Similar key prefixes
must not match. Unrelated bytes need no whole-file UTF-8 conversion.

This is deliberately a checked canonical subset, not a complete reproduction
of the native configuration reader's prefix matching, scanf handling, duplicate
policy, malformed input or default-eight fallback. Errors remain explicit.
It is not an MDF preview-coordinate validator or writer.

The position helper requires count 2..8, then resolves `Camera_1` through
`Camera_n` in numerical order. Use the first exact matching path and its first
point. Preserve all three finite coordinates, including Y, off-map positions,
fractional values and coincident starts. Do not quantize, mirror, clamp, sort
paths, deduplicate starts or skip an empty first duplicate to find a later one.
Missing, empty or nonfinite requested paths reject the whole result; never
return a misleading partial list. The source `MapData` remains unchanged.

Errors distinguish missing/duplicate/invalid/out-of-range count and
missing/empty/nonfinite camera slots. No neutral entry is fabricated. Returned
element `i` is the base point for native team `i + 1`, ready for a subsequent
content adapter to map to existing STA64 slot `i`.

## Integration boundary

Current `MapDef::convert_bzn_in` has no selected-file argument. The client
overwrites the converted map name only afterward, so the BZN's internal
`msn_filename` cannot safely name a renamed mod map's companion. A subsequent
small additive seam must carry the selected VFS basename/path through shared
conversion before loading MDF. Keep campaign discovery, loose/archive/mod
precedence and existing A2 starts intact. Do not change shared core types or
client callers as a shortcut in this package.

These helpers alone do not enable A1 skirmish, camera motion, path-following,
AOI behaviour, multiplayer protocol compatibility or full A1 gameplay.

## Validation plan

Synthetic tests cover the canonical count syntax and refusals, precise XYZ,
case-sensitive/first-match labels, empty/missing/nonfinite paths, identical and
off-map starts, count limits, deterministic ordering and no source mutation.
An explicitly ignored opt-in corpus test reads all 49 installed A1 BZNs and
28 MDF companions, resolves each announced start and verifies a known precise
`2blue` point independently of MDF grid values. Hash original inputs before
and after; never commit game data or private binary excerpts.

Run scoped BZN and relevant consumer tests, strict scoped Clippy, workspace
all-targets checking and an Android ARM64 library check where available. These
checks do not build an APK or establish device/gameplay acceptance. The
December 2026 desktop alpha stays the priority, with no new release dependency
or invented completion date.
