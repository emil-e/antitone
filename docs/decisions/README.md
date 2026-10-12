# Architectural decisions

Rationale for consequential choices and their tradeoffs. The records below document accepted decisions; current behavior is specified in [the contracts](../README.md#current-contracts).

| Decision | Status |
| --- | --- |
| [Established LibRaw development](established-raw-development.md) | Accepted |
| [Bounded upstream parallel LibRaw](parallel-libraw.md) | Accepted; same development/pixels, pinned local OpenMP and lazy immutable diagnostics |
| [Separate inversion, photography and interchange spaces](colour-space-boundaries.md) | AP0 inversion choice superseded; downstream role separation retained |
| [Native-camera inversion](native-camera-inversion.md) | Accepted; outright replacement, schema/model 4, no AP0-grade migration |
| [Unchanged ACES, matching SDR outputs and managed presentation](sdr-output-and-presentation.md) | Accepted |
| [Output-native scopes and independent magnification](output-native-scopes.md) | Accepted; responsive accepted-frame proxy slice implemented |
| [RGB-only calibration references](rgb-only-calibration.md) | Accepted; replaces original provenance-bearing design |
| [Compact RGB16 GPU source](rgb16-gpu-source.md) | Accepted; replaces float-AP0 upload |
| [Versioned adjustable film response](versioned-film-response.md) | Response mathematics retained; compatibility/version clauses superseded |

## When to add a record

Write an ADR when a choice constrains future changes, has meaningful alternatives/tradeoffs, or is likely to be reopened after context loss. Link contracts and evidence rather than duplicating them. Ordinary local implementation details do not need ADRs.

Use a descriptive filename and this small shape; dates are optional when known, never invented retrospectively:

```markdown
# Decision title
Status: proposed / accepted / superseded (link replacement)

## Context
Problem, constraints and relevant alternatives.

## Decision
Choice and rationale actually supported by the agreement/evidence.

## Consequences
Tradeoffs, limits and conditions for revisiting.

## Evidence
Links to contracts, research, tests or historical approval records.
```

Proposed decisions require explicit acceptance before they describe project policy. Preserve accepted rationale; supersede explicitly if an approved later decision replaces it. Research may explore alternatives without altering any accepted decision.
