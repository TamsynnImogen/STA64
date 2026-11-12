# Initial hull and shields

Pre-implementation contract for the user-confirmed Armada II pool rules.
Implements modding decisions D1/D2/D5/D9/D10/D11. Original game data remains
read-only; no executable or hook is loaded.

The content seam gains optional typed starting hull and shield strengths.
Absent starting values mean the corresponding maximum, including synthetic
definitions constructed in Rust. Existing maximum fields keep their meaning.
The shared Entity::from_def path applies these values to map, starting-fleet,
script and production spawns; presentation reads the resulting live pools.

Read maxHealth as maximum hull and curHealth as starting hull. Accept singular
maxShield/curShield as well as the existing plural maxShields/curShields.
When both spellings exist, singular takes precedence for that value. Missing
current strength uses its maximum. Preserve explicit zero, including a zero
shield maximum; presence is checked independently of numeric truthiness.
Retain the existing current-only shield rule: if only shield current is
authored, it also supplies the maximum.

For Sta2Classic and Fo4 lineages, when neither shield current nor maximum is
authored, copy the resolved health pair into shields. shieldRate alone does
not define a strength. A1 and Native retain their existing absent-shield rule;
this change does not assert unverified A1 defaults. Explicit starting values
and both spellings are accepted by the common ODF reader in all profiles.
Resolved ODF inheritance remains authoritative; no new rules file or setting.

Starting values join the canonical content digest when they change effective
initialization. An omitted current and an explicit current equal to maximum
are equivalent. Native synthetic definitions keep their historical hashes.
Live pool representation and snapshot/replay formats do not change: restoring
a save preserves its serialized current values rather than rerunning spawn
defaults. Changed legacy content rules are rejected by existing content-hash
admission; do not weaken those checks or fabricate a migration.

Tests must cover partial currents, singular/plural spellings and precedence,
inherited values and zero overrides, the A2/FO health fallback and Native/A1
isolation, effective/no-op content hashes, shared spawning (including
directional shields), production and save continuation. Run relevant shared
suites, client regression tests, strict Clippy, workspace and ARM64 checks.
Stage an optimized desktop candidate and keep manual HUD/FPS acceptance
separate from headless and compiler evidence. December desktop alpha remains
the priority; this is one shared implementation across platforms.
