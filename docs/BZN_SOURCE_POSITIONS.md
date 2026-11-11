# BZN prefix and final positions

Pre-code contract for the format-only prerequisite tracked by #307, under
SIM-22/#287. This does not implement map confinement or choose its lifecycle.

## Source information

An object prefix contains a `pos` point before the later transform matrix.
They can differ. The current A2 binary reader uses the point to construct an
initial matrix and then replaces that matrix when a transform record exists.
The text reader uses `pos` only as a fallback when `position.x` is missing.
Consequently neither A2 path retains a distinct prefix after parsing.

The A1 reader already preserves resized prefix points in
`MapData.initial_positions`, separately from final matrices. Source/native
load-order evidence requires keeping these distinct before selecting any
future initialization policy; equality in stock maps is not a safe substitute.

## Representation and parsing

Add `sta_bzn::MapObject.initial_position: Option<[f32; 3]>`, containing the
prefix point in the same coordinate system as the returned transform. `None`
means unavailable; it must not mean the final matrix or an invented origin.

| Input | New metadata | Existing final placement |
|---|---|---|
| Supported A1 binary | `Some` of the already checked/resized prefix point; exactly agrees with existing A1 parallel array | Unchanged checked/resized matrix |
| A2/FO binary | `Some` of the prefix vector already required to recognize an object | Later matrix, or existing identity/prefix fallback |
| A2 text with complete parsable `pos` XYZ | `Some` independently of `position.*` | Existing transform precedence and fallback |
| A2 text without a complete parsable prefix | `None` | Existing partial-coordinate/default behavior, unchanged |
| Programmatically constructed default object | `None` | Unchanged default transform |

Do not replace A1's existing public parallel array or change its ordering.
Populate both from the same checked point. Keep all counts, object ordering,
headings, transforms, metadata and A1 resize rules unchanged. In particular,
do not rewrite the text transform fallback to require a complete prefix: that
would change tolerated partial maps. Retaining metadata adds no new A2 float
sanitization or claim that every point is suitable for simulation.

## Scope and compatibility

Only the BZN format struct and narrow parser assignments change. Workspace
literal callers already using `Default` remain source-compatible; the fully
specified parser literals must initialize the new field. External fully
specified `sta_bzn::MapObject` literals will also need the new field or
`..Default::default()`.

This BZN type is not serde-encoded by the current workspace. Do not add a
field to `sta_content::MapObject` or pass this metadata into gameplay yet.
Content conversion continues to use the final matrix, so replay/save bodies,
content/simulation hashes and current placement remain unchanged. Future
content integration needs its own explicit versioned replay decision; serde
defaults alone do not make positional postcard layouts compatible.

No dependencies, Bevy upgrade, platform branch, new default or native file
write belongs here. The same format reader serves desktop and Android.

## Validation gates

- Synthetic binary and text objects with deliberately different prefix and
  final positions; no-matrix fallback; missing/unparseable/partial text
  prefixes; explicit zero and multiple-object association.
- A1 ordinary and resized prefixes agree with the retained parallel array,
  including the existing deliberately different prefix/final fixture.
- Existing BZN and content consumer tests, plus a read-only original map
  corpus check of counts, prefix availability and unchanged placement.
- Strict scoped Clippy, workspace all-target checking and Android ARM64
  library checking where practical; unchanged dependency lock and original
  input hashes.

Format retention is not native gameplay or device-runtime proof. The December
2026 desktop alpha remains the priority.
