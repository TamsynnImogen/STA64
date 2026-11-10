# Resolved legacy station flag (CON-24)

## Status and source decision

Issue #270 tracks this additive content seam. This decision is committed before
core edits; it does not claim a working A2.IsStation binding. The follow-on Lua
query still needs its native object/class-family guard and caller validation.
Keep the December 2026 desktop alpha and shared Android implementation intact.

The pinned original GOG A1, stock A2 and Fleet Operations host references each
initialize CraftClass's is_starbase flag to zero. Their ODF copy readers use
the source class value as fallback, so inheritance and a child's explicit zero
are significant. A1 reads an integer and normalizes nonzero to true; A2/FO use
a Boolean getter. The class identity was checked through A2's named map and
the A1/FO constructor vtables' CraftClass RTTI. Native layouts differ; none
of their offsets are reused as Rust layouts. Private input hashes and excerpts
are retained outside the repository. No binaries, addresses or assembly are
included here, and no original game files were altered or executed.

The named A2 IsStation wrapper checks the object, requires the native Craft
type flag and rejects its dead state, then reads this class flag. Craft's
constructor sets that native type flag. This is not an RTTI Station cast and
is not equivalent to STA64 EntityKind::Station. Stock mobile producers can
author is_starbase=0 even though their class label is a producer/station label.
The owned A1Compat source also preserves is_starbase as an inherited behavioral
marker. Its additional menu/pod policy is not a substitute for the A2 query.

## Shared metadata contract

Add UnitDef.is_starbase: bool, defaultfalse in UnitDef::new. Resolve it from
the unit's owning ODF key is_starbase after the existing include resolution.
Do not infer true from a class label, movement speed, build list or actor kind.
A child zero/false overrides a nonzero parent. Retain Raw ODF data unchanged.
Weapons do not interpret the key. No profile switch, new dependency or platform
branch is required for this immutable metadata.

The initial accepted forms follow the verified native integer-prefix reads:

- Sta1Classic: a signed decimal integer prefix representable in 32 bits;
  zero is false and nonzero is true. Trailing text is ignored, as in the
  verified native integer scan.
- Sta2Classic, Fo4 and Native: those prefixes plus case-insensitive true/false.
  The A2/FO Boolean paths were traced to the imported integer conversion.
  Thus 1.5 means integer one, while 0x1 means integer zero; do not silently
  replace these native rules with Rust whole-string integer parsing.

Whitespace and ODF quoting follow the existing reader. Absent values use false.
Values with no supported numeric prefix or Boolean word, and overflowing
prefixes, warn and use false; native invalid scanf/atoi and overflow behavior
is not claimed as reproduced. In particular A1 Boolean
words are outside the verified integer contract. Do not silently clamp values,
borrow A2 lexical behavior for A1 or treat explicit zero as absence.

Use a dedicated station-flag reader so this improved prefix handling does not
change the existing queueInstantDeduct reader's separately documented bounded
contract, diagnostics, profile gating or precedence. Do not change the generic
ODF reader or unrelated legacy keys.

## Identity and scope boundaries

Hash effective true with a dedicated conditional marker, distinct from the
pending queue-capacity marker. Effective false and absent/no-op values retain
the existing hash path, including Native fake content and the original v8
mining fixture. Content that gains a true resolved flag intentionally has a
new guard; no general old-save migration or cross-build replay compatibility
is implied. No snapshot tuple, command tag or persisted body field changes.

This seam adds metadata and content identity only. It does not change
EntityKind::of, mobile producer movement/construction, resource neutrality,
colony ownership, unit limits, UI or simulation outcomes on its own. A later
Lua binding must use the verified flag together with the native-backed object
guard; mapping the API to the broader actor category is not an accepted
shortcut. The generated API report must not count a binding before it exists.

## Validation before delivery

Use synthetic ODFs to test zero/nonzero, numeric prefixes and profile lexical
forms, parent one
with child zero, invalid/overflow diagnostics and raw preservation. Include
opposing class labels and authored flags to catch accidental kind inference,
weapon exclusion, programmatic field hashing, default/no-op hash stability
and the Native fake hash. Run the genuine v8 fixture and shared/consumer tests,
strict scoped all-targets Clippy and workspace all-targets checks. Coordinate
pending #268 and #271 additions without replacing their queue/resource rules.
No native game, GUI, Waydroid activity or input automation is part of this
headless validation; runtime acceptance remains separate.
