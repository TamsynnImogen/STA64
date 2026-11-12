# Station build queue limits (SIM-20)

## Status and evidence

The implementation decision was committed before core changes. Issue #267
tracks delivery of the shared profile/ODF capacity resolver and station
admission check. Before this change the shared queue admitted five total jobs
for every profile. The active head remains in the count until completion.

Original limits (read from the executables, static; not original-game runtime
acceptance): Armada 1 admits at most five jobs per ordinary producer, the head
staying in the count until it finishes; stock Armada 2 and Fleet Operations
limit their native linked queues to ten. Private hashes and excerpts are
retained outside the repository.

The owned A2FOExtensions source also describes a separate logical queue of
100 jobs, grouped into ten visible slots and materialized into the ten-node
native queue. That optional extension is not a vanilla FO default, and source
presence does not prove its hooks are installed. This package does not add its
logical sidecar, grouping UI or hook detection.

## Resolved rule

Use profile defaults of five for Sta1Classic, ten for Sta2Classic and Fo4, and
five for Native (retaining STA64's existing native-content policy). An additive
BuildDef.queue_limit: Option<u32> stores an authored override; absence selects
the profile default. Keep numeric policy out of the Boolean Features switches.
A ContentDb accessor resolves the effective limit for a producer.
The old public QUEUE_MAX constant remains as a deprecated five-job fallback
for downstream source compatibility; admission no longer consults it.

All profiles may author the STA64-owned unit ODF key STA64_queueLimit:

```odf
// Total station jobs, including the active head.
STA64_queueLimit = 12
```

The existing ODF include resolution supplies inheritance. A child zero must
override a nonzero parent: zero disables new station queue admissions. It does
not cancel or resize an existing queue. This key is a STA64 extension, not a
newly discovered native key. It accepts unsigned decimal integers in the u32
range (0 through 4294967295); malformed, negative, fractional or overflowing
values warn and retain the profile fallback. Do not clamp to a guessed limit.
Weapons do not interpret the key. Retain Raw ODF data unchanged.

Resolve the cap from the builder, never the product. Reject a full or disabled
queue before payment or mutation. Rig deployment, payment modes, cancellation,
construction placement and capture policy retain their existing contracts.
Only presentation can differ across platforms: Android uses this same rule.
No new permissions, platform dependency or December alpha milestone is needed.

## Persistence and hashes

No command tag, saved queue tuple or snapshot format changes. Include the
effective capacity in the existing content hash when it differs from the
previous global five. This intentionally rejects old A2/FO saves and replays
whose content guard represents the old five-item rule. An authored five can
retain that behavior; a no-op override must hash identically to its effective
fallback. Native fake content and the genuine pre-edit v8 fixture must keep
their existing hashes. Do not claim general cross-build replay compatibility.

## Validation before delivery

Test profile boundaries including an active head, per-builder overrides,
zero and inherited zero, values above ten, invalid/overflow diagnostics,
weapon exclusion and builder-versus-product resolution. Check that rejected
orders do not charge or alter the queue, and that completing one job opens
one slot. Verify content hash changes and no-op stability, snapshot/replay
continuation, the original v8 fixture and affected consumers. Run scoped
strict Clippy and workspace all-targets checks. No GUI or game execution is
part of these headless checks; native runtime and grouped queue UI remain
separate acceptance work.

## Recorded headless validation

After merging main's construction-site and upfront-affordability changes,
the six affected/shared/consumer crates report 934 passing Rust tests, zero
failures, ten intentionally ignored private installed-content probes and one
ignored client documentation example. This includes the genuine v8 mining
fixture continuation, long-queue snapshot continuation/content mismatch,
existing replay checks and the new capacity boundaries. Strict all-targets
Clippy for those crates and the workspace all-targets check pass. Cargo's
workspace lock refresh changes no packages.

The older rig payment test was adapted to issue an affordable deferred order
and spend its funds before arrival, retaining deferred/prepaid arrival
coverage. A constant-only frontend test assertion was moved into a const
block for strict Clippy; runtime UI behavior is unchanged. These integration
adjustments were declared on #267 before editing. Original-game and
Android/Waydroid runtime acceptance remain unverified.
