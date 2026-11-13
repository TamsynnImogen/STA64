# Architecture decision records

One file per decision (or per set of decisions signed off together). An
accepted ADR is binding on agent and human work in the area it covers.

| ADR | Decides | Status | Date |
|---|---|---|---|
| [0001](0001-engine-choice.md) | Bevy for the client; simulation, formats and tools independent of it | accepted (foundational) | 2026-11-07 |
| [0002](0002-deterministic-simulation.md) | Deterministic simulation rules | accepted (foundational) | 2026-11-07 |
| [0003](0003-clean-room-content-policy.md) | No game content, converted content or research in the repository or a release | accepted | 2026-11-07, extended 2026-11-13 |
| [0004](0004-modding-v1.md) | Modding v1, D1 to D13 (formerly `docs/MODDING_DECISIONS.md`) | accepted | 2026-11-11, amended 2026-11-12 |
| [0005](0005-network-after-net-01.md) | Network model after NET-01, N1 to N6 (formerly `docs/NET_DECISIONS.md`) | accepted | 2026-11-12 |
| [0006](0006-fo-profile-detection-order.md) | Profile detection order for Fleet Operations installs | accepted | 2026-11-12 |

0007 is taken by the UI text, markup and icon proposal (PR #663).

ADRs 0001 to 0006 were moved here on 2026-11-13 from the documents that held
them. The decision text was moved unchanged; only the form is new. Where the
source recorded no reason, alternative or approver, the ADR says "not
recorded" instead of supplying one.

## Rules

- **Numbers** are four digits, in order of decision date, never reused. A new
  ADR takes the next free number; check open pull requests for one already
  taken.
- **Decision ids inside an ADR** (D1 to D13, N1 to N6, T1 to T8) are stable
  and are what issues, pull requests and code comments cite (`Implements D7`).
- **Status** is one of: proposed, accepted, superseded by ADR NNNN. A proposed
  ADR binds nothing. Who has to approve is stated in the ADR; decisions on
  modding need both Tammy and Cedric.
- **Changing an accepted ADR:** as the ADR itself says. Where it says nothing,
  through a pull request that edits it, states which decision changes and why,
  is approved by the people who accepted it, and leaves a dated amendment note
  under the decision (as D5 and D10 of ADR 0004 do). A decision that is
  replaced is marked "Superseded by ..." and kept.
- **Content rules of the repository apply** (ADR 0003): the rule, the data
  keys and the evidence grade in a line or two; no research write-ups,
  measurement tables or game content.

## Layout

```markdown
# ADR NNNN: <title>

- **Status:** proposed | accepted | superseded by ADR NNNN
- **Date:** YYYY-MM-DD
- **Deciders:** who approves, and the pull request that records it
- **Relates to:** other ADRs, documents, WORKPLAN packages

## Context
## Decision
## Alternatives considered
## Consequences
## Open questions
```
