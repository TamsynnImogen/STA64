# Replay file framing

Pre-code contract for the narrow SIM-22 prerequisite tracked by #305.
The larger [map-confinement integration](MAP_CONFINEMENT_INTEGRATION.md)
remains separate. This change affects reading one replay file, not gameplay.

## Observed behavior

The pre-change STAR version-1 reader passed the body to `postcard::from_bytes`.
In pinned postcard 1.1.3, that API returns the deserialized value while
discarding unused input. The version-specific APIs are documented at
[`from_bytes`](https://docs.rs/postcard/1.1.3/postcard/fn.from_bytes.html) and
[`take_from_bytes`](https://docs.rs/postcard/1.1.3/postcard/fn.take_from_bytes.html).

A frozen, hash-verified pre-change STA64 decoder accepted all three inputs:

| Input | Bytes | Result before the fix |
|---|---:|---|
| Genuine own-authored Native v1 fixture | 486 | Five commands, end tick 600 |
| Same file followed by one zero byte | 487 | Same accepted replay |
| Two copies concatenated | 972 | Same accepted replay |

These are headless STA64 decoder results. They do not involve executing an
original game or modifying game content. The synthetic negative inputs are
copies; the genuine fixture and its original generation receipts are retained.

## Contract

`Replay::decode` now consumes exactly one complete replay body. It uses
`postcard::take_from_bytes::<Replay>` and returns `ReplayError::Corrupt` if
the returned remainder is nonempty. The existing magic, format, seam,
content and command-order checks remain. There is no permitted extension
suffix in STAR version 1.

The writer, STAR version 1, seam revision 1, command tags, valid encoded bytes,
content hashes and simulation digests remain unchanged. No dependency update,
snapshot layout, public type field, networking or gameplay change belongs in
this prerequisite. Callers continue through the same shared `Replay::decode`
entry point on every platform.

Rejecting a previously ignored suffix is the intended behavior change.
This cannot make an already released old decoder strict. In particular, it
does not make future fields safe to append to version-1 replay bodies:
`MapDef`, `MapObject` and commands are positional postcard data. Future map
confinement integration still needs an explicit replay-version/legacy-body
and seam compatibility decision before those fields change.

## Verification gates

- Add the unchanged pre-integration Native replay and its provenance record
  under `crates/sta_sim/tests/fixtures/`. It was generated through the old
  public `Recorder` API, not by rewriting a version byte or using legacy
  game data.
- First demonstrate suffix and concatenation tests fail against the old
  reader, then pass with strict consumption. Include a nonzero suffix and
  preserve existing malformed-header/body rejection coverage.
- Decode and re-encode the genuine fixture byte-for-byte. Replay its five
  commands to tick 600 and continue without commands to tick 900, preserving
  the recorded old digests and Native movement beyond raw map bounds.
- Run scoped replay/snapshot and shared-consumer tests, strict Clippy,
  workspace all-target checking, and Android ARM64 library checking where
  tooling permits. Keep the dependency lock unchanged.

An Android library check is not APK or device proof. No graphics, lifecycle,
input or Waydroid acceptance claim follows from these headless tests. The
December 2026 desktop alpha remains the priority.

## Format 4 (CON-35)

Following the rule above (positional postcard fields change only with a
format bump), CON-35 bumps the replay to STAR format 4 and the snapshot to
STAS format 15 together (after SIM-27's replay format 3 and SIM-28's snapshot format 14). Format 4 adds, after the content hash, the
recording peer's content lineage and pack names and a list of the other
peers' (empty for a local game; NET-02 fills it from `Start`), per
[ADR 0004](adr/0004-modding-v1.md) D10. Format 15 adds the same
lineage and pack names as a record between the fixed header and the body.
Both are informational and never checked.

The content hash in both is now the first eight bytes of the keyed rules
digest (`sta_content::digest`), so no earlier file's hash can match a
current database. Earlier formats are therefore refused by version, with
an error that says the file is from an older STA64, rather than read and
then refused for content; the narrow snapshot compatibility paths for
formats 8 to 14 are removed. The genuine old fixtures stay in
`crates/sta_sim/tests/fixtures/` as refusal cases. The games of
`mining-v8.stas` and `unit-status-v12.stas` are rebuilt in tests, and the
native control game is rerun from the v1 replay's commands; all still reach
their recorded digests with unit crew left out and the old content hash
substituted (`digest_before_crew`). `mining-idle-v9.stas` is only a refusal
case now.
